using System;
using System.Runtime.InteropServices;
using My.Company;
using My.Company.Common;
using Xunit;
using Interop = My.Company.Interop;

// The malformed-tag P/Invoke lives in `Bindings/MalformedFixture.cs`, not here. `Unmanaged` and
// `Marshaller` are `internal` to that assembly, so a `LibraryImport` declared in this one cannot
// use the custom marshaller at all - CS0122, measured 2026-08-28. Recorded in `Issues.md`.

/// Executed assertions for union marshalling and the discriminated-union surface.
///
/// Everything here was previously covered only by snapshot, which asserts text. Well-formed text
/// that does not compile survived 78 green tests in August 2026; well-formed code that throws at
/// runtime would survive a snapshot too.
public partial class TestPatternUnion
{
    // See `Bindings/MalformedFixture.cs` for the declaration this test calls.

    /// Item 5f: an invalid native tag throws rather than yielding a malformed managed value.
    ///
    /// Before item 4a (`056b9e4`) an unrecognised tag fell through every `if` and returned a
    /// value carrying that tag with no payload set - silently malformed, and indistinguishable
    /// from a well-formed one until something later read the wrong field.
    [Fact]
    public void an_invalid_native_tag_throws()
    {
        var ex = Assert.Throws<InteropException>(() => MalformedFixture.MalformedResultTag());

        Assert.Contains("99", ex.Message);
        Assert.Contains("ResultUintError", ex.Message);
    }

    /// `_variant` sits at `[FieldOffset(0)]` overlapping every `Unmanaged{Variant}`, so each
    /// variant struct has to carry its own leading discriminant. A payload whose low bytes are
    /// zero would survive a clobbered tag by accident; every byte of `0xDEADBEEF` is set, so this
    /// round trip cannot pass by luck.
    [Fact]
    public void round_trip_preserves_a_payload_with_every_byte_set()
    {
        var result = Interop.pattern_result_1(ResultUintError.Ok(0xDEADBEEF));
        Assert.Equal(0xDEADBEEFu, result.AsOk());
    }

    /// The marshal-out guard, asserted rather than observed. A default union is a legal C# value -
    /// the language spec requires `Value` to be null for the default of a union type - but it
    /// corresponds to no Rust variant, so it cannot cross.
    [Fact]
    public void marshalling_a_default_union_out_throws()
    {
        var ex = Assert.Throws<InvalidOperationException>(() => Interop.pattern_result_1(new ResultUintError()));
        Assert.Contains("ResultUintError", ex.Message);
    }

    /// The other direction. Without this the guard could be made unconditional and nothing would
    /// notice; before this file existed, the only evidence the happy path worked was a test written
    /// for an unrelated purpose.
    [Fact]
    public void marshalling_a_constructed_union_out_does_not_throw()
    {
        Assert.Equal(1u, Interop.pattern_result_1(ResultUintError.Ok(1)).AsOk());
    }

    /// Soundness, in the language spec's sense: the default value of a union type has a null
    /// `Value`. This is what makes the guard above necessary rather than redundant.
    [Fact]
    public void a_default_union_reports_no_value()
    {
        var empty = new ResultUintError();

        Assert.False(empty.HasValue);
        Assert.Null(empty.Value);
    }

    [Fact]
    public void a_constructed_union_exposes_its_case_through_value()
    {
        var ok = ResultUintError.Ok(7);

        Assert.True(ok.HasValue);
        Assert.Equal(new ResultUintError.OkCase(7), ok.Value);
    }

    /// `TryGetValue` is the non-boxing access pattern. It must agree with `Value`, and must return
    /// false for a case that is not active rather than yielding a defaulted payload.
    [Fact]
    public void try_get_value_selects_only_the_active_case()
    {
        var ok = ResultUintError.Ok(7);

        Assert.True(ok.TryGetValue(out ResultUintError.OkCase okCase));
        Assert.Equal(7u, okCase.Value);
        Assert.False(ok.TryGetValue(out ResultUintError.ErrCase _));
    }
}