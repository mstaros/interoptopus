//! Functions using all supported type patterns.

pub mod alignment;
pub mod array;
pub mod behavior;
pub mod enum_constants;
pub mod enums;
pub mod fnptrs;
pub mod generic;
// Not registered in the inventory on purpose: it exists to hand C# a value the generated
// converter must reject. See the module docs.
pub mod malformed;
pub mod meta;
pub mod modules;
pub mod primitive;
pub mod ptrs;
pub mod refs;
pub mod repr;
pub mod structs;
