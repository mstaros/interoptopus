//! Item 3a: a class-backed enum gets a private parameterless constructor.
//!
//! `docs/csharp-unions.md` Step 3 "Class-backed". There is no empty class instance —
//! `default(E)` is a null reference and every non-null instance is valid. Today's *implicit*
//! public parameterless constructor breaks that: `new EnumX()` produces a variant-zero
//! instance from outside the type, which is not a variant the Rust side ever sent.
//!
//! Declaring any constructor removes the implicit one, so a single `private E() { }` is the
//! whole fix. Internal construction is unaffected — factories and `Unmanaged.ToManaged()`
//! construct from inside the type, and a nested type may call the enclosing type's private
//! members.
//!
//! Struct-backed enums are deliberately untouched. A struct always has a default value
//! reachable as `default(E)`, so a private constructor would buy nothing there; the empty
//! state is handled instead by `_hasValue` (`8c70868d`) and item 4.
//!
//! **This is a breaking change, unconditionally.** External `new EnumX()` stops compiling for
//! every class-backed enum.

use interoptopus::lang::meta::FileEmission;
use interoptopus::wire::Wire;
use interoptopus::{extra_type, ffi, function};
use interoptopus_csharp::config::HeaderConfig;
use interoptopus_csharp::dispatch::Dispatch;
use interoptopus_csharp::output::Target;

/// Carries a `String`, so its managed conversion is `Into`, so `struct_class` emits it as a
/// class. Same fixture shape as `tests/output/wire/option_value_type.rs`.
#[ffi]
pub enum Choice {
    Empty,
    Text(String),
}

#[ffi]
pub struct Holder {
    pub choice: Option<Choice>,
    pub label: String,
}

/// Blittable payload, so this stays struct-backed.
#[ffi]
pub enum Plain {
    Nothing,
    Num(u32),
}

#[ffi(export = unique)]
pub fn class_ctor_wire(mut holder: Wire<Holder>) -> bool {
    holder.unwire().label.is_empty()
}

fn generated_interop() -> String {
    let mut inventory = interoptopus::inventory::RustInventory::new();
    let _ = inventory.register(function!(class_ctor_wire));
    let _ = inventory.register(extra_type!(Plain));
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

/// Guards the premise. If `Choice` ever stops being class-backed, or `Plain` stops being
/// struct-backed, the assertions below prove nothing — a missing constructor and an absent
/// type look identical to a `contains` check.
#[test]
fn the_fixtures_are_actually_class_and_struct_backed() {
    let cs = generated_interop();
    assert!(cs.contains("class Choice"), "Choice is no longer class-backed; the ctor test proves nothing");
    assert!(cs.contains("struct Plain"), "Plain is no longer struct-backed; the negative test proves nothing");
}

#[test]
fn a_class_backed_enum_gets_a_private_parameterless_ctor() {
    let cs = generated_interop();

    assert!(
        cs.contains("private Choice() { }"),
        "class-backed enum must declare a private parameterless constructor, or `new Choice()` \
         still produces a variant-zero instance from outside the type"
    );
}

#[test]
fn a_struct_backed_enum_does_not_get_one() {
    let cs = generated_interop();

    assert!(
        !cs.contains("private Plain()"),
        "struct-backed enums are handled by `_hasValue` and item 4, not by constructor visibility"
    );
}