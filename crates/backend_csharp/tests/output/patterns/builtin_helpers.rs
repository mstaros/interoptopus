use interoptopus::inventory::RustInventory;
use interoptopus::lang::types::TypeInfo;
use interoptopus::{extra_type, ffi};
use interoptopus_csharp::RustLibrary;

#[ffi]
pub struct Layer1<T: TypeInfo> {
    pub maybe_2: ffi::Vec<T>,
}

#[ffi]
pub enum VecPayload {
    Empty,
    Values(ffi::Vec<u16>),
}

fn generation_error(inventory: RustInventory) -> String {
    match RustLibrary::builder(inventory).build().process() {
        Ok(_) => panic!("generation unexpectedly succeeded without the required builtin helpers"),
        Err(error) => error.to_string(),
    }
}

/// The measured failure was not a direct function parameter: `ffi::Vec<T>` arrived through a
/// concrete instantiation of a generic composite. Keep that route explicit so this cannot be
/// simplified into a direct-parameter test that loses the transitive coverage.
#[test]
fn missing_vec_helpers_reached_through_generic_composite_instantiation_are_reported() {
    let mut inventory = RustInventory::new();
    let _ = inventory.register(extra_type!(Layer1<u8>));
    let error = generation_error(inventory.validate());

    assert!(error.contains("Cannot emit C# type `VecByte`"), "{error}");
    assert!(error.contains("collected transitively through a field or enum payload"), "{error}");
    assert!(error.contains("Register `builtins_vec!(T)`"), "{error}");
}

#[test]
fn missing_vec_helpers_reached_through_an_enum_payload_are_reported() {
    let mut inventory = RustInventory::new();
    let _ = inventory.register(extra_type!(VecPayload));
    let error = generation_error(inventory.validate());

    assert!(error.contains("Cannot emit C# type `VecUshort`"), "{error}");
    assert!(error.contains("enum payload"), "{error}");
    assert!(error.contains("Register `builtins_vec!(T)`"), "{error}");
}

#[test]
fn missing_utf8string_helpers_are_reported_explicitly() {
    let mut inventory = RustInventory::new();
    let _ = inventory.register(extra_type!(ffi::String));
    let error = generation_error(inventory.validate());

    assert!(error.contains("Cannot emit C# type `Utf8String`"), "{error}");
    assert!(error.contains("create, destroy, and clone helpers were not all registered"), "{error}");
    assert!(error.contains("Register `builtins_string!()`"), "{error}");
    assert!(!error.starts_with("Template error:"), "the old failure was an incidental missing-template-context error: {error}");
}