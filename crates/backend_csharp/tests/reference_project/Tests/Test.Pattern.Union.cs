using System;
using System.Reflection;
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

    /// Item 5e: a class-backed union has no reachable empty state, where a struct-backed one is
    /// merely guarded against its own.
    ///
    /// The two are not symmetric. `ResultUintError` above is a struct: `default` is a legal,
    /// non-null, variant-zero value, which is why `_hasValue` and the marshal-out guard exist.
    /// `OptionUtf8String` is a class: `default` is a null reference, and every non-null instance
    /// was produced from inside the type. Item 3a is what keeps that true — declaring any
    /// constructor removes the implicit public one, so a single `private OptionUtf8String() { }`
    /// stops `new OptionUtf8String()` yielding a variant-zero instance from outside.
    ///
    /// Reflection rather than a call. `new OptionUtf8String()` from here would fail to compile,
    /// which does prove the point but leaves nothing that runs and nothing that can regress. Until
    /// now the only check was `enum_class_ctor::a_class_backed_enum_gets_a_private_parameterless_ctor`,
    /// which matches text in a snapshot — so making the constructor public again would produce a
    /// snapshot diff a reviewer could accept, and no test would fail.
    [Fact]
    public void a_class_backed_union_cannot_be_constructed_empty()
    {
        var parameterless = typeof(OptionUtf8String).GetConstructor(
            BindingFlags.Instance | BindingFlags.Public | BindingFlags.NonPublic,
            binder: null,
            types: Type.EmptyTypes,
            modifiers: null);

        Assert.NotNull(parameterless);
        Assert.False(
            parameterless!.IsPublic,
            "item 3a made this constructor private; public again means `new OptionUtf8String()` can "
                + "produce a variant-zero instance from outside the type, which corresponds to no Rust variant");

        // No empty state to guard: absence is a null reference, not a zeroed instance.
        Assert.Null(default(OptionUtf8String));

        // Which is why `HasValue` is a constant here rather than a field read, and `Value` is
        // never null on an instance that exists at all.
        var none = OptionUtf8String.None;
        Assert.True(none.HasValue);
        Assert.NotNull(none.Value);
    }

    /// Item 5i: a default struct-backed union does not read as its variant-zero case.
    ///
    /// This is the behaviour item 4c changed, and until now nothing executed it. `IsOk` read
    /// `_variant == 0` and `AsOk()` tested only the variant, so a default reported `HasValue`
    /// false and `IsOk` true at the same time, and `AsOk()` returned a payload read out of
    /// uninitialised memory — a fabricated value reaching a consumer, not merely a wrong flag.
    ///
    /// The exception *type* is deliberately not pinned. `ExceptionForVariant()` still ignores
    /// `_hasValue` and hands back variant zero's `EnumException`; correcting that is item 4b.
    /// Asserting the type here would force churn when 4b lands, and the contract 4c established
    /// is "throws rather than returning a value", which is what this asserts.
    [Fact]
    public void a_default_struct_union_does_not_read_as_its_variant_zero_case()
    {
        var result = new ResultUintError();

        Assert.False(result.IsOk, "a default union holds no value, so no case is active");
        Assert.ThrowsAny<Exception>(() => result.AsOk());
    }

    /// The `Option` half of 4c's soundness obligation: `default(OptionX)` is not `NoneCase`.
    ///
    /// Asserted through `HasValue`/`Value` rather than `IsNone`, so it does not depend on which
    /// variant happens to be tag zero — the distinction 4c exists to preserve.
    [Fact]
    public void a_default_option_is_empty_rather_than_none()
    {
        var option = new OptionUint();

        Assert.False(option.HasValue);
        Assert.Null(option.Value);
    }

    /// Item 5h: a union that never crosses the FFI boundary still gets the whole union surface.
    ///
    /// `DataEnum` is managed-only. It carries `[Union]` and implements `IUnion`, but has no
    /// `[NativeMarshalling]` attribute and no nested `Unmanaged` mirror, because there is nothing
    /// to marshal it to. That combination is the case item 3d had to get right: `[Union]` sits
    /// *outside* the `is_managed_only` guard, since that guard governs the mirror and the
    /// marshaller rather than the projection. An earlier reading put it inside, which would have
    /// silently dropped the union surface from exactly this type.
    ///
    /// The absent `Unmanaged` is the load-bearing assertion. Without it this is just another union
    /// test; with it, it is the only check that projection and crossing are independent.
    [Fact]
    public void a_managed_only_union_is_still_projected_as_a_union()
    {
        var type = typeof(DataEnum);

        Assert.Contains(typeof(IUnion), type.GetInterfaces());
        Assert.Null(
            type.GetNestedType("Unmanaged", BindingFlags.Public | BindingFlags.NonPublic),
            "DataEnum is managed-only, so it should have no unmanaged mirror; if one appeared, the "
                + "union projection and the FFI crossing have been coupled again");

        var s = DataEnum.S("hello");

        Assert.True(s.IsS);
        Assert.True(s.HasValue);
        Assert.Equal(new DataEnum.SCase("hello"), s.Value);
        Assert.True(s.TryGetValue(out DataEnum.SCase sCase));
        Assert.Equal("hello", sCase.Value);
    }
}
