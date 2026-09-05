//! Wire output passes for `Wire<T>` C# code generation.
//!
//! Split into focused submodules:
//! - [`WireCodeGen`] — Shared C# code generation logic (type mapping, serialize/deserialize/size emission)
//! - [`wire_type`] — Renders `WireOf*` structs for each `Wire<T>` pattern
//! - [`helper_classes`] — Emits managed classes for nested structs with `WireOnly` fields
//! - [`all`] — Assembles `wire_type` and `helper_classes` results per output file

pub mod all;
pub mod cs_names;
pub mod helper_classes;
pub mod wire_type;

use self::cs_names::{CsLayout, CsNames, CsProjection};
use interoptopus::inventory::{TypeId, Types as RsTypes};
use interoptopus::lang::types::{Array, Layout, Primitive, Struct, TypeKind as RsTypeKind, WireOnly};

/// Generates C# serialization code for the wire format by walking Rust types.
///
/// A shared utility used by the wire output passes. It walks the Rust type graph recursively
/// translating primitives, `WireOnly` types, and user structs into inline C# statements.
pub struct WireCodeGen<'a> {
    pub rs_types: &'a RsTypes,
    /// Wire's identifier view of the resolved C# model. Deliberately not the model itself.
    pub cs: CsNames<'a>,
    /// Whether `T?` yields `Nullable<T>` or a nullable reference. A separate question from
    /// identifiers, with a separate authority; see `Issues.md` 31248473.
    pub layout: CsLayout<'a>,
    /// Whether an enum is emitted as a plain C# `enum`. A third question with a third
    /// authority, kept apart for the reason `cs_names.rs` gives.
    pub projection: CsProjection<'a>,
}

