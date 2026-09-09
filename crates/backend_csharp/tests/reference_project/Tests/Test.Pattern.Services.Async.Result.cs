using System.Threading.Tasks;
using My.Company;
using My.Company.Common;
using Xunit;

public class TestPatternServicesAsyncResult
{
    [Fact]
    public async Task Success()
    {
        using var s = ServiceAsyncResult.Create();
        await s.SuccessAsync(TestContext.Current.CancellationToken);
    }

    [Fact]
    public async Task Fail()
    {
        var exceptionThrown = false;
        using var s = ServiceAsyncResult.Create();

        try
        {
            await s.FailAsync(TestContext.Current.CancellationToken);
        }
        catch (EnumException<Error> e)
        {
            Assert.Equal(Error.Fail, e.Value);
            exceptionThrown = true;
        }

        Assert.True(exceptionThrown);
    }
}
