using System;
using System.Threading;
using System.Threading.Tasks;
using My.Company;
using My.Company.Common;
using Xunit;
using Interop = My.Company.Interop;

public class TestResourceConcurrency
{
    [Fact]
    public void empty_native_callback_rejects_call_and_transfer_without_entering_native_code()
    {
        using var empty = Interop.disposal_empty_callback();
        using var text = "live".Utf8();
        using var vector = Interop.pattern_vec_1();
        Assert.Throws<ObjectDisposedException>(() => empty.Call(0));
        Assert.Throws<ObjectDisposedException>(() => Interop.disposal_borrow_resources(in text, in vector, in empty));
        Assert.Throws<ObjectDisposedException>(() => Interop.pattern_callback_1(empty, 0));
        text.Dispose();
        vector.Dispose();
        Parallel.For(0, 16, _ => empty.Dispose());
    }

    [Fact]
    public void native_void_callback_holds_its_slice_argument_until_return()
    {
        using var slice = SliceMutByte.From(new byte[] { 1 });
        Exception? conflict = null;
        using var managed = new CallbackSliceMut(input =>
        {
            conflict = Record.Exception(slice.Dispose);
            input[0] = 42;
        });
        using var callback = Interop.disposal_native_slice_callback(managed);
        callback.Call(slice);
        Assert.IsType<InvalidOperationException>(conflict);
        Assert.Equal((byte)42, slice[0]);
        slice.Dispose();
    }

    [Theory]
    [InlineData(false)]
    [InlineData(true)]
    public async Task borrowed_resources_reject_disposal_and_transfer_until_native_return(bool nativeCallback)
    {
        AllocationProbe.Settle();
        var baseline = Interop.__test_live_allocations();
        using (var text = "hold".Utf8())
        using (var vector = VecByte.From(new byte[] { 1, 2, 3 }))
        using (var entered = new ManualResetEventSlim())
        using (var release = new ManualResetEventSlim())
        {
            MyCallback callback = null!;
            Exception? textReentry = null, vectorReentry = null, callbackReentry = null;
            bool released = false;
            using var managed = new MyCallback(input =>
            {
                textReentry = Record.Exception(text.Dispose);
                vectorReentry = Record.Exception(vector.Dispose);
                callbackReentry = Record.Exception(callback.Dispose);
                entered.Set();
                released = release.Wait(TimeSpan.FromSeconds(10));
                return input;
            });
            callback = nativeCallback ? Interop.disposal_native_callback(managed) : managed;
            using (callback)
            {
                var call = Task.Run(() => Interop.disposal_borrow_resources(in text, in vector, in callback));
                try
                {
                    Assert.True(entered.Wait(TimeSpan.FromSeconds(10)));
                    Assert.Throws<InvalidOperationException>(text.Dispose);
                    Assert.Throws<InvalidOperationException>(vector.Dispose);
                    Assert.Throws<InvalidOperationException>(callback.Dispose);
                    Assert.Throws<InvalidOperationException>(() => text.IntoString());
                    Assert.Throws<InvalidOperationException>(() => Interop.pattern_vec_2(vector));
                    if (nativeCallback)
                        Assert.Throws<InvalidOperationException>(() => Interop.pattern_callback_1(callback, 0));
                    Assert.Equal("hold", text.String);
                    using var clone = text.Clone();
                    Assert.Equal("hold", clone.String);
                    Assert.Equal(new byte[] { 1, 2, 3 }, vector.ToArray());
                }
                finally
                {
                    release.Set();
                    Assert.Equal(14u, await call.WaitAsync(TimeSpan.FromSeconds(10)));
                }
                Assert.True(released);
                Assert.IsType<InvalidOperationException>(textReentry);
                Assert.IsType<InvalidOperationException>(vectorReentry);
                Assert.IsType<InvalidOperationException>(callbackReentry);
            }
        }
        Assert.Equal(baseline, Interop.__test_live_allocations());
    }

    [Theory]
    [InlineData(0)]
    [InlineData(1)]
    [InlineData(2)]
    public async Task concurrent_transfers_have_exactly_one_owner(int kind)
    {
        AllocationProbe.Settle();
        var baseline = Interop.__test_live_allocations();
        for (int iteration = 0; iteration < 16; ++iteration)
        {
            using var managed = new MyCallback(input => input);
            IDisposable owner;
            Action transfer;
            if (kind == 0)
            {
                var text = "one owner".Utf8();
                owner = text;
                transfer = () => Assert.Equal("one owner", text.IntoString());
            }
            else if (kind == 1)
            {
                var vector = Interop.pattern_vec_1();
                owner = vector;
                transfer = () => Interop.pattern_vec_2(vector);
            }
            else
            {
                var callback = Interop.disposal_native_callback(managed);
                owner = callback;
                transfer = () => Assert.Equal(42u, Interop.pattern_callback_1(callback, 42));
            }
            using (owner)
            using (var start = new Barrier(2))
            {
                Task<Exception?> Attempt() => Task.Run(() =>
                {
                    if (!start.SignalAndWait(TimeSpan.FromSeconds(10))) throw new TimeoutException();
                    return Record.Exception(transfer);
                });
                var results = await Task.WhenAll(Attempt(), Attempt()).WaitAsync(TimeSpan.FromSeconds(15));
                Assert.Single(results, error => error == null);
                Assert.Single(results, error => error is ObjectDisposedException);
                await Task.Run(() => Parallel.For(0, 16, _ => owner.Dispose()));
            }
        }
        Assert.Equal(baseline, Interop.__test_live_allocations());
    }

    [Fact]
    public void nested_borrows_retain_original_fields_across_callback_mutation()
    {
        using var first = "first".Utf8();
        using var second = "second".Utf8();
        using var holder = new UseString { s1 = first, s2 = second };
        using var slice = SliceUseString.From(new[] { holder });
        holder.s1 = "replacement".Utf8();
        Assert.Throws<InvalidOperationException>(first.Dispose);
        Assert.Throws<InvalidOperationException>(second.Dispose);
        using var returned = Interop.pattern_ffi_slice_9(slice);
        Assert.Equal("first", returned.String);
        slice.Dispose();
        first.Dispose();
        second.Dispose();
        Assert.Equal("replacement", holder.s1.String);
    }

    [Fact]
    public void transferred_composite_keeps_borrowed_fields_alive_during_the_call()
    {
        using var text = "borrowed".Utf8();
        using var slice = SliceUtf8String.From(new[] { text });
        using var vector = VecUtf8String.From(new[] { text });
        using var holder = new UseSliceAndVec { s1 = slice, s2 = vector };
        Exception? conflict = null;
        Assert.Equal(1u, Interop.disposal_borrow_owned_fields(holder, _ =>
        {
            conflict = Record.Exception(slice.Dispose);
            return 0;
        }));
        Assert.IsType<InvalidOperationException>(conflict);
        slice.Dispose();
        text.Dispose();
    }

    [Fact]
    public void vector_span_is_an_independent_copy()
    {
        using var vector = Interop.pattern_vec_1();
        var copy = vector.AsSpan();
        vector.Dispose();
        Assert.Equal(new byte[] { 1, 2, 3 }, copy.ToArray());
    }
}