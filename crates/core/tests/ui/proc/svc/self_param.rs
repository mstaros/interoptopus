use interoptopus::inventory::RustInventory;
use interoptopus::{ffi, service};

#[ffi]
enum Error {
    Something,
}

#[ffi(service)]
struct Service;

// `Self` in parameter types must name the service in the generated free functions.
#[ffi]
impl Service {
    pub fn create() -> ffi::Result<Self, Error> {
        ffi::Ok(Self)
    }

    pub fn from_other(other: &Self) -> ffi::Result<Self, Error> {
        let _ = other;
        ffi::Ok(Self)
    }

    pub fn merge(&self, other: &Self) -> u32 {
        let _ = other;
        0
    }
}

fn main() {
    let inventory = RustInventory::new().register(service!(Service)).validate();

    assert!(inventory.functions.contains_key(&<service_from_other as interoptopus::lang::function::FunctionInfo>::id()));
    assert!(inventory.functions.contains_key(&<service_merge as interoptopus::lang::function::FunctionInfo>::id()));
}