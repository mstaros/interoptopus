//! Resolves collision-free C# member names for every `DataEnum` variant.
//!
//! Union projection adds nested payload cases and an eligible constants enum to the members the
//! generator already emits (factory, `IsX`, `AsX`, payload field, unmanaged helper), and
//! the union contract itself claims a set of fixed names. All of these share a single C#
//! declaration space, so names are allocated centrally rather than formatted
//! independently in nine output passes.
//!
//! The policy is **preservation-biased**: a variant keeps the exact name the generator
//! emits today unless a fixed union member makes that impossible. New union-only names
//! (the case type) move on collision instead of forcing a rename of existing public API.
//!
//! Stems are the Rust variant name *verbatim*. `enum_variants.rs` clones
//! `rust_variant.name` and the templates emit it unmodified, so verbatim is what the
//! generator produces today; re-casing here would silently rename members on enums that
//! have no collision at all. Templates must not re-sanitize after this pass, or
//! uniqueness is no longer guaranteed. Keyword escaping is deliberately out of scope --
//! see `Issues.md` `7c8cb22e`.
//!
//! Names are written into `type_kinds`, the source of truth. `type_all` rebuilds its
//! `Type` values from `type_kinds`, so writing there instead would populate a downstream
//! copy that is later overwritten -- which is what happened for the `Result` and `Option`
//! enums synthesised by `fallback.rs` and injected by `type_map_patterns`.
//!
//! See `docs/csharp-unions.md`, item 1.

use crate::lang::TypeId;
use crate::lang::types::kind::{DataEnum, TypeKind, TypePattern, Variant};
use crate::pass::Outcome::Unchanged;
use crate::pass::{ModelResult, PassInfo, model};
use std::collections::HashSet;

/// Members every generated union declares, which no variant-derived name may take.
///
/// `ToManaged` is deliberately absent: it is declared inside the nested `Unmanaged` and
/// `Marshaller` types, so it never shares a declaration space with an outer factory.
/// `_variant` and `_hasValue` are present because a variant named `variant` would derive
/// the field `_variant` and collide with the discriminant itself.
const RESERVED: &[&str] = &[
    "Value",
    "HasValue",
    "TryGetValue",
    "Unmanaged",
    "Marshaller",
    "MarshallerMeta",
    "ToUnmanaged",
    "AsUnmanaged",
    "ToString",
    "Dispose",
    "ExceptionForVariant",
    "_variant",
    "_hasValue",
];

#[derive(Default)]
pub struct Config {}

/// The complete emitted name family for one variant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VariantNames {
    /// Collision-free base. Equals the Rust variant name unless it had to move.
    pub stem: String,
    /// `public static E {factory}` / `public static E {factory}(T value)`.
    pub factory: String,
    /// `public bool {is_check}`.
    pub is_check: String,
    /// `public T {accessor}()`.
    pub accessor: String,
    /// Managed and unmanaged payload member names, in payload order.
    pub field: Vec<String>,
    /// Positional property names inside this variant's case type.
    pub case_fields: Vec<String>,
    /// Nested per-variant unmanaged helper struct.
    pub unmanaged: String,
    /// Nested case type introduced by union projection.
    pub case_type: String,
    /// True when `stem` differs from the Rust variant name.
    pub renamed: bool,
}

pub struct Pass {
    info: PassInfo,
}

/// Borrows the `DataEnum` a kind carries, if any.
///
/// A `DataEnum` reaches the model by three routes: directly from `enum_variants` for a
/// plain `#[ffi]` enum, and wrapped inside `TypePattern::Option` or `TypePattern::Result`
/// for the shapes `fallback.rs` synthesises and `type_map_patterns` installs. Matching
/// only the first left `Ok` and `Err` unresolved, which emitted `_` for every payload
/// field and factory.
pub(crate) fn data_enum(kind: &TypeKind) -> Option<&DataEnum> {
    match kind {
        TypeKind::DataEnum(e) => Some(e),
        TypeKind::TypePattern(TypePattern::Option(_, e) | TypePattern::Result(_, _, e)) => Some(e),
        _ => None,
    }
}

