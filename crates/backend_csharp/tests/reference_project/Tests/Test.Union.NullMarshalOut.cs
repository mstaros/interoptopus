using System;
using My.Company;
using My.Company.Common;
using Xunit;
using Interop = My.Company.Interop;

/// Contract tests for `docs/csharp-unions.md` item 4b.
///
/// A null class-backed union is not a Rust enum value. Every managed marshal-out position rejects
/// it with `InvalidOperationException` before native entry rather than fabricating variant zero.
public class TestNullAtMarshalOut
{
    [Fact]
    public void a_null_class_backed_union_argument_throws()
    {
        AssertNullVariant(
            () => Interop.enums_4(null!),
            nameof(Layer3String)
        );
    }

    [Fact]
    public void a_null_borrowed_class_backed_union_argument_throws()
    {
        AssertNullVariant(
            () => Interop.pattern_option_string_borrow(null!),
            nameof(OptionUtf8String)
        );
    }

    [Fact]
    public void a_null_class_backed_union_composite_field_throws()
    {
        var l1 = new Layer1String
        {
            maybe_1 = null!,
            maybe_2 = VecUtf8String.Empty(),
            maybe_3 = "hello".Utf8()
        };
        var l2 = new Layer2String
        {
            layer_1 = l1,
            strings = new[] { "hello".Utf8() }.IntoVec(),
            vec = new Vec3f32 { x = 0, y = 0, z = 0 },
            the_enum = EnumPayload.A
        };
        using var l3 = Layer3String.B(l2);

        AssertNullVariant(
            () => Interop.enums_4(l3),
            "Layer1String.maybe_1"
        );
    }

    [Fact]
    public void a_union_carrying_a_null_class_backed_payload_throws()
    {
        var outer = OptionOptionResultOptionUtf8StringError.Some(null!);

        AssertNullVariant(
            () => Interop.pattern_ffi_option_3(outer),
            "OptionOptionResultOptionUtf8StringError.Some"
        );
    }

    [Fact]
    public void the_same_calls_succeed_with_non_null_values()
    {
        var l1 = new Layer1String
        {
            maybe_1 = OptionUtf8String.None,
            maybe_2 = VecUtf8String.Empty(),
            maybe_3 = "hello".Utf8()
        };
        var l2 = new Layer2String
        {
            layer_1 = l1,
            strings = new[] { "hello".Utf8() }.IntoVec(),
            vec = new Vec3f32 { x = 0, y = 0, z = 0 },
            the_enum = EnumPayload.A
        };
        using var l3 = Layer3String.B(l2);

        Assert.Equal("hello", Interop.enums_4(l3).String);
        Interop.pattern_option_string_borrow(OptionUtf8String.None);
    }

    private static void AssertNullVariant(Action action, string expectedPath)
    {
        var exception = Assert.Throws<InvalidOperationException>(action);
        Assert.Contains(expectedPath, exception.Message);
        Assert.Contains("no Rust variant", exception.Message);
    }
}
