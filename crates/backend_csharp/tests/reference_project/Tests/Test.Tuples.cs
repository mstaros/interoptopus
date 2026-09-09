using System;
using System.Collections.Generic;
using System.Linq;
using System.Threading.Tasks;
using My.Company;
using My.Company.Common;
using Rust.Linq;
using Xunit;
using Interop = My.Company.Interop;

public class TestTuples
{
    [Fact]
    public void positional_calls_use_value_tuples_in_both_directions()
    {
        (uint, float) result = Interop.tuple_echo((17u, 2.5f));
        Assert.Equal((17u, 2.5f), result);
        var method = typeof(Interop).GetMethod("tuple_echo")!;
        Assert.Equal(typeof((uint, float)), method.ReturnType);
        Assert.Equal(typeof((uint, float)), method.GetParameters()[0].ParameterType);
    }

    [Fact]
    public void named_value_structs_have_constructors_and_deconstruction()
    {
        var point = new Vec3f32(1, 2, 3);
        var (x, y, z) = point;
        Assert.Equal((1f, 2f, 3f), (x, y, z));
        Assert.Equal(typeof(Vec3f32), point.GetType());
    }

    [Fact]
    public void nested_tuples_copy_booleans_and_named_values()
    {
        var point = new Vec3f32(1, 2, 3);
        var result = Interop.tuple_nested_echo(((7u, 1.25f), (true, false), point));
        Assert.Equal((7u, 1.25f), result.Item1);
        Assert.Equal((true, false), result.Item2);
        Assert.Equal(point.x, result.Item3.x);
        Assert.Equal(point.z, result.Item3.z);
    }

    [Fact]
    public void native_alignment_and_eight_element_tuples_round_trip()
    {
        Assert.Equal(((byte)255, 0xfedcba9876543210UL),
            Interop.tuple_aligned_echo((255, 0xfedcba9876543210UL)));
        (byte, ushort, uint, ulong, sbyte, short, int, long) input =
            (1, 65535, uint.MaxValue, ulong.MaxValue, -1, short.MinValue, int.MinValue, long.MinValue);
        Assert.Equal(input, Interop.tuple_eight_echo(input));
    }

    [Fact]
    public void named_struct_fields_and_deconstruction_use_nested_tuples()
    {
        var value = new TupleContainer((1u, 2f), (3u, 4f));
        var result = Interop.tuple_container_echo(value);
        Assert.Equal((1u, 2f), result.first);
        var (first, second) = result;
        Assert.Equal((1u, 2f), first);
        Assert.Equal((3u, 4f), second);
    }

    [Fact]
    public void func_and_action_callbacks_accept_tuples()
    {
        Func<(uint, float), (uint, float)> transform = x => (x.Item1 + 2, x.Item2 * 2);
        Assert.Equal((5u, 3f), Interop.tuple_apply(transform, (3u, 1.5f)));
        (uint, float) seen = default;
        Action<(uint, float)> visit = x => seen = x;
        Interop.tuple_visit(visit, (9u, 4.5f));
        Assert.Equal((9u, 4.5f), seen);
    }

    [Fact]
    public void callbacks_received_from_rust_accept_and_return_tuples()
    {
        using var callback = Interop.tuple_native_callback();
        Assert.Equal((5u, 2.5f), callback.Call((4u, 2f)));
    }

    [Fact]
    public void callback_exceptions_keep_their_identity()
    {
        var expected = new InvalidOperationException("tuple callback failed");
        Func<(uint, float), (uint, float)> callback = _ => throw expected;
        var actual = Assert.Throws<InvalidOperationException>(() => Interop.tuple_apply(callback, (1u, 1f)));
        Assert.Same(expected, actual);
    }

    [Fact]
    public void service_results_and_callback_arguments_are_tuples()
    {
        using var service = ServiceTuple.Create();
        Assert.Equal((3u, 4f), service.Pair((3u, 4f)));
        Assert.Equal((5u, 6f), service.TryPair((5u, 6f)));
        Assert.Equal((7u, 9f), service.Apply(x => (x.Item1, x.Item2 + 1), (7u, 8f)));
        Assert.Equal(typeof((uint, float)), typeof(ServiceTuple).GetMethod("TryPair")!.ReturnType);
    }

    [Fact]
    public async Task async_service_results_use_task_of_value_tuple()
    {
        using var service = ServiceTuple.Create();
        Task<(uint, float)> direct = service.PairLaterAsync((3u, 4f), TestContext.Current.CancellationToken);
        Task<(uint, float)> wrapped = service.TryPairLaterAsync((5u, 6f), TestContext.Current.CancellationToken);
        Assert.Equal((3u, 4f), await direct);
        Assert.Equal((5u, 6f), await wrapped);
    }

