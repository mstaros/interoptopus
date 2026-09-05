use crate::inventory::TypeId;
use crate::lang::meta::Docs;
use crate::lang::types::{Field, Repr};

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
    /// A tuple variant carrying payload types in declaration order.
    Tuple(Vec<TypeId>),
    /// A struct variant carrying named fields in declaration order.
    Struct(Vec<Field>),
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

/// One payload slot of a variant, in declaration order.
///
/// A tuple variant's slots are unnamed. A named variant's slots carry the declared field
/// name. Both shapes preserve field declaration order; see
/// `docs/csharp-multi-field-variants.md`.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Payload<'a> {
    /// The slot's type.
    pub ty: TypeId,
    /// The declared field name, present only for a named variant.
    pub name: Option<&'a str>,
}

impl Variant {
    pub fn new(name: impl AsRef<str>, tag: isize, kind: VariantKind) -> Self {
        Self { name: name.as_ref().to_string(), docs: Docs::default(), tag, kind }
    }

    /// The variant's payload slots, in declaration order.
    ///
    /// Read payloads through this to handle tuple and named fields uniformly without losing
    /// declaration order or omitting later payloads.
    pub fn payloads(&self) -> impl Iterator<Item = Payload<'_>> + '_ {
        let tuple = match &self.kind {
            VariantKind::Tuple(types) => types.as_slice(),
            _ => &[],
        };
        let named = match &self.kind {
            VariantKind::Struct(fields) => fields.as_slice(),
            _ => &[],
        };
        tuple.iter().map(|ty| Payload { ty: *ty, name: None }).chain(named.iter().map(|field| Payload { ty: field.ty, name: Some(&field.name) }))
    }

    /// Whether this variant uses a payload-capable declaration (including empty tuple/struct shapes).
    #[must_use]
    pub fn has_payload(&self) -> bool {
        !matches!(self.kind, VariantKind::Unit)
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
