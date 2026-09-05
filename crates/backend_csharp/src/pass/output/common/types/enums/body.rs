//! Renders enum body definitions using the `enum_body.cs` template.

use crate::lang::TypeId;
use crate::lang::types::kind::{DataEnum, Primitive, TypeKind, TypePattern, Variant};
use crate::pass::{OutputResult, PassInfo, model, output};
use interoptopus_backends::template::{Context, Value};
use std::collections::HashMap;

#[derive(Default)]
pub struct Config {}

pub struct Pass {
    info: PassInfo,
    enum_body: HashMap<TypeId, String>,
}

impl Pass {
    #[must_use]
    pub fn new(_: Config) -> Self {
        Self { info: PassInfo { name: file!() }, enum_body: HashMap::default() }
    }

    pub fn process(
        &mut self,
        _pass_meta: &mut crate::pass::PassMeta,
        output_master: &output::common::master::Pass,
        types: &model::common::types::all::Pass,
        struct_class: &model::common::types::info::struct_class::Pass,
        disposable: &model::common::types::info::disposable::Pass,
        projection: &model::common::types::info::projection::Pass,
        nullable: &model::common::types::info::nullable::Pass,
        enum_body_case_types: &output::common::types::enums::body_case_types::Pass,
        enum_body_union_members: &output::common::types::enums::body_union_members::Pass,
        enum_body_unmanaged_variant: &output::common::types::enums::body_unmanaged_variant::Pass,
        enum_body_unmanaged: &output::common::types::enums::body_unmanaged::Pass,
        enum_body_to_unmanaged: &output::common::types::enums::body_to_unmanaged::Pass,
        enum_body_as_unmanaged: &output::common::types::enums::body_as_unmanaged::Pass,
        enum_body_ctors: &output::common::types::enums::body_ctors::Pass,
        enum_body_from_call: &output::common::types::enums::body_from_call::Pass,
        enum_body_exception_for_variant: &output::common::types::enums::body_exception_for_variant::Pass,
        enum_body_tostring: &output::common::types::enums::body_tostring::Pass,
        managed: &output::common::conversion::unmanaged_conversion::Pass,
        mode: crate::pass::OperationMode,
    ) -> OutputResult {
        let templates = output_master.templates();

        for (type_id, ty) in types.iter() {
            let type_kind = &ty.kind;
            match type_kind {
                TypeKind::DataEnum(_) => {}
                TypeKind::TypePattern(TypePattern::Result(_, _, _)) => {}
                TypeKind::TypePattern(TypePattern::Option(_, _)) => {}
                _ => continue,
            }

            if projection.is_plain_enum(*type_id) {
                self.enum_body.insert(*type_id, String::new());
                continue;
            }

            let name = &ty.name;
            let visibility = ty.visibility.to_string();

            // DataEnum carrying a WireOnly variant payload (e.g. `S(String)`) has no
            // FFI-safe Unmanaged form — it only flows through `Wire<T>`. Such enums
            // also hold no native resources (their payloads are GC-managed) so they
            // are not disposable.
            let has_wire_only_payload = projection.has_wire_only_payload(*type_id).unwrap_or(false);

            // Managed-only types have no Unmanaged representation:
            //   - Result/Option whose Ok side is a Service.
            //   - DataEnum with WireOnly variant payloads.
            let is_managed_only = !projection.crosses_ffi(*type_id).unwrap_or(true);

            let ty = *type_id;
            let struct_or_class = if struct_class.is_struct(ty) { "struct" } else { "class" };
            let is_disposable = if has_wire_only_payload {
                false
            } else {
                disposable.is_disposable(*type_id).unwrap_or(false)
            };

            let case_types = enum_body_case_types.get(*type_id).unwrap_or(&[]);
            let union_members = enum_body_union_members.get(*type_id).map_or("", std::string::String::as_str);
            let unmanaged_variants = enum_body_unmanaged_variant.get(*type_id).unwrap_or(&[]);
            let unmanaged = enum_body_unmanaged.get(*type_id).map_or("", std::string::String::as_str);
            let to_unmanaged = enum_body_to_unmanaged.get(*type_id).map_or("", std::string::String::as_str);
            let as_unmanaged = enum_body_as_unmanaged.get(*type_id).map_or("", std::string::String::as_str);
            let ctors = enum_body_ctors.get(*type_id).map_or("", std::string::String::as_str);
            let from_call = enum_body_from_call.get(*type_id).map_or("", std::string::String::as_str);
            let exception_for_variant = enum_body_exception_for_variant.get(*type_id).map_or("", std::string::String::as_str);
            let to_string = enum_body_tostring.get(*type_id).map_or("", std::string::String::as_str);

            // Collect disposable variant fields for the Dispose() method.
            let disposable_variants: Vec<HashMap<&str, Value>> = if is_disposable {
                let variants: &[Variant] = match type_kind {
                    TypeKind::DataEnum(de) => &de.variants,
                    TypeKind::TypePattern(TypePattern::Option(_, de)) => &de.variants,
                    TypeKind::TypePattern(TypePattern::Result(_, _, de)) => &de.variants,
                    _ => &[],
                };
                variants
                    .iter()
                    .flat_map(|v| {
                        model::common::types::union_names::payload_fields(v)
                            .filter(|(payload_ty, _)| disposable.is_disposable(*payload_ty).unwrap_or(false))
                            .map(move |(_, field)| {
                                let mut m = HashMap::new();
                                m.insert("name", Value::normal_string(&field));
                                m.insert("tag", Value::from(v.tag as i64));
                                m
                            })
                    })
                    .collect()
            } else {
                Vec::new()
            };

            // Position (a) of `docs/csharp-unions.md` Open items 1: the union arrives here by
            // argument, and the marshaller dereferences it. `NullPolicy::Throw` is exactly "this
            // is a class-backed union", already computed and already carrying its own not-ready
            // handling, so it is reused rather than re-derived from `struct_class` +
            // `projection`.
            let rejects_null = nullable.null_policy(*type_id) == Some(model::common::types::info::nullable::NullPolicy::Throw);

            // Item 5g: a default struct-backed union holds no value, so `Dispose()` must free
            // nothing. Today it matches on `_variant` alone and survives only because the
            // null-conditional short-circuits on a null payload — correct by accident. There is
            // no struct-backed disposable union in the corpus (disposability comes from `Into`
            // payloads, and `Into` is what makes the union class-backed), so this guard is
            // written in advance and is expected to change no snapshot.
            let has_empty_state = struct_class.is_struct(*type_id) && projection.is_union(*type_id);

            let marshaller_to_unmanaged = managed.to_unmanaged_name(*type_id);
            let marshaller_to_managed = managed.to_managed_name(*type_id);
            let result_interface = result_interface(type_kind, types, mode);

            let mut context = Context::new();
            context.insert("has_empty_state", &has_empty_state);
            context.insert("rejects_null", &rejects_null);
            context.insert("name", name);
            context.insert("struct_or_class", struct_or_class);
            context.insert("is_disposable", &is_disposable);
            context.insert("is_managed_only", &is_managed_only);

            // Item 3d. `[Union]` and `IUnion` follow the same eligibility rule as the case
            // types and members they describe: an enum with no payload-capable variant gets
            // neither. Deliberately *not* gated on `is_managed_only` — that guard governs the
            // `Unmanaged` mirror and the marshaller, and a managed-only enum can still be a
            // union. `DataEnum` in the reference project is exactly that case.
            let is_union_projected = projection.is_union(ty);
            context.insert("is_union_projected", &is_union_projected);
            context.insert("visibility", &visibility);
            context.insert("is_result", &result_interface.is_some());
            context.insert("result_ok_name", result_interface.as_ref().map_or("", |r| r.ok_name.as_str()));
            context.insert("result_err_name", result_interface.as_ref().map_or("", |r| r.err_name.as_str()));
            context.insert("result_ok_is_unit", &result_interface.as_ref().is_some_and(|r| r.ok_is_unit));
            context.insert("result_err_is_unit", &result_interface.as_ref().is_some_and(|r| r.err_is_unit));
            context.insert("result_has_unit_methods", &result_interface.as_ref().is_some_and(|r| r.ok_is_unit || r.err_is_unit));
            context.insert("disposable_variants", &disposable_variants);
            context.insert("case_types", &case_types);
            context.insert("union_members", &union_members);
            context.insert("unmanaged_variants", &unmanaged_variants);
            context.insert("unmanaged", &unmanaged);
            context.insert("to_unmanaged", &to_unmanaged);
            context.insert("as_unmanaged", &as_unmanaged);
            context.insert("ctors", &ctors);
            context.insert("from_call", &from_call);
            context.insert("exception_for_variant", &exception_for_variant);
            context.insert("to_string", &to_string);
            context.insert("marshaller_to_unmanaged", marshaller_to_unmanaged);
            context.insert("marshaller_to_managed", marshaller_to_managed);

            let rendered = templates.render("common/types/enums/body.cs", &context)?;
            self.enum_body.insert(*type_id, rendered);
        }

        Ok(())
    }

