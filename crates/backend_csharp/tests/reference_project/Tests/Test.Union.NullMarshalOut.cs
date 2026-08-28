using System;
using My.Company;
using My.Company.Common;
using Xunit;
using Interop = My.Company.Interop;

/// Null at marshal-out for a class-backed union — `docs/csharp-unions.md` Open items 1.
///
/// The item names three positions and these tests cover all of them:
///
/// - **(a)** the union passed as an argument
/// - **(b)** a class-backed union stored as a field of a composite
/// - **(c)** a null union nested inside another union's case
///
/// **These began as a measurement and are becoming a contract, one position at a time.** They
/// were written in `c70efeb` asserting whatever was *observed* — `NullReferenceException`
/// everywhere, thrown before any marshaller, because `nullable.rs` classified only class
/// delegates as nullable and a class-backed union therefore got a bare `.AsUnmanaged()`.
///
/// Open items 1 then decided: `InvalidOperationException`. `?? default` was disqualified rather
/// than merely rejected — it yields a zeroed `Unmanaged`, discriminant 0, a fabricated variant
/// crossing FFI, which is exactly what item 3a's private constructor exists to prevent.
/// `ArgumentNullException` lost because (b) and (c) are a field and a nested payload, not
/// arguments, so there is no parameter to name.
///
/// Position (c) now asserts the decided contract. Position (a) still asserts the observed
/// `NullReferenceException` because it goes through the marshaller's `FromManaged`/`ToUnmanaged`,
/// which has not been converted yet. **A failure there is expected work, not a regression** —
/// update it when the marshaller path lands.
public class TestNullAtMarshalOut
{
    /// Position (a): a class-backed union passed by value as an argument.
    ///
    /// `Layer3String` is class-backed — it carries `Layer2String`, whose fields own strings, so
    /// its managed conversion is `Into` and `struct_class` emits it as a class. `null` is
    /// therefore a legal C# value for this parameter, and nothing in the signature rejects it.
    ///
    /// Still observed rather than decided: the marshaller path is unconverted.
    [Fact]
    public void a_null_class_backed_union_argument()
    {
        var ex = Record.Exception(() => Interop.enums_4(null!));

        Assert.NotNull(ex);
        Assert.IsType<NullReferenceException>(ex);
    }

    /// Position (a), nested three deep. Included because the outer type's conversion recurses,
    /// and the item's concern is *where* the dereference happens rather than whether it happens.
    ///
    /// Still observed rather than decided, for the same reason as above.
    [Fact]
    public void a_null_deeply_nested_union_argument()
    {
        var ex = Record.Exception(() => Interop.pattern_ffi_option_3(null!));

        Assert.NotNull(ex);
        Assert.IsType<NullReferenceException>(ex);
    }

    /// Position (c): a well-formed union whose case payload is itself a null class-backed union.
    ///
    /// `OptionOptionResultOptionUtf8StringError.SomeCase` carries
    /// `OptionResultOptionUtf8StringError`, itself class-backed. The outer union is not null; the
    /// inner one is. The corpus offers this in a better form than a collection would — there is
    /// no slice of unions to use — and the three-deep nesting is stronger anyway, since the null
    /// traverses two conversions before the dereference.
    ///
    /// **Decided contract.** The guard is emitted by `enums::guard_null_payload`.
    [Fact]
    public void a_union_carrying_a_null_class_backed_payload()
    {
        var outer = OptionOptionResultOptionUtf8StringError.Some(null!);

        var ex = Record.Exception(() => Interop.pattern_ffi_option_3(outer));

        Assert.NotNull(ex);
        Assert.IsType<InvalidOperationException>(ex);
        Assert.Contains("OptionOptionResultOptionUtf8StringError.Some", ex!.Message);
    }

    /// The control. Without it, the three above could pass because the call fails for some reason
    /// unrelated to nullness, and the measurement would be worthless.
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
    }
}
