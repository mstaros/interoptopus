using System;
using System.Collections.Generic;
using System.Linq;
using System.Threading;
using System.Threading.Tasks;
using My.Company;
using My.Company.Common;
using Rust.Linq;
using Xunit;
using Interop = My.Company.Interop;

public class TestPatternAsyncIterators
{
    [Fact]
    public async Task delayed_items_are_ordered_and_requested_one_at_a_time()
    {
        var before = Interop.pattern_async_iterator_live();
        var produced = Interop.pattern_async_iterator_produced();
        await using var source = Interop.pattern_async_iterator_create(3, 5, 0);
        Assert.Equal(produced, Interop.pattern_async_iterator_produced());
        await using var enumerator = source.GetAsyncEnumerator(TestContext.Current.CancellationToken);
        Assert.Equal(produced, Interop.pattern_async_iterator_produced());
        Assert.Throws<InvalidOperationException>(() => enumerator.Current);
        for (uint i = 0; i < 3; ++i)
        {
            Assert.True(await enumerator.MoveNextAsync());
            Assert.Equal(i, enumerator.Current);
            Assert.Equal(produced + i + 1, Interop.pattern_async_iterator_produced());
        }
        Assert.False(await enumerator.MoveNextAsync());
        Assert.False(await enumerator.MoveNextAsync());
        Assert.Throws<InvalidOperationException>(() => enumerator.Current);
        Assert.Equal(before, Interop.pattern_async_iterator_live());
    }

    [Fact]
    public async Task await_foreach_break_releases_native_stream()
    {
        var before = Interop.pattern_async_iterator_live();
        var produced = Interop.pattern_async_iterator_produced();
        await foreach (var value in Interop.pattern_async_iterator_create(100, 1, 0)
            .WithCancellation(TestContext.Current.CancellationToken))
            if (value == 1) break;
        Assert.Equal(produced + 2, Interop.pattern_async_iterator_produced());
        Assert.Equal(before, Interop.pattern_async_iterator_live());
    }

    [Fact]
    public async Task framework_async_linq_and_task_predicates_compose()
    {
        var before = Interop.pattern_async_iterator_live();
        var query = Interop.pattern_async_iterator_create(10, 1, 0)
            .Where(x => Task.FromResult(x % 2 == 0))
            .Select(x => x * 10)
            .Take(3);
        Assert.Equal(new uint[] { 0, 20, 40 }, await query.ToArrayAsync(TestContext.Current.CancellationToken));
        Assert.Equal(before, Interop.pattern_async_iterator_live());
    }

    [Fact]
    public async Task empty_stream_and_unstarted_disposal_release_state()
    {
        var before = Interop.pattern_async_iterator_live();
        Assert.Empty(await Interop.pattern_async_iterator_create(0, 0, 0).ToArrayAsync(TestContext.Current.CancellationToken));
        var source = Interop.pattern_async_iterator_create(3, 0, 0);
        Assert.Equal(before + 1, Interop.pattern_async_iterator_live());
        await source.DisposeAsync();
        await source.DisposeAsync();
        Assert.Equal(before, Interop.pattern_async_iterator_live());
        Assert.Throws<ObjectDisposedException>(() => source.GetAsyncEnumerator(TestContext.Current.CancellationToken));
    }

    [Fact]
    public async Task disposing_an_unstarted_enumerator_releases_state()
    {
        var before = Interop.pattern_async_iterator_live();
        var source = Interop.pattern_async_iterator_create(3, 0, 0);
        var enumerator = source.GetAsyncEnumerator(TestContext.Current.CancellationToken);
        Assert.Throws<ObjectDisposedException>(() => source.GetAsyncEnumerator(TestContext.Current.CancellationToken));
        await enumerator.DisposeAsync();
        Assert.Equal(before, Interop.pattern_async_iterator_live());
    }

    [Fact]
    public async Task cancellation_of_a_pending_pull_is_acknowledged_before_disposal_finishes()
    {
        var before = Interop.pattern_async_iterator_live();
        using var cancellation = new CancellationTokenSource();
        await using var source = Interop.pattern_async_iterator_create(1, 0, 2);
        await using var enumerator = source.GetAsyncEnumerator(cancellation.Token);
        var pending = enumerator.MoveNextAsync().AsTask();
        Assert.False(pending.IsCompleted); // This Rust stream never produces an item.
        cancellation.Cancel();
        await Assert.ThrowsAnyAsync<OperationCanceledException>(() => pending);
        Assert.True(pending.IsCanceled);
        await enumerator.DisposeAsync();
        Assert.Equal(before, Interop.pattern_async_iterator_live());
    }

