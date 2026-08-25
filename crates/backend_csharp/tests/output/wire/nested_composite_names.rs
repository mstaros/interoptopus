//! `wire::nested` must name the composites it registers the way `names.rs` would.
//!
//! Structs carrying `WireOnly` fields are skipped by `struct_fields` and registered instead
//! by `wire::nested`, which used to write `rust_ty.name.clone()` — the raw inventory string.
//! `names.rs:72` is first-write-wins, so that raw name won and became the model's answer for
//! the type.
//!
//! For an instantiated generic the inventory string is `Boxed<u32>`, which is not a legal C#
//! identifier. It reached the generated file in three places, one of them a declaration:
//!
//! ```text
//!     public required Boxed<u32> inner;
//!         result.inner = (Boxed<u32>)...GetUninitializedObject(typeof(Boxed<u32>));
//! public partial class Boxed<u32>
//! ```
//!
//! A live compile error in generated code, not a latent one — found while implementing item
//! 1d (`Issues.md` `4e9a17c3`) because routing wire through the model is what made the model's
//! own answer observable. Wire was reproducing it faithfully; the defect was one layer down.
//!
//! Fixed by applying `sanitize_rust_name` at the write site, matching the arm `names.rs:98`
//! already uses for `WireOnly::Composite`. `nested` writing names at all is a second naming
//! authority and should eventually go — the naming pass should own it and the convergence
//! loop should carry it — but that is a larger change than this one needs to be.

use interoptopus::lang::meta::FileEmission;
use interoptopus::lang::types::TypeInfo;
use interoptopus::wire::Wire;
use interoptopus::{ffi, function};
use interoptopus_csharp::config::HeaderConfig;
use interoptopus_csharp::dispatch::Dispatch;
use interoptopus_csharp::output::Target;

/// Generic, so the inventory name is not a valid C# identifier, and carries a `String`, so
/// `struct_fields` skips it and `wire::nested` is what registers it. Both properties are
/// required to reach the defect.
#[ffi]
pub struct Boxed<T: TypeInfo> {
    pub weight: T,
    pub note: String,
}

#[ffi]
pub struct Shipment {
    pub inner: Boxed<u32>,
    pub label: String,
}

#[ffi(export = unique)]
pub fn nested_generic_wire(mut shipment: Wire<Shipment>) -> bool {
    shipment.unwire().label.is_empty()
}

fn generated_interop() -> String {
    let mut inventory = interoptopus::inventory::RustInventory::new();
    let _ = inventory.register(function!(nested_generic_wire));
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

/// Guards the premise: if the inventory stops recording the instantiated generic verbatim,
/// there is no unsanitized name left to get wrong and the assertions below prove nothing.
#[test]
fn the_nested_composite_name_actually_needs_sanitizing() {
    assert_eq!(interoptopus_backends::casing::sanitize_rust_name("Boxed<u32>"), "BoxedU32");
}

#[test]
fn nested_wire_composites_get_a_valid_csharp_identifier() {
    let cs = generated_interop();

    assert!(!cs.contains("Boxed<u32>"), "the raw inventory name reached the generated C#, which does not compile");
    assert!(cs.contains("public partial class BoxedU32"), "the helper class should be declared under the sanitized name");
    assert!(cs.contains("typeof(BoxedU32)"), "wire should deserialize through the sanitized name");
}