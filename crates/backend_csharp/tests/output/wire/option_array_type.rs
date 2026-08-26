//! Wire must not treat a fixed-size array as a C# value type.
//!
//! `[u8; 4]` maps to `byte[]`, and C# arrays are reference types. `is_cs_value_type` answered
//! `Array(_) => true`, so `Option<[u8; 4]>` inside a `Wire<T>` emitted `.HasValue` and `.Value`
//! on a `byte[]`, which has neither.
//!
//! Corrected alongside `Issues.md` `31248473`, but *unverified* at the time: no snapshot moved,
//! because nothing in the reference corpus builds this shape. This is the missing execution.
//!
//! Companion to `option_value_type.rs`, which covers the same defect for a class-backed enum.
//! Two arms of one match, two reasons, two tests.

use interoptopus::lang::meta::FileEmission;
use interoptopus::wire::Wire;
use interoptopus::{ffi, function};
use interoptopus_csharp::config::HeaderConfig;
use interoptopus_csharp::dispatch::Dispatch;
use interoptopus_csharp::output::Target;

#[ffi]
pub struct ArrayHolder {
    pub bytes: Option<[u8; 4]>,
    pub label: String,
}

#[ffi(export = unique)]
pub fn option_array_wire(mut holder: Wire<ArrayHolder>) -> bool {
    holder.unwire().label.is_empty()
}

fn generated_interop() -> String {
    let mut inventory = interoptopus::inventory::RustInventory::new();
    let _ = inventory.register(function!(option_array_wire));
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

/// Guards the premise: if the field stops being an array-typed option this proves nothing.
#[test]
fn the_field_is_actually_an_array_option() {
    let cs = generated_interop();
    assert!(cs.contains("byte[]"), "fixture no longer emits an array; the test below is vacuous");
}

#[test]
fn array_option_uses_a_null_check_not_has_value() {
    let cs = generated_interop();

    let value_type_access: Vec<&str> = cs
        .lines()
        .map(str::trim)
        .filter(|l| l.contains("bytes.HasValue") || l.contains("bytes.Value"))
        .collect();

    assert!(value_type_access.is_empty(), "wire treated a C# array as a value type: {value_type_access:#?}");
    assert!(cs.contains("value.bytes != null"), "expected a null check for a reference type");
}