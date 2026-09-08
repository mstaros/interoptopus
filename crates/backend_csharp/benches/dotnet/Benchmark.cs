using System;
using System.Collections.Generic;
using System.Linq;
using System.Threading.Tasks;
using Interoptopus;
using My.Company;
using My.Company.Common;
using Interop = My.Company.Interop;

static class Benchmark {

    static async Task Main(string[] args)
    {
        var smoke = args.Contains("--smoke");
        uint iterations = smoke ? 16u : 100_000u;
        if (smoke) await VerifyAsyncMeasurement();
        Console.WriteLine($"Running benchmarks ({iterations} iterations) ...");

        MeasureResult result;
        var writer = new MarkdownTableWriter();

        using var short_vec = SliceVec3f32.From(new Vec3f32[10]);
        using var short_byte = SliceByte.From(new byte[10]);
        using var short_byte_mut = SliceMutByte.From(new byte[10]);
        using var long_vec = SliceVec3f32.From(new Vec3f32[100_000]);
        var tupled = new Tupled { field_0 = 0 };
        using var callback_huge_prealloc = new CallbackHugeVecSlice(x => x[0]);
        using var serviceAsync = ServiceAsyncResult.Create();
        using var native_callback = Interop.pattern_callback_6();
        var span_values = new uint[10];
        using var hello_world = "hello world".Utf8();
        var deeply_nested = new My.Company.DeeplyNestedWire1 {
            name = new string('x', 50),
            values = Enumerable.Range(0, 3).ToDictionary(
                i => (uint)i,
                i => new My.Company.DeeplyNestedWire2 {
                    values = Enumerable.Range(0, 3).Select(j => new My.Company.DeeplyNestedWire3 {
                        x = Enumerable.Range(0, 3).ToDictionary(k => (uint)k, k => new My.Company.DeeplyNestedWire4 { a = (uint)k }),
                        y = new string('y', 50)
                    }).ToList()
                }
            )
        };

        MeasureResult.Calibrate(iterations, () => {});

        result = MeasureResult.Measure(iterations, () => Interop.primitive_void());
        writer.Add("primitive_void()", result);

        result = MeasureResult.Measure(iterations, () => Interop.primitive_u8(0));
        writer.Add("primitive_u8(0)", result);

        result = MeasureResult.Measure(iterations, () => Interop.primitive_u16(0));
        writer.Add("primitive_u16(0)", result);

        result = MeasureResult.Measure(iterations, () => Interop.primitive_u32(0));
        writer.Add("primitive_u32(0)", result);

        result = MeasureResult.Measure(iterations, () => Interop.primitive_u64(0));
        writer.Add("primitive_u64(0)", result);

        result = MeasureResult.Measure(iterations, () => Interop.pattern_ffi_option_1(OptionInner.None));
        writer.Add("pattern_ffi_option_1(OptionInner.None)", result);

        result = MeasureResult.Measure(iterations, () => Interop.pattern_ffi_slice_delegate(x => x[0]));
        writer.Add("pattern_ffi_slice_delegate(x => x[0])", result);

        result = MeasureResult.Measure(iterations, () => Interop.pattern_ffi_slice_delegate_huge(x => x[0]));
        writer.Add("pattern_ffi_slice_delegate_huge(x => x[0])", result);

        result = MeasureResult.Measure(iterations, () => Interop.pattern_ffi_slice_delegate_huge(callback_huge_prealloc));
        writer.Add("pattern_ffi_slice_delegate_huge(callback_huge_prealloc)", result);

        result = MeasureResult.Measure(iterations, () => Interop.pattern_ffi_slice_2(short_vec, 0));
        writer.Add("pattern_ffi_slice_2(short_vec, 0)", result);

        result = MeasureResult.Measure(iterations, () => Interop.pattern_ffi_slice_2(long_vec, 0));
        writer.Add("pattern_ffi_slice_2(long_vec, 0)", result);

        result = MeasureResult.Measure(iterations, () => Interop.pattern_ffi_slice_4(short_byte, short_byte_mut));
        writer.Add("pattern_ffi_slice_4(short_byte, short_byte)", result);

        result = MeasureResult.Measure(iterations, () => Interop.pattern_ascii_pointer_1("hello world"));
        writer.Add("pattern_ascii_pointer_1('hello world')", result);

        result = MeasureResult.Measure(iterations, () => Interop.pattern_string_10("hello world".Utf8()));
        writer.Add("pattern_string_10('hello world'.Utf8())", result);

        result = MeasureResult.Measure(iterations, () => Interop.pattern_string_10(hello_world.Clone()));
        writer.Add("pattern_string_10(hello_world.Clone())", result);

        result = MeasureResult.Measure(iterations, () => Interop.pattern_string_11(in hello_world));
        writer.Add("pattern_string_11(ref hello_world)", result);

        result = MeasureResult.Measure(iterations, () => "hello world".Utf8().Dispose());
        writer.Add("'hello world'.Utf8().Dispose()", result);

        result = MeasureResult.Measure(iterations, () => hello_world.Clone().Dispose());
        writer.Add("hello_world.Clone().Dispose()", result);

        result = MeasureResult.Measure(iterations, () => VecByte.Empty().Dispose());
        writer.Add("VecU8.Empty().Dispose()", result);

        result = MeasureResult.Measure(iterations, () => VecUtf8String.Empty().Dispose());
        writer.Add("VecUtf8String.Empty().Dispose()", result);

        result = MeasureResult.Measure(iterations, () => Interop.pattern_vec_1().Dispose());
        writer.Add("pattern_vec_u8_return().Dispose()", result);

        result = await MeasureResult.MeasureAsync(iterations, () => Task.CompletedTask);
        writer.Add("await Task.CompletedTask", result);

        result = await MeasureResult.MeasureAsync(iterations, () => serviceAsync.Success());
        writer.Add("await serviceAsync.Success()", result);

        result = MeasureResult.Measure(iterations, () => Interop.wire_accept_string_1(WireOfString.From("hello world")));
        writer.Add("wire_accept_string_1()", result);

        result = MeasureResult.Measure(iterations, () => Interop.wire_deeply_nested_1(deeply_nested.Wire()));
        writer.Add("wire_deeply_nested_1(deeply_nested.Wire())", result);

        result = MeasureResult.Measure(iterations, () => Interop.pattern_ffi_slice_1(span_values.AsSpan()));
        writer.Add("pattern_ffi_slice_1(span)", result);
        result = MeasureResult.Measure(iterations, () => native_callback.Call(1, 2));
        writer.Add("native_callback.Call(1, 2)", result);

        if (!smoke) writer.Write("RESULTS.md", iterations);
    }

    static async Task VerifyAsyncMeasurement()
    {
        var completion = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        var calls = 0;
        var pending = MeasureResult.MeasureAsync(1, () => { ++calls; return completion.Task; });
        if (pending.IsCompleted) throw new InvalidOperationException("Async measurement did not await the invocation.");
        completion.SetResult();
        await pending;
        if (calls != 2) throw new InvalidOperationException("Warmup and measured invocation must both finish.");

        try
        {
            await MeasureResult.MeasureAsync(1, () => Task.FromException(new InvalidOperationException("expected")));
        }
        catch (InvalidOperationException e) when (e.Message == "expected")
        {
            return;
        }
        throw new InvalidOperationException("Async measurement swallowed a failure.");
    }
}
