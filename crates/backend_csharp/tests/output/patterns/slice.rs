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
    let _ = inventory.register(function!(borrow_pointer));
    let output = interoptopus_csharp::RustLibrary::builder(inventory.validate()).build().process().unwrap().to_string();
    assert!(output.contains("__span_sum_slice"));
    assert!(!output.contains("__span_borrow_slice"));
    assert!(!output.contains("__span_borrow_attribute"));
    assert!(!output.contains("__span_borrow_pointer"));
}