impl WireCodeGen<'_> {
    /// Canonical C# name for a nominal Rust type.
    ///
    /// Absence is a **model-initialization** invariant violation: `id_map` maps every
    /// inventory type, `enum_variants` excludes no enum, and `wire::nested` backfills the
    /// structs `struct_fields` skips. There is deliberately no fallback - deriving a name
    /// here would make wire a second naming authority, which is the defect `4e9a17c3`
    /// describes.
    fn model_type_name(&self, ty_id: TypeId, rust_name: &str) -> String {
        let Some(name) = self.cs.mapped_type_name(ty_id) else {
            panic!("wire codegen: model-initialization invariant violated - no resolved C# type for Rust type `{rust_name}` ({ty_id})");
        };
        name.to_string()
    }

    /// Allocated stem for one variant, identified by its tag.
    ///
    /// Absence is a **model/output synchronization** invariant violation - a different
    /// failure from the one above, and worth telling apart when debugging: the model was
    /// built, but `union_names` had not resolved this variant. Falling back to
    /// `variant.name` would silently reinstate the bug this accessor exists to prevent.
    fn variant_stem(&self, ty_id: TypeId, tag: isize) -> &str {
        let Some(stem) = self.cs.variant_stem(ty_id, tag) else {
            panic!("wire codegen: model/output synchronization invariant violated - no resolved stem for variant tag {tag} of Rust enum {ty_id}");
        };
        stem
    }

    /// Single-field accessors return the payload; multi-field accessors return a case.
    fn variant_payload_value(&self, ty_id: TypeId, tag: isize, val: &str, index: usize, count: usize) -> String {
        let accessor = format!("{val}.As{}()", self.variant_stem(ty_id, tag));
        if count == 1 {
            accessor
        } else {
            let field = self.cs.variant_case_field(ty_id, tag, index).expect("wire codegen requires the allocated case field");
            format!("{accessor}.{field}")
        }
    }

    /// Maps a Rust type to its C# managed type name.
    #[must_use]
    pub fn cs_type_name(&self, ty_id: TypeId) -> String {
        let Some(ty) = self.rs_types.get(&ty_id) else {
            return "object".to_string();
        };
        match &ty.kind {
            RsTypeKind::Primitive(p) => cs_primitive_name(*p).to_string(),
            RsTypeKind::WireOnly(WireOnly::String) => "string".to_string(),
            RsTypeKind::WireOnly(WireOnly::Vec(inner)) => {
                format!("List<{}>", self.cs_type_name(*inner))
            }
            RsTypeKind::WireOnly(WireOnly::Map(k, v)) => {
                format!("Dictionary<{}, {}>", self.cs_type_name(*k), self.cs_type_name(*v))
            }
            RsTypeKind::WireOnly(WireOnly::Option(inner)) => {
                let inner_name = self.cs_type_name(*inner);
                format!("{inner_name}?")
            }
            RsTypeKind::Struct(_) => self.model_type_name(ty_id, &ty.name),
            RsTypeKind::Enum(_) | RsTypeKind::TypePattern(interoptopus::lang::types::TypePattern::Utf8String) => self.model_type_name(ty_id, &ty.name),
            RsTypeKind::Array(arr) => format!("{}[]", self.cs_type_name(arr.ty)),
            RsTypeKind::TypePattern(interoptopus::lang::types::TypePattern::Option(inner)) => {
                let inner_name = self.cs_type_name(*inner);
                format!("{inner_name}?")
            }
            _ => "object".to_string(),
        }
    }

    /// Generates the serialize method body for a struct.
    #[must_use]
    pub fn serialize_struct_body(&self, s: &Struct, val: &str) -> String {
        let mut lines = Vec::new();
        for f in &s.fields {
            let access = format!("{val}.{}", f.name);
            self.emit_serialize(&mut lines, f.ty, &access, 0, 0);
        }
        lines.join("\n")
    }

    /// Generates the deserialize method body for a struct.
    #[must_use]
    pub fn deserialize_struct_body(&self, s: &Struct, type_name: &str) -> String {
        let mut lines = Vec::new();
        lines.push(format!("var result = ({type_name})System.Runtime.CompilerServices.RuntimeHelpers.GetUninitializedObject(typeof({type_name}));"));
        for f in &s.fields {
            let target = format!("result.{}", f.name);
            self.emit_deserialize(&mut lines, f.ty, &target, 0, 0);
        }
        lines.push("return result;".to_string());
        let owns_strings = s.fields.iter().any(|field| contains_owned_wire(field.ty, self.rs_types, &mut std::collections::HashSet::new()));
        finish_deserialize_body(&lines, owns_strings)
    }

    /// Generates a complete deserializer, retaining native allocations until the value is complete.
    #[must_use]
    pub fn deserialize_type_body(&self, ty_id: TypeId) -> String {
        let name = self.cs_type_name(ty_id);
        let mut lines = vec![format!("{name} result = default;")];
        self.emit_deserialize(&mut lines, ty_id, "result", 0, 0);
        lines.push("return result;".to_string());
        let owns_strings = contains_owned_wire(ty_id, self.rs_types, &mut std::collections::HashSet::new());
        finish_deserialize_body(&lines, owns_strings)
    }

    /// Generates the size calculation body for a struct.
    #[must_use]
    pub fn size_struct_body(&self, s: &Struct, val: &str) -> String {
        let mut lines = Vec::new();
        lines.push("var _size = 0;".to_string());
        for f in &s.fields {
            let access = format!("{val}.{}", f.name);
            self.emit_size(&mut lines, f.ty, &access, 0, 0);
        }
        lines.push("return _size;".to_string());
        lines.join("\n")
    }

    /// Emits C# statements to serialize a value of the given Rust type.
    pub fn emit_serialize(&self, lines: &mut Vec<String>, ty_id: TypeId, val: &str, depth: usize, indent: usize) {
        let Some(ty) = self.rs_types.get(&ty_id) else { return };
        let p = pad(indent);
        match &ty.kind {
            RsTypeKind::Primitive(prim) => {
                if *prim == Primitive::Bool {
                    lines.push(format!("{p}writer.Write({val} ? (byte)1 : (byte)0);"));
                } else {
                    lines.push(format!("{p}writer.Write({val});"));
                }
            }
            RsTypeKind::WireOnly(WireOnly::String) => {
                lines.push(format!("{p}{{ var _bytes = System.Text.Encoding.UTF8.GetBytes({val} ?? \"\"); writer.Write((uint)_bytes.Length); writer.Write(_bytes); }}"));
            }
            RsTypeKind::TypePattern(interoptopus::lang::types::TypePattern::Utf8String) => {
                lines.push(format!("{p}{{ var _bytes = System.Text.Encoding.UTF8.GetBytes({val}.String); writer.Write((uint)_bytes.Length); writer.Write(_bytes); }}"));
            }
            RsTypeKind::WireOnly(WireOnly::Vec(inner_id)) => {
                let iter = format!("_item{depth}");
                let pi = pad(indent + 1);
                lines.push(format!("{p}writer.Write((uint)({val}?.Count ?? 0));"));
                lines.push(format!("{p}if ({val} != null)"));
                lines.push(format!("{p}{{"));
                lines.push(format!("{pi}foreach (var {iter} in {val})"));
                lines.push(format!("{pi}{{"));
                self.emit_serialize(lines, *inner_id, &iter, depth + 1, indent + 2);
                lines.push(format!("{pi}}}"));
                lines.push(format!("{p}}}"));
            }
            RsTypeKind::WireOnly(WireOnly::Map(k_id, v_id)) => {
                let kv = format!("_kv{depth}");
                let pi = pad(indent + 1);
                lines.push(format!("{p}writer.Write((uint)({val}?.Count ?? 0));"));
                lines.push(format!("{p}if ({val} != null)"));
                lines.push(format!("{p}{{"));
                lines.push(format!("{pi}foreach (var {kv} in {val})"));
                lines.push(format!("{pi}{{"));
                self.emit_serialize(lines, *k_id, &format!("{kv}.Key"), depth + 1, indent + 2);
                self.emit_serialize(lines, *v_id, &format!("{kv}.Value"), depth + 1, indent + 2);
                lines.push(format!("{pi}}}"));
                lines.push(format!("{p}}}"));
            }
            RsTypeKind::WireOnly(WireOnly::Option(inner_id)) => {
                self.emit_option_serialize(lines, *inner_id, val, depth, indent);
            }
            RsTypeKind::Array(arr) => {
                let idx = format!("_i{depth}");
                let pi = pad(indent + 1);
                lines.push(format!("{p}for (int {idx} = 0; {idx} < {len}; {idx}++)", len = arr.len));
                lines.push(format!("{p}{{"));
                self.emit_serialize(lines, arr.ty, &format!("{val}[{idx}]"), depth + 1, indent + 1);
                lines.push(format!("{p}}}"));
            }
            RsTypeKind::Enum(e) => {
                self.emit_enum_serialize(lines, ty_id, e, val, depth, indent);
            }
            RsTypeKind::Struct(s) => {
                for f in &s.fields {
                    self.emit_serialize(lines, f.ty, &format!("{val}.{}", f.name), depth, indent);
                }
            }
            RsTypeKind::TypePattern(interoptopus::lang::types::TypePattern::Option(inner_id)) => {
                self.emit_option_serialize(lines, *inner_id, val, depth, indent);
            }
            _ => {
                lines.push(format!("{p}/* unsupported wire type for {val} */"));
            }
        }
    }

    /// Emits C# statements to deserialize a value and assign it to `target`.
    pub fn emit_deserialize(&self, lines: &mut Vec<String>, ty_id: TypeId, target: &str, depth: usize, indent: usize) {
        let Some(ty) = self.rs_types.get(&ty_id) else { return };
        let p = pad(indent);
        match &ty.kind {
            RsTypeKind::Primitive(prim) => {
                lines.push(format!("{p}{target} = {};", cs_read_primitive(*prim)));
            }
            RsTypeKind::WireOnly(WireOnly::String) => {
                lines.push(format!(
                    "{p}{{ var _len = reader.ReadUInt32(); {target} = _len > 0 ? System.Text.Encoding.UTF8.GetString(reader.ReadBytes((int)_len)) : \"\"; }}"
                ));
            }
            RsTypeKind::TypePattern(interoptopus::lang::types::TypePattern::Utf8String) => {
                let name = self.model_type_name(ty_id, &ty.name);
                lines.push(format!(
                    "{p}{{ var _len = reader.ReadUInt32(); var _bytes = reader.ReadBytes(checked((int)_len)); if (_bytes.Length != _len) throw new System.IO.EndOfStreamException(); {target} = {name}.From(new System.Text.UTF8Encoding(false, true).GetString(_bytes)); _wireOwned.Add({target}); }}"
                ));
            }
            RsTypeKind::WireOnly(WireOnly::Vec(inner_id)) => {
                let cs_inner = self.cs_type_name(*inner_id);
                let count = format!("_count{depth}");
                let idx = format!("_i{depth}");
                let elem_var = format!("_elem{depth}");
                let pi = pad(indent + 1);
                lines.push(format!("{p}{{"));
                lines.push(format!("{pi}var {count} = reader.ReadUInt32();"));
                lines.push(format!("{pi}{target} = new List<{cs_inner}>((int){count});"));
                lines.push(format!("{pi}for (uint {idx} = 0; {idx} < {count}; {idx}++)"));
                lines.push(format!("{pi}{{"));
                lines.push(format!("{pi}    {cs_inner} {elem_var} = default;"));
                self.emit_deserialize(lines, *inner_id, &elem_var, depth + 1, indent + 2);
                lines.push(format!("{pi}    {target}.Add({elem_var});"));
                lines.push(format!("{pi}}}"));
                lines.push(format!("{p}}}"));
            }
            RsTypeKind::WireOnly(WireOnly::Map(k_id, v_id)) => {
                let cs_k = self.cs_type_name(*k_id);
                let cs_v = self.cs_type_name(*v_id);
                let count = format!("_count{depth}");
                let idx = format!("_i{depth}");
                let k_var = format!("_key{depth}");
                let v_var = format!("_val{depth}");
                let pi = pad(indent + 1);
                let pi2 = pad(indent + 2);
                lines.push(format!("{p}{{"));
                lines.push(format!("{pi}var {count} = reader.ReadUInt32();"));
                lines.push(format!("{pi}{target} = new Dictionary<{cs_k}, {cs_v}>((int){count});"));
                lines.push(format!("{pi}for (uint {idx} = 0; {idx} < {count}; {idx}++)"));
                lines.push(format!("{pi}{{"));
                lines.push(format!("{pi2}{cs_k} {k_var} = default;"));
                self.emit_deserialize(lines, *k_id, &k_var, depth + 1, indent + 2);
                lines.push(format!("{pi2}{cs_v} {v_var} = default;"));
                self.emit_deserialize(lines, *v_id, &v_var, depth + 1, indent + 2);
                lines.push(format!("{pi2}{target}[{k_var}] = {v_var};"));
                lines.push(format!("{pi}}}"));
                lines.push(format!("{p}}}"));
            }
            RsTypeKind::WireOnly(WireOnly::Option(inner_id)) => {
                self.emit_option_deserialize(lines, *inner_id, target, depth, indent);
            }
            RsTypeKind::Array(arr) => {
                let cs_elem = self.cs_type_name(arr.ty);
                let idx = format!("_i{depth}");
                let pi = pad(indent + 1);
                lines.push(format!("{p}{target} = new {cs_elem}[{len}];", len = arr.len));
                lines.push(format!("{p}for (int {idx} = 0; {idx} < {len}; {idx}++)", len = arr.len));
                lines.push(format!("{p}{{"));
                self.emit_deserialize(lines, arr.ty, &format!("{target}[{idx}]"), depth + 1, indent + 1);
                lines.push(format!("{p}}}"));
            }
            RsTypeKind::Enum(e) => {
                self.emit_enum_deserialize(lines, ty_id, e, target, depth, indent);
            }
            RsTypeKind::Struct(s) => {
                let struct_name = self.cs_type_name(ty_id);
                lines.push(format!("{p}{target} = ({struct_name})System.Runtime.CompilerServices.RuntimeHelpers.GetUninitializedObject(typeof({struct_name}));"));
                for f in &s.fields {
                    self.emit_deserialize(lines, f.ty, &format!("{target}.{}", f.name), depth, indent);
                }
            }
            RsTypeKind::TypePattern(interoptopus::lang::types::TypePattern::Option(inner_id)) => {
                self.emit_option_deserialize(lines, *inner_id, target, depth, indent);
            }
            _ => {
                lines.push(format!("{p}/* unsupported wire type for {target} */"));
            }
        }
    }

    /// Emits C# statements that add the wire size of `val` to `_size`.
    pub fn emit_size(&self, lines: &mut Vec<String>, ty_id: TypeId, val: &str, depth: usize, indent: usize) {
        let Some(ty) = self.rs_types.get(&ty_id) else { return };
        let p = pad(indent);
        match &ty.kind {
            RsTypeKind::Primitive(prim) => {
                lines.push(format!("{p}_size += {};", cs_primitive_size(*prim)));
            }
            RsTypeKind::WireOnly(WireOnly::String) => {
                lines.push(format!("{p}_size += 4 + System.Text.Encoding.UTF8.GetByteCount({val} ?? \"\");"));
            }
            RsTypeKind::TypePattern(interoptopus::lang::types::TypePattern::Utf8String) => {
                lines.push(format!("{p}_size += 4 + System.Text.Encoding.UTF8.GetByteCount({val}.String);"));
            }
            RsTypeKind::WireOnly(WireOnly::Vec(inner_id)) => {
                let iter = format!("_item{depth}");
                let pi = pad(indent + 1);
                lines.push(format!("{p}_size += 4;"));
                lines.push(format!("{p}if ({val} != null)"));
                lines.push(format!("{p}{{"));
                lines.push(format!("{pi}foreach (var {iter} in {val})"));
                lines.push(format!("{pi}{{"));
                self.emit_size(lines, *inner_id, &iter, depth + 1, indent + 2);
                lines.push(format!("{pi}}}"));
                lines.push(format!("{p}}}"));
            }
            RsTypeKind::WireOnly(WireOnly::Map(k_id, v_id)) => {
                let kv = format!("_kv{depth}");
                let pi = pad(indent + 1);
                lines.push(format!("{p}_size += 4;"));
                lines.push(format!("{p}if ({val} != null)"));
                lines.push(format!("{p}{{"));
                lines.push(format!("{pi}foreach (var {kv} in {val})"));
                lines.push(format!("{pi}{{"));
                self.emit_size(lines, *k_id, &format!("{kv}.Key"), depth + 1, indent + 2);
                self.emit_size(lines, *v_id, &format!("{kv}.Value"), depth + 1, indent + 2);
                lines.push(format!("{pi}}}"));
                lines.push(format!("{p}}}"));
            }
            RsTypeKind::WireOnly(WireOnly::Option(inner_id)) => {
                self.emit_option_size(lines, *inner_id, val, depth, indent);
            }
            RsTypeKind::Array(arr) => {
                let idx = format!("_i{depth}");
                let pi = pad(indent + 1);
                lines.push(format!("{p}for (int {idx} = 0; {idx} < {len}; {idx}++)", len = arr.len));
                lines.push(format!("{p}{{"));
                self.emit_size(lines, arr.ty, &format!("{val}[{idx}]"), depth + 1, indent + 1);
                lines.push(format!("{p}}}"));
            }
            RsTypeKind::Enum(e) => {
                self.emit_enum_size(lines, ty_id, e, val, depth, indent);
            }
            RsTypeKind::Struct(s) => {
                for f in &s.fields {
                    self.emit_size(lines, f.ty, &format!("{val}.{}", f.name), depth, indent);
                }
            }
            RsTypeKind::TypePattern(interoptopus::lang::types::TypePattern::Option(inner_id)) => {
                self.emit_option_size(lines, *inner_id, val, depth, indent);
            }
            _ => {}
        }
    }

    /// Wire-serialize an enum by branching on each variant (`IsX`), writing the
    /// discriminant, then serializing the variant's payload (if any).
    fn emit_enum_serialize(&self, lines: &mut Vec<String>, ty_id: TypeId, e: &interoptopus::lang::types::Enum, val: &str, depth: usize, indent: usize) {
        let prim = enum_repr_primitive(e);
        let prim_cs = cs_primitive_name(prim);
        let p = pad(indent);
        let pi = pad(indent + 1);

        // A plain C# `enum` has no `Is{Stem}` accessors - those belong to the union and
        // discriminant-struct projections. Its underlying value *is* the tag, so the whole
        // chain collapses to one write. Emitting the chain here is what produced CS1061 on
        // `MyEnum` once step three started projecting unit-only enums as plain enums.
        if self.projection.is_plain_enum(ty_id) {
            lines.push(format!("{p}writer.Write(({prim_cs}){val});"));
            return;
        }

        for (index, variant) in e.variants.iter().enumerate() {
            let kw = if index == 0 { "if" } else { "else if" };
            let tag = variant.tag;
            lines.push(format!("{p}{kw} ({val}.Is{name})", name = self.variant_stem(ty_id, variant.tag)));
            lines.push(format!("{p}{{"));
            lines.push(format!("{pi}writer.Write(({prim_cs}){tag});"));
            for (index, payload) in variant.payloads().enumerate() {
                let payload_val = self.variant_payload_value(ty_id, variant.tag, val, index, variant.payloads().count());
                self.emit_serialize(lines, payload.ty, &payload_val, depth + 1, indent + 1);
            }
            lines.push(format!("{p}}}"));
        }
        if !e.variants.is_empty() {
            lines.push(format!("{p}else {{ throw {val}.ExceptionForVariant(); }}"));
        }
    }

    /// Wire-deserialize an enum by reading the discriminant and constructing the
    /// matching variant via the public `EnumName.VariantName(...)` factory.
    fn emit_enum_deserialize(&self, lines: &mut Vec<String>, ty_id: TypeId, e: &interoptopus::lang::types::Enum, target: &str, depth: usize, indent: usize) {
        let prim = enum_repr_primitive(e);
        let read_expr = cs_read_primitive(prim);
        let enum_name = self.cs_type_name(ty_id);
        let p = pad(indent);
        let pi = pad(indent + 1);
        let pi2 = pad(indent + 2);
        let tag_var = format!("_tag{depth}");

        lines.push(format!("{p}{{"));
        lines.push(format!("{pi}var {tag_var} = {read_expr};"));

        for (index, variant) in e.variants.iter().enumerate() {
            let kw = if index == 0 { "if" } else { "else if" };
            let tag = variant.tag;
            lines.push(format!("{pi}{kw} ({tag_var} == ({prim_cs}){tag})", prim_cs = cs_primitive_name(prim)));
            lines.push(format!("{pi}{{"));
            let mut payload_vars = Vec::new();
            for (index, payload) in variant.payloads().enumerate() {
                let payload_cs = self.cs_type_name(payload.ty);
                let payload_var = if index == 0 { format!("_p{depth}") } else { format!("_p{depth}_{index}") };
                lines.push(format!("{pi2}{payload_cs} {payload_var} = default;"));
                self.emit_deserialize(lines, payload.ty, &payload_var, depth + 1, indent + 2);
                payload_vars.push(payload_var);
            }
            if payload_vars.is_empty() {
                lines.push(format!("{pi2}{target} = {enum_name}.{};", self.variant_stem(ty_id, variant.tag)));
            } else {
                let arguments = payload_vars.join(", ");
                lines.push(format!("{pi2}{target} = {enum_name}.{}({arguments});", self.variant_stem(ty_id, variant.tag)));
            }
            lines.push(format!("{pi}}}"));
        }
        if !e.variants.is_empty() {
            // `InteropException`, not `InvalidOperationException`: this is a tag arriving from
            // the native side that matches no variant, which `docs/csharp-unions.md` §Exceptions
            // assigns to "unknown native discriminant" — corruption crossing the FFI boundary,
            // meaning *severe error, should never happen*. The serializer's `else` above instead
            // delegates to `ExceptionForVariant()`: an unmatched managed value is either an empty
            // struct-backed union (`InvalidOperationException`) or an illegal managed state
            // (`InteropException`). The old backend threw `InteropException` here too, so this
            // restores the native-input classification rather than inventing it.
            lines.push(format!("{pi}else {{ throw new InteropException(\"Unknown variant tag\"); }}"));
        }
        lines.push(format!("{p}}}"));
    }

    /// Wire size of an enum: discriminant size plus payload size for the active variant.
    fn emit_enum_size(&self, lines: &mut Vec<String>, ty_id: TypeId, e: &interoptopus::lang::types::Enum, val: &str, depth: usize, indent: usize) {
        let prim = enum_repr_primitive(e);
        let p = pad(indent);

        lines.push(format!("{p}_size += {};", cs_primitive_size(prim)));

        for variant in &e.variants {
            let payloads: Vec<_> = variant.payloads().collect();
            if payloads.is_empty() {
                continue;
            }
            lines.push(format!("{p}if ({val}.Is{name})", name = self.variant_stem(ty_id, variant.tag)));
            lines.push(format!("{p}{{"));
            for (index, payload) in payloads.into_iter().enumerate() {
                let payload_val = self.variant_payload_value(ty_id, variant.tag, val, index, variant.payloads().count());
                self.emit_size(lines, payload.ty, &payload_val, depth + 1, indent + 1);
            }
            lines.push(format!("{p}}}"));
        }
    }

    fn emit_option_serialize(&self, lines: &mut Vec<String>, inner_id: TypeId, val: &str, depth: usize, indent: usize) {
        let p = pad(indent);
        if is_cs_value_type(inner_id, self.rs_types, &self.layout) {
            lines.push(format!("{p}writer.Write((byte)({val}.HasValue ? 1 : 0));"));
            lines.push(format!("{p}if ({val}.HasValue)"));
            lines.push(format!("{p}{{"));
            self.emit_serialize(lines, inner_id, &format!("{val}.Value"), depth, indent + 1);
        } else {
            // `!= null`, deliberately, and NOT interchangeable with `is null` once unions land.
            // Pattern matching on a union unwraps to `Value`, so for a class-backed union
            // `x is null` succeeds when the reference is null *or* `Value` is null. The `!=`
            // operator tests only the reference, which is what `Option` means here: a present
            // but empty union is `Some`, not `None`. Rewriting this to `is not null` would
            // silently conflate the two. See `docs/csharp-unions.md` §Null matching.
            lines.push(format!("{p}writer.Write((byte)({val} != null ? 1 : 0));"));
            lines.push(format!("{p}if ({val} != null)"));
            lines.push(format!("{p}{{"));
            self.emit_serialize(lines, inner_id, val, depth, indent + 1);
        }
        lines.push(format!("{p}}}"));
    }

    fn emit_option_deserialize(&self, lines: &mut Vec<String>, inner_id: TypeId, target: &str, depth: usize, indent: usize) {
        let p = pad(indent);
        let pi = pad(indent + 1);
        let pi2 = pad(indent + 2);
        let has_var = format!("_has{depth}");
        lines.push(format!("{p}{{"));
        lines.push(format!("{pi}var {has_var} = reader.ReadByte() != 0;"));
        lines.push(format!("{pi}if ({has_var})"));
        lines.push(format!("{pi}{{"));
        if is_cs_value_type(inner_id, self.rs_types, &self.layout) {
            let cs_inner = self.cs_type_name(inner_id);
            let tmp_var = format!("_optVal{depth}");
            lines.push(format!("{pi2}{cs_inner} {tmp_var} = default;"));
            self.emit_deserialize(lines, inner_id, &tmp_var, depth + 1, indent + 2);
            lines.push(format!("{pi2}{target} = {tmp_var};"));
        } else {
            self.emit_deserialize(lines, inner_id, target, depth + 1, indent + 2);
        }
        lines.push(format!("{pi}}}"));
        lines.push(format!("{pi}else"));
        lines.push(format!("{pi}{{"));
        lines.push(format!("{pi2}{target} = null;"));
        lines.push(format!("{pi}}}"));
        lines.push(format!("{p}}}"));
    }

    fn emit_option_size(&self, lines: &mut Vec<String>, inner_id: TypeId, val: &str, depth: usize, indent: usize) {
        let p = pad(indent);
        lines.push(format!("{p}_size += 1;"));
        if is_cs_value_type(inner_id, self.rs_types, &self.layout) {
            lines.push(format!("{p}if ({val}.HasValue)"));
            lines.push(format!("{p}{{"));
            self.emit_size(lines, inner_id, &format!("{val}.Value"), depth, indent + 1);
        } else {
            lines.push(format!("{p}if ({val} != null)"));
            lines.push(format!("{p}{{"));
            self.emit_size(lines, inner_id, val, depth, indent + 1);
        }
        lines.push(format!("{p}}}"));
    }
}

