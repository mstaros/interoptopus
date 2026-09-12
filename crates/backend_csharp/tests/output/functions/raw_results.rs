use interoptopus::inventory::RustInventory;
use interoptopus::pattern::asynk::AsyncCallback;
use interoptopus::{callback, extra_type, ffi, function, service};
use interoptopus_csharp::RustLibrary;

fn rejected(register: impl Fn(&mut RustInventory), api: &str, path: &str) {
    let inventory = RustInventory::new().register(register).validate();
    let error = RustLibrary::builder(inventory).build().process().err().expect("raw result must fail generation").to_string();
    for expected in [api, path, "raw pointer result", "SafeHandle", "Return an owned typed value"] {
        assert!(error.contains(expected), "missing {expected:?}: {error}");
    }
}

#[ffi]
#[derive(Clone, Copy)]
pub struct RawRecord { pub address: *const u8 }

#[ffi]
pub enum RawChoice { Empty, Pointer { address: *mut u8 } }

#[ffi(export = unique)]
pub fn raw_record() -> RawRecord { RawRecord { address: std::ptr::null() } }

#[ffi(export = unique)]
pub fn raw_choice() -> RawChoice { RawChoice::Empty }

#[ffi(export = unique)]
pub fn raw_option() -> ffi::Option<*const u8> { ffi::None }

#[ffi(export = unique)]
pub fn raw_result() -> ffi::Result<u32, *mut u8> { ffi::Ok(1) }

#[ffi(export = unique)]
pub fn raw_vector() -> ffi::Vec<RawRecord> { ffi::Vec::from_vec(Vec::new()) }

#[ffi]
pub struct RawArray { pub values: [RawRecord; 1] }

#[ffi(export = unique)]
pub fn raw_array() -> RawArray { RawArray { values: [RawRecord { address: std::ptr::null() }] } }

#[ffi(export = unique)]
pub fn async_pointer(_callback: AsyncCallback<*const u8>) {}

#[ffi(export = unique)]
pub fn interoptopus_user_pointer() -> *const u8 { std::ptr::null() }

callback!(RawResultCallback() -> *const u8);

#[test]
fn direct_pointers_and_rust_references_are_rejected() {
    use reference_project::functions::{ptrs, refs};
    use reference_project::patterns::primitive;
    let cases: [(fn(&mut RustInventory), &str); 7] = [
        (function!(ptrs::ptr1), "ptr1"), (function!(ptrs::ptr2), "ptr2"), (function!(ptrs::ptr3), "ptr3"),
        (function!(refs::ref1), "ref1"), (function!(refs::ref2), "ref2"),
        (function!(primitive::pattern_ffi_cchar_const_pointer), "pattern_ffi_cchar_const_pointer"),
        (function!(primitive::pattern_ffi_cchar_mut_pointer), "pattern_ffi_cchar_mut_pointer"),
    ];
    for (register, name) in cases { rejected(register, name, "return"); }
    rejected(function!(interoptopus_user_pointer), "interoptopus_user_pointer", "return");
}

#[test]
fn nested_results_identify_the_pointer_path() {
    rejected(function!(raw_record), "raw_record", "return.address");
    rejected(function!(raw_choice), "raw_choice", "return.Pointer.address");
    rejected(function!(raw_option), "raw_option", "return.Some.0");
    rejected(function!(raw_result), "raw_result", "return.Err");
    rejected(function!(raw_vector), "raw_vector", "return.element.address");
    rejected(function!(raw_array), "raw_array", "return.values[].address");
}

#[test]
fn async_and_callback_results_are_rejected() {
    rejected(function!(async_pointer), "async_pointer", "async result");
    rejected(extra_type!(RawResultCallback), "RawResultCallback", "callback return");
}

#[test]
fn service_methods_cannot_return_unowned_service_pointers() {
    #[ffi(service)]
    struct PointerService;
    #[ffi(export = unique)]
    impl PointerService {
        pub fn create() -> Self { Self }
        pub fn address(&self) -> *const PointerService { self }
    }
    rejected(service!(PointerService), "address", "return");
}

#[test]
fn constructor_error_payloads_are_not_exempt() {
    #[ffi(service)]
    struct FailingService;
    #[ffi(export = unique)]
    impl FailingService {
        pub fn create() -> ffi::Result<Self, *const u8> { ffi::Ok(Self) }
    }
    rejected(service!(FailingService), "create", "return.Err");
}

#[test]
fn owned_service_constructors_and_typed_results_still_generate() {
    #[ffi]
    pub enum Error { Failed }
    #[ffi(service)]
    struct OwnedService;
    #[ffi(export = unique)]
    impl OwnedService {
        pub fn create() -> Self { Self }
        pub fn try_create() -> ffi::Result<Self, Error> { ffi::Ok(Self) }
        pub fn read_number(&self) -> u32 { 42 }
    }
    let inventory = RustInventory::new().register(service!(OwnedService)).validate();
    let output = RustLibrary::builder(inventory).build().process().unwrap().to_string();
    assert!(output.contains(": SafeHandle"));
    assert!(output.contains("OwnedService"));
}