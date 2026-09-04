using System;
using My.Company;
using My.Company.Common;
using Xunit;
using Interop = My.Company.Interop;

// The allocation probes report process-global native state, so snapshots must not overlap other FFI tests.
[assembly: CollectionBehavior(DisableTestParallelization = true)]

public class TestPatternStrings
{
    [Fact]
    public void pattern_ascii_pointer_1()
    {
        var x = Utf8String.From("hello world");
        Assert.Equal(11u, Interop.pattern_ascii_pointer_1("hello world"));
    }

    [Fact]
    public void pattern_ascii_pointer_2()
    {
        var rval = Interop.pattern_ascii_pointer_2();
        Assert.Equal("hello.world", rval);
    }

    [Fact]
    public void pattern_string_1()
    {
        Assert.Equal("hello world", Interop.pattern_string_1("hello world".Utf8()).String);
        Assert.Equal("hello world", Interop.pattern_string_1("hello world".Utf8()).String);
    }

    [Fact]
    public void pattern_string_2()
    {
        Assert.Equal(11u, Interop.pattern_string_2("hello world".Utf8()));
    }

    /// <summary>
    /// Establishes what a by-value pass does to the managed <c>Utf8String</c>.
    /// </summary>
    /// <remarks>
    /// <c>Marshaller.ToUnmanaged()</c> calls <c>Utf8String.IntoUnmanaged()</c>, which nulls
    /// <c>_ptr</c> on the managed side after handing the buffer over, and the type is declared
    /// <c>[CustomMarshaller(..., MarshalMode.Default, ...)]</c> — one implementation serving in,
    /// out, ref and return alike. Reading those together predicts that a second by-value pass of
    /// the same instance throws.
    ///
    /// Nothing tested it. <c>pattern_string_1</c> builds a fresh <c>.Utf8()</c> per call, and
    /// the <c>pattern_string_6b</c> write-back allocation test reuses its value by <c>ref</c>,
    /// where the callee may write
    /// it back. This is the missing case, and it is written because the prediction came from
    /// reading rather than running.
    ///
    /// It pins *ownership*, not a defect. Where the Rust signature takes <c>Utf8String</c> by
    /// value, Rust owns and drops it, and consuming the managed instance is correct. The
    /// observable consequence is the one the reference fixtures already work around by calling
    /// <c>Clone()</c> before passing: after a by-value pass, the managed instance is spent.
    ///
    /// If this test fails, the reading above is wrong somewhere and the <c>Clone()</c> calls in
    /// <c>Test.Core.Enums.cs</c> have some other explanation — which is worth knowing either way.
    /// </remarks>
    [Fact]
    public void passing_a_string_by_value_consumes_it()
    {
        var s = "hello world".Utf8();

        Assert.Equal(11u, Interop.pattern_string_2(s));
        Assert.ThrowsAny<Exception>(() => Interop.pattern_string_2(s));
    }

    [Fact]
    public void pattern_string_3()
    {
        Assert.Equal("pattern_string_3", Interop.pattern_string_3().String);
    }

    [Fact]
    public void pattern_string_4()
    {
        var w = new UseString { s1 = "hello".Utf8(), s2 = "world".Utf8() };
        var s = Interop.pattern_string_4(w);
        Assert.Equal("hello", s.s1.String);
        Assert.Equal("world", s.s2.String);
    }

    [Fact]
    public void pattern_string_6()
    {
        var r1 = new UseString { s1 = "hello".Utf8(), s2 = "world".Utf8() };
        Interop.pattern_string_6a(in r1).AsOk();
        Interop.pattern_string_6a(in r1).AsOk();
        Assert.Equal("hello", r1.s1.String);
        Assert.Equal("world", r1.s2.String);
        r1.Dispose();

        var y = new UseString { s1 = "".Utf8(), s2 = "".Utf8() };
        Interop.pattern_string_6b(ref y).AsOk();
        Assert.Equal("s1", y.s1.String);
        Assert.Equal("s2", y.s2.String);
    }

    [Fact]
    public void pattern_string_7()
    {
        var slice = SliceUtf8String.From(["hello".Utf8(), "world".Utf8()]);
        var r1 = Interop.pattern_string_7(slice, 0).AsOk();
        var r2 = Interop.pattern_string_7(slice, 1).AsOk();
        Assert.Equal("hello", r1.String);
        Assert.Equal("world", r2.String);
    }

    [Fact]
    public void pattern_string_8()
    {
        var x = new UseString[]
        {
            new() { s1 = "hello1".Utf8(), s2 = "world1".Utf8() },
            new() { s1 = "hello2".Utf8(), s2 = "world2".Utf8() }
        };

        var slice = SliceUseString.From(x);

        var r1 = Interop.pattern_string_8(slice, 0).AsOk();
        var r2 = Interop.pattern_string_8(slice, 1).AsOk();

        Assert.Equal("hello1", r1.s1.String);
        Assert.Equal("world2", r2.s2.String);
    }

