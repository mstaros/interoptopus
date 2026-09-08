using System;
using System.Linq;
using System.Runtime.InteropServices;
using My.Company;
using My.Company.Common;
using Xunit;
using Interop = My.Company.Interop;

public class TestPatternDelegates
{
    [Fact]
    public void pattern_callback_1_adhoc()
    {
        var x = Interop.pattern_callback_1(value => value + 1, 0);
        Assert.Equal(1u, x);
    }

    [Fact]
    public void pattern_callback_1_retained()
    {
        using var cb = new MyCallback(value => value + 1);
        var x = Interop.pattern_callback_1(cb, 0);
        Assert.Equal(1u, x);
    }

    [Fact]
    public void pattern_callback_2()
    {
        var called = false;
        var cb = new MyCallbackVoid(ptr => { called = true; });
        var x = Interop.pattern_callback_2(cb);
        x.Call(IntPtr.Zero);

        // TODO
        // Assert.True(called);
        var _ = called;
    }


    [Fact]
    public void pattern_callback_4()
    {
        using var cb = new MyCallbackNamespaced(value => value);
        var y = Interop.pattern_callback_4(cb, 5);
        Assert.Equal(5u, y);
    }

    [Fact]
    public void pattern_callback_5()
    {
        using var cb = Interop.pattern_callback_5();
        cb.Call();
    }

    [Fact]
    public void pattern_callback_6()
    {
        using var cb = Interop.pattern_callback_6();
        var rval = cb.Call(1, 2);

        Assert.Equal(3, rval);
    }

    [Fact]
    public void pattern_ffi_slice_delegate()
    {
        Interop.pattern_ffi_slice_delegate(x =>
        {
            Assert.Equal(10, x.Count);
            Assert.Equal(0, x[0]);
            Assert.Equal(5, x[5]);

            // Test IEnumerable using LINQ
            var arr = x.ToArray();
            Assert.Equal(10, arr.Length);
            Assert.Equal(0, arr[0]);
            Assert.Equal(5, arr[5]);

            return x[0];
        });
    }

    [Fact]
    public void pattern_callback_7_exception_handling()
    {
        // This value will be changed to `6` before the callbacks are invoked, and
        // changed to `8` after callbacks return.
        var rval = 0;
        var called_c1 = false;
        var called_c2 = false;
        var called_exception = false;


        ResultVoidError C1(int x, int y)
        {
            // This should see `6` here, and it does, because the function has set that value.
            Assert.Equal(6, rval);
            called_c1 = true;

            // However, when we now throw we would expect .NET to unwind that exception back
            // into Rust (and therefore eventually observe rval to be `8`). When we use this
            // safe wrapper it does that, but only because we implemented special handling for
            // it for callbacks that return a FFIError.
            throw new Exception("We handled this");
        }

        void C2(int x, int y)
        {
            // This callback looks very similar. However, it does not come with special handling
            // support. When this throws, the .NET runtime will NOT(!!!) unwind back through
            // Rust but instead just return the stack up without ever calling Rust again. That means
            // in particular that any code that you would expect to run in Rust subsequent to the invocation
            // of this callback (esp. `drop` code and friends) will NOT fire, leading to unexpected
            // memory loss or worse.
            Assert.Equal(6, rval);
            called_c2 = true;
            throw new Exception("Unchecked callback which we didn't handle. Comment this out and see the test fail.");
        }

        ;

        var cc1 = new SumDelegateReturn(C1);
        var cc2 = new SumDelegateReturn2(C2);

        try
        {
            // Interop.pattern_callback_7(C1, C2, 3, 7, out rval);
            Interop.pattern_callback_7(cc1, cc2, 3, 7, ref rval).AsOk();
            cc1.Dispose();
            cc2.Dispose();
        }
        catch (Exception)
        {
            // If everything works Rust code after invoking C1 and C2 is still executed,
            // setting this variable to `8`.
            called_exception = true;
            Assert.Equal(8, rval);
        }

        Assert.True(called_c1);
        Assert.True(called_c2);
        Assert.True(called_exception);
    }

    [Fact]
    public void pattern_callback_8()
    {
        var r1 = string.Empty;
        var r2a = string.Empty;
        var r2b = string.Empty;

        Interop.pattern_callback_8(s => { r1 = s.IntoString(); }, s =>
        {
            r2a = s.s1.IntoString();
            r2b = s.s2.IntoString();
        }, "hello world".Utf8());

        Assert.Equal("hello world", r1);
        Assert.Equal("hello world", r2a);
        Assert.Equal("hello world", r2b);
    }


    [Fact]
    public void pattern_callback_9()
    {
        var result = Interop.pattern_callback_9((x, y) =>
        {
            Marshal.ReadInt32(x);
            Marshal.ReadInt32(y);
            Marshal.WriteInt32(y, 42);
        });
        Assert.Equal(42, result);
    }

    [Fact]
    public void returned_callback_moves_ownership_and_survives_gc()
    {
        var calls = 0;
        using var source = new MyCallbackVoid(_ => ++calls);
        using var first = Interop.pattern_callback_2(source);
        using var second = Interop.pattern_callback_2(first);
        Assert.Throws<ObjectDisposedException>(() => first.Call(IntPtr.Zero));
        Assert.Throws<ObjectDisposedException>(() => Interop.pattern_callback_2(first));
        first.Dispose();
        GC.Collect();
        GC.WaitForPendingFinalizers();
        second.Call(IntPtr.Zero);
        Assert.Equal(1, calls);
        second.Dispose();
        second.Dispose();
    }

    [Fact]
    public void callback_disposal_invalidates_even_when_rethrowing()
    {
        var callback = new MyCallback(_ => throw new InvalidOperationException("callback failure"));
        Interop.pattern_callback_1(callback, 0);
        Assert.Throws<InvalidOperationException>(() => callback.Dispose());
        Assert.Throws<ObjectDisposedException>(() => callback.Call(0));
        callback.Dispose();
    }

    [Fact]
    public void managed_void_callback_is_invoked_once()
    {
        var count = 0;
        using var callback = new MyCallbackVoid(_ => ++count);
        callback.Call(IntPtr.Zero);
        Assert.Equal(1, count);
    }

    [Fact]
    public void rust_retains_managed_callback_after_wrapper_leaves_scope()
    {
        var count = 0;
        MyCallbackVoid Retain()
        {
            using var callback = new MyCallbackVoid(_ => ++count);
            return Interop.pattern_callback_2(callback);
        }
        using var retained = Retain();
        GC.Collect();
        GC.WaitForPendingFinalizers();
        GC.Collect();
        retained.Call(IntPtr.Zero);
        Assert.Equal(1, count);
    }

    [Fact]
    public void native_callback_calls_do_not_allocate_delegates()
    {
        using var callback = Interop.pattern_callback_6();
        for (var i = 0; i < 512; ++i) callback.Call(1, 2);
        var before = GC.GetAllocatedBytesForCurrentThread();
        var sum = 0;
        for (var i = 0; i < 1024; ++i) sum += callback.Call(1, 2);
        var allocated = GC.GetAllocatedBytesForCurrentThread() - before;
        Assert.Equal(3072, sum);
        Assert.Equal(0, allocated);
    }
}
