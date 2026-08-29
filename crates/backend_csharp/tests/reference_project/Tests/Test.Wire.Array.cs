using My.Company.Common;
using Xunit;
using Interop = My.Company.Interop;

public class TestWireArray
{
    [Fact]
    public void wire_accept_byte_array()
    {
        var data = new byte[32];
        data[0] = 42;
        var result = Interop.wire_accept_byte_array(data.Wire());
        Assert.Equal(42, result);
    }

    [Fact]
    public void wire_dispose_and_move_restore_rust_allocation_baseline()
    {
        var baselineBytes = Interop.__test_live_bytes();
        var baselineAllocations = Interop.__test_live_allocations();
        var returned = Interop.wire_return_byte_array();

        try
        {
            var ownedBytes = Interop.__test_live_bytes();
            var ownedAllocations = Interop.__test_live_allocations();

            Assert.True(ownedBytes > baselineBytes);
            Assert.True(ownedAllocations > baselineAllocations);
            Assert.Equal(42, returned.Unwire()[0]);
            Assert.Equal(ownedBytes, Interop.__test_live_bytes());
            Assert.Equal(ownedAllocations, Interop.__test_live_allocations());

            returned.Dispose();

            Assert.Equal(baselineBytes, Interop.__test_live_bytes());
            Assert.Equal(baselineAllocations, Interop.__test_live_allocations());
        }
        finally
        {
            returned.Dispose();
        }

        var data = new byte[32];
        data[0] = 42;
        var moved = data.Wire();

        try
        {
            Assert.True(Interop.__test_live_bytes() > baselineBytes);
            Assert.True(Interop.__test_live_allocations() > baselineAllocations);
            Assert.Equal(42, Interop.wire_accept_byte_array(moved));
            Assert.Equal(baselineBytes, Interop.__test_live_bytes());
            Assert.Equal(baselineAllocations, Interop.__test_live_allocations());
        }
        finally
        {
            moved.Dispose();
        }
    }

    [Fact]
    public void wire_return_byte_array()
    {
        using var wire = Interop.wire_return_byte_array();
        var result = wire.Unwire();
        Assert.Equal(32, result.Length);
        Assert.Equal(42, result[0]);
    }
}