    [Fact]
    public void pattern_string_9()
    {
        var rval = Interop.pattern_string_9();

        // Should not crash attempting to de-serialize a non-existing string

        Assert.Equal(rval.AsErr(), Error.Fail);
    }

    [Fact]
    public void pattern_string_10()
    {
        var s = "hello world".Utf8();
        Interop.pattern_string_10(s);
    }

    [Fact]
    public void pattern_string_11_preserves_rust_allocation_snapshot()
    {
        AllocationProbe.Settle();

        var baselineBytes = Interop.__test_live_bytes();
        var baselineAllocations = Interop.__test_live_allocations();

        {
            using var s = "hello world".Utf8();
            var ownedBytes = Interop.__test_live_bytes();
            var ownedAllocations = Interop.__test_live_allocations();

            Assert.True(ownedBytes > baselineBytes);
            Assert.True(ownedAllocations > baselineAllocations);

            for (var i = 0; i < 1024; i++) Interop.pattern_string_11(in s);

            Assert.Equal("hello world", s.String);
            Assert.Equal(ownedBytes, Interop.__test_live_bytes());
            Assert.Equal(ownedAllocations, Interop.__test_live_allocations());
        }

        Assert.Equal(baselineBytes, Interop.__test_live_bytes());
        Assert.Equal(baselineAllocations, Interop.__test_live_allocations());
    }

    [Fact]
    public void pattern_string_12()
    {
        var rval = Interop.pattern_string_12(s1 => s1 + "world", "hello");
        Assert.Equal("helloworld", rval);
    }

    [Fact]
    public void pattern_string_13()
    {
        var value = new UseString { s1 = "hello".Utf8(), s2 = "world".Utf8() };

        Assert.Equal(2u, Interop.pattern_string_13(in value, x => x + 1));
        Assert.Equal("hello", value.s1.String);
        Assert.Equal("world", value.s2.String);
        value.Dispose();
    }


    [Fact]
    public void string_by_in_preserves_rust_allocation_snapshot()
    {
        AllocationProbe.Settle();

        var baselineBytes = Interop.__test_live_bytes();
        var baselineAllocations = Interop.__test_live_allocations();

        {
            using var w = new UseString { s1 = "hello".Utf8(), s2 = "world".Utf8() };
            var ownedBytes = Interop.__test_live_bytes();
            var ownedAllocations = Interop.__test_live_allocations();

            Assert.True(ownedBytes > baselineBytes);
            Assert.True(ownedAllocations > baselineAllocations);

            for (var i = 0; i < 1024; i++) Interop.pattern_string_6a(in w);

            Assert.Equal("hello", w.s1.String);
            Assert.Equal("world", w.s2.String);
            Assert.Equal(ownedBytes, Interop.__test_live_bytes());
            Assert.Equal(ownedAllocations, Interop.__test_live_allocations());
        }

        Assert.Equal(baselineBytes, Interop.__test_live_bytes());
        Assert.Equal(baselineAllocations, Interop.__test_live_allocations());
    }

    [Fact]
    public void string_by_ref_write_back_preserves_replacement_allocation_snapshot()
    {
        AllocationProbe.Settle();

        var baselineBytes = Interop.__test_live_bytes();
        var baselineAllocations = Interop.__test_live_allocations();
        var w = new UseString { s1 = "hello".Utf8(), s2 = "world".Utf8() };

        try
        {
            // The first call replaces "hello"/"world" with the smaller "s1"/"s2" allocation pair.
            Interop.pattern_string_6b(ref w).AsOk();
            var replacementBytes = Interop.__test_live_bytes();
            var replacementAllocations = Interop.__test_live_allocations();

            Assert.True(replacementBytes > baselineBytes);
            Assert.True(replacementAllocations > baselineAllocations);

            for (var i = 0; i < 1024; i++) Interop.pattern_string_6b(ref w).AsOk();

            Assert.Equal("s1", w.s1.String);
            Assert.Equal("s2", w.s2.String);
            Assert.Equal(replacementBytes, Interop.__test_live_bytes());
            Assert.Equal(replacementAllocations, Interop.__test_live_allocations());
        }
        finally
        {
            w.Dispose();
        }

        Assert.Equal(baselineBytes, Interop.__test_live_bytes());
        Assert.Equal(baselineAllocations, Interop.__test_live_allocations());
    }

    [Fact]
    public void string_clones()
    {
        using var s1 = "hello".Utf8();

        for (var i = 0; i < 1000; i++)
        {
            using var s2 = s1.Clone();
            Assert.Equal(s1.String, s2.String);
        }
    }

    [Fact]
    public void string_multiple_dispose()
    {
        var s1 = "hello".Utf8();
        s1.Dispose();
        s1.Dispose();
        s1.Dispose();
    }
}