    #[must_use]
    pub fn get(&self, type_id: TypeId) -> Option<&String> {
        self.enum_body.get(&type_id)
    }
}

struct ResultInterface {
    ok_name: String,
    err_name: String,
    ok_is_unit: bool,
    err_is_unit: bool,
}

fn result_interface(type_kind: &TypeKind, types: &model::common::types::all::Pass, mode: crate::pass::OperationMode) -> Option<ResultInterface> {
    let TypeKind::TypePattern(TypePattern::Result(ok_ty, err_ty, _)) = type_kind else {
        return None;
    };

    let ok_ty = super::resolve_service_variant(*ok_ty, types, mode);
    let err_ty = super::resolve_service_variant(*err_ty, types, mode);
    let ok_is_unit = is_unit_type(ok_ty, types);
    let err_is_unit = is_unit_type(err_ty, types);
    let ok_name = result_type_name(ok_ty, types);
    let err_name = result_type_name(err_ty, types);

    Some(ResultInterface { ok_name, err_name, ok_is_unit, err_is_unit })
}

fn result_type_name(type_id: TypeId, types: &model::common::types::all::Pass) -> String {
    if is_unit_type(type_id, types) {
        "Unit".to_string()
    } else {
        types.get(type_id).map_or_else(|| "Unit".to_string(), |t| t.name.clone())
    }
}

fn is_unit_type(type_id: TypeId, types: &model::common::types::all::Pass) -> bool {
    matches!(types.get(type_id).map(|t| &t.kind), Some(TypeKind::Primitive(Primitive::Void) | TypeKind::TypePattern(TypePattern::CVoid)))
}
