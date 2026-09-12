use interoptopus::{ffi, function};

#[ffi]
#[derive(Clone)]
pub struct Attribute<'a> {
    pub bytes: ffi::Slice<'a, u8>,
}

#[ffi(export = unique)]
pub fn sum_slice(values: ffi::Slice<u32>) -> u32 {
    values.iter().sum()
}

#[ffi(export = unique)]
pub fn count_attributes(values: ffi::Slice<Attribute>) -> u32 {
    values.len() as u32
}

#[test]
fn basic() {
    test_output!("Interop.cs", [function!(sum_slice)]);
}

#[test]
fn non_blittable() {
    test_output!("Interop.cs", [function!(count_attributes)]);
}

#[ffi(export = unique)]
pub fn borrow_slice(values: ffi::Slice<u32>) -> ffi::Slice<u32> {
    values
}

#[ffi(export = unique)]
pub fn borrow_attribute(values: ffi::Slice<'_, u8>) -> Attribute<'_> {
    Attribute { bytes: values }
}

#[ffi(export = unique)]
pub fn borrow_pointer(values: ffi::Slice<u32>) -> *const u32 {
    values.as_slice().as_ptr()
}

#[test]
fn span_overloads_do_not_return_borrowed_memory() {
    let mut inventory = interoptopus::inventory::RustInventory::new();
    let _ = inventory.register(function!(sum_slice));
    let _ = inventory.register(function!(borrow_slice));
    let _ = inventory.register(function!(borrow_attribute));
    let output = interoptopus_csharp::RustLibrary::builder(inventory.validate()).build().process().unwrap().to_string();
    assert!(output.contains("__span_sum_slice"));
    assert!(!output.contains("__span_borrow_slice"));
    assert!(!output.contains("__span_borrow_attribute"));

}

#[test]
fn raw_pointer_result_is_rejected_before_span_emission() {
    let inventory = interoptopus::inventory::RustInventory::new().register(function!(borrow_pointer)).validate();
    let error = interoptopus_csharp::RustLibrary::builder(inventory).build().process().err().expect("raw pointer result must be rejected");
    assert!(error.to_string().contains("raw pointer result"), "{error}");
    assert!(error.to_string().contains("borrow_pointer"), "{error}");
}
