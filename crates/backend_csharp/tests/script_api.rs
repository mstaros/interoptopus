#![allow(dead_code)]

use interoptopus::inventory::RustInventory;
use interoptopus::pattern::asynk::Async;
use interoptopus::rt::Tokio;
use interoptopus::{AsyncRuntime, ffi, service};
use interoptopus_csharp::RustLibrary;

#[ffi(service)]
#[derive(AsyncRuntime)]
pub struct NamingCollision {
    runtime: Tokio,
}

#[ffi(export = unique)]
impl NamingCollision {
    pub fn create() -> Self {
        Self { runtime: Tokio::new() }
    }

    pub async fn lookup(_: Async<Self>) -> u32 {
        1
    }

    pub fn lookup_async(&self) -> u32 {
        2
    }
}

#[test]
fn rejects_collision_between_async_suffix_and_existing_method() {
    let mut inventory = RustInventory::new();
    inventory.register(service!(NamingCollision));
    let error = match RustLibrary::builder(inventory).build().process() {
        Ok(_) => panic!("colliding managed method names must be rejected"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("conflicting generated member LookupAsync"), "{error}");
}

#[test]
fn async_constructor_suffix_is_not_duplicated() -> Result<(), Box<dyn std::error::Error>> {
    let bindings = RustLibrary::builder(reference_project::inventory()).build().process()?;
    let text = bindings.to_string();
    assert!(text.contains("Task<ServiceAsyncCtor> NewAsync("));
    assert!(!text.contains("NewAsyncAsync("));
    assert!(text.contains("Task<ulong> ReturnAfterMsAsync("));
    assert!(text.contains("service_async_sleep_return_after_ms("));
    Ok(())
}
