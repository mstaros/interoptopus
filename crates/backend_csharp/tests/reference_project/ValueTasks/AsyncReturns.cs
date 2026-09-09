// Tests deliberately exercise internal generated callback entry points.
#pragma warning disable CS0618
using System;
using System.Collections;
using System.Reflection;
using System.Runtime.CompilerServices;
using System.Runtime.InteropServices;
using System.Threading;
using System.Threading.Tasks;
using My.Company;
using My.Company.Common;
using Xunit;
using Interop = My.Company.Interop;

[assembly: CollectionBehavior(DisableTestParallelization = true)]

public class AsyncReturns
{
    [Fact]
    public async Task generic_value_tasks_preserve_primitive_struct_wire_and_tuple_results()
    {
        using var scalar = ServiceAsyncRval.Simple();
        ValueTask<uint> number = scalar.NumberAsync(TestContext.Current.CancellationToken);
        Assert.Equal(123u, await number);
        Assert.Equal(new Vec3f32(0, 0, 0), await scalar.Vecf32Async(TestContext.Current.CancellationToken));
        Assert.Equal("hello", (await scalar.WireAsync(TestContext.Current.CancellationToken)).Unwire());
        using var tuples = ServiceTuple.Create();
        ValueTask<(uint, float)> direct = tuples.PairLaterAsync((3u, 4f), TestContext.Current.CancellationToken);
        ValueTask<(uint, float)> result = tuples.TryPairLaterAsync((5u, 6f), TestContext.Current.CancellationToken);
        Assert.Equal((3u, 4f), await direct);
        Assert.Equal((5u, 6f), await result);
    }

    [Fact]
    public async Task void_value_task_and_as_task_composition_work()
    {
        using var service = ServiceAsyncPanic.Create();
        ValueTask nothing = service.NotPanickingAsync(TestContext.Current.CancellationToken);
        await nothing;
        using var numbers = ServiceAsyncRval.Simple();
        var first = numbers.NumberAsync(TestContext.Current.CancellationToken).AsTask();
        var second = numbers.NumberAsync(TestContext.Current.CancellationToken).AsTask();
        Assert.Equal(new uint[] { 123, 123 }, await Task.WhenAll(first, second));
        Assert.Equal(123u, await first);
        Assert.Equal(123u, await first);
    }

    [Fact]
    public async Task panics_fault_value_tasks_and_the_service_remains_usable()
    {
        using var service = ServiceAsyncPanic.Create();
        var task = service.PanickingAsync(TestContext.Current.CancellationToken).AsTask();
        await Assert.ThrowsAsync<InvalidOperationException>(() => task);
        Assert.True(task.IsFaulted);
        Assert.False(task.IsCanceled);
        await service.NotPanickingAsync(TestContext.Current.CancellationToken);
    }

    [Fact]
    public async Task cancellation_preserves_status_and_token()
    {
        using var service = ServiceAsyncCancel.Create();
        using var cancellation = new CancellationTokenSource();
        var task = service.SleepForeverAsync(cancellation.Token).AsTask();
        cancellation.Cancel();
        var error = await Assert.ThrowsAnyAsync<OperationCanceledException>(() => task);
        Assert.True(task.IsCanceled);
        Assert.Equal(cancellation.Token, error.CancellationToken);
        var alreadyCancelled = service.SleepForeverAsync(cancellation.Token).AsTask();
        var early = await Assert.ThrowsAnyAsync<OperationCanceledException>(() => alreadyCancelled);
        Assert.True(alreadyCancelled.IsCanceled);
        Assert.Equal(cancellation.Token, early.CancellationToken);
    }

    [Fact]
    public async Task async_constructors_and_result_errors_keep_their_surface_types()
    {
        using var runtime = ServiceAsyncBasic.Simple();
        ValueTask<ServiceAsyncCtor> result = ServiceAsyncCtor.NewAsync(runtime, 42, TestContext.Current.CancellationToken);
        ValueTask<ServiceAsyncCtor> bare = ServiceAsyncCtor.SimpleAsync(runtime, 17, TestContext.Current.CancellationToken);
        using var first = await result;
        using var second = await bare;
        Assert.Equal(42u, first.GetValue());
        Assert.Equal(17u, second.GetValue());
        using var service = ServiceAsyncResult.Create();
        ValueTask nothing = service.SuccessAsync(TestContext.Current.CancellationToken);
        await nothing;
        var task = service.FailAsync(TestContext.Current.CancellationToken).AsTask();
        var error = await Assert.ThrowsAsync<EnumException<Error>>(() => task);
        Assert.Equal(Error.Fail, error.Value);
        Assert.True(task.IsFaulted);
    }

    private static int PendingStrings()
    {
        var field = typeof(AsyncTrampolineResultUtf8StringError).GetField("InFlight", BindingFlags.Static | BindingFlags.NonPublic)!;
        var pending = (IDictionary)field.GetValue(null)!;
        lock (pending) { return pending.Count; }
    }

    [Fact]
    public async Task input_marshalling_failure_removes_pending_completion()
    {
        using var service = ServiceAsyncVecString.Create();
        var text = "already disposed".Utf8();
        text.Dispose();
        var before = PendingStrings();
        for (int i = 0; i < 100; ++i)
            await Assert.ThrowsAsync<ObjectDisposedException>(() => service.HandleStringAsync(text, TestContext.Current.CancellationToken).AsTask());
        Assert.Equal(before, PendingStrings());
        using var result = await service.HandleStringAsync("still works".Utf8(), TestContext.Current.CancellationToken);
        Assert.Equal("still works", result.String);
        Assert.Equal(before, PendingStrings());
    }

