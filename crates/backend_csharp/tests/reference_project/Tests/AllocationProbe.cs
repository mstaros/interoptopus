using System;
using System.Diagnostics;
using System.Threading;
using Interop = My.Company.Interop;

/// <summary>
/// The Rust allocation probes (<c>__test_live_bytes</c>, <c>__test_live_allocations</c>) read
/// process-global native counters. Two things move them behind a test's back: the .NET finalizer
/// thread releasing objects earlier tests never disposed, and the library's own tokio runtime
/// finishing async work from earlier tests. A probe test must let both settle before it takes a
/// baseline, otherwise the baseline includes memory that is about to disappear.
/// </summary>
internal static class AllocationProbe
{
    private const int StableReadsRequired = 3;
    private static readonly TimeSpan PollInterval = TimeSpan.FromMilliseconds(50);
    private static readonly TimeSpan MaxWait = TimeSpan.FromSeconds(5);

    public static void Settle()
    {
        GC.Collect();
        GC.WaitForPendingFinalizers();
        GC.Collect();

        // Wait for the native counters to hold still. Bounded: if they never do, return and let
        // the probe fail loudly rather than hang the suite.
        var clock = Stopwatch.StartNew();
        var bytes = Interop.__test_live_bytes();
        var allocations = Interop.__test_live_allocations();
        var stable = 0;
        while (stable < StableReadsRequired && clock.Elapsed < MaxWait)
        {
            Thread.Sleep(PollInterval);
            var nextBytes = Interop.__test_live_bytes();
            var nextAllocations = Interop.__test_live_allocations();
            if (nextBytes == bytes && nextAllocations == allocations)
            {
                stable++;
                continue;
            }
            stable = 0;
            bytes = nextBytes;
            allocations = nextAllocations;
        }
    }
}