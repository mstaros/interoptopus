//! Wire's view of the resolved C# model: canonical identifiers, and nothing else.
//!
//! `wire/mod.rs` walks the **Rust** type graph, because `WireOnly` fields have no C#
//! `TypeKind` and traversal has to follow the shape that actually exists. That is correct
//! for *composition* — `List<T>`, `Dictionary<K, V>`, `T?`, `T[]` are shapes, not
//! identifiers. It is wrong for *nominal identifiers*: `Is{stem}`, `As{stem}()`, the
//! factory, and the C# name of a struct or enum are all owned by the model.
//!
//! The split worth keeping: **the Rust graph decides traversal and composition; the C#
//! model decides canonical identifiers.**
//!
//! This type is deliberately two methods wide. Handing `WireCodeGen` the whole model would
//! make the right thing easy without making the wrong thing hard, and the wrong thing is
//! exactly what `Issues.md` `4e9a17c3` is about — `is_cs_value_type` in this same file
//! re-derives struct-vs-class, which `types::info::struct_class` owns, and that is what
//! gets written when the model is simply in scope. Widening this surface should require
//! adding a method, so the question "should wire be deciding this?" is asked in review
//! rather than skipped.
//!
//! Lookups return `Option`. The strict, panicking accessors live on `WireCodeGen`, one per
//! invariant. Keeping `Option` here is what lets [`variant_stem_of`] be unit-tested against
//! a hand-built `DataEnum` with no pass, no inventory and no snapshot. That matters: this
//! defect survived three rounds precisely because the invariant was observable only through
//! generated output, on a corpus where it never diverged.

use crate::lang::types::kind::TypeKind;
use crate::pass::model;
use crate::pass::model::common::types::union_names::data_enum;
use interoptopus::inventory::TypeId as RsTypeId;

/// Resolves C# identifiers for the Rust types wire is walking.
///
/// Keyed by **Rust** `TypeId`, because that is what wire holds while walking the Rust graph.
pub struct CsNames<'a> {
    types: &'a model::common::types::all::Pass,
    id_map: &'a model::common::id_map::Pass,
}

impl<'a> CsNames<'a> {
    #[must_use]
    pub fn new(types: &'a model::common::types::all::Pass, id_map: &'a model::common::id_map::Pass) -> Self {
        Self { types, id_map }
    }

    /// The canonical C# name for a Rust type, or `None` if the model has no entry.
    ///
    /// `None` is a model-initialization failure, not a legitimate wire-only state: `id_map`
    /// maps every inventory type, `enum_variants` excludes no enum, and `wire::nested`
    /// backfills the structs `struct_fields` skips. It is deliberately not defaulted to
    /// `sanitize_rust_name(&ty.name)` — a second place that decides names is how this family
    /// of defects starts.
    #[must_use]
    pub fn mapped_type_name(&self, rust_id: RsTypeId) -> Option<&str> {
        let cs_id = self.id_map.ty(rust_id)?;
        self.types.get(cs_id).map(|ty| ty.name.as_str())
    }

    /// The allocated stem for one variant, identified by its tag.
    ///
    /// Tag, never index and never name. Index is positional and breaks under the variant
    /// filtering several output passes already do; name is the thing being corrected. Since
    /// Step 0 (`c928d53e`) the tag is the one identity both models agree on.
    #[must_use]
    pub fn variant_stem(&self, rust_enum_id: RsTypeId, tag: isize) -> Option<&str> {
        let cs_id = self.id_map.ty(rust_enum_id)?;
        let ty = self.types.get(cs_id)?;
        variant_stem_of(&ty.kind, tag)
    }
}

/// Wire's view of one further model decision: does `T?` yield `Nullable<T>`, or a nullable
/// reference?
///
/// Kept separate from [`CsNames`] rather than added to it. That type is named for identifiers
/// and is deliberately two methods wide; this is a different question with a different
/// authority, and folding it in would dissolve the boundary `CsNames` exists to hold.
///
/// The question matters more than "struct or class" suggests. C# 15 unions must declare
/// `HasValue` and `Value` (`docs/csharp-unions.md` §"Reserved names"), and wire's `Option`
/// emission uses those same two identifiers to mean `Nullable<T>`'s members. For a
/// struct-backed union `Nullable<T>` shadows them and wire is correct. For a class-backed one
/// nothing shadows, and `.HasValue` binds to the union contract instead — which
/// `struct_class` deliberately produces for `OptionUtf8String` and owned `Result` types.
pub struct CsLayout<'a> {
    id_map: &'a model::common::id_map::Pass,
    struct_class: &'a model::common::types::info::struct_class::Pass,
}

impl<'a> CsLayout<'a> {
    #[must_use]
    pub fn new(id_map: &'a model::common::id_map::Pass, struct_class: &'a model::common::types::info::struct_class::Pass) -> Self {
        Self { id_map, struct_class }
    }

    /// Whether the C# type for `rust_id` is a value type, per the model.
    ///
    /// Only meaningful for *nominal* types. `struct_class` answers `false` for anything it has
    /// not registered, so callers must not hand it primitives or `WireOnly` kinds and read the
    /// answer as authoritative — a primitive would come back "reference type" and
    /// `Option<u32>` would emit a null check on a `uint?`.
    #[must_use]
    pub fn is_value_type(&self, rust_id: RsTypeId) -> Option<bool> {
        let cs_id = self.id_map.ty(rust_id)?;
        Some(self.struct_class.is_struct(cs_id))
    }
}

