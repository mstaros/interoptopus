//! Item 1d regression, type-name half: wire must emit the model's C# name, not the raw
//! Rust inventory string.
//!
//! `Issues.md` `4e9a17c3`. Every path through `names.rs` applies `sanitize_rust_name`;
//! `cs_type_name` used to return `ty.name.clone()` for `Struct` and `Enum`, skipping it.
//! Invisible on the current corpus because `sanitize_rust_name` is identity for ordinary
//! Rust identifiers — so the corpus needs a name where it is not.
//!
//! `Parcel<u32>` is that name: the inventory records the instantiated generic verbatim, and
//! `sanitize_rust_name("Parcel<u32>")` is `ParcelU32`. The raw form is not even valid C# in
//! this position, so emitting it produces a compile error in generated code — the symptom
//! `4e9a17c3` predicts.
//!
//! Deliberately scoped to a struct with **no** `WireOnly` fields. Structs that do carry them
//! are registered by `wire::nested`, which sets their model name to `rust_ty.name.clone()`
//! raw, bypassing `names.rs` — a second naming authority inside the model, tracked
//! separately. Asserting on one of those would test that gap rather than this fix.

use interoptopus::lang::meta::FileEmission;
use interoptopus::lang::types::TypeInfo;
use interoptopus::wire::Wire;
use interoptopus::{ffi, function};
use interoptopus_csharp::config::HeaderConfig;
use interoptopus_csharp::dispatch::Dispatch;
use interoptopus_csharp::output::Target;

/// All fields FFI-safe, so this goes through the ordinary `struct_fields` path and gets its
/// C# name from `names.rs` — which is the authority wire is being made to consult.
#[ffi]
pub struct Parcel<T: TypeInfo> {
    pub weight: T,
}

/// The `String` field makes this a `WireOnly` composite, which is what pulls `Parcel<u32>`
/// into the wire type graph with it.
#[ffi]
pub struct Manifest {
    pub parcel: Parcel<u32>,
    pub label: String,
}

#[ffi(export = unique)]
pub fn sanitized_wire(mut manifest: Wire<Manifest>) -> bool {
    manifest.unwire().label.is_empty()
}

fn generated_interop() -> String {
    let mut inventory = interoptopus::inventory::RustInventory::new();
    let _ = inventory.register(function!(sanitized_wire));
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

/// Guards the premise: if the inventory ever stops recording the instantiated generic
/// verbatim, `sanitize_rust_name` becomes identity here and the test below proves nothing.
#[test]
fn the_rust_name_actually_needs_sanitizing() {
    assert_eq!(interoptopus_backends::casing::sanitize_rust_name("Parcel<u32>"), "ParcelU32");
}

#[test]
fn wire_emits_the_sanitized_model_name() {
    let cs = generated_interop();

    assert!(cs.contains("ParcelU32"), "wire should emit the C# name the model resolved");
    assert!(!cs.contains("Parcel<u32>"), "wire emitted the raw Rust inventory name, which is not a valid C# identifier here");
}