    [Fact]
    public async Task precancelled_enumeration_releases_the_unstarted_source()
    {
        var before = Interop.pattern_async_iterator_live();
        using var cancellation = new CancellationTokenSource();
        cancellation.Cancel();
        var source = Interop.pattern_async_iterator_create(3, 0, 0);
        await Assert.ThrowsAnyAsync<OperationCanceledException>(async () =>
        {
            await foreach (var _ in source.WithCancellation(cancellation.Token)) { }
        });
        Assert.Equal(before, Interop.pattern_async_iterator_live());
    }

    [Fact]
    public async Task concurrent_advance_is_rejected_and_repeated_disposal_waits_for_cleanup()
    {
        var before = Interop.pattern_async_iterator_live();
        var source = Interop.pattern_async_iterator_create(1, 0, 2);
        var enumerator = source.GetAsyncEnumerator(TestContext.Current.CancellationToken);
        var pending = enumerator.MoveNextAsync().AsTask();
        Assert.Throws<InvalidOperationException>(() => enumerator.MoveNextAsync());
        var first = enumerator.DisposeAsync().AsTask();
        var second = enumerator.DisposeAsync().AsTask();
        await Task.WhenAll(first, second);
        await Assert.ThrowsAnyAsync<OperationCanceledException>(() => pending);
        Assert.Equal(before, Interop.pattern_async_iterator_live());
    }

    [Fact]
    public async Task native_poll_and_destructor_panics_fault_the_move_and_release_state()
    {
        var before = Interop.pattern_async_iterator_live();
        foreach (uint mode in new uint[] { 1, 3 })
        {
            await using var source = Interop.pattern_async_iterator_create(0, 0, mode);
            await using var enumerator = source.GetAsyncEnumerator(TestContext.Current.CancellationToken);
            await Assert.ThrowsAsync<InvalidOperationException>(() => enumerator.MoveNextAsync().AsTask());
            await enumerator.DisposeAsync();
            Assert.Equal(before, Interop.pattern_async_iterator_live());
        }
    }

    [Fact]
    public async Task managed_predicate_failure_closes_native_stream()
    {
        var before = Interop.pattern_async_iterator_live();
        var failure = new InvalidOperationException("predicate");
        var query = Interop.pattern_async_iterator_create(3, 0, 0)
            .Where((Func<uint, Task<bool>>)(_ => Task.FromException<bool>(failure)));
        var actual = await Assert.ThrowsAsync<InvalidOperationException>(() => query.ToArrayAsync(TestContext.Current.CancellationToken).AsTask());
        Assert.Same(failure, actual);
        Assert.Equal(before, Interop.pattern_async_iterator_live());
    }

    [Fact]
    public async Task async_scope_releases_abandoned_queries_and_pending_enumerators()
    {
        var before = Interop.pattern_async_iterator_live();
        var scope = new ScriptScope();
        _ = Interop.pattern_async_iterator_create(5, 0, 0).Where(x => Task.FromResult(x > 0)).Take(0);
        var enumerator = Interop.pattern_async_iterator_create(1, 0, 2).GetAsyncEnumerator(TestContext.Current.CancellationToken);
        var pending = enumerator.MoveNextAsync().AsTask();
        await scope.DisposeAsync();
        await scope.DisposeAsync();
        await Assert.ThrowsAnyAsync<OperationCanceledException>(() => pending);
        Assert.Equal(before, Interop.pattern_async_iterator_live());
    }

    [Fact]
    public async Task synchronous_scope_close_can_be_awaited_afterwards()
    {
        var before = Interop.pattern_async_iterator_live();
        var scope = new ScriptScope();
        var enumerator = Interop.pattern_async_iterator_create(1, 0, 2).GetAsyncEnumerator(TestContext.Current.CancellationToken);
        var pending = enumerator.MoveNextAsync().AsTask();
        scope.Dispose();
        await scope.DisposeAsync();
        await Assert.ThrowsAnyAsync<OperationCanceledException>(() => pending);
        Assert.Equal(before, Interop.pattern_async_iterator_live());
    }