/// The Rust discriminant primitive for an enum's declared layout.
///
/// `#[ffi]` always emits `Layout::Primitive`, so the fallback is reached only by
/// hand-written inventory entries - the `enum_union_members` fixture is one. It
/// answers `I32` because that is what a `#[repr(C)]` enum discriminant is in both
/// Rust and C.
///
/// Delegates to [`Repr::discriminant_primitive`], the single place this is
/// decided. Before it existed this pass and the model pass each carried their
/// own fallback and disagreed: `uint` here, `int` there.
fn enum_repr_primitive(e: &interoptopus::lang::types::Enum) -> Primitive {
    e.repr.discriminant_primitive()
}

fn pad(indent: usize) -> String {
    "    ".repeat(indent)
}

fn cs_primitive_name(p: Primitive) -> &'static str {
    match p {
        Primitive::Void => "void",
        Primitive::Bool => "bool",
        Primitive::U8 => "byte",
        Primitive::U16 => "ushort",
        Primitive::U32 => "uint",
        Primitive::U64 => "ulong",
        Primitive::I8 => "sbyte",
        Primitive::I16 => "short",
        Primitive::I32 => "int",
        Primitive::I64 => "long",
        Primitive::F32 => "float",
        Primitive::F64 => "double",
        Primitive::Usize | Primitive::Isize => "long",
    }
}

