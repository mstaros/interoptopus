using System;
using System.Collections.Generic;
using System.Runtime.CompilerServices;
using System.Runtime.InteropServices;
using System.Threading;
using System.Threading.Tasks;
using My.Company;
using My.Company.Common;
using Xunit;
using Interop = My.Company.Interop;

public partial class TestDisposal
{
    [MethodImpl(MethodImplOptions.NoInlining)]
    private static WeakReference CreateCollectibleService(bool asyncService, bool dispose, Func<uint, uint> onDrop)
    {
        IDisposable service = asyncService
            ? ServiceAsyncCancel.CreateWithDropCallback(onDrop)
            : ServiceCallbacks.CreateWithDropCallback(onDrop);
        if (dispose) service.Dispose();
        return new WeakReference(service);
    }

    [Theory]
    [InlineData(false, false)]
    [InlineData(false, true)]
    [InlineData(true, false)]
    [InlineData(true, true)]
    public void service_ownership_is_released_once_with_or_without_dispose(bool asyncService, bool dispose)
    {
        int drops = 0;
        var service = CreateCollectibleService(asyncService, dispose, _ => (uint)Interlocked.Increment(ref drops));
        if (dispose) Assert.Equal(1, Volatile.Read(ref drops));
        GC.Collect();
        GC.WaitForPendingFinalizers();
        GC.Collect();
        Assert.False(service.IsAlive);
        Assert.Equal(1, Volatile.Read(ref drops));
    }

    [Theory]
    [InlineData(false)]
    [InlineData(true)]
    public async Task service_borrows_reject_conflicts_and_release_after_native_return(bool mutable)
    {
        int drops = 0;
        int unexpectedEntries = 0;
        using var entered = new ManualResetEventSlim();
        using var release = new ManualResetEventSlim();
        using var service = ServiceCallbacks.CreateWithDropCallback(_ => (uint)Interlocked.Increment(ref drops));
        using var other = ServiceCallbacks.Create();
        bool released = false;
        uint Hold(uint value)
        {
            entered.Set();
            released = release.Wait(TimeSpan.FromSeconds(10));
            return value;
        }
        var call = Task.Run(() =>
        {
            if (mutable) service.CallbackSimple(Hold);
            else Interop.disposal_call_services(service, service, Hold);
        });
        try
        {
            Assert.True(entered.Wait(TimeSpan.FromSeconds(10)));
            Assert.Throws<InvalidOperationException>(() => service.CallbackSimple(_ =>
            {
                Interlocked.Increment(ref unexpectedEntries);
                return 0;
            }));
            using var callback = new SumDelegateReturn((_, _) =>
            {
                Interlocked.Increment(ref unexpectedEntries);
                return ResultVoidError.Ok;
            });
            Assert.Throws<InvalidOperationException>(() => service.CallbackWithSlice(callback, new[] { 1, 2 }.AsSpan()));
            if (mutable)
                Assert.Throws<InvalidOperationException>(() => service.InvokeStoredCallback(0));
            else
                Assert.Equal(0u, service.InvokeStoredCallback(0));

            Assert.Throws<InvalidOperationException>(() => Interop.disposal_call_services_shared_mut(other, service, _ =>
            {
                Interlocked.Increment(ref unexpectedEntries);
                return 0;
            }));
            // A failed later acquisition releases the earlier, independent service.
            other.CallbackSimple(_ => 0);
            Assert.Equal(0, Volatile.Read(ref unexpectedEntries));
        }
        finally
        {
            release.Set();
            await call.WaitAsync(TimeSpan.FromSeconds(10));
        }
        Assert.True(released);
        service.CallbackSimple(_ => 0);
        service.Dispose();
        Assert.Equal(1, Volatile.Read(ref drops));
    }

    [Fact]
    public void conflicting_callback_reentry_fails_without_waiting_or_losing_disposal()
    {
        int drops = 0;
        int dropsInsideCallback = -1;
        Exception? sharedError = null;
        Exception? mutableError = null;
        using var service = ServiceCallbacks.CreateWithDropCallback(_ => (uint)++drops);
        service.CallbackSimple(_ =>
        {
            sharedError = Record.Exception(() => service.InvokeStoredCallback(0));
            mutableError = Record.Exception(() => service.CallbackSimple(_ => 0));
            service.Dispose();
            dropsInsideCallback = drops;
            return 0;
        });
        Assert.IsType<InvalidOperationException>(sharedError);
        Assert.IsType<InvalidOperationException>(mutableError);
        Assert.Equal(0, dropsInsideCallback);
        Assert.Equal(1, drops);
    }

