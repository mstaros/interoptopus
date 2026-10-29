use interoptopus::inventory::RustInventory;
use interoptopus::{ffi, service};

#[ffi]
enum Error {
    Something,
}

#[ffi(service)]
struct Service;

// Methods named like the `TypeInfo` and `ServiceInfo` functions the macros emit.
#[ffi]
impl Service {
    pub fn create() -> ffi::Result<Self, Error> {
        ffi::Ok(Self)
    }

    pub fn kind(&self) -> ffi::Result<u32, Error> {
        ffi::Ok(0)
    }

    pub fn id(&self) -> u32 {
        1
    }

    pub fn ty(&self) -> u32 {
        2
    }

    pub fn service(&self) -> u32 {
        3
    }
}

fn main() {
    let inventory = RustInventory::new().register(service!(Service)).validate();

    assert!(inventory.services.contains_key(&<Service as interoptopus::lang::service::ServiceInfo>::id()));
    assert!(inventory.types.contains_key(&<Service as interoptopus::lang::types::TypeInfo>::id()));
}