// Hand-written in the consumer assembly to pin the accessibility contract for generated custom
// marshallers. If the unmanaged mirror or either marshaller becomes non-public, LibraryImport
// source generation in this project fails with CS0122 before the runtime tests can execute.

using System.Runtime.InteropServices;
using System.Runtime.InteropServices.Marshalling;

[assembly: System.Runtime.CompilerServices.DisableRuntimeMarshalling]

namespace My.Company;

/// Consumer-owned native declarations for marshaller accessibility and malformed-tag validation.
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

    /// Borrows a generated union through a LibraryImport owned by this consumer assembly.
    [LibraryImport("reference_project", EntryPoint = "pattern_result_borrow")]
    public static partial void BorrowResult(
        [MarshalUsing(typeof(ResultUintError.InMarshallerMeta))] in ResultUintError result);
}
