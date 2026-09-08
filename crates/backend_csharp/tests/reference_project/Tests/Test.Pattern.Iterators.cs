using System;
using System.Collections;
using System.Collections.Generic;
using System.Linq;
using Rust.Linq;
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
        using var returned = Interop.pattern_iterator_echo((IteratorUint)filtered);
        using var query = returned.Where(_ => { ++downstream; return true; }).Take(2);
        var actual = Assert.Throws<ApplicationException>(() => query.Any());
        Assert.Same(expected, actual);
        Assert.Equal(1, calls);
        Assert.Equal(0, downstream);
        Assert.Equal(before, Interop.pattern_iterator_live());
    }

    private sealed class Capture { public uint Threshold = 2; }

    [MethodImpl(MethodImplOptions.NoInlining)]
    private static IRustEnumerable<uint> CapturingQuery(out WeakReference capture)
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
        IRustEnumerable<uint> query = null!;
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

    [Fact]
    public void torust_selects_operators_without_conflicting_with_system_linq()
    {
        IEnumerable<int> values = new[] { 1, 2, 3, 4 };
        Assert.Equal(new[] { 3, 4 }, values.Where(x => x > 2).ToArray());
        using IRustEnumerable<int> selected = values.ToRust().Where(x => x > 1).Take(2);
        Assert.Equal(new[] { 2, 3 }, selected.ToArray());
        Assert.True(selected.Any(x => x == 3));
        Assert.True(selected.Any()); // Managed sources retain normal repeatable enumeration.
        Assert.Same(selected, selected.ToRust());
    }

    [Fact]
    public void managed_adapter_is_deferred_and_accepts_unregistered_reference_types()
    {
        int visits = 0;
        int disposed = 0;
        IEnumerable<string> Source()
        {
            try
            {
                foreach (var value in new[] { "a", "bb", "ccc" })
                {
                    ++visits;
                    yield return value;
                }
            }
            finally { ++disposed; }
        }

        using var query = Source().ToRust().Where(x => x.Length > 1).Take(1);
        Assert.Equal(0, visits);
        Assert.True(query.Any());
        Assert.Equal(2, visits);
        Assert.Equal(1, disposed);
        Assert.Equal(new[] { "bb" }, query.ToArray());
        Assert.Equal(2, disposed);
    }

    [Fact]
    public void managed_exception_disposes_the_active_enumerator()
    {
        bool disposed = false;
        var expected = new ApplicationException("managed predicate");
        IEnumerable<string> Source()
        {
            try { yield return "a"; }
            finally { disposed = true; }
        }

        using var query = Source().ToRust().Where(_ => throw expected);
        Assert.Same(expected, Assert.Throws<ApplicationException>(() => query.Any()));
        Assert.True(disposed);
    }

    [Fact]
    public void enumerable_upcast_preserves_native_dispatch()
    {
        var before = Interop.pattern_iterator_live();
        using var native = Interop.pattern_iterator_create(20);
        IEnumerable<uint> values = native;
        Assert.Same(native, values.ToRust());
        using var query = values.ToRust().Where(x => x % 2 == 0).Take(2);
        Assert.IsType<IteratorUint>(query);
        Assert.True(query.Any(x => x == 2));
        Assert.Equal(before, Interop.pattern_iterator_live());
    }

    [Fact]
    public void native_queries_support_foreach_and_dispose_on_early_break()
    {
        var before = Interop.pattern_iterator_live();
        using var query = Interop.pattern_iterator_create(20).Where(x => x >= 3);
        foreach (uint value in query)
        {
            Assert.Equal(3U, value);
            break;
        }
        Assert.Equal(before, Interop.pattern_iterator_live());
        Assert.Throws<ObjectDisposedException>(() => query.GetEnumerator());
    }

    [Fact]
    public void native_enumeration_copies_values_and_releases_on_exhaustion()
    {
        var before = Interop.pattern_iterator_live();
        using var query = Interop.pattern_iterator_create(6).Where(x => x % 2 == 0);
        Assert.Equal(new uint[] { 0, 2, 4 }, query.ToArray());
        Assert.Equal(before, Interop.pattern_iterator_live());

        using var points = Interop.pattern_iterator_points();
        var point = Assert.Single(points.ToArray());
        Assert.Equal(1, point.x);
        Assert.Equal(3, point.z);
        using var flags = Interop.pattern_iterator_bools();
        Assert.Equal(new[] { false, true }, flags.ToArray());
    }

    [Fact]
    public void native_enumerator_disposal_before_first_move_does_not_evaluate()
    {
        var before = Interop.pattern_iterator_live();
        var visits = Interop.pattern_iterator_visits();
        using var query = Interop.pattern_iterator_create(3);
        using var enumerator = query.GetEnumerator();
        Assert.Throws<InvalidOperationException>(() => enumerator.Current);
        Assert.Throws<NotSupportedException>(() => enumerator.Reset());
        enumerator.Dispose();
        enumerator.Dispose();
        Assert.False(enumerator.MoveNext());
        Assert.Equal(visits, Interop.pattern_iterator_visits());
        Assert.Equal(before, Interop.pattern_iterator_live());
    }

    [Fact]
    public void native_enumeration_rethrows_predicate_failure_and_native_panic_after_cleanup()
    {
        var before = Interop.pattern_iterator_live();
        var expected = new ApplicationException("enumeration predicate");
        using var query = Interop.pattern_iterator_create(3).Where(_ => throw expected);
        using var enumerator = query.GetEnumerator();
        Assert.Same(expected, Assert.Throws<ApplicationException>(() => enumerator.MoveNext()));
        Assert.False(enumerator.MoveNext());
        Assert.Equal(before, Interop.pattern_iterator_live());

        using var panicking = Interop.pattern_iterator_panic();
        using var failed = panicking.GetEnumerator();
        Assert.Throws<InvalidOperationException>(() => failed.MoveNext());
        Assert.Equal(before, Interop.pattern_iterator_live());
    }

    [Fact]
    public void disposing_an_enumerator_from_its_predicate_defers_native_release()
    {
        var before = Interop.pattern_iterator_live();
        IEnumerator<uint> enumerator = null!;
        using var query = Interop.pattern_iterator_create(3).Where(_ =>
        {
            enumerator.Dispose();
            Assert.Equal(before + 1, Interop.pattern_iterator_live());
            return true;
        });
        enumerator = query.GetEnumerator();
        using (enumerator)
            Assert.False(enumerator.MoveNext());
        Assert.Equal(before, Interop.pattern_iterator_live());
    }

    [Fact]
    public void torust_rejects_null_and_nonpositive_managed_take_remains_deferred()
    {
        IEnumerable<string> missing = null!;
        Assert.Throws<ArgumentNullException>(() => missing.ToRust());
        using var source = new[] { "a" }.ToRust();
        Assert.Throws<ArgumentNullException>(() => source.Where(null!));
        Assert.Throws<ArgumentNullException>(() => source.Any(null!));
        Assert.False(source.Where(_ => throw new Exception("must not run")).Take(-1).Any());
    }

    [Fact]
    public void predicate_can_request_disposal_on_another_thread()
    {
        var before = Interop.pattern_iterator_live();
        IEnumerator<uint> enumerator = null!;
        using var query = Interop.pattern_iterator_create(3).Where(_ =>
        {
            var release = new System.Threading.Thread(enumerator.Dispose);
            release.Start();
            Assert.True(release.Join(TimeSpan.FromSeconds(5)), "Dispose must not wait on the executing predicate.");
            Assert.Equal(before + 1, Interop.pattern_iterator_live());
            return true;
        });
        enumerator = query.GetEnumerator();
        using (enumerator)
            Assert.False(enumerator.MoveNext());
        Assert.Equal(before, Interop.pattern_iterator_live());
    }
}