    [Fact]
    public void native_iterator_predicates_and_managed_projections_receive_tuples()
    {
        using var service = ServiceTuple.Create();
        using var values = service.Pairs();
        using var filtered = values.Where(x => x.Item1 > 0).Take(2);
        var rows = filtered.Select(x => new { Id = x.Item1, Weight = x.Item2 }).ToArray();
        Assert.Equal(new uint[] { 1, 2 }, rows.Select(x => x.Id));
        Assert.Equal(new float[] { 1, 2 }, rows.Select(x => x.Weight));
        using var another = service.Pairs();
        Assert.True(another.Any(x => x == (3u, 3f)));
    }

    [Fact]
    public async Task native_async_streams_expose_value_tuples()
    {
        using var service = ServiceTuple.Create();
        await using var scope = new ScriptScope(TestContext.Current.CancellationToken);
        IAsyncEnumerable<(uint, float)> values = service.PairStreamAsync();
        var result = await values.Where(x => x.Item1 > 1).ToArrayAsync(TestContext.Current.CancellationToken);
        Assert.Equal(new[] { (2u, 2f), (3u, 3f) }, result);
    }

    [Fact]
    public void borrowed_inputs_keep_the_native_wrapper()
    {
        TuplePair raw = (7u, 2.5f);
        Assert.Equal((7u, 2.5f), Interop.tuple_borrow(in raw));
        using var slice = SliceTuplePair.From(new TuplePair[] { (1u, 1f), (2u, 2f) });
        Assert.Equal(3u, Interop.tuple_slice_sum(slice));
        Assert.Equal((7u, 2.5f), ((uint, float))raw);
    }

    [Fact]
    public void single_field_borrowed_and_owning_structs_retain_identity()
    {
        var id = new TupleId(42);
        Assert.Equal(42UL, Interop.tuple_id_echo(id).field_0);
        Assert.Equal(typeof(TupleId), typeof(Interop).GetMethod("tuple_id_echo")!.ReturnType);
        Assert.Equal(typeof(TupleOwned), typeof(Interop).GetMethod("tuple_owned_echo")!.ReturnType);
        Assert.True(typeof(IDisposable).IsAssignableFrom(typeof(TupleOwned)));
        Assert.Equal(typeof(TupleBorrowed), typeof(Interop).GetMethod("tuple_borrowed_sum")!.GetParameters()[0].ParameterType);
    }

    [Fact]
    public void union_cases_keep_identity_while_exposing_tuple_payloads()
    {
        TupleChoice left = new TupleChoice.LeftCase((2u, 3f));
        TupleChoice right = new TupleChoice.RightCase((2u, 3f));
        static string Describe(TupleChoice choice) => choice switch
        {
            TupleChoice.LeftCase(var pair) => $"left:{pair.Item1}",
            TupleChoice.RightCase(var pair) => $"right:{pair.Item1}",
            TupleChoice.EmptyCase => "empty",
            null => "unset"
        };
        Assert.Equal("left:2", Describe(Interop.tuple_choice_echo(left)));
        Assert.Equal("right:2", Describe(Interop.tuple_choice_echo(right)));
        Assert.Equal("empty", Describe(Interop.tuple_choice_echo(TupleChoice.Empty)));
        Assert.Equal("unset", Describe(default));
        Assert.True(left.TryGetValue(out TupleChoice.LeftCase selected));
        Assert.Equal((2u, 3f), selected.Value);
        Assert.False(left.TryGetValue(out TupleChoice.RightCase _));
        Assert.Equal(typeof((uint, float)), typeof(TupleChoice.LeftCase).GetProperty("Value")!.PropertyType);
        Assert.Equal((2u, 3f), left.AsLeft());
    }

    [Fact]
    public void option_and_result_helpers_preserve_tuple_payload_types()
    {
        var option = Interop.tuple_option_echo(OptionTuplePair.Some((4u, 5f)));
        Assert.Equal((4u, 5f), option.AsSome());
        var result = Interop.tuple_result_echo(ResultTuplePairError.FromCall(() => (6u, 7f)));
        Assert.Equal((6u, 7f), result.AsOk());
        Assert.Equal(typeof((uint, float)), typeof(ResultTuplePairError).GetMethod("AsOk")!.ReturnType);
    }

    [Fact]
    public void bare_native_callbacks_use_explicit_native_structs()
    {
        var point = new Vec3f32(1, 2, 3);
        var result = Interop.tuple_bare_apply(native =>
        {
            var marshaller = new TupleNested.TupleMarshaller();
            marshaller.FromUnmanaged(native);
            var (pair, flags, point) = marshaller.ToManaged();
            marshaller.FromManaged(((pair.Item1 + 1, pair.Item2), flags, point));
            return marshaller.ToUnmanaged();
        }, ((7u, 1.25f), (true, false), point));
        Assert.Equal((8u, 1.25f), result.Item1);
        Assert.Equal((true, false), result.Item2);
        Assert.Equal(3f, result.Item3.z);
    }
}