    [Fact]
    public void incompatible_service_aliases_fail_before_native_entry_and_roll_back()
    {
        int entries = 0;
        int drops = 0;
        using var service = ServiceCallbacks.CreateWithDropCallback(_ => (uint)++drops);
        uint Enter(uint value) { ++entries; return value; }
        Assert.Throws<InvalidOperationException>(() => Interop.disposal_call_services_mut_shared(service, service, Enter));
        Assert.Throws<InvalidOperationException>(() => Interop.disposal_call_services_shared_mut(service, service, Enter));
        Assert.Throws<InvalidOperationException>(() => Interop.disposal_call_services_mut_mut(service, service, Enter));
        Assert.Equal(0, entries);
        // Compatible aliases still enter Rust, and each failed exclusive scope released its reservation.
        Assert.Equal(0u, Interop.disposal_call_services(service, service, Enter));
        service.CallbackSimple(Enter);
        Assert.Equal(2, entries);
        service.Dispose();
        Assert.Equal(1, drops);
    }

    [Fact]
    public async Task concurrent_shared_borrows_keep_exclusive_access_closed_until_all_return()
    {
        const int count = 8;
        using var entered = new CountdownEvent(count);
        using var release = new ManualResetEventSlim();
        using var service = ServiceCallbacks.Create();
        var calls = new Task[count];
        int timeouts = 0;
        for (int i = 0; i < count; ++i)
        {
            calls[i] = Task.Run(() => Interop.disposal_call_services(service, service, _ =>
            {
                entered.Signal();
                if (!release.Wait(TimeSpan.FromSeconds(15))) Interlocked.Increment(ref timeouts);
                return 0;
            }));
        }
        try
        {
            Assert.True(entered.Wait(TimeSpan.FromSeconds(15)));
            Assert.Throws<InvalidOperationException>(() => service.CallbackSimple(_ => 0));
            Assert.Equal(0u, service.InvokeStoredCallback(0));
        }
        finally
        {
            release.Set();
            await Task.WhenAll(calls).WaitAsync(TimeSpan.FromSeconds(15));
        }
        Assert.Equal(0, Volatile.Read(ref timeouts));
        service.CallbackSimple(_ => 0);
    }

    [Theory]
    [InlineData(false)]
    [InlineData(true)]
    public async Task active_call_defers_native_destruction(bool span)
    {
        int drops = 0;
        using var entered = new ManualResetEventSlim();
        using var release = new ManualResetEventSlim();
        using var service = ServiceCallbacks.CreateWithDropCallback(_ => (uint)Interlocked.Increment(ref drops));
        bool released = false;
        var call = Task.Run(() =>
        {
            if (span)
            {
                using var callback = new SumDelegateReturn((_, _) =>
                {
                    entered.Set();
                    released = release.Wait(TimeSpan.FromSeconds(10));
                    return ResultVoidError.Ok;
                });
                service.CallbackWithSlice(callback, new[] { 1, 2 }.AsSpan());
            }
            else
            {
                service.CallbackSimple(_ =>
                {
                    entered.Set();
                    released = release.Wait(TimeSpan.FromSeconds(10));
                    return 0;
                });
            }
        });
        try
        {
            Assert.True(entered.Wait(TimeSpan.FromSeconds(10)));
            await Task.Run(() => Parallel.For(0, 32, _ => service.Dispose())).WaitAsync(TimeSpan.FromSeconds(5));
            Assert.Equal(0, Volatile.Read(ref drops));
            Assert.Throws<ObjectDisposedException>(() => service.InvokeStoredCallback(0));
        }
        finally
        {
            release.Set();
            await call.WaitAsync(TimeSpan.FromSeconds(10));
        }
        Assert.True(released);
        Assert.Equal(1, Volatile.Read(ref drops));
        Assert.Throws<ObjectDisposedException>(() => service.CallbackWithSlice(null!, new[] { 1, 2 }.AsSpan()));
    }

    [Theory]
    [InlineData(false)]
    [InlineData(true)]
    public void callback_can_dispose_its_service_without_waiting(bool viaInterop)
    {
        int drops = 0;
        int dropsInsideCallback = -1;
        using var service = ServiceCallbacks.CreateWithDropCallback(_ => (uint)++drops);
        // Service imports are internal; exercise the generated typed destructor directly.
        Action<ServiceCallbacks> dispose = viaInterop
            ? typeof(Interop).GetMethod("service_callbacks_destroy", System.Reflection.BindingFlags.Static | System.Reflection.BindingFlags.NonPublic,
                null, new[] { typeof(ServiceCallbacks) }, null)!.CreateDelegate<Action<ServiceCallbacks>>()
            : value => value.Dispose();
        service.CallbackSimple(_ =>
        {
            dispose(service);
            dispose(service);
            dropsInsideCallback = drops;
            return 0;
        });
        Assert.Equal(0, dropsInsideCallback);
        Assert.Equal(1, drops);
    }

