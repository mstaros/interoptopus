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

/// One payload slot of a variant, in declaration order.
///
/// A tuple variant's slots are unnamed. A named variant's slots carry the declared field
/// name. Today every payload-carrying variant has exactly one unnamed slot, because
/// [`VariantKind::Tuple`] holds one [`TypeId`] and the proc macro rejects both multi-field
/// and named variants; see `docs/csharp-multi-field-variants.md`.
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
    /// Read payloads through this rather than matching [`Variant::kind`] directly. An inline
    /// `VariantKind::Tuple(ty) => ty` sees exactly one payload by construction, so it keeps
    /// compiling and silently drops the rest once variants widen. This accessor is the one
    /// seam that widens with them.
    pub fn payloads(&self) -> impl Iterator<Item = Payload<'_>> + '_ {
        match &self.kind {
            VariantKind::Unit => None,
            VariantKind::Tuple(ty) => Some(Payload { ty: *ty, name: None }),
        }
        .into_iter()
    }

    /// Whether this variant's declaration has at least one payload slot.
    #[must_use]
    pub fn has_payload(&self) -> bool {
        matches!(self.kind, VariantKind::Tuple(_))
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
