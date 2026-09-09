using System;
using System.Reflection;
using System.Runtime.CompilerServices;
using My.Company;
using My.Company.Common;
using Xunit;
using Interop = My.Company.Interop;

public class TestEnumConstants
{
    [Fact]
    public void constants_are_one_enum_case_with_original_signed_wide_tags()
    {
        Assert.Equal(typeof(long), Enum.GetUnderlyingType(typeof(EnumConstants.Constants)));
        Assert.Equal(-7L, (long)EnumConstants.Constants.Ready);
        Assert.Equal(1_099_511_627_776L, (long)EnumConstants.Constants.End);
        Assert.Equal(2, Enum.GetValues<EnumConstants.Constants>().Length);
        Assert.Null(typeof(EnumConstants).GetNestedType("ReadyCase"));
        Assert.Null(typeof(EnumConstants).GetNestedType("EndCase"));

        EnumConstants ready = EnumConstants.Constants.Ready;
        Assert.True(ready.IsReady);
        ready.AsReady();
        Assert.Equal(EnumConstants.Ready, ready);
        Assert.Equal(EnumConstants.Constants.Ready, ready.Value);
        Assert.True(ready.TryGetValue(out EnumConstants.Constants constant));
        Assert.Equal(EnumConstants.Constants.Ready, constant);
        Assert.False(ready.TryGetValue(out EnumConstants.NumberCase _));
        Assert.Equal("Ready", ready.ToString());
    }

    [Fact]
    public void constructor_rejects_undefined_and_payload_tags()
    {
        foreach (long tag in new long[] { 0, 11, 21, 55, 56, 123, long.MinValue, long.MaxValue })
            Assert.Throws<ArgumentOutOfRangeException>(() => new EnumConstants((EnumConstants.Constants)tag));
    }

    [Fact]
    public void payload_cases_and_empty_payload_declarations_keep_their_identity()
    {
        EnumConstants number = new EnumConstants.NumberCase(42);
        EnumConstants count = new EnumConstants.CountCase(42);
        Assert.NotEqual(number.Value, count.Value);
        Assert.True(number.TryGetValue(out EnumConstants.NumberCase numberCase));
        Assert.Equal(42L, numberCase.Value);
        Assert.False(number.TryGetValue(out EnumConstants.Constants _));
        Assert.False(count.TryGetValue(out EnumConstants.NumberCase _));
        Assert.IsType<EnumConstants.EmptyTupleCase>(EnumConstants.EmptyTuple.Value);
        Assert.IsType<EnumConstants.EmptyNamedCase>(EnumConstants.EmptyNamed.Value);
        Assert.NotEqual(EnumConstants.EmptyTuple.Value, EnumConstants.EmptyNamed.Value);
    }

    [Fact]
    public void constant_and_payload_patterns_match_the_underlying_union_value()
    {
        Assert.Equal(-7L, Classify(EnumConstants.Ready));
        Assert.Equal(1_099_511_627_776L, Classify(EnumConstants.End));
        Assert.Equal(123L, Classify(EnumConstants.Number(123)));
        Assert.Equal(456L, Classify(EnumConstants.Count(456)));
        Assert.Equal(55L, Classify(EnumConstants.EmptyTuple));
        Assert.Equal(56L, Classify(EnumConstants.EmptyNamed));
    }

    private static long Classify(EnumConstants value) => value switch
    {
        EnumConstants.Constants.Ready => -7,
        EnumConstants.Constants.End => 1_099_511_627_776,
        EnumConstants.NumberCase(var number) => number,
        EnumConstants.CountCase(var count) => count,
        EnumConstants.EmptyTupleCase => 55,
        EnumConstants.EmptyNamedCase => 56,
        EnumConstants.Constants => throw new InvalidOperationException("Unnamed constant"),
        null => throw new InvalidOperationException("Empty union"),
    };

    [MethodImpl(MethodImplOptions.NoInlining)]
    private static bool IsReady(EnumConstants value) => value is EnumConstants.Constants.Ready;

    [Fact]
    public void typed_constant_matching_does_not_box_each_value()
    {
        var values = new[] { EnumConstants.Ready, EnumConstants.End };
        for (int i = 0; i < 512; i++) IsReady(values[i & 1]);
        long before = GC.GetAllocatedBytesForCurrentThread();
        int found = 0;
        for (int i = 0; i < 10_000; i++)
            if (IsReady(values[i & 1])) found++;
        long allocated = GC.GetAllocatedBytesForCurrentThread() - before;
        Assert.Equal(5_000, found);
        Assert.True(allocated < 4096, $"Constant patterns allocated {allocated} bytes.");
    }

