//! Tests for the `#[ffi]` type macro.

use super::ffi;
use quote::quote;

/// A declared `repr` used to be removed silently by `add_repr_attribute`.
///
/// The cost of that silence, concretely: a caller wrote `#[repr(u32)]`, saw
/// `: byte` in the generated C#, and had nothing connecting the two. The macro
/// is entitled to own the representation - it must say so.
#[test]
fn a_declared_repr_on_an_enum_is_rejected() {
    ffi(
        quote! {},
        quote! {
            pub enum Outcome {
                Applied,
                Mismatch,
            }
        },
    )
    .expect("an enum that declares no repr is unaffected");

    let error = ffi(
        quote! {},
        quote! {
            #[repr(u32)]
            pub enum Outcome {
                Applied,
                Mismatch,
            }
        },
    )
    .expect_err("a declared repr must not be accepted");

    assert!(
        error.to_string().contains("chooses the representation itself"),
        "the message must explain why, not merely fail: {error}"
    );
}

/// Structs are covered for the same reason: the macro enforces `#[repr(C)]` on
/// them, or `#[ffi(packed)]` / `#[ffi(transparent)]` / `#[ffi(opaque)]`.
#[test]
fn a_declared_repr_on_a_struct_is_rejected() {
    ffi(
        quote! {},
        quote! {
            #[repr(packed)]
            pub struct Point {
                pub x: f32,
                pub y: f32,
            }
        },
    )
    .expect_err("a declared repr must not be accepted on a struct either");
}

/// Services are exempt. `add_repr_attribute` returns early for them and never
/// substituted a representation in the first place, so there is nothing to
/// conflict with and the declared one is theirs to keep.
#[test]
fn a_service_keeps_its_declared_repr() {
    ffi(
        quote! { service },
        quote! {
            #[repr(C)]
            pub struct Handle {
                slot: u64,
            }
        },
    )
    .expect("services are exempt from the repr rule");
}
