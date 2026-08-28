using System;
using My.Company;
using My.Company.Common;
using Xunit;
using Interop = My.Company.Interop;

/// Measurement for `docs/csharp-unions.md` Open items 1 — null at marshal-out for a class-backed
/// union. **Not a specification of desired behaviour.**
///
/// The item names three positions and records only one as measured:
///
/// - **(a)** the union passed as an argument — unmeasured until now
/// - **(b)** a class-backed union stored as a field of a composite — measured: NREs today, and it
///   does so *before* any marshaller, because `nullable.rs:35` defines nullability as
///   `TypeKind::Delegate(d) if d.kind == DelegateKind::Class` — class delegates only — so the
///   `?{suffix} ?? default` guard is never emitted for a class-backed union field
/// - **(c)** a null union nested inside another union's case — unmeasured until now
///
/// The item asks for (a) and (c) before item 4b, because "a different failure mode in either
/// changes what 4b implements, and 4b is the wrong place to discover it". `Issues.md` `b4e07f12`
/// has the same precondition from the other direction: delegating `nullable.rs` to
/// `struct_class::is_class` would put these types on the `?? default` branch and answer Open
/// items 1 by accident, emitting a zeroed `Unmanaged` — discriminant 0, a fabricated variant
/// crossing FFI, which is what 3a's private constructor exists to prevent.
///
/// **These tests assert whichever exception is observed, not one that was chosen.** If a future
/// change alters the exception type deliberately, update them; if it alters it accidentally, they
/// fail, which is the point. Item 4b will replace them with assertions about a decided contract.
public class TestNullAtMarshalOut
{
    /// Position (a): a class-backed union passed by value as an argument.
    ///
    /// `Layer3String` is class-backed — it carries `Layer2String`, whose fields own strings, so
    /// its managed conversion is `Into` and `struct_class` emits it as a class. `null` is
    /// therefore a legal C# value for this parameter, and nothing in the signature rejects it.
    [Fact]
    public void a_null_class_backed_union_argument()
    {
        var ex = Record.Exception(() => Interop.enums_4(null!));

        Assert.NotNull(ex);
        Assert.IsType<NullReferenceException>(ex);
    }

    /// Position (a), nested three deep. Included because the outer type's conversion recurses,
    /// and the item's concern is *where* the dereference happens rather than whether it happens.
    [Fact]
    public void a_null_deeply_nested_union_argument()
    {
        var ex = Record.Exception(() => Interop.pattern_ffi_option_3(null!));

        Assert.NotNull(ex);
        Assert.IsType<NullReferenceException>(ex);
    }

    /// Position (c): a well-formed union whose case payload is itself a null class-backed union.
    ///
    /// `ResultOptionUtf8StringError.OkCase` carries an `OptionUtf8String`, which is class-backed.
    /// The outer union is not null; the inner one is. This is the nesting the item asks about, and
    /// the corpus offers it in a better form than a collection would — there is no slice of unions
    /// to use.
    [Fact]
    public void a_union_carrying_a_null_class_backed_payload()
    {
        var outer = ResultOptionUtf8StringError.Ok(null!);

        var ex = Record.Exception(() => Interop.pattern_ffi_option_2(outer));

        Assert.NotNull(ex);
        Assert.IsType<NullReferenceException>(ex);
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
