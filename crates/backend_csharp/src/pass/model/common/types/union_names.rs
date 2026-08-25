//! Resolves collision-free C# member names for every `DataEnum` variant.
//!
//! Union projection adds one nested case type per variant on top of the members the
//! generator already emits (factory, `IsX`, `AsX`, payload field, unmanaged helper), and
//! the union contract itself claims a set of fixed names. All of these share a single C#
//! declaration space, so names have to be allocated centrally rather than formatted
//! independently in seven templates.
//!
//! The policy is **preservation-biased**: a variant keeps the exact name the generator
//! emits today unless a fixed union member makes that impossible. New union-only names
//! (the case type) move on collision instead of forcing a rename of existing public API.
//!
//! Stems are the Rust variant name *verbatim*. `enum_variants.rs` clones
//! `rust_variant.name` and the templates emit `{{ v.name }}` unmodified, so verbatim is
//! what the generator produces today; re-casing here would silently rename members on
//! enums that have no collision at all. Templates must not re-sanitize after this pass,
//! or uniqueness is no longer guaranteed.
//!
//! See `docs/csharp-unions.md`, item 1.

use crate::lang::TypeId;
use crate::lang::types::kind::TypeKind;
use crate::pass::Outcome::Unchanged;
use crate::pass::{ModelResult, PassInfo, model};
use std::collections::{HashMap, HashSet};

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
    /// Managed payload field.
    pub field: String,
    /// Nested per-variant unmanaged helper struct.
    pub unmanaged: String,
    /// Nested case type introduced by union projection.
    pub case_type: String,
    /// True when `stem` differs from the Rust variant name.
    pub renamed: bool,
}

pub struct Pass {
    info: PassInfo,
    variants: HashMap<TypeId, Vec<VariantNames>>,
}

/// Names derived from a stem that the generator already emits today.
///
/// All five occupy the enum's declaration space, so a stem is only usable when every
/// member of its family is free.
fn family(stem: &str) -> [String; 5] {
    [stem.to_string(), format!("Is{stem}"), format!("As{stem}"), format!("_{stem}"), format!("Unmanaged{stem}")]
}

/// Allocates the name family for one enum.
fn resolve(enclosing: &str, stems: &[String]) -> Vec<VariantNames> {
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
        let fam = family(stem);
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
            let fam = family(&cand);
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
    stems
        .iter()
        .zip(chosen)
        .map(|(original, stem)| {
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

            let [factory, is_check, accessor, field, unmanaged] = family(&stem);
            VariantNames { renamed: &stem != original, stem, factory, is_check, accessor, field, unmanaged, case_type }
        })
        .collect()
}

impl Pass {
    #[must_use]
    pub fn new(_: Config) -> Self {
        Self { info: PassInfo { name: file!() }, variants: HashMap::default() }
    }

    pub fn process(&mut self, _pass_meta: &mut crate::pass::PassMeta, types: &model::common::types::all::Pass) -> ModelResult {
        let mut outcome = Unchanged;

        for (id, ty) in types.iter() {
            let TypeKind::DataEnum(data_enum) = &ty.kind else { continue };

            if self.variants.contains_key(id) {
                continue;
            }

            let stems: Vec<String> = data_enum.variants.iter().map(|v| v.name.clone()).collect();
            self.variants.insert(*id, resolve(&ty.name, &stems));
            outcome.changed();
        }

        Ok(outcome)
    }

    /// Returns the name family for `ty`, or `None` when the count does not match.
    ///
    /// Consumers must index positionally against `DataEnum::variants`. The length check
    /// exists because `zip` truncates silently: a mismatch would drop a variant from the
    /// generated output with no error anywhere. Callers should treat `None` as a
    /// generation failure, not as "no unions here".
    #[must_use]
    pub fn get(&self, ty: TypeId, variant_count: usize) -> Option<&[VariantNames]> {
        let names = self.variants.get(&ty)?;
        (names.len() == variant_count).then(|| names.as_slice())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(enclosing: &str, stems: &[&str]) -> Vec<VariantNames> {
        resolve(enclosing, &stems.iter().map(|s| (*s).to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn ordinary_enum_keeps_every_existing_name() {
        let r = names("Color", &["Red", "Green", "Blue"]);
        assert_eq!(r[0].factory, "Red");
        assert_eq!(r[0].is_check, "IsRed");
        assert_eq!(r[0].accessor, "AsRed");
        assert_eq!(r[0].field, "_Red");
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
        // claim `ValueVariant` for the variant that already emits it before pass 2 runs.
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
    fn every_emitted_name_is_unique() {
        let r = names("E", &["Value", "ValueVariant", "B", "BCase", "Foo", "IsFoo", "UnmanagedFoo", "E", "variant"]);
        let mut seen = HashSet::new();
        for v in &r {
            for n in [&v.factory, &v.is_check, &v.accessor, &v.field, &v.unmanaged, &v.case_type] {
                assert!(seen.insert(n.clone()), "duplicate emitted name: {n}");
            }
        }
        for n in RESERVED {
            assert!(!seen.contains(*n), "emitted name collides with fixed member: {n}");
        }
        assert!(!seen.contains("E"), "emitted name collides with the enclosing type");
    }
}