fn cs_read_primitive(p: Primitive) -> &'static str {
    match p {
        Primitive::Bool => "reader.ReadByte() != 0",
        Primitive::U8 => "reader.ReadByte()",
        Primitive::U16 => "reader.ReadUInt16()",
        Primitive::U32 => "reader.ReadUInt32()",
        Primitive::U64 => "reader.ReadUInt64()",
        Primitive::I8 => "reader.ReadSByte()",
        Primitive::I16 => "reader.ReadInt16()",
        Primitive::I32 => "reader.ReadInt32()",
        Primitive::I64 => "reader.ReadInt64()",
        Primitive::F32 => "reader.ReadSingle()",
        Primitive::F64 => "reader.ReadDouble()",
        Primitive::Usize | Primitive::Isize => "reader.ReadInt64()",
        Primitive::Void => "default",
    }
}

fn cs_primitive_size(p: Primitive) -> &'static str {
    match p {
        Primitive::Void => "0",
        Primitive::Bool | Primitive::U8 | Primitive::I8 => "1",
        Primitive::U16 | Primitive::I16 => "2",
        Primitive::U32 | Primitive::I32 | Primitive::F32 => "4",
        Primitive::U64 | Primitive::I64 | Primitive::F64 | Primitive::Usize | Primitive::Isize => "8",
    }
}

