use crate::lang::TypeId;
use crate::lang::types::kind::{Field, Primitive};
use interoptopus::lang::meta::Docs;

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct Variant {
    /// The Rust variant name, as declared.
    pub name: String,
    pub docs: Docs,
    pub tag: isize,
    /// Resolved payload fields in declaration order; an empty name denotes a tuple slot.
    pub fields: Vec<Field>,
    /// Collision-free backing member names, allocated by `union_names`.
    pub field_names: Vec<String>,
    /// Collision-free positional property names inside the generated case type.
    pub case_fields: Vec<String>,
    /// Whether this variant uses a payload-capable declaration, including empty tuple/struct
    /// shapes and payload types that resolve to no C# fields.
    ///
    /// This is not the same question as `!fields.is_empty()`. `Result<(), ()>` declares `Ok(T)` and
    /// `Err(E)` — payload-carrying positions — but `fallback.rs` resolves a `()` payload to
    /// zero fields, so both read as payloadless downstream and become indistinguishable from a true
    /// unit variant like `Color::Red`.
    ///
    /// Union eligibility must ask *can a variant carry a payload*, which is a property of the
    /// declaration, not *does one carry data*, which is a property of the instantiation. Asking
    /// the latter made `Result<(), ()>` lose its union projection while `Result<u32, Error>`
    /// kept it — the same shape classified differently by a type argument.
    pub can_carry_payload: bool,
    /// Collision-free base for every emitted member of this variant.
    ///
    /// Equals [`Variant::name`] unless a fixed union member made that impossible; see
    /// `pass::model::common::types::union_names`. Output passes must derive the factory,
    /// `IsX`, `AsX` and unmanaged helper from this, and use the allocated payload names,
    /// or uniqueness is not guaranteed.
    ///
    /// Carried on the variant rather than in a side table because several output passes
    /// filter variants before emitting, and a parallel vector would misalign silently.
    pub stem: String,
    /// Nested case type introduced by union projection.
    pub case_type: String,
}

/// One resolved payload slot of a variant, in declaration order.
///
/// A tuple variant's slots are unnamed. A named variant's slots carry the declared field
/// name. All resolved slots are stored once in [`Variant::fields`]; see
/// `docs/csharp-multi-field-variants.md`.
///
/// These are *resolved* payloads, not declared ones. `Result<(), ()>` declares `Ok(T)` but
/// resolves the `()` to no C# payload, so it yields no slots while
/// [`Variant::can_carry_payload`] stays true. Union eligibility asks the latter.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Payload<'a> {
    /// The slot's C# type.
    pub ty: TypeId,
    /// The declared field name, present only for a named variant.
    pub name: Option<&'a str>,
}

impl Variant {
    /// The variant's resolved payload slots, in declaration order.
    ///
    /// This view preserves field order and declared names without duplicating storage.
    pub fn payloads(&self) -> impl Iterator<Item = Payload<'_>> + '_ {
        self.fields.iter().map(|field| Payload { ty: field.ty, name: if field.name.is_empty() { None } else { Some(&field.name) } })
    }
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct DataEnum {
    pub variants: Vec<Variant>,
    /// The C# primitive used for the discriminant field.
    pub discriminant_type: Primitive,
}

impl DataEnum {
    /// Whether this enum receives C# union projection.
    ///
    /// True when any variant *can* carry a payload. See [`Variant::can_carry_payload`] for why
    /// that is not `ty.is_some()`; the original regression is covered by
    /// `enum_union_members::eligibility_asks_can_carry_not_does_carry`.
    ///
    /// The rule is per *enum*, not per variant: an enum projected as a union gives every
    /// variant a case type, unit variants included, while an enum with no payload-capable
    /// variant receives no case types, no `Value`, no `HasValue` and no `TryGetValue`. See
    /// `docs/csharp-unions.md` "Two layers, two rules" and `Issues.md` `79be256e`.
    ///
    /// Call this rather than re-deriving the `any`. Five output sites carried their own copy
    /// before this existed, which is the same shape as the gate `Issues.md` `5d1ae4c7` records.
    #[must_use]
    pub fn is_union_projected(&self) -> bool {
        self.variants.iter().any(|v| v.can_carry_payload)
    }
}