fn data_enum_mut(kind: &mut TypeKind) -> Option<&mut DataEnum> {
    match kind {
        TypeKind::DataEnum(e) => Some(e),
        TypeKind::TypePattern(TypePattern::Option(_, e) | TypePattern::Result(_, _, e)) => Some(e),
        _ => None,
    }
}

/// Backing names depend on arity, not on separately allocated case-property names.
/// Historical unit reservations and single-payload fields keep their exact spelling.
fn field_names(stem: &str, count: usize) -> Vec<String> {
    if count <= 1 {
        vec![format!("_{stem}")]
    } else {
        (0..count).map(|index| format!("_{stem}_{index}")).collect()
    }
}

/// The variant's payloads paired with their centrally allocated backing member names.
pub(crate) fn payload_fields(variant: &Variant) -> impl Iterator<Item = (TypeId, String)> + '_ {
    variant.payloads().enumerate().map(move |(index, payload)| {
        let field = variant.field_names.get(index).expect("union_names must allocate every payload member").clone();
        (payload.ty, field)
    })
}

/// The factory, checks, helper and every backing field occupy the union's declaration space.
fn family(stem: &str, count: usize) -> Vec<String> {
    let mut names = vec![stem.to_string(), format!("Is{stem}"), format!("As{stem}"), format!("Unmanaged{stem}")];
    names.extend(field_names(stem, count));
    names
}

/// Case properties preserve declared names; tuple slots use Value or Item1..ItemN.
/// Existing one-slot tuple cases retain Value even though outer unions reserve it.
fn case_field_names(case_type: &str, payloads: &[Option<String>], outer: &HashSet<String>) -> Vec<String> {
    if matches!(payloads, [None]) {
        return vec!["Value".to_string()];
    }
    let mut claimed = outer.clone();
    claimed.insert(case_type.to_string());
    claimed.extend(["Equals", "GetHashCode", "Deconstruct", "PrintMembers", "EqualityContract"].map(str::to_string));
    payloads.iter().enumerate().map(|(index, name)| {
        let base = name.as_deref().map_or_else(|| format!("Item{}", index + 1), |name| name.trim_start_matches("r#").to_string());
        let mut candidate = base.clone();
        let mut suffix = 1usize;
        while claimed.contains(&candidate) {
            candidate = if suffix == 1 { format!("{base}Field") } else { format!("{base}Field{suffix}") };
            suffix += 1;
        }
        claimed.insert(candidate.clone());
        candidate
    }).collect()
}

/// Allocates the name family for one enum.
fn resolve(enclosing: &str, stems: &[String], payloads: &[Vec<Option<String>>]) -> Vec<VariantNames> {
    // Names that can never be taken: the union contract, plus the enclosing type itself
    // (CS0542 forbids a member matching the name of its containing type).
    let mut fixed: HashSet<String> = RESERVED.iter().map(|s| (*s).to_string()).collect();
    fixed.insert(enclosing.to_string());

    let mut claimed = fixed.clone();
    let mut chosen: Vec<Option<String>> = vec![None; stems.len()];

    // Pass 1 -- preservation. Claim every stem that can keep its current name, before
    // any fallback is allocated. Without this ordering a variant needing a fallback can
    // steal a name another variant is already emitting: given `Value` and `ValueVariant`,
    // allocating in declaration order would hand `ValueVariant` to the first and displace
    // the second, breaking API that had no collision.
    for (i, stem) in stems.iter().enumerate() {
        let fam = family(stem, payloads[i].len());
        if fam.iter().all(|n| !claimed.contains(n)) {
            claimed.extend(fam);
            chosen[i] = Some(stem.clone());
        }
    }

    // Pass 2 -- fallback for stems that could not be kept.
    for (i, stem) in stems.iter().enumerate() {
        if chosen[i].is_some() {
            continue;
        }
        let mut n = 1usize;
        let candidate = loop {
            let cand = if n == 1 { format!("{stem}Variant") } else { format!("{stem}Variant{n}") };
            let fam = family(&cand, payloads[i].len());
            if fam.iter().all(|x| !claimed.contains(x)) {
                claimed.extend(fam);
                break cand;
            }
            n += 1;
        };
        chosen[i] = Some(candidate);
    }

    // Pass 3 -- case types. These are new, so they move on collision rather than
    // disturbing a stem that is already emitting fine. Nested types share the members'
    // declaration space, so they are checked against the same claimed set.
    let mut resolved: Vec<_> = stems
        .iter()
        .zip(chosen)
        .enumerate()
        .map(|(index, (original, stem))| {
            let stem = stem.expect("every stem is assigned in pass 1 or 2");
            let mut n = 1usize;
            let case_type = loop {
                let cand = if n == 1 { format!("{stem}Case") } else { format!("{stem}Case{n}") };
                if !claimed.contains(&cand) {
                    claimed.insert(cand.clone());
                    break cand;
                }
                n += 1;
            };

            let factory = stem.clone();
            let is_check = format!("Is{stem}");
            let accessor = format!("As{stem}");
            let field = field_names(&stem, payloads[index].len());
            let unmanaged = format!("Unmanaged{stem}");
            VariantNames { renamed: &stem != original, stem, factory, is_check, accessor, field, case_fields: Vec::new(), unmanaged, case_type }
        })
        .collect();
    for (variant, payloads) in resolved.iter_mut().zip(payloads) {
        variant.case_fields = case_field_names(&variant.case_type, payloads, &claimed);
    }
    resolved
}

