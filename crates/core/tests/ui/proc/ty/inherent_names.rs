#![allow(dead_code)]

use interoptopus::inventory::RustInventory;
use interoptopus::{extra_type, ffi};

#[ffi]
struct Foo {
    x: u8,
}

// Associated functions and methods named like the `TypeInfo` functions the macro emits.
impl Foo {
    fn id() -> u32 {
        0
    }

    fn kind(&self) -> u32 {
        u32::from(self.x)
    }

    fn ty() -> u32 {
        0
    }
}

#[ffi]
enum Bar {
    A,
    B(u8),
}

impl Bar {
    fn id(&self) -> u32 {
        0
    }

    fn kind() -> u32 {
        0
    }

    fn ty(&self) -> u32 {
        0
    }
}

fn main() {
    let inventory = RustInventory::new().register(extra_type!(Foo)).register(extra_type!(Bar)).validate();

    assert!(inventory.types.contains_key(&<Foo as interoptopus::lang::types::TypeInfo>::id()));
    assert!(inventory.types.contains_key(&<Bar as interoptopus::lang::types::TypeInfo>::id()));
}