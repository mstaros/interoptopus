use interoptopus::inventory::RustInventory;
use interoptopus::{ffi, extra_type, function};
use interoptopus_csharp::RustLibrary;

#[ffi]
#[derive(Clone)]
pub struct BorrowedRecord<'a> {
    pub bytes: ffi::Slice<'a, u8>,
}

#[ffi]
#[derive(Clone)]
pub struct OwnedRecord {
    pub text: ffi::String,
}

#[ffi(export = unique)]
pub fn numbers() -> ffi::Iterator<u32> {
    ffi::Iterator::new(0..10)
}

#[test]
fn registered_iterator_generates_extensions_and_standard_func() {
    let inventory = RustInventory::new().register(function!(numbers)).validate();
    let output = RustLibrary::builder(inventory).build().process().unwrap().to_string();
    assert!(output.contains("Where(this IteratorUint source, global::System.Func<uint, bool> predicate)"));
    assert!(output.contains("Take(this IteratorUint source, int count)"));
    assert!(output.contains("Any(this IteratorUint source)"));
    assert!(output.contains("delegate* unmanaged[Cdecl]"));
}

#[test]
fn owning_and_borrowed_elements_are_rejected_before_emission() {
    for inventory in [
        RustInventory::new().register(extra_type!(ffi::Iterator<ffi::String>)),
        RustInventory::new().register(extra_type!(ffi::Iterator<OwnedRecord>)),
        RustInventory::new().register(extra_type!(ffi::Iterator<ffi::Slice<'static, u8>>)),
        RustInventory::new().register(extra_type!(ffi::Iterator<BorrowedRecord<'static>>)),
        RustInventory::new().register(extra_type!(ffi::Iterator<*const u32>)),
    ] {
        let error = RustLibrary::builder(inventory.validate()).build().process().err().expect("unsafe element must be rejected");
        assert!(error.to_string().contains("Iterator predicates support"), "{error}");
    }
}
