using System;
using My.Company;
using My.Company.Common;
using Xunit;
using Interop = My.Company.Interop;

public class TestPatternSlices
{
    [Fact]
    public void pattern_ffi_slice_1()
    {
        using var data = new uint[100_000].Slice();
        var result = Interop.pattern_ffi_slice_1(data);
        Assert.Equal(100_000u, result);
    }

    [Fact]
    public void slice_rejects_negative_indices()
    {
        using var data = new byte[] { 1, 2, 3 }.Slice();
        Assert.Throws<IndexOutOfRangeException>(() => _ = data[-1]);
    }

    [Fact]
    public void pattern_ffi_slice_of_structs_from_native_memory()
    {
        Interop.pattern_ffi_slice_of_structs_callback(attributes =>
        {
            var attribute = attributes[0];
            Assert.Equal(3, attribute.bytes.Count);
            Assert.Equal(2, attribute.bytes[1]);
        });
    }


    [Fact]
    public void pattern_ffi_slice_2()
    {
        using var data = new Vec3f32[]
        {
            new() { x = 1.0f, y = 2.0f, z = 3.0f },
            new() { x = 4.0f, y = 5.0f, z = 6.0f },
            new() { x = 7.0f, y = 8.0f, z = 9.0f }
        }.Slice();

        var result = Interop.pattern_ffi_slice_2(data, 1);

        Assert.Equal(4.0f, result.x);
        Assert.Equal(5.0f, result.y);
        Assert.Equal(6.0f, result.z);
    }

    [Fact]
    public void pattern_ffi_slice_3()
    {
        using var data = new byte[100_000].SliceMut();

        Interop.pattern_ffi_slice_3(data, slice =>
        {
            slice[0] = 1;
            slice[1] = 100;
        });

        Assert.Equal(1, data[0]);
        Assert.Equal(100, data[1]);
    }

    [Fact]
    public void pattern_ffi_slice_5()
    {
        var data1 = new byte[100_000].Slice();
        var data2 = new byte[100_000].SliceMut();

        Interop.pattern_ffi_slice_5(ref data1, ref data2);
        data1.Dispose();
        data2.Dispose();
    }

    [Fact]
    public void pattern_ffi_slice_6()
    {
        var data = new byte[] { 1, 2, 3 }.SliceMut();

        using var callback = new CallbackU8(x =>
        {
            Assert.Equal(1, x);
            return 0;
        });
        Interop.pattern_ffi_slice_6(ref data, callback);
        data.Dispose();
    }

    // [Fact]
    // public void pattern_ffi_slice7()
    // {
    //     var data = new CharArray { str = "test", str_2 = "test2" };
    //     var slice = new SliceMut<CharArray>([data]);
    //     Interop.pattern_ffi_slice_8(ref slice, (ca) => {
    //         Assert.Equal("test", ca.str);
    //         Assert.Equal("test2", ca.str_2);
    //     });
    // }

    [Fact]
    public void pattern_ffi_slice_delegate_huge()
    {
        var result = Interop.pattern_ffi_slice_delegate_huge(x => x[0]);
        Assert.Equal(0, result.x);
    }

    [Fact]
    public void pattern_ffi_slice_9()
    {
        using var use_string = new UseString
        {
            s1 = "hello".Utf8(),
            s2 = "world".Utf8()
        };

        using var slice = new[]
        {
            use_string,
            use_string,
            use_string
        }.Slice();

        var rval = Interop.pattern_ffi_slice_9(slice).IntoString();

        Assert.Equal("hello", rval);
    }

    [Fact]
    public void pattern_ffi_slice_1b()
    {
        using var data = new uint[500].SliceMut();
        var result = Interop.pattern_ffi_slice_1b(data);
        Assert.Equal(500u, result);
    }

    [Fact]
    public void pattern_ffi_slice_4()
    {
        using var slice = new byte[] { 1, 2, 3 }.Slice();
        using var sliceMut = new byte[] { 4, 5, 6 }.SliceMut();
        Interop.pattern_ffi_slice_4(slice, sliceMut);
    }

    [Fact]
    public void slice_of_class_backed_unions_marshals_valid_elements()
    {
        using var some = OptionUtf8String.Some("hello".Utf8());
        using var slice = new[] { some, OptionUtf8String.None }.Slice();

        Assert.Equal(1u, Interop.pattern_ffi_slice_of_option_string(slice));
        Assert.Throws<InvalidOperationException>(() => some.Dispose());
    }

