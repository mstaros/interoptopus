use interoptopus::ffi;

#[test]
fn result_variants() {
    assert!(ffi::Result::<u32, u32>::Ok(1).is_ok());
    assert!(!ffi::Result::<u32, u32>::Err(1).is_ok());
    assert!(!ffi::Result::<u32, u32>::Panic.is_ok());
    assert!(!ffi::Result::<u32, u32>::Null.is_ok());
}

#[test]
fn result_roundtrip() {
    let ffi_ok: ffi::Result<u32, u32> = Result::Ok(42).into();
    assert_eq!(ffi_ok.unwrap(), 42);
    let ffi_err: ffi::Result<u32, u32> = Result::Err(99).into();
    assert_eq!(ffi_err.unwrap_err(), 99);
}

type AliasedResult = ffi::Result<u32, u32>;

#[ffi]
fn aliased_result_panics(input: ffi::String) -> AliasedResult {
    drop(input);
    panic!("owned input dropped");
}

#[test]
fn exported_panic_becomes_result_even_through_a_type_alias() {
    let _ = interoptopus::inventory::RustInventory::new().register(interoptopus::function!(aliased_result_panics));
    let result = aliased_result_panics("owned".to_string().into());
    assert!(matches!(result, ffi::Result::Panic));
}

#[ffi]
fn bare_panic() {
    panic!("bare exports cannot represent failure");
}

#[test]
fn bare_export_panics_abort_instead_of_unwinding() {
    let _ = interoptopus::inventory::RustInventory::new().register(interoptopus::function!(bare_panic));
    const CHILD: &str = "INTEROPTOPUS_TEST_BARE_PANIC";
    if std::env::var_os(CHILD).is_some() {
        bare_panic();
        return;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "types::result::bare_export_panics_abort_instead_of_unwinding"])
        .env(CHILD, "1")
        .output()
        .unwrap();
    assert!(!output.status.success());
}
