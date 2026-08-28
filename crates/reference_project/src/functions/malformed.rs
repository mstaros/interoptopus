//! A deliberately malformed native value, for asserting the tag-validation arm in generated C#.
//!
//! `Unmanaged.ToManaged()` ends in a default arm that throws `InteropException` when the
//! discriminant matches no variant — item 4a, `056b9e4`. Nothing exercised it, because there was
//! no way to produce such a value.
//!
//! Every obvious way of producing one is undefined behaviour. A Rust enum value must always be
//! one of its declared variants, so `transmute`-ing 99 into a `Result<u32, Error>` breaks a
//! promise the compiler relies on: it may optimise the throw arm away on the grounds that the
//! value cannot exist, and the test then measures luck rather than behaviour.
//!
//! So this does not build a bad enum. It builds an ordinary struct with the same layout and puts
//! an ordinary integer in the tag slot. An integer holding 99 is entirely legal; nothing here is
//! unsound. The mismatch lives on the C# side, in a hand-written `LibraryImport` that declares
//! the return type as the enum — see `Test.Pattern.Union.cs`.
//!
//! Layout verified against emitted code, not assumed (`Bindings/Interop.cs:9750-9793`):
//! `Unmanaged` is `LayoutKind.Explicit` with `uint _variant`, `UnmanagedOk { uint; uint }` and
//! `UnmanagedErr { uint; Error }` all at offset 0, and `Error` is `: byte`. Eight bytes,
//! `[u32 tag][u32 payload]`, valid tags 0 through 3.
//!
//! Deliberately **not** `#[ffi]`. Registering it would put it in the inventory and generate a
//! binding returning `MalformedResult`, and the enum's converter would never run. Keeping it out
//! confines the misdeclaration to the one test that needs it.

/// Mirrors the unmanaged layout of a `Result<u32, Error>` as projected into C#.
#[repr(C)]
pub struct MalformedResult {
    /// Discriminant slot. Valid values are 0..=3; this fixture emits one that is not.
    pub variant: u32,
    /// Payload slot. Never read — the C# converter throws before reaching it.
    pub payload: u32,
}

/// A tag no variant of `Result<u32, Error>` uses.
const INVALID_TAG: u32 = 99;

/// Returns a value shaped like `Result<u32, Error>` carrying an out-of-range discriminant.
///
/// Exported by name rather than through the inventory; the only caller is the C# test.
#[unsafe(no_mangle)]
pub extern "C" fn reference_malformed_result_tag() -> MalformedResult {
    MalformedResult { variant: INVALID_TAG, payload: 0 }
}