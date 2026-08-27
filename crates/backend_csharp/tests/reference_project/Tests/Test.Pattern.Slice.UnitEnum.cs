using My.Company;
using My.Company.Common;
using Xunit;
using Interop = My.Company.Interop;

/// <summary>
/// Answers the question left open in <c>docs/csharp-unions-handoff.md</c> §9, step three:
/// does a unit-only enum survive the FFI boundary now that it is projected as a plain C#
/// <c>enum</c> and its slice has moved onto the pinning path?
///
/// After <c>ca6aafa</c>, <c>EnumDocumented</c> is emitted as <c>enum EnumDocumented : byte</c>,
/// its <c>ManagedConversion</c> is <c>AsIs</c>, and <c>SliceEnumDocumented</c> is rendered from
/// <c>fast.cs</c>: it holds a <c>GCHandle</c> and pins the managed array instead of copying
/// element by element. Until this test existed, that path was generated but never executed.
///
/// The assertions are content-derived deliberately. <c>pattern_ffi_slice_of_unit_enum</c> counts
/// elements equal to <c>EnumDocumented::B</c> on the Rust side, so a wrong element stride or a
/// wrong discriminant width yields a wrong count rather than merely failing to throw.
/// </summary>
public class TestPatternSliceUnitEnum
{
    [Fact]
    public void pattern_ffi_slice_of_unit_enum_1()
    {
        var values = new[] { EnumDocumented.A, EnumDocumented.B, EnumDocumented.B };

        using var slice = values.Slice();

        Assert.Equal(2u, Interop.pattern_ffi_slice_of_unit_enum(slice));
    }

    /// A different length and arrangement, so a stride error cannot coincidentally satisfy
    /// both this and the case above.
    [Fact]
    public void pattern_ffi_slice_of_unit_enum_2()
    {
        var values = new[]
        {
            EnumDocumented.B,
            EnumDocumented.A,
            EnumDocumented.B,
            EnumDocumented.C,
            EnumDocumented.B,
        };

        using var slice = values.Slice();

        Assert.Equal(3u, Interop.pattern_ffi_slice_of_unit_enum(slice));
    }

    /// No matches. Separates "read zero matches" from "read nothing at all".
    [Fact]
    public void pattern_ffi_slice_of_unit_enum_3()
    {
        var values = new[] { EnumDocumented.A, EnumDocumented.C, EnumDocumented.A };

        using var slice = values.Slice();

        Assert.Equal(0u, Interop.pattern_ffi_slice_of_unit_enum(slice));
    }
}
