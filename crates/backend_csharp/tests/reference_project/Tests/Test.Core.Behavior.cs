using My.Company.Common;
using System;
using My.Company;
using Xunit;
using Interop = My.Company.Interop;

public class TestBehavior
{
    [Fact]
    public void behavior_panics()
    {
        Assert.Equal(ResultVoidError.Panic, Interop.behavior_panics());
    }

    [Fact]
    public void panic_after_ownership_transfer_does_not_run_managed_rollback()
    {
        AllocationProbe.Settle();
        var baseline = Interop.__test_live_allocations();
        using var value = "panic-owned".Utf8();
        Assert.Equal(ResultVoidError.Panic, Interop.behavior_panics_with_owned_string(value));
        Assert.Throws<ObjectDisposedException>(() => _ = value.String);
        Assert.Equal(baseline, Interop.__test_live_allocations());
    }

    [Fact]
    public void behavior_panics_via_result()
    {
        var v = Interop.behavior_panics_via_result();
        Assert.Equal(ResultVoidError.Panic, v);
    }

    [Fact]
    public void behavior_sleep()
    {
        Interop.behavior_sleep(10);
    }
}
