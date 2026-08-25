use crate::types::basic::Vec3f32;
use interoptopus::ffi;

/// Documented enum.
#[ffi]
pub enum EnumDocumented {
    /// Variant A.
    A,
    /// Variant B.
    B,
    /// Variant B.
    C,
}

#[ffi(name = "EnumRenamed")]
#[derive(Debug)]
pub enum EnumRenamedXYZ {
    X,
}

#[ffi]
#[derive(Clone)]
pub enum EnumPayload {
    A,
    B(Vec3f32),
    C(u32),
    // We don't support these for now
    // D { x: Vec3f32 },
    // E(u8, u8, u8),
}

#[ffi]
pub enum EnumNegative {
    A = -1,
    B = -0,
    C = 1,
}

/// Explicit discriminant followed by implicit ones.
///
/// Rust assigns 5, 6, 7 here: an implicit discriminant is the previous one plus one.
/// Present to pin down what the generator actually assigns. See `Issues.md` `09b82d44`.
#[ffi]
pub enum EnumExplicitThenImplicit {
    A = 5,
    B,
    C,
}

/// Explicit discriminants mixed with payload-carrying variants.
///
/// Rust assigns 10, 11, 12, 20. This is the only reference enum that gives a
/// payload variant a real discriminant: `EnumNegative` is explicit but unit-only,
/// and `EnumPayload` has payloads but no explicit discriminants. Without this,
/// nothing distinguishes a variant's tag from its positional index.
/// See `Issues.md` `09b82d44` defects 2 and 3.
#[ffi]
#[derive(Clone)]
pub enum EnumExplicitPayload {
    A = 10,
    B(u32),
    C(Vec3f32),
    D = 20,
}
