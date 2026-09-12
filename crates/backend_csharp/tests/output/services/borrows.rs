use interoptopus::inventory::RustInventory;
use interoptopus::{ffi, service};
use interoptopus_csharp::RustLibrary;

#[ffi]
pub struct BorrowedRecord<'a> {
    pub bytes: ffi::Slice<'a, u8>,
}

#[test]
fn mutable_service_results_are_rejected() {
    #[ffi(service)]
    struct MutableService { bytes: Vec<u8> }
    #[ffi(export = unique)]
    impl MutableService {
        pub fn create() -> Self { Self { bytes: vec![1] } }
        pub fn bytes(&mut self) -> ffi::SliceMut<'_, u8> { ffi::SliceMut::from_slice(&mut self.bytes) }
    }
    let inventory = RustInventory::new().register(service!(MutableService)).validate();
    let error = RustLibrary::builder(inventory).build().process().err().expect("mutable borrow must be diagnosed");
    assert!(error.to_string().contains("escaping borrow"), "{error}");
    assert!(error.to_string().contains("Return owned data"), "{error}");
}

#[test]
fn nested_service_borrows_are_rejected() {
    #[ffi(service)]
    struct NestedService { bytes: Vec<u8> }
    #[ffi(export = unique)]
    impl NestedService {
        pub fn create() -> Self { Self { bytes: vec![1] } }
        pub fn bytes(&self) -> BorrowedRecord<'_> { BorrowedRecord { bytes: ffi::Slice::from_slice(&self.bytes) } }
    }
    let inventory = RustInventory::new().register(service!(NestedService)).validate();
    let error = RustLibrary::builder(inventory).build().process().err().expect("nested borrow must be diagnosed");
    assert!(error.to_string().contains("escaping borrow"), "{error}");
}

#[test]
fn readonly_service_results_are_copied_before_releasing_the_call() {
    #[ffi(service)]
    struct CopyService { bytes: Vec<u8> }
    #[ffi(export = unique)]
    impl CopyService {
        pub fn create() -> Self { Self { bytes: vec![1] } }
        pub fn bytes(&self) -> ffi::Slice<'_, u8> { ffi::Slice::from_slice(&self.bytes) }
    }
    let inventory = RustInventory::new().register(service!(CopyService)).validate();
    let output = RustLibrary::builder(inventory).build().process().unwrap().to_string();
    let copy = output.find(".__Copy();").expect("copy service result");
    let release = output[copy..].find(".__ReleaseCall(").expect("release call after copying");
    assert!(release > 0);
}