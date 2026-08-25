//! Wire must not treat a class-backed enum as a C# value type.
//!
//! `Choice` carries a `String`, so its managed conversion is `Into`, so `struct_class` emits it
//! as a class — the same category `OptionUtf8String` and owned `Result` types fall into by
//! design (`docs/csharp-unions.md` §1). `is_cs_value_type` answered `Enum(_) => true` for every
//! enum regardless, so `Option<Choice>` inside a `Wire<T>` emitted `.HasValue` and `.Value` on a
//! reference type: eight uncompilable lines across the serializer, deserializer and sizer.
//!
//! Recorded as `Issues.md` `31248473`, predicted but unmeasured until this test reproduced it.
//!
//! The question is sharper than "struct or class". C# 15 unions must declare `HasValue` and
//! `Value` (`docs/csharp-unions.md` §"Reserved names"), and wire uses those same identifiers to
//! mean `Nullable<T>`'s members. When the enum is a struct, `Nullable<T>` shadows them and wire
//! is right. When it is a class, nothing shadows, and after Step 3c `.HasValue` would bind to
//! the union's own constant-`true` member instead — turning today's compile error into a silent
//! one. That is why this is fixed before 3c rather than with it.

use interoptopus::lang::meta::FileEmission;
use interoptopus::wire::Wire;
use interoptopus::{ffi, function};
use interoptopus_csharp::config::HeaderConfig;
use interoptopus_csharp::dispatch::Dispatch;
use interoptopus_csharp::output::Target;

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

#[ffi(export = unique)]
pub fn option_enum_wire(mut holder: Wire<Holder>) -> bool {
    holder.unwire().label.is_empty()
}

fn generated_interop() -> String {
    let mut inventory = interoptopus::inventory::RustInventory::new();
    let _ = inventory.register(function!(option_enum_wire));
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

/// Guards the premise. If `Choice` ever becomes struct-backed this fixture stops exercising the
/// defect, and the test below would pass for the wrong reason.
#[test]
fn the_enum_is_actually_class_backed() {
    let cs = generated_interop();
    assert!(cs.contains("class Choice"), "fixture no longer class-backed; the test below proves nothing");
}

#[test]
fn class_backed_enum_in_an_option_uses_a_null_check() {
    let cs = generated_interop();

    let value_type_access: Vec<&str> = cs
        .lines()
        .map(str::trim)
        .filter(|l| l.contains("choice.HasValue") || l.contains("choice.Value"))
        .collect();

    assert!(value_type_access.is_empty(), "wire treated a class as a value type: {value_type_access:#?}");
    assert!(cs.contains("value.choice != null"), "expected a null check for a reference type");
}