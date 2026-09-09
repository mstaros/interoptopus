using System.Threading.Tasks;
using My.Company;
using Xunit;

public class TestPatternServicesAsyncRval
{
    [Fact]
    public async Task Create()
    {
        using var s = ServiceAsyncRval.Simple();
        Assert.Equal(123u, await s.NumberAsync(TestContext.Current.CancellationToken));
        Assert.Equal(new Vec3f32 { x = 0, y = 0, z = 0 }, await s.Vecf32Async(TestContext.Current.CancellationToken));
        Assert.Equal("hello", (await s.WireAsync(TestContext.Current.CancellationToken)).Unwire());
    }

}