    [Fact]
    public void slice_of_class_backed_unions_rejects_null_elements()
    {
        var exception = Assert.Throws<InvalidOperationException>(
            () => new OptionUtf8String[] { null! }.Slice()
        );

        Assert.Contains("SliceOptionUtf8String element at index 0", exception.Message);
        Assert.Contains("no Rust variant", exception.Message);
    }

    [Fact]
    public void slice_of_class_backed_unions_preserves_post_allocation_conversion_failure()
    {
        var invalid = OptionUtf8String.Some(null!);
        var direct = Assert.Throws<NullReferenceException>(
            () => Interop.pattern_option_string_borrow(invalid)
        );
        var fromSlice = Assert.Throws<NullReferenceException>(
            () => new[] { invalid }.Slice()
        );

        Assert.Equal(direct.Message, fromSlice.Message);

        using var some = OptionUtf8String.Some("hello".Utf8());
        using var valid = new[] { some, OptionUtf8String.None }.Slice();
        Assert.Equal(1u, Interop.pattern_ffi_slice_of_option_string(valid));
    }

    [Fact]
    public void borrowed_native_slice_disposal_does_not_free_rust_memory()
    {
        for (var i = 0; i < 3; ++i)
        {
            Interop.pattern_ffi_slice_of_structs_callback(slice =>
            {
                Assert.Equal(1, slice.Count);
                Assert.Equal((byte)2, slice[0].bytes[1]);
                slice.Dispose();
                slice.Dispose();
                Assert.Throws<ObjectDisposedException>(() => _ = slice.Count);
            });
        }
    }

    [Fact]
    public void disposed_slices_cannot_be_passed_to_rust()
    {
        var fast = SliceByte.From(new byte[] { 1 });
        fast.Dispose();
        using var mutable = SliceMutByte.From(System.Array.Empty<byte>());
        Assert.Throws<ObjectDisposedException>(() => Interop.pattern_ffi_slice_4(fast, mutable));

        var value = OptionUtf8String.Some("value".Utf8());
        try
        {
            var marshalled = SliceOptionUtf8String.From(new[] { value });
            marshalled.Dispose();
            marshalled.Dispose();
            Assert.Throws<ObjectDisposedException>(() => _ = marshalled.Count);
            Assert.Throws<ObjectDisposedException>(() => Interop.pattern_ffi_slice_of_option_string(marshalled));
        }
        finally
        {
            value.Dispose();
        }
    }

    [Fact]
    public void span_overloads_accept_stack_memory_and_write_back()
    {
        ReadOnlySpan<uint> values = stackalloc uint[] { 1, 2, 3 };
        Assert.Equal(3u, Interop.pattern_ffi_slice_1(values));
        Assert.Equal(0u, Interop.pattern_ffi_slice_1(ReadOnlySpan<uint>.Empty));
        Span<byte> mutable = stackalloc byte[] { 4, 5 };
        using var callback = new CallbackSliceMut(slice =>
        {
            Assert.Equal((byte)5, slice[0]);
            slice.Span[1] = 42;
        });
        Interop.pattern_ffi_slice_3(mutable, callback);
        Assert.Equal((byte)5, mutable[0]);
        Assert.Equal((byte)42, mutable[1]);
        Interop.pattern_ffi_slice_4(ReadOnlySpan<byte>.Empty, mutable);
        using var service = ServiceVariousSlices.Create();
        Assert.Equal((byte)5, service.MutSelf(mutable));
        ReadOnlySpan<EnumDocumented> enums = stackalloc EnumDocumented[] { EnumDocumented.B, EnumDocumented.A };
        Assert.Equal(1u, Interop.pattern_ffi_slice_of_unit_enum(enums));
    }

    [Fact]
    public void span_overload_does_not_allocate_managed_wrappers()
    {
        ReadOnlySpan<uint> values = stackalloc uint[] { 1, 2, 3 };
        for (var i = 0; i < 512; ++i) Interop.pattern_ffi_slice_1(values);
        var before = GC.GetAllocatedBytesForCurrentThread();
        uint result = 0;
        for (var i = 0; i < 1024; ++i) result += Interop.pattern_ffi_slice_1(values);
        var allocated = GC.GetAllocatedBytesForCurrentThread() - before;
        Assert.Equal(3072u, result);
        Assert.Equal(0, allocated);
    }
}