impl Pass {
    #[must_use]
    pub fn new(_: Config) -> Self {
        Self { info: PassInfo { name: file!() } }
    }

    /// Writes resolved names onto each variant of every `DataEnum` in `kinds`.
    ///
    /// Names live on the variant rather than in a side table keyed by `TypeId`. Several
    /// output passes filter variants before emitting -- `body` keeps only disposable
    /// ones, `body_as_unmanaged` only payload-carrying ones -- so a parallel vector
    /// indexed positionally would misalign silently after any filter.
    pub fn process(
        &mut self,
        pass_meta: &mut crate::pass::PassMeta,
        kinds: &mut model::common::types::kind::Pass,
        names: &model::common::types::names::Pass,
    ) -> ModelResult {
        let mut outcome = Unchanged;

        // An empty stem marks an unresolved variant. Enums with no variants are trivially
        // resolved and must not re-trigger on every convergence cycle.
        let pending: Vec<(TypeId, TypeKind)> = kinds
            .iter()
            .filter(|(_, kind)| data_enum(kind).is_some_and(|e| e.variants.iter().any(|v| v.stem.is_empty())))
            .map(|(id, kind)| (*id, kind.clone()))
            .collect();

        for (id, mut kind) in pending {
            // The enclosing type name is reserved, so wait for it rather than resolving
            // against a name that is not yet known.
            let Some(enclosing) = names.get(id).cloned() else {
                pass_meta.lost_found.missing(self.info, crate::pass::MissingItem::CsType(id));
                continue;
            };

            let Some(target) = data_enum_mut(&mut kind) else { continue };
            let stems: Vec<String> = target.variants.iter().map(|v| v.name.clone()).collect();
            let payloads: Vec<_> = target.variants.iter().map(|variant| variant.payloads().map(|payload| payload.name.map(str::to_string)).collect()).collect();
            for (variant, resolved) in target.variants.iter_mut().zip(resolve(&enclosing, &stems, &payloads)) {
                variant.stem = resolved.stem;
                variant.case_type = resolved.case_type;
                variant.field_names = resolved.field;
                variant.case_fields = resolved.case_fields;
            }

            if target.groups_constants() {
                // Preserve existing factories and payload case names. The shared enum moves
                // on collision, including a Rust variant or enclosing type named Constants.
                let mut claimed: HashSet<String> = RESERVED.iter().map(|name| (*name).to_string()).collect();
                claimed.insert(enclosing.clone());
                for variant in &target.variants {
                    claimed.extend(family(&variant.stem, variant.fields.len()));
                    claimed.insert(variant.case_type.clone());
                }
                let mut constants = "Constants".to_string();
                let mut suffix = 2usize;
                while claimed.contains(&constants) {
                    constants = format!("Constants{suffix}");
                    suffix += 1;
                }
                claimed.insert(constants.clone());
                for (variant, payloads) in target.variants.iter_mut().zip(&payloads) {
                    if variant.can_carry_payload {
                        variant.case_fields = case_field_names(&variant.case_type, payloads, &claimed);
                    } else {
                        variant.case_type.clone_from(&constants);
                    }
                }
            }

            kinds.set(id, kind);
            outcome.changed();
        }

        Ok(outcome)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(enclosing: &str, stems: &[&str]) -> Vec<VariantNames> {
        resolve(enclosing, &stems.iter().map(|s| (*s).to_string()).collect::<Vec<_>>(), &vec![vec![None]; stems.len()])
    }

    #[test]
    fn ordinary_enum_keeps_every_existing_name() {
        let r = names("Color", &["Red", "Green", "Blue"]);
        assert_eq!(r[0].factory, "Red");
        assert_eq!(r[0].is_check, "IsRed");
        assert_eq!(r[0].accessor, "AsRed");
        assert_eq!(r[0].field, ["_Red"]);
        assert_eq!(r[0].unmanaged, "UnmanagedRed");
        assert_eq!(r[0].case_type, "RedCase");
        assert!(r.iter().all(|v| !v.renamed));
    }

    #[test]
    fn variant_named_value_moves_because_value_is_fixed() {
        let r = names("E", &["Value"]);
        assert_eq!(r[0].factory, "ValueVariant");
        assert_eq!(r[0].case_type, "ValueVariantCase");
        assert!(r[0].renamed);
    }

    #[test]
    fn existing_value_variant_is_preserved_and_value_moves_past_it() {
        // Declaration order deliberately puts the problematic variant first: pass 1 must
        // claim `ValueVariant` for the variant already emitting it before pass 2 runs.
        let r = names("E", &["Value", "ValueVariant"]);
        assert_eq!(r[1].factory, "ValueVariant");
        assert!(!r[1].renamed);
        assert_eq!(r[0].factory, "ValueVariant2");
        assert!(r[0].renamed);
    }

    #[test]
    fn case_type_moves_rather_than_the_stems() {
        let r = names("E", &["B", "BCase"]);
        assert_eq!(r[0].factory, "B");
        assert_eq!(r[1].factory, "BCase");
        assert!(r.iter().all(|v| !v.renamed));
        assert_eq!(r[0].case_type, "BCase2");
        assert_eq!(r[1].case_type, "BCaseCase");
    }

    #[test]
    fn is_prefixed_variant_collides_with_a_check() {
        // `Foo` emits `IsFoo`; a sibling variant `IsFoo` emits factory `IsFoo`. Broken in
        // the current generator; one of them must move.
        let r = names("E", &["Foo", "IsFoo"]);
        assert_eq!(r[0].factory, "Foo");
        assert_eq!(r[1].factory, "IsFooVariant");
        assert!(r[1].renamed);
    }

    #[test]
    fn as_prefixed_variant_collides_with_an_accessor() {
        let r = names("E", &["Foo", "AsFoo"]);
        assert_eq!(r[1].factory, "AsFooVariant");
    }

    #[test]
    fn unmanaged_prefixed_variant_collides_with_a_helper() {
        let r = names("E", &["Foo", "UnmanagedFoo"]);
        assert_eq!(r[0].unmanaged, "UnmanagedFoo");
        assert_eq!(r[1].factory, "UnmanagedFooVariant");
    }

    #[test]
    fn variant_matching_enclosing_type_moves() {
        let r = names("E", &["E"]);
        assert_eq!(r[0].factory, "EVariant");
        assert!(r[0].renamed);
    }

    #[test]
    fn variant_named_variant_collides_with_the_discriminant_field() {
        // Field would be `_variant`, which is the discriminant storage.
        let r = names("E", &["variant"]);
        assert_eq!(r[0].factory, "variantVariant");
    }

    #[test]
    fn repeated_fallback_collision_is_deterministic() {
        let r = names("E", &["Value", "ValueVariant", "ValueVariant2"]);
        assert_eq!(r[1].factory, "ValueVariant");
        assert_eq!(r[2].factory, "ValueVariant2");
        assert_eq!(r[0].factory, "ValueVariant3");
    }

    #[test]
    fn result_and_option_variants_resolve_unchanged() {
        // The shapes `fallback.rs` synthesises. Regression guard: these reach the model
        // through `type_map_patterns` rather than `enum_variants`, and an earlier version
        // of this pass missed them entirely, emitting `_` for every payload field.
        let r = names("ResultUintError", &["Ok", "Err", "Panic", "Null"]);
        assert_eq!(r[0].factory, "Ok");
        assert_eq!(r[0].field, ["_Ok"]);
        assert_eq!(r[0].unmanaged, "UnmanagedOk");
        assert_eq!(r[1].field, ["_Err"]);
        assert!(r.iter().all(|v| !v.renamed));

        let o = names("OptionUint", &["Some", "None"]);
        assert_eq!(o[0].field, ["_Some"]);
        assert_eq!(o[1].field, ["_None"]);
        assert!(o.iter().all(|v| !v.renamed));
    }

    #[test]
    fn every_emitted_name_is_unique() {
        let r = names("E", &["Value", "ValueVariant", "B", "BCase", "Foo", "IsFoo", "UnmanagedFoo", "E", "variant"]);
        let mut seen = HashSet::new();
        for v in &r {
            for n in [&v.factory, &v.is_check, &v.accessor, &v.unmanaged, &v.case_type].into_iter().chain(&v.field) {
                assert!(seen.insert(n.clone()), "duplicate emitted name: {n}");
            }
        }
        for n in RESERVED {
            assert!(!seen.contains(*n), "emitted name collides with fixed member: {n}");
        }
        assert!(!seen.contains("E"), "emitted name collides with the enclosing type");
    }

    #[test]
    fn multi_field_backing_names_share_the_outer_collision_scope() {
        let r = resolve("E", &["Pair".to_string(), "Pair_0".to_string()], &[vec![None, None, None], vec![None]]);
        assert_eq!(r[0].field, ["_Pair_0", "_Pair_1", "_Pair_2"]);
        assert_eq!(r[0].case_fields, ["Item1", "Item2", "Item3"]);
        assert_eq!(r[1].stem, "Pair_0Variant");
        let mut seen: HashSet<String> = RESERVED.iter().map(|name| (*name).to_string()).collect();
        seen.insert("E".to_string());
        for variant in &r {
            for name in [&variant.factory, &variant.is_check, &variant.accessor, &variant.unmanaged, &variant.case_type].into_iter().chain(&variant.field) {
                assert!(seen.insert(name.clone()), "duplicate outer member: {name}");
            }
        }
    }

    #[test]
    fn named_case_properties_reserve_record_and_union_members() {
        let names = ["Value", "ValueField", "Equals", "GetHashCode", "Deconstruct", "PairCase", "Item1", "HasValue"];
        let payloads = names.map(|name| Some(name.to_string())).to_vec();
        let r = resolve("E", &["Pair".to_string()], &[payloads]);
        assert_eq!(r[0].case_fields, ["ValueField", "ValueFieldField", "EqualsField", "GetHashCodeField", "DeconstructField", "PairCaseField", "Item1", "HasValueField"]);
        let mut seen: HashSet<&str> = RESERVED.iter().copied().collect();
        seen.extend(["Equals", "GetHashCode", "Deconstruct", "PrintMembers", "EqualityContract", "PairCase", "Pair", "IsPair", "AsPair", "UnmanagedPair"]);
        for name in &r[0].case_fields {
            assert!(seen.insert(name), "case property collides with another generated member: {name}");
        }
    }

    #[test]
    fn named_fields_are_independent_per_case_and_keep_declared_spelling() {
        let r = resolve("E", &["A".to_string(), "B".to_string()], &[vec![Some("r#number".to_string())], vec![Some("number".to_string())]]);
        assert_eq!(r[0].case_fields, ["number"]);
        assert_eq!(r[1].case_fields, ["number"]);
        assert_eq!(r[0].field, ["_A"]);
        assert_eq!(r[1].field, ["_B"]);
    }

}