    [Fact]
    public void default_union_is_empty_and_cannot_cross_ffi()
    {
        EnumConstants empty = default;
        Assert.False(empty.HasValue);
        Assert.Null(empty.Value);
        Assert.False(empty.TryGetValue(out EnumConstants.Constants _));
        Assert.False(IsReady(empty));
        Assert.Throws<InvalidOperationException>(() => Interop.enum_constants_echo(empty));
    }

    [Fact]
    public void every_variant_round_trips_through_native_and_wire()
    {
        foreach (var value in new[] {
            EnumConstants.Ready, EnumConstants.End,
            EnumConstants.Number(long.MinValue), EnumConstants.Count(long.MaxValue),
            EnumConstants.EmptyTuple, EnumConstants.EmptyNamed })
        {
            var native = Interop.enum_constants_echo(value);
            Assert.True(native.HasValue);
            Assert.Equal(value.Value, native.Value);
            Assert.Equal(Classify(value), Classify(native));
            using var wire = value.Wire();
            using var returned = Interop.enum_constants_wire_echo(wire);
            var decoded = returned.Unwire();
            Assert.Equal(value.Value, decoded.Value);
        }
        // The compiler's implicit enum-to-union conversion also works as an FFI argument.
        Assert.True(Interop.enum_constants_echo(EnumConstants.Constants.Ready).IsReady);
    }

    [Fact]
    public void grouping_works_on_owned_class_unions_without_leaking_payloads()
    {
        Assert.True(typeof(EnumConstantsOwned).IsClass);
        var baseline = Interop.__test_live_allocations();
        foreach (var constant in Enum.GetValues<EnumConstantsOwned.Constants>())
        {
            using var original = new EnumConstantsOwned(constant);
            Assert.True(original.TryGetValue(out EnumConstantsOwned.Constants selected));
            Assert.Equal(constant, selected);
            using var native = Interop.enum_constants_owned_echo(original);
            Assert.Equal(constant, native.Value);
            using var wire = native.Wire();
            using var returned = Interop.enum_constants_owned_wire_echo(wire);
            using var decoded = returned.Unwire();
            Assert.Equal(constant, decoded.Value);
        }
        using (var original = EnumConstantsOwned.Text("payload 🌍".Utf8()))
        using (var native = Interop.enum_constants_owned_echo(original))
        {
            Assert.False(native.TryGetValue(out EnumConstantsOwned.Constants _));
            Assert.Equal("payload 🌍", native.AsText().String);
            using var wire = native.Wire();
            using var returned = Interop.enum_constants_owned_wire_echo(wire);
            using var decoded = returned.Unwire();
            Assert.Equal("payload 🌍", decoded.AsText().String);
        }
        Assert.Equal(baseline, Interop.__test_live_allocations());
    }

    [Fact]
    public void group_names_move_on_collision_and_enum_backing_names_are_escaped()
    {
        Assert.Equal(typeof(ulong), Enum.GetUnderlyingType(typeof(EnumConstantsNames.Constants3)));
        EnumConstantsNames first = EnumConstantsNames.Constants3.Constants;
        EnumConstantsNames second = EnumConstantsNames.Constants3.Constants2;
        Assert.Equal(EnumConstantsNames.Constants.Value, first.Value);
        Assert.Equal(EnumConstantsNames.Constants2.Value, second.Value);
        Assert.Equal(EnumConstantsNames.Constants3.value__Constant2, EnumConstantsNames.value__.Value);
        Assert.Equal(EnumConstantsNames.Constants3.value__Constant, EnumConstantsNames.value__Constant.Value);
        var payload = EnumConstantsNames.Payload(17);
        Assert.True(payload.TryGetValue(out EnumConstantsNames.PayloadCase selected));
        Assert.Equal(17u, selected.Constants3Field);
    }

    [Fact]
    public void result_constants_group_while_unit_payloads_and_single_unit_options_remain_cases()
    {
        var panic = new ResultUintError(ResultUintError.Constants.Panic);
        Assert.True(panic.IsPanic);
        Assert.Equal(ResultUintError.Constants.Panic, panic.Value);
        Assert.Equal(ResultUintError.Constants.Null, ResultUintError.Null.Value);
        Assert.Throws<ArgumentOutOfRangeException>(() => new ResultUintError((ResultUintError.Constants)0));
        Assert.IsType<ResultVoidError.OkCase>(ResultVoidError.Ok.Value);
        Assert.IsType<OptionUint.NoneCase>(OptionUint.None.Value);
        Assert.IsType<EnumPayload.ACase>(EnumPayload.A.Value);
    }
}
