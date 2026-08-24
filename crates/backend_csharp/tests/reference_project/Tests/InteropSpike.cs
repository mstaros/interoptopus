using System.Collections.Generic;
using System.Text;
using System.Text.Json;
using My.Company;
using My.Company.Common;
using TUnit.Core;

public class InteropSpikeTests
{
    private const int JsonPayloadBytes = 100 * 1024;
    private const int ConcurrentCallCount = 16;

    [Test]
    public async Task PatchedRuntimeLoadsNativeBindings()
    {
        if (Environment.Version.Major < 11)
            throw new InvalidOperationException($"Expected the patched .NET 11 runtime, got {Environment.Version}.");

        using var service = ServiceAsyncWire.Create();
        const string json = "{\"runtime\":\"patched\"}";
        var request = new Dictionary<string, string> { ["json"] = json };
        using var response = await service.WirePassthrough(request.Wire(), CancellationToken.None);

        if (!string.Equals(response.Unwire()["json"], json, StringComparison.Ordinal))
            throw new InvalidOperationException("The native binding did not round-trip under the patched runtime.");
    }

    [Test]
    public async Task JsonPayloadRoundTripsAt100KiB()
    {
        using var service = ServiceAsyncWire.Create();
        var json = CreateJsonPayload(0, JsonPayloadBytes);
        using var document = JsonDocument.Parse(json);

        if (Encoding.UTF8.GetByteCount(json) != JsonPayloadBytes)
            throw new InvalidOperationException("The test JSON is not exactly 100 KiB.");

        var request = new Dictionary<string, string> { ["json"] = json };
        using var response = await service.WirePassthrough(request.Wire(), CancellationToken.None);
        var roundTripped = response.Unwire()["json"];

        if (!string.Equals(roundTripped, json, StringComparison.Ordinal))
            throw new InvalidOperationException("The 100 KiB JSON payload changed during the round trip.");
    }

    [Test]
    public async Task ConcurrentCallsKeepResponsesIsolated()
    {
        using var service = ServiceAsyncWire.Create();
        var requests = Enumerable.Range(0, ConcurrentCallCount)
            .Select(index => CreateJsonPayload(index, 4096))
            .ToArray();

        var tasks = requests
            .Select(async request =>
            {
                var payload = new Dictionary<string, string> { ["json"] = request };
                using var response = await service.WirePassthrough(payload.Wire(), CancellationToken.None);
                return response.Unwire()["json"];
            })
            .ToArray();

        var responses = await Task.WhenAll(tasks);
        for (var index = 0; index < requests.Length; index++)
        {
            if (!string.Equals(responses[index], requests[index], StringComparison.Ordinal))
                throw new InvalidOperationException($"Concurrent response {index} did not match its request.");
        }
    }

    [Test]
    public async Task ManagedCancellationStopsRustWork()
    {
        using var service = ServiceAsyncCancel.Create();
        using var cancellation = new CancellationTokenSource();
        var work = service.CountingWork(200, 20, cancellation.Token);

        await Task.Delay(250, CancellationToken.None);
        cancellation.Cancel();

        try
        {
            await work;
            throw new InvalidOperationException("The Rust operation completed instead of observing cancellation.");
        }
        catch (OperationCanceledException)
        {
        }

        var countAtCancellation = service.Counter();
        await Task.Delay(250, CancellationToken.None);
        var countAfterWait = service.Counter();

        if (countAtCancellation == 0 || countAtCancellation >= 200)
            throw new InvalidOperationException("The Rust operation did not make bounded progress before cancellation.");
        if (countAfterWait != countAtCancellation)
            throw new InvalidOperationException("The Rust operation continued after managed cancellation.");
    }

    private static string CreateJsonPayload(int id, int utf8Bytes)
    {
        var prefix = $"{{\"id\":{id},\"payload\":\"";
        const string suffix = "\"}";
        var payloadBytes = utf8Bytes - Encoding.UTF8.GetByteCount(prefix) - Encoding.UTF8.GetByteCount(suffix);
        if (payloadBytes < 0)
            throw new ArgumentOutOfRangeException(nameof(utf8Bytes));

        return prefix + new string('x', payloadBytes) + suffix;
    }
}
