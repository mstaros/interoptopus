using My.Company;
using Interop = My.Company.Interop;
using Xunit;

public class TestPatternServicesDependent
{
    [Fact]
    public void shared_services_restore_nested_rust_allocation_baselines()
    {
        AllocationProbe.Settle();

        var baselineBytes = Interop.__test_live_bytes();
        var baselineAllocations = Interop.__test_live_allocations();

        {
            using var main = ServiceMain.Create(123);
            var mainBytes = Interop.__test_live_bytes();
            var mainAllocations = Interop.__test_live_allocations();

            Assert.True(mainBytes > baselineBytes);
            Assert.True(mainAllocations > baselineAllocations);

            {
                using var dependent = ServiceDependent.FromMain(main);
                var dependentBytes = Interop.__test_live_bytes();
                var dependentAllocations = Interop.__test_live_allocations();

                Assert.True(dependentBytes > mainBytes);
                Assert.True(dependentAllocations > mainAllocations);
                Assert.Equal(123u, dependent.Get());
                dependent.PassMain(main);
                Assert.Equal(dependentBytes, Interop.__test_live_bytes());
                Assert.Equal(dependentAllocations, Interop.__test_live_allocations());
            }

            Assert.Equal(mainBytes, Interop.__test_live_bytes());
            Assert.Equal(mainAllocations, Interop.__test_live_allocations());
        }

        Assert.Equal(baselineBytes, Interop.__test_live_bytes());
        Assert.Equal(baselineAllocations, Interop.__test_live_allocations());
    }

    [Fact]
    public void NewMainAndDependent()
    {
        using var main = ServiceMain.Create(123);
        using var dependent = ServiceDependent.FromMain(main);


        var rval = dependent.Get();
        Assert.Equal(123u, rval);

        dependent.PassMain(main);
    }
}