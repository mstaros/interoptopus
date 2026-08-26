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

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct DataEnum {
    pub variants: Vec<Variant>,
    /// The C# primitive used for the discriminant field.
    pub discriminant_type: Primitive,
}
