using System;
using System.Collections.Generic;
using System.Runtime.CompilerServices;
using System.Runtime.InteropServices;
using System.Threading.Tasks;
using My.Company;
using My.Company.Common;
using Xunit;
using Interop = My.Company.Interop;

public partial class TestDisposal
{
    [Fact]
    public void disposed_service_rejects_native_calls_and_repeated_disposal()
    {
        var service = ServiceVariousSlices.Create();
        service.Dispose();
        service.Dispose();
        Assert.Throws<ObjectDisposedException>(() => service.ReturnSlice());
        Assert.Throws<ObjectDisposedException>(() => service.ReturnSliceMut());
    }

    [Fact]
    public async Task disposed_async_service_faults_before_starting_native_work()
    {
        var service = ServiceAsyncCancel.Create();
        service.Dispose();
        service.Dispose();
        Assert.Throws<ObjectDisposedException>(() => service.Counter());
        for (int i = 0; i < 16; ++i)
        {
            Task<ulong> work = service.LongRunningAsync(1, 1, TestContext.Current.CancellationToken);
            await Assert.ThrowsAsync<ObjectDisposedException>(() => work);
            Assert.True(work.IsFaulted);
        }
    }

    [Fact]
    public void failed_union_transfer_releases_the_already_moved_field()
    {
        AllocationProbe.Settle();
        var baseline = Interop.__test_live_allocations();
        using (var text = "aliased".Utf8())
        using (var source = EnumMultiOwned.Tuple(text, text, 0))
        {
            Assert.Throws<ObjectDisposedException>(() => Interop.enums_multi_owned_echo(source));
            source.Dispose();
            source.Dispose();
        }
        Assert.Equal(baseline, Interop.__test_live_allocations());

        using (var first = "first".Utf8())
        using (var source = EnumMultiOwned.Named(first, null!, 0))
            Assert.Throws<NullReferenceException>(() => Interop.enums_multi_owned_echo(source));
        Assert.Equal(baseline, Interop.__test_live_allocations());
    }

    [Fact]
    public void failed_composite_transfer_releases_the_already_moved_field()
    {
        AllocationProbe.Settle();
        var baseline = Interop.__test_live_allocations();
        using (var text = "aliased".Utf8())
        using (var source = new UseString { s1 = text, s2 = text })
            Assert.Throws<ObjectDisposedException>(() => Interop.pattern_string_4(source));
        Assert.Equal(baseline, Interop.__test_live_allocations());
    }

    [Fact]
    public void later_argument_failure_releases_earlier_native_arguments()
    {
        AllocationProbe.Settle();
        var baseline = Interop.__test_live_allocations();
        using (var text = "aliased arguments".Utf8())
            Assert.Throws<ObjectDisposedException>(() => Interop.disposal_take_strings(text, text));
        Assert.Equal(baseline, Interop.__test_live_allocations());
    }

    [LibraryImport("reference_project", EntryPoint = "__missing_disposal_test_export")]
    private static partial void MissingExport(Utf8String value);

    [Fact]
    public void entry_point_resolution_failure_rolls_back_the_argument()
    {
        AllocationProbe.Settle();
        var baseline = Interop.__test_live_allocations();
        using (var text = "no native call".Utf8())
            Assert.Throws<EntryPointNotFoundException>(() => MissingExport(text));
        Assert.Equal(baseline, Interop.__test_live_allocations());
    }

    [MethodImpl(MethodImplOptions.NoInlining)]
    private static WeakReference FailedCallbackTransfer()
    {
        using var callback = new MyCallback(value => value + 1);
        using var invalid = EnumMultiOwned.Single(null!);
        Assert.Throws<NullReferenceException>(() => Interop.disposal_take_callback(callback, invalid));
        return new WeakReference(callback);
    }

    [Fact]
    public void failed_later_argument_does_not_retain_the_callback_root()
    {
        var callback = FailedCallbackTransfer();
        GC.Collect();
        GC.WaitForPendingFinalizers();
        GC.Collect();
        Assert.False(callback.IsAlive);
    }