/// Returns `true` if the Rust type maps to a C# value type (struct/primitive/enum)
/// rather than a reference type (class, string, List, Dictionary).
/// Structs with `WireOnly` fields are emitted as C# classes, so they are reference types.
fn is_cs_value_type(ty_id: TypeId, rs_types: &RsTypes, layout: &CsLayout<'_>) -> bool {
    let Some(ty) = rs_types.get(&ty_id) else { return false };
    match &ty.kind {
        // Decided here, not by the model: `struct_class` registers only types that reach
        // `types::all` with a managed conversion, and answers `false` for everything else.
        // Delegating a primitive would report "reference type" and turn `Option<u32>` into a
        // null check on a `uint?`.
        RsTypeKind::Primitive(_) => true,
        // C# arrays are reference types. This arm said `true` until `Issues.md` 31248473.
        RsTypeKind::Array(_) => false,
        // `string`, `List<T>`, `Dictionary<K, V>` — all reference types.
        RsTypeKind::WireOnly(_) => false,
        // Nominal types: the model decides, and six other output passes already ask it.
        // Wire re-deriving this is what 31248473 records; a `DataEnum` with a `WireOnly`
        // payload is emitted as a class, and the old `Enum(_) => true` made wire emit
        // `.HasValue`/`.Value` on it.
        RsTypeKind::Struct(_) | RsTypeKind::Enum(_) => layout.is_value_type(ty_id).unwrap_or(false),
        _ => false,
    }
}


