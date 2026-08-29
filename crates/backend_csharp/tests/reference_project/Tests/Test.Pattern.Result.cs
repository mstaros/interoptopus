using System;
using My.Company;
using My.Company.Common;
using Xunit;
using Interop = My.Company.Interop;

public class TestPatternResult
{
    [Fact]
    public void pattern_result_1()
    {
        var x = ResultUintError.Ok(123);
        Interop.pattern_result_1(x).AsOk();
    }

    [Fact]
    public void pattern_result_2()
    {
        var result = Interop.pattern_result_2();
        Assert.True(result.IsOk);
    }

    [Fact]
    public void pattern_result_3()
    {
        Assert.True(Interop.pattern_result_3(ResultVoidError.Ok).IsOk);
        Assert.Equal(ResultVoidError.Ok, Interop.pattern_result_3(ResultVoidError.Ok));
        Assert.Equal(ResultVoidError.Null, Interop.pattern_result_3(ResultVoidError.Null));
    }

    [Fact]
    public void pattern_result_4()
    {
        Assert.True(Interop.pattern_result_4(ResultVoidVoid.Ok).IsOk);
    }

    [Fact]
    public void pattern_string_5()
    {
        var w = new UseString { s1 = "hello".Utf8(), s2 = "world".Utf8() };
        var result = Interop.pattern_string_5(w);
        var ok = result.AsOk();
        Assert.Equal("hello", ok.s1.String);
        Assert.Equal("world", ok.s2.String);
    }

    [Fact]
    public void pattern_result_interface()
    {
        var result = Interop.pattern_result_2();
        IResult<Unit, Error> resultInterface = result;
        Assert.True(resultInterface.IsOk);
        Assert.False(resultInterface.IsErr);
        Assert.False(resultInterface.IsPanic);
        Assert.False(resultInterface.IsNull);

        resultInterface.AsOk();
        Assert.Throws<EnumException>(() => resultInterface.AsErr());
    }

    /// The C# 15 union conversion is supplied by the compiler from the generated public
    /// single-parameter case constructor. Keeping the IResult assignment in the same test proves
    /// that the new case-type surface coexists with the established Result interface.
    [Fact]
    public void case_type_implicitly_converts_and_preserves_iresult()
    {
        ResultUintError result = new ResultUintError.OkCase(5);
        IResult<uint, Error> resultInterface = result;

        Assert.Equal(5u, resultInterface.AsOk());
    }

    /// Step 6: body_from_call consumes the established Result factories. Exercise both branches
    /// that exist for an ordinary payload Result: Ok(value) and the unit Panic property.
    [Fact]
    public void from_call_uses_payload_ok_and_panic_factories()
    {
        var ok = ResultUintError.FromCall(() => 9u);
        var panic = ResultUintError.FromCall(() => throw new InvalidOperationException("boom"));

        Assert.True(ok.IsOk);
        Assert.Equal(9u, ok.AsOk());
        Assert.True(panic.IsPanic);
    }

    /// The payloadless template branch spells the Ok factory as a property rather than a method.
    [Fact]
    public void from_call_uses_the_unit_ok_factory()
    {
        var called = false;

        var ok = ResultVoidError.FromCall(() => { called = true; });

        Assert.True(called);
        Assert.True(ok.IsOk);
    }

    [Fact]
    public void pattern_result_custom_matches()
    {
        var resultVoidVoid = ResultVoidVoid.Ok;
        var resultVoidError = Interop.pattern_result_2();
        var resultUIntError = ResultUintError.Ok(42);

        var match1 = my_match(resultVoidVoid, v => v.ToString(), e => e.ToString());
        var match2 = my_match(resultVoidError, v => v.ToString(), e => e.ToString());
        var match3 = my_match(resultUIntError, v => v.ToString(), e => e.ToString());

        Assert.Equal("()", match1);
        Assert.Equal("()", match2);
        Assert.Equal("42", match3);
    }

    private static TResult my_match<T, TErr, TResult>(
        IResult<T, TErr> res,
        Func<T, TResult> okMatcher,
        Func<TErr, TResult> errMatcher
    )
    {
        if (res.IsOk)
            return okMatcher(res.AsOk());
        if (res.IsErr)
            return errMatcher(res.AsErr());
        throw res.ExceptionForVariant();
    }
}
