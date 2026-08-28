using My.Company;
using My.Company.Common;
using Xunit;
using Interop = My.Company.Interop;

public class TestEnums
{
    [Fact]
    public void enum1()
    {
        Interop.enums_1(EnumPayload.A);
        Interop.enums_1(EnumPayload.B(new Vec3f32 { x = 0, y = 0, z = 0 }));
        Interop.enums_1(EnumPayload.C(123));
    }

    [Fact]
    public void enum2()
    {
        var v1 = new Vec3f32
        {
            x = 1.0f,
            y = 2.0f,
            z = 3.0f
        };

        var v2 = new Vec3f32
        {
            x = 2.0f,
            y = 4.0f,
            z = 6.0f
        };

        var r1 = Interop.enums_2(EnumPayload.A);
        var r2 = Interop.enums_2(EnumPayload.B(v1));
        var r3 = Interop.enums_2(EnumPayload.C(123));

        Assert.Equal(EnumPayload.A, r1);
        Assert.Equal(EnumPayload.B(v2), r2);
        Assert.Equal(EnumPayload.C(246), r3);
    }

    [Fact]
    public void enum3()
    {
        var v1 = new Vec3f32
        {
            x = 1.0f,
            y = 2.0f,
            z = 3.0f
        };

        var v2 = new Vec3f32
        {
            x = 2.0f,
            y = 4.0f,
            z = 6.0f
        };

        var e1 = EnumPayload.A;
        var e2 = EnumPayload.B(v1);
        var e3 = EnumPayload.C(123);

        Interop.enums_3(ref e1);
        Interop.enums_3(ref e2);
        Interop.enums_3(ref e3);

        Assert.Equal(EnumPayload.A, e1);
        Assert.Equal(v2, e2.AsB());
        Assert.Equal(246u, e3.AsC());
    }

    [Fact]
    public void enum4()
    {
        var a = "hello".Utf8();
        var b = "world".Utf8();
        var v = VecUtf8String.Empty();

        var l1 = new Layer1String
        {
            maybe_1 = OptionUtf8String.None,
            maybe_2 = v,
            maybe_3 = a.Clone()
        };
        var l2 = new Layer2String
        {
            layer_1 = l1,
            strings = new[]
            {
                a.Clone(),
                b.Clone()
            }.IntoVec(),
            vec = new Vec3f32 { x = 0, y = 0, z = 0 },
            the_enum = EnumPayload.A
        };
        using var l3 = Layer3String.B(l2);

        Assert.Equal("hello", Interop.enums_4(l3).String);
    }

    /// <summary>
    /// Item 3e: the compiler synthesises an implicit conversion from a case type to its union.
    /// </summary>
    /// <remarks>
    /// Nothing is generated for 3e. <c>aa2d550d</c> emits the case constructors
    /// — <c>public EnumPayload(CCase value)</c> — and the C# 15 union feature is specified to
    /// synthesise the conversion from them, so 3e reduces to "the constructors are public and
    /// single-parameter", which is already true.
    ///
    /// That reasoning was taken from the specification and never compiled, which is why the item
    /// stayed "satisfied, unverified": two other specification-derived claims had already turned
    /// out wrong — 3d's recorded gate, and the namespace <c>IUnion</c> resolves from.
    ///
    /// The assertions below are almost incidental. **If the conversion is not synthesised, this
    /// file does not compile**, and that is the actual proof. It only became possible to state it
    /// this way once <c>csharp_suite</c> began compiling and running the generated bindings from
    /// <c>cargo test</c>; before that, nothing in the repository compiled this output.
    /// </remarks>
    [Fact]
    public void a_case_type_converts_implicitly_to_its_union()
    {
        // Assignment position, unit case and payload-carrying case.
        EnumPayload unit = new EnumPayload.ACase();
        EnumPayload payload = new EnumPayload.CCase(123);

        Assert.Equal(EnumPayload.A, unit);
        Assert.Equal(EnumPayload.C(123), payload);

        // Argument position, and across the FFI boundary rather than only inside C#: enums_2
        // doubles the payload, so a value that survives the round trip proves the conversion
        // produced a well-formed union and not merely something that type-checked.
        Assert.Equal(EnumPayload.C(246), Interop.enums_2(new EnumPayload.CCase(123)));
    }
}
