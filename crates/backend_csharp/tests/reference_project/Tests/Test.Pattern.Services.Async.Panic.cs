using System;
using My.Company;
using System.Threading.Tasks;
using Xunit;

public class TestPatternServicesAsyncPanic
{
    [Fact]
    public async Task PanickingThrowsException()
    {
        using var s = ServiceAsyncPanic.Create();

        // Rust panics must fault the Task; cancellation has a distinct wire tag.
        var task = s.PanickingAsync(TestContext.Current.CancellationToken);
        var error = await Assert.ThrowsAsync<InvalidOperationException>(async () => await task);
        Assert.Contains("panicked", error.Message);
        Assert.True(task.IsFaulted);
        Assert.False(task.IsCanceled);
    }

    [Fact]
    public async Task NotPanickingSucceeds()
    {
        using var s = ServiceAsyncPanic.Create();
        await s.NotPanickingAsync(TestContext.Current.CancellationToken);
    }

    [Fact]
    public async Task ServiceWorksAfterPanic()
    {
        using var s = ServiceAsyncPanic.Create();

        // First call panics
        await Assert.ThrowsAsync<InvalidOperationException>(async () =>
        {
            await s.PanickingAsync(TestContext.Current.CancellationToken);
        });

        // Service is still functional after the panic
        await s.NotPanickingAsync(TestContext.Current.CancellationToken);
    }

    [Fact]
    public async Task ServiceWorksAfterRepeatedPanics()
    {
        using var s = ServiceAsyncPanic.Create();

        for (int i = 0; i < 10; i++)
        {
            await Assert.ThrowsAsync<InvalidOperationException>(async () =>
            {
                await s.PanickingAsync(TestContext.Current.CancellationToken);
            });
        }

        // Service is still functional after repeated panics
        await s.NotPanickingAsync(TestContext.Current.CancellationToken);
    }
}