/// Wire's view of one further model decision: is this enum emitted as a plain C# `enum`?
///
/// Kept separate from [`CsNames`] for the same reason [`CsLayout`] is: that type is named for
/// identifiers and is deliberately narrow, and this is a different question with a different
/// authority - `types::info::projection` owns it.
///
/// **Do not re-derive this predicate.** Two proxies look right and are not:
///
/// - `data_enum(kind).is_some()` - a plain-projected enum is *still* `TypeKind::DataEnum` in the
///   model. Step three changed emission, not kind. `MyEnum` resolves a stem through
///   [`CsNames::variant_stem`] and would answer "union" here.
/// - "every variant is `VariantKind::Unit`" - `fallback.rs::payload_variant` sets
///   `can_carry_payload: true` unconditionally, so synthesised `Result`/`Option` carriers stay
///   union-projected while their Rust variants look unit-only.
///
/// Both are the shape of the mistake `docs/csharp-unions-handoff.md` section 8 records twice.
pub struct CsProjection<'a> {
    id_map: &'a model::common::id_map::Pass,
    projection: &'a model::common::types::info::projection::Pass,
}

impl<'a> CsProjection<'a> {
    #[must_use]
    pub fn new(id_map: &'a model::common::id_map::Pass, projection: &'a model::common::types::info::projection::Pass) -> Self {
        Self { id_map, projection }
    }

    /// Whether the C# type for `rust_id` is emitted as a plain C# `enum`.
    ///
    /// `false` for non-enums and for types the model has no entry for. Safe here: every caller
    /// has already established it is looking at an enum, and the fallback is the union path that
    /// was the only path before step three.
    #[must_use]
    pub fn is_plain_enum(&self, rust_id: RsTypeId) -> bool {
        self.id_map.ty(rust_id).is_some_and(|cs_id| self.projection.is_plain_enum(cs_id))
    }
}

/// Borrows the stem of the variant carrying `tag`, if the kind carries a `DataEnum` at all.
///
/// Split out from [`CsNames`] so it is testable without constructing a pass. Resolution goes
/// through `union_names::data_enum` rather than matching `TypeKind::DataEnum` inline: a
/// `DataEnum` reaches the model by three routes, and the other two — wrapped in
/// `TypePattern::Option` and `TypePattern::Result` — are exactly the ones an inline match
/// drops on the floor.
///
/// An empty stem is `union_names`' own "unresolved" sentinel, the same convention it uses for
/// convergence, so it is reported as absent rather than emitted as an empty identifier.
#[must_use]
pub fn variant_stem_of(kind: &TypeKind, tag: isize) -> Option<&str> {
    let variant = data_enum(kind)?.variants.iter().find(|v| v.tag == tag)?;
    if variant.stem.is_empty() { None } else { Some(variant.stem.as_str()) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lang::TypeId;
    use crate::lang::types::kind::{DataEnum, Primitive, TypePattern, Variant};
    use interoptopus::lang::meta::Docs;

    fn variant(name: &str, stem: &str, tag: isize) -> Variant {
        Variant { name: name.to_string(), docs: Docs::default(), tag, ty: None, can_carry_payload: false, stem: stem.to_string(), case_type: String::new() }
    }

    fn data_enum_kind(variants: Vec<Variant>) -> TypeKind {
        TypeKind::DataEnum(DataEnum { variants, discriminant_type: Primitive::UInt })
    }

    /// The case the reference project cannot produce, and the whole reason 1d exists: a
    /// variant whose stem was moved off its name by collision resolution.
    #[test]
    fn colliding_variant_resolves_to_the_moved_stem_not_the_rust_name() {
        let kind = data_enum_kind(vec![variant("Foo", "FooVariant", 0), variant("IsFoo", "IsFoo", 1)]);

        assert_eq!(variant_stem_of(&kind, 0), Some("FooVariant"));
        assert_eq!(variant_stem_of(&kind, 1), Some("IsFoo"));
    }

    /// Tags are authoritative and need not match position; resolving by index would return
    /// the wrong stem here.
    #[test]
    fn lookup_is_by_tag_not_by_position() {
        let kind = data_enum_kind(vec![variant("A", "A", 7), variant("B", "B", 3)]);

        assert_eq!(variant_stem_of(&kind, 7), Some("A"));
        assert_eq!(variant_stem_of(&kind, 3), Some("B"));
        assert_eq!(variant_stem_of(&kind, 0), None);
    }

    /// `union_names` writes an empty stem to mean "not yet resolved". Emitting that would
    /// produce `value.Is` — reported as absent so the strict accessor fails loudly instead.
    #[test]
    fn unresolved_stem_reads_as_absent() {
        let kind = data_enum_kind(vec![variant("Foo", "", 0)]);

        assert_eq!(variant_stem_of(&kind, 0), None);
    }

    /// A `DataEnum` also reaches the model wrapped in `Option` and `Result`. An inline
    /// `TypeKind::DataEnum` match returns `None` for both.
    #[test]
    fn wrapped_data_enums_resolve_through_the_shared_helper() {
        let inner = DataEnum { variants: vec![variant("Ok", "OkVariant", 0)], discriminant_type: Primitive::UInt };
        let ok = TypeId::new(1);
        let err = TypeId::new(2);

        let as_option = TypeKind::TypePattern(TypePattern::Option(ok, inner.clone()));
        let as_result = TypeKind::TypePattern(TypePattern::Result(ok, err, inner));

        assert_eq!(variant_stem_of(&as_option, 0), Some("OkVariant"));
        assert_eq!(variant_stem_of(&as_result, 0), Some("OkVariant"));
    }

    #[test]
    fn a_kind_that_carries_no_data_enum_resolves_to_none() {
        assert_eq!(variant_stem_of(&TypeKind::Opaque, 0), None);
    }
}