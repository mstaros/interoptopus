// Hand-written, and deliberately in this project rather than in Tests.
//
// `ResultUintError.Unmanaged` and `.Marshaller` are emitted `internal`, so the `LibraryImport`
// source generator can only wire the custom marshaller from inside this assembly. Declaring the
// same P/Invoke in Tests fails: without `DisableRuntimeMarshalling` the generator falls back to
// runtime marshalling and reports SYSLIB1051; with it, the fallback is refused and the real cause
// surfaces as five CS0122 "inaccessible due to its protection level" on `Unmanaged`, `Marshaller`,
// `FromUnmanaged`, `ToManaged` and `Free`. Both measured 2026-08-28.
//
// Return-by-value is not the constraint - `Interop.pattern_result_1` returns this same type by
// value from this assembly and has always compiled. Accessibility is. Recorded in `Issues.md`.
//
// This file is not generated. `Bindings/` gitignores only `Interop*.cs`.

using System.Runtime.InteropServices;

namespace My.Company;

/// Native values that are deliberately malformed, for asserting the generated tag validation.
public static partial class MalformedFixture
{
    /// Returns bytes shaped like `ResultUintError` carrying a discriminant no variant uses.
    ///
    /// The native side is an ordinary `#[repr(C)] { u32 tag; u32 payload; }` holding 99 — see
    /// `crates/reference_project/src/functions/malformed.rs`. Nothing on the Rust side builds a
    /// malformed enum, which would be undefined behaviour and could entitle the compiler to
    /// delete the very arm this exists to reach. The mismatch is confined to this declaration.
    [LibraryImport("reference_project", EntryPoint = "reference_malformed_result_tag")]
    public static partial ResultUintError MalformedResultTag();
}