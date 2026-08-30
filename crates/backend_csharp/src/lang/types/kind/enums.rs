use crate::lang::TypeId;
use crate::lang::types::kind::Primitive;
use interoptopus::lang::meta::Docs;

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct Variant {
    /// The Rust variant name, as declared.
    pub name: String,
    pub docs: Docs,
    pub tag: isize,
    pub ty: Option<TypeId>,
    /// Whether this variant's *declaration* has a payload slot, irrespective of whether that
    /// slot resolves to anything in C#.
    ///
    /// This is not the same question as `ty.is_some()`. `Result<(), ()>` declares `Ok(T)` and
    /// `Err(E)` — payload-carrying positions — but `fallback.rs` resolves a `()` payload to
    /// `None`, so both read as payloadless downstream and become indistinguishable from a true
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
    /// `IsX`, `AsX`, payload field and unmanaged helper from this rather than from `name`,
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
/// name. Today every variant yields at most one unnamed slot, because [`Variant::ty`] holds
/// one optional [`TypeId`]; see `docs/csharp-multi-field-variants.md`.
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
    /// Read payloads through this rather than through [`Variant::ty`] directly. A bare
    /// `v.ty` sees exactly one payload by construction, so it keeps compiling and silently
    /// drops the rest once variants widen. This accessor is the one seam that widens with
    /// them.
    pub fn payloads(&self) -> impl Iterator<Item = Payload<'_>> + '_ {
        self.ty.map(|ty| Payload { ty, name: None }).into_iter()
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
    /// that is not `ty.is_some()`; the two predicates are separated by exactly one fixture,
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
