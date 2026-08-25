use crate::inventory::TypeId;
use crate::lang::meta::Docs;
use crate::lang::types::Repr;

/// The payload shape of an enum variant.
///
/// This describes payload only. The discriminant lives on [`Variant::tag`] and is
/// unconditional, so a variant's tag never depends on whether it carries a payload.
/// See `Issues.md` `09b82d44`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum VariantKind {
    /// A unit variant carrying no payload.
    Unit,
    /// A tuple variant carrying a single payload type.
    Tuple(TypeId),
}

/// A single variant of an FFI enum.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Variant {
    /// The variant name.
    pub name: String,
    /// Documentation extracted from `///` comments.
    pub docs: Docs,
    /// The variant's discriminant, as Rust assigns it.
    ///
    /// Explicit where one was written, otherwise the previous variant's plus one.
    /// Independent of [`Variant::kind`]: payload-carrying variants have real
    /// discriminants too, and backends must never substitute a positional index.
    pub tag: isize,
    /// The variant's payload shape.
    pub kind: VariantKind,
}

impl Variant {
    pub fn new(name: impl AsRef<str>, tag: isize, kind: VariantKind) -> Self {
        Self { name: name.as_ref().to_string(), docs: Docs::default(), tag, kind }
    }
}

/// An FFI enum definition with its variants and memory representation.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Enum {
    /// The enum's variants.
    pub variants: Vec<Variant>,
    /// The memory representation (e.g., `#[repr(u32)]`).
    pub repr: Repr,
}