    private static unsafe void CompleteMalformedEnum(AsyncCallbackCommonNative callback)
    {
        callback.UnsafeComplete(new ResultVoidError.Unmanaged { _variant = 255 });
    }

    [Fact]
    public async Task output_conversion_exception_is_contained_inside_native_callback()
    {
        var trampoline = new AsyncTrampolineResultVoidError();
        var (callback, completion) = trampoline.NewCall(CancellationToken.None);
        CompleteMalformedEnum(callback); // Must return without an exception crossing reverse P/Invoke.
        await Assert.ThrowsAsync<InteropException>(() => completion.AsTask());
    }

    private static unsafe void CompleteTag(AsyncCallbackCommonNative callback, byte tag)
    {
        ((delegate* unmanaged[Cdecl]<IntPtr, IntPtr, void>)callback._ptr)((IntPtr)(&tag), callback._ts);
    }

    [Theory]
    [InlineData(2)]
    [InlineData(255)]
    public async Task panic_and_unknown_tags_fault_without_reading_a_payload(byte tag)
    {
        var trampoline = new My.Company.AsyncTrampolineUint();
        var (callback, completion) = trampoline.NewCall(CancellationToken.None);
        CompleteTag(callback, tag);
        await Assert.ThrowsAsync<InvalidOperationException>(() => completion.AsTask());
    }

    private static unsafe void CompleteTwice(AsyncCallbackCommonNative callback)
    {
        callback.UnsafeComplete(7u);
        callback.UnsafeComplete(9u);
    }

    [Fact]
    public async Task duplicate_callback_does_not_replace_the_result_or_throw()
    {
        var trampoline = new My.Company.AsyncTrampolineUint();
        var (callback, completion) = trampoline.NewCall(CancellationToken.None);
        CompleteTwice(callback);
        Assert.Equal(7u, await completion);
    }

    private static uint _next;
    private static int _drops;
    private static int _status;

    [UnmanagedCallersOnly(CallConvs = new[] { typeof(CallConvCdecl) })]
    private static unsafe TaskHandle Next(IntPtr data, IntPtr callback, IntPtr context)
    {
        uint value = _next++;
        ((delegate* unmanaged[Cdecl]<IntPtr, int, IntPtr, void>)callback)(context, _status, (IntPtr)(&value));
        return default;
    }

    [UnmanagedCallersOnly(CallConvs = new[] { typeof(CallConvCdecl) })]
    private static void Drop(IntPtr data) { Interlocked.Increment(ref _drops); }

    private static unsafe AsyncIteratorUint SynchronousStream(int status = 1)
    {
        _status = status;
        return new AsyncIteratorUint.Unmanaged
        {
            _data = (IntPtr)1,
            _next = (IntPtr)(delegate* unmanaged[Cdecl]<IntPtr, IntPtr, IntPtr, TaskHandle>)&Next,
            _drop = (IntPtr)(delegate* unmanaged[Cdecl]<IntPtr, void>)&Drop
        }.IntoManaged();
    }

    [Theory]
    [InlineData(-2)]
    [InlineData(-1)]
    [InlineData(2)]
    public async Task synchronous_stream_failures_keep_cancellation_distinct(int status)
    {
        using var source = SynchronousStream(status);
        await using var enumerator = source.GetAsyncEnumerator(CancellationToken.None);
        var task = enumerator.MoveNextAsync().AsTask();
        if (status == -2)
        {
            var error = await Assert.ThrowsAnyAsync<OperationCanceledException>(() => task);
            Assert.True(task.IsCanceled);
            Assert.Equal(CancellationToken.None, error.CancellationToken);
        }
        else
        {
            await Assert.ThrowsAsync<InvalidOperationException>(() => task);
            Assert.True(task.IsFaulted);
        }
    }

    private static T Synchronous<T>(ValueTask<T> operation)
    {
        if (!operation.IsCompleted) throw new InvalidOperationException("The test stream must complete synchronously.");
        return operation.GetAwaiter().GetResult();
    }

    [Fact]
    public void synchronous_stream_does_not_allocate_a_task_per_item()
    {
        using var source = SynchronousStream();
        var enumerator = source.GetAsyncEnumerator(CancellationToken.None);
        for (int i = 0; i < 256; ++i) Assert.True(Synchronous(enumerator.MoveNextAsync()));
        var before = GC.GetAllocatedBytesForCurrentThread();
        uint sum = 0;
        for (int i = 0; i < 10000; ++i)
        {
            if (!Synchronous(enumerator.MoveNextAsync())) throw new InvalidOperationException();
            sum += enumerator.Current;
        }
        var allocated = GC.GetAllocatedBytesForCurrentThread() - before;
        Assert.True(sum > 0);
        Assert.True(allocated < 4096, $"Stream advancement allocated {allocated} bytes for 10000 items.");
        int drops = _drops;
        var closed = enumerator.DisposeAsync();
        Assert.True(closed.IsCompletedSuccessfully);
        Assert.Equal(drops + 1, _drops);
    }

    [Fact]
    public async Task disposal_acknowledges_an_unconsumed_pending_move()
    {
        var before = Interop.pattern_async_iterator_live();
        using var source = Interop.pattern_async_iterator_create(1, 0, 2);
        var enumerator = source.GetAsyncEnumerator(CancellationToken.None);
        _ = enumerator.MoveNextAsync();
        await enumerator.DisposeAsync();
        Assert.Equal(before, Interop.pattern_async_iterator_live());
    }
}
