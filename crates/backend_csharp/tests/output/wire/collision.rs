//! Item 1d regression: wire must emit the allocated stem, not the Rust variant name.
//!
//! `Issues.md` `4e9a17c3`. Nothing in the reference project collides, so `stem == name`
//! everywhere and every existing snapshot passes whether wire reads the model or the raw
//! inventory. This file supplies the case the corpus lacks.
//!
//! Asserted directly rather than snapshotted, deliberately. A snapshot records whatever the
//! generator currently emits, so it cannot fail on the day someone reverts the six emission
//! sites — it just re-accepts. That is the shape of failure this defect had for three rounds.

use interoptopus::lang::meta::FileEmission;
use interoptopus::wire::Wire;
use interoptopus::{ffi, function};
use interoptopus_csharp::config::HeaderConfig;
use interoptopus_csharp::dispatch::Dispatch;
use interoptopus_csharp::output::Target;

/// `Foo` claims the family `Foo` / `IsFoo` / `AsFoo` first, so the sibling variant
/// literally named `IsFoo` cannot keep its own name and `union_names` moves it to
/// `IsFooVariant`. See `union_names::tests::is_prefixed_variant_collides_with_a_check`.
///
/// The payload sits on the *moved* variant on purpose: that is the one whose accessor
/// `As{stem}()` wire emits, so a regression shows up in both the check and the accessor. A
/// primitive payload keeps `Choice` struct-backed, supplying item 4d's default empty state.
#[ffi]
pub enum Choice {
    Foo,
    IsFoo(u32),
}

/// A `String` field makes this struct `WireOnly`, which is what routes it — and the
/// `Choice` field with it — through `WireCodeGen` rather than the ordinary composite path.
#[ffi]
pub struct Envelope {
    pub choice: Choice,
    pub label: String,
}

#[ffi(export = unique)]
pub fn colliding_wire(mut envelope: Wire<Envelope>) -> u32 {
    envelope.unwire().label.len() as u32
}

fn generated_interop() -> String {
    let mut inventory = interoptopus::inventory::RustInventory::new();
    let _ = inventory.register(function!(colliding_wire));
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

/// Guards the premise. If `union_names` ever stops moving this stem the two tests below
/// would pass vacuously, having asserted nothing about wire at all.
#[test]
fn the_collision_actually_moves_a_stem() {
    let cs = generated_interop();
    assert!(cs.contains("IsFooVariant"), "expected union_names to move the colliding `IsFoo` stem to `IsFooVariant`");
}

#[test]
fn wire_serializer_and_sizer_use_the_stem() {
    let cs = generated_interop();

    assert!(!cs.contains(".IsIsFoo)"), "wire emitted `Is{{variant.name}}`, a member the union does not declare");
    assert!(!cs.contains(".AsIsFoo()"), "wire emitted `As{{variant.name}}`, a member the union does not declare");

    assert!(cs.contains(".IsIsFooVariant"), "wire should check the allocated stem");
    assert!(cs.contains(".AsIsFooVariant()"), "wire should access through the allocated stem");
}

#[test]
fn wire_serializer_delegates_an_empty_struct_union_to_its_classifier() {
    let cs = generated_interop();

    assert!(cs.contains("public partial struct Choice"), "the fixture must remain struct-backed so default(Choice) has an empty state");
    assert!(
        cs.contains("else { throw value.choice.ExceptionForVariant(); }"),
        "wire should use the union's shared empty-state and illegal-state classifier"
    );
    assert!(!cs.contains("InvalidOperationException(\"Unknown variant\")"), "wire must not hand-roll the empty-state exception");
}

#[test]
fn wire_deserializer_calls_the_stem_factory() {
    let cs = generated_interop();

    assert!(cs.contains("Choice.IsFooVariant"), "wire should construct through the allocated stem factory");
    assert!(!cs.contains("Choice.IsFoo("), "wire called a factory named after the Rust variant, which does not exist");
}