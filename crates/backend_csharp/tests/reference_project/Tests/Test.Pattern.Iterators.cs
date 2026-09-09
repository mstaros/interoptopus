using System;
using System.Collections;
using System.Collections.Generic;
using System.Linq;
using Rust.Linq;
using System.Runtime.CompilerServices;
using System.Threading;
using System.Threading.Tasks;
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
            Assert.False(query.Where((Func<uint, bool>)(_ => throw new Exception("must not run"))).Take(count).Any());
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
        using var filtered = source.Where((Func<uint, bool>)(_ => { ++calls; throw expected; }));
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
        Assert.Throws<ArgumentNullException>(() => query.Where((Func<uint, bool>)null!));
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

        using var query = Source().ToRust().Where((Func<string, bool>)(_ => throw expected));
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
        using var query = Interop.pattern_iterator_create(3).Where((Func<uint, bool>)(_ => throw expected));
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
        Assert.Throws<ArgumentNullException>(() => source.Where((Func<string, bool>)null!));
        Assert.Throws<ArgumentNullException>(() => source.Any(null!));
        Assert.False(source.Where((Func<string, bool>)(_ => throw new Exception("must not run"))).Take(-1).Any());
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

    [Fact]
    public async Task async_filter_is_deferred_sequential_and_keeps_native_prefix()
    {
        var before = Interop.pattern_iterator_live();
        var visits = Interop.pattern_iterator_visits();
        using var source = Interop.pattern_iterator_create(20).Where(x => x % 2 == 0);
        var gate = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        int calls = 0;
        Func<uint, Task<bool>> predicate = async _ =>
        {
            ++calls;
            await gate.Task;
            return true;
        };
        var query = source.Where(predicate).Take(2);
        Assert.Equal(0, calls);
        Assert.Equal(visits, Interop.pattern_iterator_visits());
        var pending = query.ToArrayAsync().AsTask();
        Assert.False(pending.IsCompleted);
        Assert.Equal(1, calls);
        Assert.Equal(visits + 1, Interop.pattern_iterator_visits());
        Assert.Equal(before + 1, Interop.pattern_iterator_live());
        gate.SetResult();
        Assert.Equal(new uint[] { 0, 2 }, await pending);
        Assert.Equal(2, calls);
        Assert.Equal(visits + 3, Interop.pattern_iterator_visits());
        Assert.Equal(before, Interop.pattern_iterator_live());
    }

    [Fact]
    public async Task await_foreach_on_rust_source_releases_on_break_and_exhaustion()
    {
        var before = Interop.pattern_iterator_live();
        using IRustEnumerable<uint> source = Interop.pattern_iterator_create(10);
        await foreach (uint item in source)
        {
            Assert.Equal(0U, item);
            break;
        }
        Assert.Equal(before, Interop.pattern_iterator_live());
        Assert.Throws<ObjectDisposedException>(() => source.GetEnumerator());

        using var flags = Interop.pattern_iterator_bools();
        var values = new List<bool>();
        await foreach (bool item in flags)
            values.Add(item);
        Assert.Equal(new[] { false, true }, values);
        using var points = Interop.pattern_iterator_points();
        var point = Assert.Single(await points.ToAsyncEnumerable().ToArrayAsync());
        Assert.Equal(3, point.z);
    }

    [Fact]
    public async Task async_managed_sources_remain_repeatable_and_support_reference_types()
    {
        int visits = 0;
        int disposed = 0;
        IEnumerable<string> Source()
        {
            try
            {
                foreach (string value in new[] { "a", "bb", "ccc" })
                {
                    ++visits;
                    yield return value;
                }
            }
            finally { ++disposed; }
        }
        using var source = Source().ToRust();
        var query = source.Where(async x => { await Task.Yield(); return x.Length > 1; })
            .Where(async x => { await Task.Yield(); return x.Length < 3; }).Take(1);
        Assert.Equal(0, visits);
        Assert.Equal(new[] { "bb" }, await query.ToArrayAsync());
        Assert.Equal(2, visits);
        Assert.Equal(1, disposed);
        Assert.True(await query.AnyAsync(async x => { await Task.Yield(); return x == "bb"; }));
        Assert.Equal(2, disposed);
    }

    [Fact]
    public async Task async_terminals_short_circuit_and_compose_with_framework_linq()
    {
        var before = Interop.pattern_iterator_live();
        using (var source = Interop.pattern_iterator_create(10))
        {
            int calls = 0;
            Assert.True(await source.AnyAsync(async x =>
            {
                ++calls;
                await Task.Yield();
                return x == 2;
            }));
            Assert.Equal(3, calls);
        }
        Assert.Equal(before, Interop.pattern_iterator_live());
        using var managed = new[] { 1, 2, 3 }.ToRust();
        Assert.True(await managed.AnyAsync());
        Assert.True(await managed.AnyAsync(x => x == 2));
        Assert.False(await managed.AnyAsync((x, _) => ValueTask.FromResult(x > 3)));
        Assert.Equal(new[] { 20, 30 }, await managed.ToAsyncEnumerable()
            .Where(x => x > 1).Select(x => x * 10).ToArrayAsync());
    }

    [Fact]
    public async Task async_predicate_and_native_failures_release_enumeration()
    {
        var before = Interop.pattern_iterator_live();
        var expected = new ApplicationException("async predicate failure");
        using var source = Interop.pattern_iterator_create(10);
        int downstream = 0;
        var query = source.Where(async _ => { await Task.Yield(); throw expected; })
            .Where(x => { ++downstream; return true; });
        Assert.Same(expected, await Assert.ThrowsAsync<ApplicationException>(() => query.AnyAsync().AsTask()));
        Assert.Equal(0, downstream);
        Assert.Equal(before, Interop.pattern_iterator_live());

        using var panicking = Interop.pattern_iterator_panic();
        await Assert.ThrowsAsync<InvalidOperationException>(() => panicking.AnyAsync().AsTask());
        Assert.Equal(before, Interop.pattern_iterator_live());
    }

    [Fact]
    public async Task async_query_cancellation_reaches_predicate_and_releases_native_state()
    {
        var before = Interop.pattern_iterator_live();
        using var source = Interop.pattern_iterator_create(10);
        using var cts = new CancellationTokenSource();
        var entered = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        var query = source.Where(async (_, token) =>
        {
            Assert.Equal(cts.Token, token);
            entered.SetResult();
            await Task.Delay(Timeout.Infinite, token);
            return true;
        });
        var pending = query.AnyAsync(cts.Token).AsTask();
        await entered.Task.WaitAsync(TimeSpan.FromSeconds(5));
        Assert.False(pending.IsCompleted);
        cts.Cancel();
        await Assert.ThrowsAnyAsync<OperationCanceledException>(() => pending);
        Assert.True(pending.IsCanceled);
        Assert.Equal(before, Interop.pattern_iterator_live());
    }

    [Fact]
    public async Task precancelled_and_empty_async_queries_do_not_visit_source()
    {
        var before = Interop.pattern_iterator_live();
        var visits = Interop.pattern_iterator_visits();
        using var source = Interop.pattern_iterator_create(10);
        using var cts = new CancellationTokenSource();
        cts.Cancel();
        await Assert.ThrowsAnyAsync<OperationCanceledException>(() => source.AnyAsync(cts.Token).AsTask());
        Assert.Equal(visits, Interop.pattern_iterator_visits());
        source.Dispose();
        Assert.Equal(before, Interop.pattern_iterator_live());

        foreach (int count in new[] { 0, -1 })
        {
            using var unused = Interop.pattern_iterator_create(10);
            var query = unused.Where((uint _) => Task.FromException<bool>(new Exception("must not run"))).Take(count);
            Assert.False(await query.AnyAsync());
        }
        Assert.Equal(visits, Interop.pattern_iterator_visits());
        Assert.Equal(before, Interop.pattern_iterator_live());
    }

    [Fact]
    public async Task async_enumerator_disposal_and_unstarted_query_ownership()
    {
        var before = Interop.pattern_iterator_live();
        var visits = Interop.pattern_iterator_visits();
        using var source = Interop.pattern_iterator_create(10);
        var unused = source.Where(x => Task.FromResult(true));
        var notStarted = unused.GetAsyncEnumerator();
        await notStarted.DisposeAsync();
        Assert.Equal(visits, Interop.pattern_iterator_visits());
        // An unstarted standard async pipeline has not taken the source's ownership.
        source.Dispose();
        Assert.Equal(before, Interop.pattern_iterator_live());

        using var active = Interop.pattern_iterator_create(10);
        var enumerator = active.Where(x => Task.FromResult(true)).GetAsyncEnumerator();
        Assert.True(await enumerator.MoveNextAsync());
        Assert.Equal(0U, enumerator.Current);
        await enumerator.DisposeAsync();
        await enumerator.DisposeAsync();
        Assert.Equal(before, Interop.pattern_iterator_live());
    }

    [Fact]
    public async Task async_predicate_can_await_generated_rust_service_task()
    {
        var before = Interop.pattern_iterator_live();
        using var service = ServiceAsyncSleep.Create();
        using var source = Interop.pattern_iterator_create(5);
        var query = source.Where(async (x, token) =>
            await service.ReturnAfterMsAsync(x, 1, token) >= 2).Take(2);
        Assert.Equal(new uint[] { 2, 3 }, await query.ToArrayAsync());
        Assert.Equal(before, Interop.pattern_iterator_live());
    }

    [Fact]
    public void async_adapters_validate_arguments_without_consuming_native_source()
    {
        using var source = Interop.pattern_iterator_create(2);
        IRustEnumerable<uint> missing = null!;
        IAsyncEnumerable<uint> missingAsync = null!;
        Assert.Throws<ArgumentNullException>(() => missing.ToAsyncEnumerable());
        Assert.Throws<ArgumentNullException>(() => source.Where((Func<uint, Task<bool>>)null!));
        Assert.Throws<ArgumentNullException>(() => source.Where((Func<uint, CancellationToken, ValueTask<bool>>)null!));
        Assert.Throws<ArgumentNullException>(() => missingAsync.Where(x => Task.FromResult(true)));
        Assert.Throws<ArgumentNullException>(() => source.AnyAsync((Func<uint, Task<bool>>)null!));
        Assert.Throws<ArgumentNullException>(() => source.AnyAsync((Func<uint, CancellationToken, ValueTask<bool>>)null!));
        Assert.True(source.Any());
    }

    [Fact]
    public async Task async_enumerator_checks_cancellation_before_the_next_native_pull()
    {
        var before = Interop.pattern_iterator_live();
        var visits = Interop.pattern_iterator_visits();
        using var source = Interop.pattern_iterator_create(10);
        using var cts = new CancellationTokenSource();
        await using var enumerator = source.GetAsyncEnumerator(cts.Token);
        Assert.True(await enumerator.MoveNextAsync());
        Assert.Equal(visits + 1, Interop.pattern_iterator_visits());
        cts.Cancel();
        await Assert.ThrowsAnyAsync<OperationCanceledException>(() => enumerator.MoveNextAsync().AsTask());
        Assert.Equal(visits + 1, Interop.pattern_iterator_visits());
        Assert.Equal(before, Interop.pattern_iterator_live());
        Assert.False(await enumerator.MoveNextAsync());
    }

    [Fact]
    public void script_scope_releases_abandoned_stages_and_enumerators()
    {
        var before = Interop.pattern_iterator_live();
        var scope = new ScriptScope();
        var abandoned = Interop.pattern_iterator_create(5).Where(x => x > 0).Take(2);
        var enumerator = Interop.pattern_iterator_create(5).GetEnumerator();
        Assert.True(enumerator.MoveNext());
        Assert.Equal(before + 2, Interop.pattern_iterator_live());
        scope.Dispose();
        scope.Dispose();
        Assert.Equal(before, Interop.pattern_iterator_live());
        Assert.False(enumerator.MoveNext());
        Assert.Throws<ObjectDisposedException>(() => abandoned.Any());
    }

    [Fact]
    public void script_scope_cleans_up_after_failure_and_native_roundtrip()
    {
        var before = Interop.pattern_iterator_live();
        Assert.Throws<InvalidOperationException>((Action)(() =>
        {
            using var scope = new ScriptScope();
            var original = Interop.pattern_iterator_create(5);
            _ = Interop.pattern_iterator_echo(original).Where(x => x > 1);
            throw new InvalidOperationException("script failure");
        }));
        Assert.Equal(before, Interop.pattern_iterator_live());
    }

    [Fact]
    public void script_scope_nesting_preserves_parent_queries()
    {
        var before = Interop.pattern_iterator_live();
        using var parent = new ScriptScope();
        var outer = Interop.pattern_iterator_create(3);
        using (var child = new ScriptScope())
            _ = Interop.pattern_iterator_create(3);
        Assert.Equal(before + 1, Interop.pattern_iterator_live());
        Assert.True(outer.Any());
        Assert.Equal(before, Interop.pattern_iterator_live());
    }

    [Fact]
    public void script_scope_disposal_inside_predicate_defers_native_drop()
    {
        var before = Interop.pattern_iterator_live();
        using var scope = new ScriptScope();
        var query = Interop.pattern_iterator_create(3).Where(x =>
        {
            Task.Run(() => scope.Dispose(), TestContext.Current.CancellationToken).GetAwaiter().GetResult();
            return true;
        });
        using var enumerator = query.GetEnumerator();
        Assert.False(enumerator.MoveNext());
        Assert.Equal(before, Interop.pattern_iterator_live());
    }

    [Fact]
    public async Task script_scope_flows_across_await_and_rejects_late_queries()
    {
        var before = Interop.pattern_iterator_live();
        var scope = new ScriptScope();
        await Task.Yield();
        _ = Interop.pattern_iterator_create(3);
        var release = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        var late = Task.Run(async () =>
        {
            await release.Task;
            Assert.Throws<ObjectDisposedException>(() => Interop.pattern_iterator_create(3));
        }, TestContext.Current.CancellationToken);
        scope.Dispose();
        Assert.Equal(before, Interop.pattern_iterator_live());
        release.SetResult();
        await late;
        Assert.Equal(before, Interop.pattern_iterator_live());
    }

    [Fact]
    public async Task script_scope_supplies_query_and_predicate_cancellation()
    {
        var before = Interop.pattern_iterator_live();
        using var cancellation = new CancellationTokenSource();
        using var scope = new ScriptScope(cancellation.Token);
        await Task.Yield();
        var query = Interop.pattern_iterator_create(4).Where(
            (uint x, CancellationToken token) =>
            {
                Assert.Equal(cancellation.Token, token);
                cancellation.Cancel();
                return ValueTask.FromResult(false);
            });
        await Assert.ThrowsAnyAsync<OperationCanceledException>(async () => await query.AnyAsync());
        Assert.Equal(before, Interop.pattern_iterator_live());
    }

    [Fact]
    public async Task script_scope_default_service_token_is_checked_before_native_work()
    {
        using var service = ServiceAsyncCancel.Create();
        using var cancellation = new CancellationTokenSource();
        cancellation.Cancel();
        using var scope = new ScriptScope(cancellation.Token);
        await Task.Yield();
        await Assert.ThrowsAnyAsync<OperationCanceledException>(() => service.CountingWorkAsync(1, 1));
        Assert.Equal(0ul, service.Counter());
        using var explicitToken = new CancellationTokenSource();
        Assert.Equal(1ul, await service.LongRunningAsync(1, 1, explicitToken.Token));
    }

    [Fact]
    public async Task script_scope_cancels_an_inflight_native_task()
    {
        using var service = ServiceAsyncCancel.Create();
        using var cancellation = new CancellationTokenSource();
        using var scope = new ScriptScope(cancellation.Token);
        var task = service.SleepForeverAsync();
        cancellation.Cancel();
        await Assert.ThrowsAnyAsync<OperationCanceledException>(() => task);
        Assert.True(task.IsCanceled);
    }

    [Fact]
    public void script_scope_restores_parent_token()
    {
        var original = ScriptScope.CurrentCancellationToken;
        using var first = new CancellationTokenSource();
        using var second = new CancellationTokenSource();
        using (var parent = new ScriptScope(first.Token))
        {
            using (var inherited = new ScriptScope())
                Assert.Equal(first.Token, ScriptScope.CurrentCancellationToken);
            using (var child = new ScriptScope(second.Token))
                Assert.Equal(second.Token, ScriptScope.CurrentCancellationToken);
            Assert.Equal(first.Token, ScriptScope.CurrentCancellationToken);
        }
        Assert.Equal(original, ScriptScope.CurrentCancellationToken);
    }

    [Fact]
    public void factory_queries_are_deferred_and_reusable()
    {
        var before = Interop.pattern_iterator_live();
        int opens = 0;
        var values = RustEnumerable.FromFactory(() =>
        {
            ++opens;
            return Interop.pattern_iterator_create(6);
        });
        var query = values.Where(x => x % 2 == 0).Take(2);
        Assert.Equal(0, opens);
        using (var unstarted = query.GetEnumerator()) { }
        Assert.Equal(0, opens);
        Assert.Equal(new uint[] { 0, 2 }, query.ToArray());
        Assert.Equal(new uint[] { 0, 2 }, query.ToArray());
        Assert.True(query.Any());
        Assert.Equal(3, opens);
        Assert.Equal(before, Interop.pattern_iterator_live());
    }

    [Fact]
    public void factory_queries_open_independent_traversals_and_observe_current_data()
    {
        var before = Interop.pattern_iterator_live();
        uint count = 2;
        var values = RustEnumerable.FromFactory(() => Interop.pattern_iterator_create(count));
        using var first = values.GetEnumerator();
        using var second = values.GetEnumerator();
        Assert.True(first.MoveNext());
        Assert.True(second.MoveNext());
        Assert.Equal(0u, first.Current);
        Assert.Equal(0u, second.Current);
        first.Dispose();
        second.Dispose();
        count = 4;
        Assert.Equal(new uint[] { 0, 1, 2, 3 }, values.ToArray());
        Assert.Equal(before, Interop.pattern_iterator_live());
    }

    [Fact]
    public void factory_query_failures_release_traversal_and_allow_retry()
    {
        var before = Interop.pattern_iterator_live();
        bool fail = true;
        var query = RustEnumerable.FromFactory(() => Interop.pattern_iterator_create(4))
            .Where(x => fail ? throw new InvalidOperationException("predicate") : x > 1);
        Assert.Throws<InvalidOperationException>(() => query.ToArray());
        Assert.Equal(before, Interop.pattern_iterator_live());
        fail = false;
        Assert.Equal(new uint[] { 2, 3 }, query.ToArray());
        Assert.Equal(before, Interop.pattern_iterator_live());
        Assert.Throws<ArgumentNullException>(() => RustEnumerable.FromFactory<uint>(null!));
        Assert.Throws<InvalidOperationException>(() => RustEnumerable.FromFactory<uint>(() => null!).Any());
    }

    [Fact]
    public async Task factory_async_queries_repeat_and_scope_cleans_abandoned_enumeration()
    {
        var before = Interop.pattern_iterator_live();
        var values = RustEnumerable.FromFactory(() => Interop.pattern_iterator_create(4));
        var query = values.Where(x => Task.FromResult(x >= 2));
        Assert.Equal(new uint[] { 2, 3 }, await query.ToArrayAsync());
        Assert.Equal(new uint[] { 2, 3 }, await query.ToArrayAsync());
        using (var scope = new ScriptScope())
        {
            var abandoned = values.GetAsyncEnumerator();
            Assert.True(await abandoned.MoveNextAsync());
        }
        Assert.Equal(before, Interop.pattern_iterator_live());
    }

    [Fact]
    public void service_collection_results_have_script_facing_types()
    {
        var before = Interop.pattern_iterator_live();
        using var service = ServiceIterator.Create(4);
        Assert.Equal(typeof(IRustEnumerable<uint>), typeof(ServiceIterator).GetMethod("Values")!.ReturnType);
        Assert.Equal(typeof(IRustEnumerable<uint>), typeof(ServiceIterator).GetMethod("TryValues")!.ReturnType);
        Assert.Equal(typeof(IRustEnumerable<bool>), typeof(ServiceIterator).GetMethod("Bools")!.ReturnType);
        var values = RustEnumerable.FromFactory(service.Values);
        Assert.Equal(new uint[] { 1, 2 }, values.Where(x => x > 0).Take(2).ToArray());
        Assert.Equal(new uint[] { 1, 2 }, values.Where(x => x > 0).Take(2).ToArray());
        Assert.Equal(new uint[] { 0, 1, 2, 3 }, service.TryValues().ToArray());
        Assert.Equal(new[] { false, true, true, true }, service.Bools().ToArray());
        Assert.Equal(before, Interop.pattern_iterator_live());
    }

    [Fact]
    public void shared_scope_provider_hooks_clean_all_resources_even_when_one_throws()
    {
        var before = Interop.pattern_iterator_live();
        var scope = new ScriptScope();
        var marker = new CleanupProbe();
        scope.Register(marker); // Compiles from a different assembly than Rust.Linq.cs.
        _ = Interop.pattern_iterator_create(3);
        Assert.Throws<AggregateException>(() => scope.Dispose());
        Assert.Equal(1, marker.Calls);
        Assert.Equal(before, Interop.pattern_iterator_live());
        scope.Dispose();
        Assert.Equal(1, marker.Calls);
    }

    [Fact]
    public void nested_scope_disposal_does_not_restore_a_closed_parent()
    {
        var original = ScriptScope.Current;
        var outer = new ScriptScope();
        var inner = new ScriptScope();
        outer.Dispose();
        inner.Dispose();
        Assert.Same(original, ScriptScope.Current);
        using var query = Interop.pattern_iterator_create(1);
        Assert.True(query.Any());
    }

    private sealed class CleanupProbe : IDisposable
    {
        public int Calls;
        public void Dispose()
        {
            ++Calls;
            throw new InvalidOperationException("cleanup failure");
        }
    }
}