    [Theory]
    [InlineData(false)]
    [InlineData(true)]
    public void cleanup_attempts_all_fields_and_preserves_every_callback_error(bool union)
    {
        AllocationProbe.Settle();
        var baseline = Interop.__test_live_allocations();
        var firstError = new InvalidOperationException("first");
        var secondError = new ArgumentException("second");
        using var first = new MyCallback(_ => throw firstError);
        using var second = new MyCallback(_ => throw secondError);
        using var text = "must be released".Utf8();
        IDisposable value;
        if (union)
        {
            var source = DisposalCallbacks.Values(first, text, second);
            Interop.disposal_callbacks_invoke(in source);
            value = source;
        }
        else
        {
            var source = new DisposalFields { first = first, text = text, second = second };
            Interop.disposal_fields_invoke(in source);
            value = source;
        }
        var error = Assert.Throws<AggregateException>(() => value.Dispose());
        Assert.Collection(error.InnerExceptions,
            actual => Assert.Same(firstError, actual),
            actual => Assert.Same(secondError, actual));
        Assert.Throws<ObjectDisposedException>(() => _ = text.String);
        Assert.Throws<ObjectDisposedException>(() => first.Call(0));
        Assert.Throws<ObjectDisposedException>(() => second.Call(0));
        value.Dispose();
        Assert.Equal(baseline, Interop.__test_live_allocations());
    }

    [Fact]
    public void a_single_cleanup_error_keeps_its_identity_after_freeing_later_fields()
    {
        var failure = new InvalidOperationException("callback failure");
        using var first = new MyCallback(_ => throw failure);
        using var second = new MyCallback(value => value);
        using var text = "later allocation".Utf8();
        using var value = new DisposalFields { first = first, text = text, second = second };
        Interop.disposal_fields_invoke(in value);
        Assert.Same(failure, Assert.Throws<InvalidOperationException>(() => value.Dispose()));
        Assert.Throws<ObjectDisposedException>(() => _ = text.String);
        Assert.Throws<ObjectDisposedException>(() => second.Call(0));
    }

    [Fact]
    public void mixed_wire_union_disposes_native_leaves_in_every_case()
    {
        Assert.True(typeof(IDisposable).IsAssignableFrom(typeof(DisposalMixed)));
        Assert.False(typeof(IDisposable).IsAssignableFrom(typeof(DataEnum)));
        AllocationProbe.Settle();
        var baseline = Interop.__test_live_allocations();
        var factories = new Func<DisposalMixed>[]
        {
            () => DisposalMixed.Empty,
            () => DisposalMixed.End,
            () => DisposalMixed.Managed("managed"),
            () => DisposalMixed.Native("native".Utf8()),
            () => DisposalMixed.Many(new List<Utf8String> { "first".Utf8(), "second".Utf8() }),
            () => DisposalMixed.Table(new Dictionary<string, Utf8String> { ["key"] = "value".Utf8() }),
            () => DisposalMixed.Nested(NewMixedFields())
        };
        foreach (var factory in factories)
        {
            using (var source = factory())
            using (var wire = source.Wire())
            using (var returned = Interop.disposal_mixed_echo(wire))
            using (var result = returned.Unwire())
            {
                source.Dispose();
                result.Dispose();
                source.Dispose();
                result.Dispose();
            }
            Assert.Equal(baseline, Interop.__test_live_allocations());
        }
    }

    private static DisposalMixedFields NewMixedFields() => new()
    {
        child = new DisposalMixedLeaf { text = "child".Utf8(), note = "managed child" },
        label = "managed",
        native = "native".Utf8(),
        list = new List<Utf8String> { "list".Utf8() },
        optional = "optional".Utf8(),
        array = new uint[] { 1, 2 }
    };

    [Fact]
    public void wire_composite_disposes_nested_lists_and_nullable_values()
    {
        Assert.True(typeof(IDisposable).IsAssignableFrom(typeof(DisposalMixedFields)));
        AllocationProbe.Settle();
        var baseline = Interop.__test_live_allocations();
        using (var source = NewMixedFields())
        using (var wire = source.Wire())
        using (var returned = Interop.disposal_mixed_fields_echo(wire))
        using (var result = returned.Unwire())
        {
            source.Dispose();
            Assert.Equal("child", result.child.text.String);
            Assert.Equal("native", result.native.String);
            Assert.Equal("optional", result.optional!.String);
            result.Dispose();
            result.Dispose();
            Assert.Throws<ObjectDisposedException>(() => _ = result.list[0].String);
            Assert.Throws<ObjectDisposedException>(() => _ = result.child.text.String);
        }
        Assert.Equal(baseline, Interop.__test_live_allocations());
    }
}