    [Fact]
    public async Task ambient_cancellation_flows_across_await()
    {
        var before = Interop.pattern_async_iterator_live();
        using var cancellation = new CancellationTokenSource();
        await using var scope = new ScriptScope(cancellation.Token);
        await Task.Yield();
        var enumerator = Interop.pattern_async_iterator_create(1, 0, 2).GetAsyncEnumerator();
        var pending = enumerator.MoveNextAsync().AsTask();
        cancellation.Cancel();
        await Assert.ThrowsAnyAsync<OperationCanceledException>(() => pending);
        await enumerator.DisposeAsync();
        Assert.Equal(before, Interop.pattern_async_iterator_live());
    }

    [Fact]
    public async Task callback_state_survives_gc_while_rust_is_waiting()
    {
        var before = Interop.pattern_async_iterator_live();
        await using var scope = new ScriptScope();
        var source = Interop.pattern_async_iterator_create(1, 0, 2);
        var enumerator = source.GetAsyncEnumerator(TestContext.Current.CancellationToken);
        var pending = enumerator.MoveNextAsync().AsTask();
        GC.Collect();
        GC.WaitForPendingFinalizers();
        GC.Collect();
        await enumerator.DisposeAsync();
        await Assert.ThrowsAnyAsync<OperationCanceledException>(() => pending);
        Assert.Equal(before, Interop.pattern_async_iterator_live());
    }

    [Fact]
    public async Task bool_and_struct_elements_use_their_managed_representations()
    {
        Assert.Equal(new[] { false, true }, await Interop.pattern_async_iterator_bools().ToArrayAsync(TestContext.Current.CancellationToken));
        var points = await Interop.pattern_async_iterator_points().ToArrayAsync(TestContext.Current.CancellationToken);
        var point = Assert.Single(points);
        Assert.Equal(1f, point.x);
        Assert.Equal(2f, point.y);
        Assert.Equal(3f, point.z);
    }

    [Fact]
    public async Task service_streams_expose_standard_async_enumerables_and_factories_repeat()
    {
        var before = Interop.pattern_async_iterator_live();
        using var service = ServiceAsyncStream.Create();
        Assert.Equal(typeof(IAsyncEnumerable<uint>), typeof(ServiceAsyncStream).GetMethod("ValuesAsync")!.ReturnType);
        Assert.Equal(typeof(IAsyncEnumerable<uint>), typeof(ServiceAsyncStream).GetMethod("TryValuesAsync")!.ReturnType);
        int opens = 0;
        var values = RustEnumerable.FromFactory(() =>
        {
            ++opens;
            return service.ValuesAsync(4, 1);
        });
        var query = values.Where(x => x > 0).Take(2);
        Assert.Equal(0, opens);
        Assert.Equal(new uint[] { 1, 2 }, await query.ToArrayAsync(TestContext.Current.CancellationToken));
        Assert.Equal(new uint[] { 1, 2 }, await query.ToArrayAsync(TestContext.Current.CancellationToken));
        Assert.Equal(2, opens);
        Assert.Equal(new uint[] { 0, 1 }, await service.TryValuesAsync(2).ToArrayAsync(TestContext.Current.CancellationToken));
        Assert.Equal(before, Interop.pattern_async_iterator_live());
    }

    [Fact]
    public async Task async_factory_does_not_open_for_unstarted_enumerators()
    {
        int opens = 0;
        var view = RustEnumerable.FromFactory((Func<IAsyncEnumerable<uint>>)(() =>
        {
            ++opens;
            return Interop.pattern_async_iterator_create(3, 0, 0);
        }));
        await using (var unstarted = view.GetAsyncEnumerator(TestContext.Current.CancellationToken)) { }
        Assert.Equal(0, opens);
        Assert.Empty(await view.Take(0).ToArrayAsync(TestContext.Current.CancellationToken));
        Assert.Equal(0, opens);
    }

    [Fact]
    public async Task native_roundtrip_transfers_stream_ownership()
    {
        var before = Interop.pattern_async_iterator_live();
        var original = Interop.pattern_async_iterator_create(3, 1, 0);
        var returned = Interop.pattern_async_iterator_echo(original);
        original.Dispose();
        Assert.Throws<ObjectDisposedException>(() => original.GetAsyncEnumerator(TestContext.Current.CancellationToken));
        Assert.Equal(new uint[] { 0, 1, 2 }, await returned.ToArrayAsync(TestContext.Current.CancellationToken));
        Assert.Equal(before, Interop.pattern_async_iterator_live());
    }
}