    [Fact]
    public void every_service_argument_is_retained_and_failed_acquisition_rolls_back()
    {
        int firstDrops = 0;
        int secondDrops = 0;
        using var first = ServiceCallbacks.CreateWithDropCallback(_ => (uint)++firstDrops);
        using var second = ServiceCallbacks.CreateWithDropCallback(_ => (uint)++secondDrops);
        int dropsInsideCallback = -1;
        Assert.Equal(42u, Interop.disposal_call_services(first, second, _ =>
        {
            first.Dispose();
            second.Dispose();
            dropsInsideCallback = firstDrops + secondDrops;
            return 42;
        }));
        Assert.Equal(0, dropsInsideCallback);
        Assert.Equal(1, firstDrops);
        Assert.Equal(1, secondDrops);

        int rollbackDrops = 0;
        using var live = ServiceCallbacks.CreateWithDropCallback(_ => (uint)++rollbackDrops);
        Assert.Throws<ObjectDisposedException>(() => Interop.disposal_call_services(live, second, _ => 0));
        live.Dispose();
        Assert.Equal(1, rollbackDrops);

        int aliasDrops = 0;
        using var aliased = ServiceCallbacks.CreateWithDropCallback(_ => (uint)++aliasDrops);
        Interop.disposal_call_services(aliased, aliased, _ =>
        {
            aliased.Dispose();
            dropsInsideCallback = aliasDrops;
            return 0;
        });
        Assert.Equal(0, dropsInsideCallback);
        Assert.Equal(1, aliasDrops);
    }

    [Fact]
    public async Task async_start_racing_dispose_either_completes_or_rejects_before_native_entry()
    {
        for (int i = 0; i < 64; ++i)
        {
            int drops = 0;
            using var service = ServiceAsyncCancel.CreateWithDropCallback(_ => (uint)Interlocked.Increment(ref drops));
            var start = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
            var call = Task.Run(async () =>
            {
                await start.Task;
                try { Assert.Equal(1ul, await service.LongRunningAsync(1, 1, TestContext.Current.CancellationToken)); }
                catch (ObjectDisposedException) { }
            }, TestContext.Current.CancellationToken);
            var dispose = Task.Run(async () =>
            {
                await start.Task;
                service.Dispose();
            }, TestContext.Current.CancellationToken);
            start.SetResult();
            await Task.WhenAll(call, dispose).WaitAsync(TimeSpan.FromSeconds(10), TestContext.Current.CancellationToken);
            Assert.True(SpinWait.SpinUntil(() => Volatile.Read(ref drops) == 1, TimeSpan.FromSeconds(5)));
            Assert.Throws<ObjectDisposedException>(() => service.Counter());
        }
    }

    [Fact]
    public async Task precancelled_call_does_not_enter_native_code_or_retain_the_service()
    {
        int drops = 0;
        int entered = 0;
        using var cancellation = new CancellationTokenSource();
        cancellation.Cancel();
        using var service = ServiceAsyncCancel.CreateWithDropCallback(_ => (uint)Interlocked.Increment(ref drops));
        await Assert.ThrowsAnyAsync<OperationCanceledException>(() => service.CallbackWorkAsync(_ =>
        {
            Interlocked.Increment(ref entered);
            return 0;
        }, false, cancellation.Token));
        service.Dispose();
        Assert.Equal(0, Volatile.Read(ref entered));
        Assert.Equal(1, Volatile.Read(ref drops));
    }

    [Theory]
    [InlineData(false)]
    [InlineData(true)]
    public async Task async_call_survives_disposal_until_completion_or_cancellation(bool cancel)
    {
        int drops = 0;
        using var entered = new ManualResetEventSlim();
        using var release = new ManualResetEventSlim();
        using var cancellation = new CancellationTokenSource();
        using var service = ServiceAsyncCancel.CreateWithDropCallback(_ => (uint)Interlocked.Increment(ref drops));
        bool released = false;
        var call = service.CallbackWorkAsync(_ =>
        {
            entered.Set();
            released = release.Wait(TimeSpan.FromSeconds(10));
            return 0;
        }, cancel, cancellation.Token);
        try
        {
            Assert.True(entered.Wait(TimeSpan.FromSeconds(10)));
            Assert.Throws<InvalidOperationException>(() => Interop.disposal_borrow_async_service(service));
            service.Dispose();
            service.Dispose();
            Assert.Equal(0, Volatile.Read(ref drops));
            Assert.Throws<ObjectDisposedException>(() => service.Counter());
            if (cancel) cancellation.Cancel();
        }
        finally
        {
            release.Set();
            // Always stop an unfinished future, including on assertion failure.
            if (cancel) cancellation.Cancel();
        }
        if (cancel)
            await Assert.ThrowsAnyAsync<OperationCanceledException>(() => call.WaitAsync(TimeSpan.FromSeconds(10)));
        else
            Assert.Equal(0ul, await call.WaitAsync(TimeSpan.FromSeconds(10)));
        Assert.True(released);
        Assert.True(SpinWait.SpinUntil(() => Volatile.Read(ref drops) == 1, TimeSpan.FromSeconds(5)));
        Assert.Equal(1, Volatile.Read(ref drops));
    }

    [Fact]
    public void disposed_service_rejects_native_calls_and_repeated_disposal()
    {
        var service = ServiceVariousSlices.Create();
        service.Dispose();
        service.Dispose();
        Assert.Throws<ObjectDisposedException>(() => service.ReturnSlice());
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