fn finish_deserialize_body(lines: &[String], owns_strings: bool) -> String {
    if !owns_strings {
        return lines.join("\n");
    }
    // Each newly allocated leaf is tracked once, including leaves nested in collections or cases.
    // On success the returned value owns them; a later read failure must release all earlier leaves.
    let body = lines.iter().map(|line| format!("    {line}")).collect::<Vec<_>>().join("\n");
    format!("var _wireOwned = new List<IDisposable>();\ntry\n{{\n{body}\n}}\ncatch\n{{\n    foreach (var owned in _wireOwned) owned.Dispose();\n    throw;\n}}")
}

fn contains_owned_wire(ty_id: TypeId, types: &RsTypes, visited: &mut std::collections::HashSet<TypeId>) -> bool {
    if !visited.insert(ty_id) {
        return false;
    }
    let Some(ty) = types.get(&ty_id) else { return false };
    match &ty.kind {
        RsTypeKind::TypePattern(interoptopus::lang::types::TypePattern::Utf8String) => true,
        RsTypeKind::Struct(s) => s.fields.iter().any(|field| contains_owned_wire(field.ty, types, visited)),
        RsTypeKind::Enum(e) => e.variants.iter().any(|variant| variant.payloads().any(|field| contains_owned_wire(field.ty, types, visited))),
        RsTypeKind::Array(array) => contains_owned_wire(array.ty, types, visited),
        RsTypeKind::WireOnly(WireOnly::Vec(inner) | WireOnly::Option(inner)) | RsTypeKind::TypePattern(interoptopus::lang::types::TypePattern::Option(inner)) => {
            contains_owned_wire(*inner, types, visited)
        }
        RsTypeKind::WireOnly(WireOnly::Map(key, value)) => contains_owned_wire(*key, types, visited) || contains_owned_wire(*value, types, visited),
        _ => false,
    }
}

fn contains_wireonly(ty_id: TypeId, rs_types: &RsTypes, visited: &mut std::collections::HashSet<TypeId>) -> bool {
    if !visited.insert(ty_id) {
        return false;
    }
    let Some(ty) = rs_types.get(&ty_id) else { return false };
    match &ty.kind {
        RsTypeKind::WireOnly(_) => true,
        RsTypeKind::Struct(s) => s.fields.iter().any(|f| contains_wireonly(f.ty, rs_types, visited)),
        RsTypeKind::TypePattern(interoptopus::lang::types::TypePattern::Option(inner)) => contains_wireonly(*inner, rs_types, visited),
        RsTypeKind::TypePattern(interoptopus::lang::types::TypePattern::Result(ok, err)) => {
            contains_wireonly(*ok, rs_types, visited) || contains_wireonly(*err, rs_types, visited)
        }
        _ => false,
    }
}
