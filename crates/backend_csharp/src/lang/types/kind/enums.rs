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
