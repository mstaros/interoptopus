//! The union eligibility rule: a `DataEnum` with no payload-carrying variant receives no
//! union machinery.
//!
//! `docs/csharp-unions.md` §"Two layers, two rules". The rule is anti-bloat and does not
//! depend on how common such enums are: a payload-free enum has nothing to put in a case
//! type, so its case types would all be empty, its `Value` would be nothing, and
//! `TryGetValue` would have nothing to get.
//!
//! These tests preserve plain enums and mixed enums with only one genuine unit
//! variant. The companion `enum_constants` tests cover automatic grouping of
//! two or more true unit variants and its enum-base and empty-payload limits.
//! Grouping preserves payload case identity while moving constants into enum-value
//! patterns. This supersedes the earlier decision in `Issues.md` `79be256e`.
//! The single-unit `Carrier` fixture below still retains its empty `GammaCase`.
//!
//! Without this test the rule is guarded only by the reference snapshot, where its effect
//! is an *absence*. Deleting the guard in `body_case_types.rs` would move a 15,000-line
//! snapshot rather than fail an assertion naming the invariant.

use interoptopus::lang::meta::FileEmission;
use interoptopus::{extra_type, ffi};
use interoptopus_csharp::config::HeaderConfig;
use interoptopus_csharp::dispatch::Dispatch;
use interoptopus_csharp::output::Target;

/// Pure unit-only. Must receive no case types at all.
#[ffi]
pub enum Scalar {
    Alpha,
    Beta,
}

/// Mixed. Must receive a case type for *both* variants, including the unit one.
#[ffi]
pub enum Carrier {
    Gamma,
    Delta(u32),
}

fn generated_interop() -> String {
    let mut inventory = interoptopus::inventory::RustInventory::new();
    let _ = inventory.register(extra_type!(Scalar));
    let _ = inventory.register(extra_type!(Carrier));
    let inventory = inventory.validate();

    let multibuf = interoptopus_csharp::RustLibrary::builder(inventory)
        .dispatch(Dispatch::custom(|x, _| match x.emission {
            FileEmission::Common => Target::new("Interop.Common.cs", "My.Company.Common"),
            FileEmission::Default | FileEmission::CustomModule(_) => Target::new("Interop.cs", "My.Company"),
        }))
        .headers(HeaderConfig { emit_version: false })
        .build()
        .process()
        .unwrap();

    multibuf.buffer("Interop.cs").unwrap().to_string()
}

/// Guards the premise. Both fixtures must actually reach the output, or the negative
/// assertion below passes for the wrong reason — an enum that was never emitted also
/// contains no case types.
#[test]
fn both_fixtures_are_actually_emitted() {
    let cs = generated_interop();
    assert!(cs.contains("Scalar"), "Scalar never reached the output; the eligibility test proves nothing");
    assert!(cs.contains("Carrier"), "Carrier never reached the output; the emission test proves nothing");
}

#[test]
fn a_unit_only_enum_gets_no_case_types() {
    let cs = generated_interop();

    let leaked: Vec<&str> = cs.lines().map(str::trim).filter(|l| l.contains("AlphaCase") || l.contains("BetaCase")).collect();

    assert!(leaked.is_empty(), "unit-only enum received union machinery: {leaked:#?}");
}

#[test]
fn a_mixed_enum_gets_a_case_type_for_every_variant_including_unit_ones() {
    let cs = generated_interop();

    assert!(
        cs.contains("public readonly record struct GammaCase();"),
        "unit variant of a union-projected enum must still get an empty case type; \
         that is what keeps a mixed enum exhaustive"
    );
    assert!(
        cs.contains("public readonly record struct DeltaCase(uint Value);"),
        "payload variant must get a positional record parameter, which is what supplies \
         both the single-parameter constructor (item 3e) and pattern deconstruction"
    );
}

/// Case types are `public` at the emission site, not inherited. A nested type in C#
/// defaults to `private`, which is unusable rather than merely suboptimal: the case type
/// could not be named outside the union, so pattern matching could not mention it.
/// `internal` fails the same way across an assembly boundary. Item 3f.
#[test]
fn case_types_are_public() {
    let cs = generated_interop();

    let non_public: Vec<&str> = cs
        .lines()
        .map(str::trim)
        .filter(|l| l.contains("record struct GammaCase") || l.contains("record struct DeltaCase"))
        .filter(|l| !l.starts_with("public "))
        .collect();

    assert!(non_public.is_empty(), "case types must be public, stated explicitly: {non_public:#?}");
}