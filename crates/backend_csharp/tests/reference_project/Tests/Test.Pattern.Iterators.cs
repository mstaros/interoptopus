using System;
using System.Runtime.CompilerServices;
using My.Company;
using My.Company.Common;
using Xunit;
using Interop = My.Company.Interop;

public class TestPatternIterators
{
    [Fact]
    public void deferred_chain_stops_after_take()
    {
        var before = Interop.pattern_iterator_live();
        var visits = Interop.pattern_iterator_visits();
        int calls = 0;
        using var source = Interop.pattern_iterator_create(100);
        using var filtered = source.Where(x => { ++calls; return x % 2 == 0; });
        using var limited = filtered.Take(2);
        Assert.Equal(0, calls);
        Assert.Equal(visits, Interop.pattern_iterator_visits());
        Assert.False(limited.Where(_ => false).Any());
        Assert.Equal(3, calls);
        Assert.Equal(visits + 3, Interop.pattern_iterator_visits());
        Assert.Equal(before, Interop.pattern_iterator_live());
    }

    [Fact]
    public void any_short_circuits_and_consumes()
    {
        var before = Interop.pattern_iterator_live();
        using var query = Interop.pattern_iterator_create(100);
        int calls = 0;
        Func<uint, bool> predicate = x => { ++calls; return x == 4; };
        Assert.True(query.Any(predicate));
        Assert.Equal(5, calls);
        Assert.Equal(before, Interop.pattern_iterator_live());
        Assert.Throws<ObjectDisposedException>(() => query.Any());
    }

    [Fact]
    public void empty_and_nonpositive_take_do_not_evaluate_predicates()
    {
        foreach (int count in new[] { 0, -1 })
        {
            var visits = Interop.pattern_iterator_visits();
            using var query = Interop.pattern_iterator_create(3);
            Assert.False(query.Where(_ => throw new Exception("must not run")).Take(count).Any());
            Assert.Equal(visits, Interop.pattern_iterator_visits());
        }
        Assert.False(Interop.pattern_iterator_create(0).Any());
    }

    [Fact]
    public void moving_and_disposing_stages_does_not_double_free()
    {
        var before = Interop.pattern_iterator_live();
        using var source = Interop.pattern_iterator_create(3);
        using var filtered = source.Where(_ => true);
        using var limited = filtered.Take(1);
        source.Dispose();
        filtered.Dispose();
        Assert.Equal(before + 1, Interop.pattern_iterator_live());
        Assert.Throws<ObjectDisposedException>(() => source.Where(_ => true));
        Assert.Throws<ObjectDisposedException>(() => filtered.Take(1));
        limited.Dispose();
        limited.Dispose();
        Assert.Equal(before, Interop.pattern_iterator_live());
    }

    [Fact]
    public void exception_survives_native_roundtrip_and_stops_downstream_callbacks()
    {
        var before = Interop.pattern_iterator_live();
        var expected = new ApplicationException("predicate failure");
        int calls = 0;
        int downstream = 0;
        using var source = Interop.pattern_iterator_create(50);
        using var filtered = source.Where(_ => { ++calls; throw expected; });
        using var returned = Interop.pattern_iterator_echo(filtered);
        using var query = returned.Where(_ => { ++downstream; return true; }).Take(2);
        var actual = Assert.Throws<ApplicationException>(() => query.Any());
        Assert.Same(expected, actual);
        Assert.Equal(1, calls);
        Assert.Equal(0, downstream);
        Assert.Equal(before, Interop.pattern_iterator_live());
    }

    private sealed class Capture { public uint Threshold = 2; }

    [MethodImpl(MethodImplOptions.NoInlining)]
    private static IteratorUint CapturingQuery(out WeakReference capture)
    {
        var state = new Capture();
        capture = new WeakReference(state);
        return Interop.pattern_iterator_create(5).Where(x => x >= state.Threshold);
    }

    private static void Collect()
    {
        GC.Collect();
        GC.WaitForPendingFinalizers();
        GC.Collect();
    }

    [Fact]
    public void predicates_are_rooted_until_disposal_or_terminal_evaluation()
    {
        using var abandoned = CapturingQuery(out var first);
        Collect();
        Assert.True(first.IsAlive);
        abandoned.Dispose();
        Collect();
        Assert.False(first.IsAlive);

        using var consumed = CapturingQuery(out var second);
        Collect();
        Assert.True(second.IsAlive);
        Assert.True(consumed.Any());
        Collect();
        Assert.False(second.IsAlive);
        GC.KeepAlive(abandoned);
        GC.KeepAlive(consumed);
    }

    [Fact]
    public void registered_struct_and_boolean_elements_use_typed_funcs()
    {
        Func<Vec3f32, bool> point = p => p.x == 1 && p.z == 3;
        Assert.True(Interop.pattern_iterator_points().Any(point));
        Func<bool, bool> flag = x => x;
        Assert.True(Interop.pattern_iterator_bools().Any(flag));
    }

    [Fact]
    public void native_panic_is_reported_after_cleanup()
    {
        var before = Interop.pattern_iterator_live();
        using var query = Interop.pattern_iterator_panic();
        Assert.Throws<InvalidOperationException>(() => query.Any());
        Assert.Equal(before, Interop.pattern_iterator_live());
    }

    [Fact]
    public void reentrant_use_of_consumed_query_is_rejected()
    {
        var before = Interop.pattern_iterator_live();
        IteratorUint query = null!;
        query = Interop.pattern_iterator_create(3).Where(_ => query.Any());
        using (query)
            Assert.Throws<ObjectDisposedException>(() => query.Any());
        Assert.Equal(before, Interop.pattern_iterator_live());
    }

    [Fact]
    public void invalid_arguments_preserve_the_source()
    {
        using var query = Interop.pattern_iterator_create(3);
        Assert.Throws<ArgumentNullException>(() => query.Where(null!));
        Assert.True(query.Any());
    }
}
