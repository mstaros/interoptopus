using System;

/// <summary>
/// The Rust allocation probes (<c>__test_live_bytes</c>, <c>__test_live_allocations</c>) read
/// process-global native counters. Anything the .NET finalizer thread frees mid-test moves them,
/// so a probe test must drain pending finalization before taking its baseline.
/// </summary>
internal static class AllocationProbe
{
    public static void Settle()
    {
        GC.Collect();
        GC.WaitForPendingFinalizers();
        GC.Collect();
    }
}