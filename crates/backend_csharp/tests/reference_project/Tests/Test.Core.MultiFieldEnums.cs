using System;
using System.IO;
using System.Reflection;
using System.Text;
using My.Company;
using My.Company.Common;
using Xunit;
using Interop = My.Company.Interop;

public class TestMultiFieldEnums
{
    [Fact]
    public void tuple_payloads_cross_native_alignment_boundaries()
    {
        foreach (var fields in new (byte Small, ulong Wide, ushort Tail)[]
        {
            (0, 0, 0),
            (0xA5, 0x0123456789ABCDEFUL, 0xCAFE),
            (0xFF, 0xFFEEDDCCBBAA0000UL, 0xFFFF)
        })
        {
            EnumMultiPayload value = new EnumMultiPayload.TupleCase(fields.Small, fields.Wide, fields.Tail);
            var expectedSum = fields.Small + fields.Wide + fields.Tail;

            Assert.Equal(expectedSum, Interop.enums_multi_borrow(in value));
            var result = Interop.enums_multi_echo(value);
            Assert.Equal(new EnumMultiPayload.TupleCase(fields.Small, fields.Wide, fields.Tail), result.AsTuple());
            Assert.Equal(expectedSum, Interop.enums_multi_borrow(in result));
        }
    }

    [Fact]
    public void named_payloads_preserve_field_order_and_spelling()
    {
        var value = EnumMultiPayload.Named(0xA5, 0x8877665544332211UL, 0xCAFE);
        var result = Interop.enums_multi_echo(value);
        var fields = result.AsNamed();

        Assert.Equal((byte)0xA5, fields.small);
        Assert.Equal(0x8877665544332211UL, fields.wide);
        Assert.Equal((ushort)0xCAFE, fields.tail);
        Assert.Equal(fields.small + fields.wide + fields.tail, Interop.enums_multi_borrow(in result));
        Assert.Equal(new EnumMultiPayload.NamedCase(0xA5, 0x8877665544332211UL, 0xCAFE), result.Value);
    }

    [Fact]
    public void empty_and_single_field_variants_keep_their_existing_factories_and_accessors()
    {
        foreach (var value in new[]
        {
            EnumMultiPayload.Unit,
            EnumMultiPayload.EmptyTuple,
            EnumMultiPayload.EmptyNamed,
            EnumMultiPayload.Single(0xDEADBEEF),
            EnumMultiPayload.NamedSingle(0xAABBCCDD)
        })
        {
            var result = Interop.enums_multi_echo(value);
            Assert.True(result.HasValue);
            Assert.Equal(value.Value, result.Value);
        }

        uint tupleSingle = Interop.enums_multi_echo(EnumMultiPayload.Single(0xDEADBEEF)).AsSingle();
        uint namedSingle = Interop.enums_multi_echo(EnumMultiPayload.NamedSingle(0xAABBCCDD)).AsNamedSingle();
        Assert.Equal(0xDEADBEEFu, tupleSingle);
        Assert.Equal(0xAABBCCDDu, namedSingle);
        Assert.Equal(0xAABBCCDDu, new EnumMultiPayload.NamedSingleCase(0xAABBCCDD).number);
        Assert.Equal(0xDEADBEEFUL, Interop.enums_multi_borrow(EnumMultiPayload.Single(tupleSingle)));
        Assert.Equal(0xAABBCCDDUL, Interop.enums_multi_borrow(EnumMultiPayload.NamedSingle(namedSingle)));
        Interop.enums_multi_echo(EnumMultiPayload.Unit).AsUnit();
        Interop.enums_multi_echo(EnumMultiPayload.EmptyTuple).AsEmptyTuple();
        Interop.enums_multi_echo(EnumMultiPayload.EmptyNamed).AsEmptyNamed();
    }

    [Fact]
    public void colliding_field_names_do_not_hide_record_members()
    {
        var result = Interop.enums_multi_echo(EnumMultiPayload.Collisions(11, 22, 33, 44));
        var fields = result.AsCollisions();
        var (first, second, third, fourth) = fields;

        Assert.Equal(11u, fields.ValueField);
        Assert.Equal(22u, fields.EqualsField);
        Assert.Equal(33u, fields.DeconstructField);
        Assert.Equal(44u, fields.CollisionsCaseField);
        Assert.Equal((11u, 22u, 33u, 44u), (first, second, third, fourth));
        Assert.Equal(new EnumMultiPayload.CollisionsCase(11, 22, 33, 44), fields);
    }

    [Fact]
    public void union_access_and_exhaustive_patterns_agree_for_every_case()
    {
        EnumMultiPayload tuple = new EnumMultiPayload.TupleCase(1, 2, 3);
        Assert.True(tuple.HasValue);
        Assert.True(tuple.TryGetValue(out EnumMultiPayload.TupleCase selected));
        Assert.Equal(new EnumMultiPayload.TupleCase(1, 2, 3), selected);
        Assert.Equal(selected, tuple.Value);
        Assert.False(tuple.TryGetValue(out EnumMultiPayload.NamedCase inactive));
        Assert.Equal(default, inactive);
        Assert.False(tuple.TryGetValue(out EnumMultiPayload.UnitCase _));

        var cases = new[]
        {
            EnumMultiPayload.Unit,
            EnumMultiPayload.Single(1),
            tuple,
            EnumMultiPayload.Named(1, 2, 3),
            EnumMultiPayload.NamedSingle(1),
            EnumMultiPayload.EmptyTuple,
            EnumMultiPayload.EmptyNamed,
            EnumMultiPayload.Collisions(1, 2, 3, 4)
        };
        var expected = new[] { "unit", "single:1", "tuple:1:2:3", "named:1:2:3", "named-single:1", "empty-tuple", "empty-named", "collisions:1:2:3:4" };
        for (var i = 0; i < cases.Length; i++)
            Assert.Equal(expected[i], Describe(cases[i]));
    }

    // No catch-all arm: C# 15 must recognize every generated case constructor and Deconstruct.
    private static string Describe(EnumMultiPayload value) => value switch
    {
        EnumMultiPayload.UnitCase => "unit",
        EnumMultiPayload.SingleCase(var number) => $"single:{number}",
        EnumMultiPayload.TupleCase(var small, var wide, var tail) => $"tuple:{small}:{wide}:{tail}",
        EnumMultiPayload.NamedCase(var small, var wide, var tail) => $"named:{small}:{wide}:{tail}",
        EnumMultiPayload.NamedSingleCase(var number) => $"named-single:{number}",
        EnumMultiPayload.EmptyTupleCase => "empty-tuple",
        EnumMultiPayload.EmptyNamedCase => "empty-named",
        EnumMultiPayload.CollisionsCase(var first, var second, var third, var fourth) => $"collisions:{first}:{second}:{third}:{fourth}",
        null => "<empty>"
    };

    [Fact]
    public void default_payload_union_is_empty_and_cannot_cross_into_rust()
    {
        var empty = default(EnumMultiPayload);

        Assert.False(empty.HasValue);
        Assert.False(empty.IsUnit);
        Assert.False(empty.IsTuple);
        Assert.Null(empty.Value);
        Assert.False(empty.TryGetValue(out EnumMultiPayload.TupleCase _));
        Assert.False(empty.TryGetValue(out EnumMultiPayload.UnitCase _));
        Assert.Equal("<empty>", Describe(empty));
        Assert.Throws<InvalidOperationException>(() => empty.AsTuple());
        Assert.Throws<InvalidOperationException>(() => Interop.enums_multi_echo(empty));
        Assert.Throws<InvalidOperationException>(() => Interop.enums_multi_borrow(in empty));
        Assert.ThrowsAny<Exception>(() => empty.Wire());
    }

    [Fact]
    public void multi_field_exceptions_carry_cases_while_single_field_exceptions_carry_payloads()
    {
        var tuple = EnumMultiPayload.Tuple(1, 2, 3);
        var tupleError = Assert.Throws<EnumException<EnumMultiPayload.TupleCase>>(() => tuple.AsNamed());
        Assert.Equal(tuple.AsTuple(), tupleError.Value);

        var named = EnumMultiPayload.Named(4, 5, 6);
        var namedError = Assert.IsType<EnumException<EnumMultiPayload.NamedCase>>(named.ExceptionForVariant());
        Assert.Equal(named.AsNamed(), namedError.Value);

        var singleError = Assert.Throws<EnumException<uint>>(() => EnumMultiPayload.Single(7).AsTuple());
        var namedSingleError = Assert.Throws<EnumException<uint>>(() => EnumMultiPayload.NamedSingle(8).AsTuple());
        Assert.Equal(7u, singleError.Value);
        Assert.Equal(8u, namedSingleError.Value);
        Assert.IsType<EnumException>(EnumMultiPayload.Unit.ExceptionForVariant());
    }

    [Fact]
    public void borrowing_owned_tuple_and_named_fields_preserves_every_allocation()
    {
        AllocationProbe.Settle();
        var baselineBytes = Interop.__test_live_bytes();
        var baselineAllocations = Interop.__test_live_allocations();

        foreach (var named in new[] { false, true })
        {
            var first = "héllo 🌍".Utf8();
            var second = "世界\0tail".Utf8();
            using (var value = named ? EnumMultiOwned.Named(first, second, 37) : EnumMultiOwned.Tuple(first, second, 37))
            {
                var ownedBytes = Interop.__test_live_bytes();
                var ownedAllocations = Interop.__test_live_allocations();
                Assert.True(ownedBytes > baselineBytes);
                Assert.True(ownedAllocations > baselineAllocations);
                var expected = (ulong)(Encoding.UTF8.GetByteCount("héllo 🌍") + Encoding.UTF8.GetByteCount("世界\0tail")) + 37;

                for (var i = 0; i < 64; i++)
                    Assert.Equal(expected, Interop.enums_multi_owned_borrow(in value));

                Assert.Equal("héllo 🌍", first.String);
                Assert.Equal("世界\0tail", second.String);
                Assert.Equal(ownedBytes, Interop.__test_live_bytes());
                Assert.Equal(ownedAllocations, Interop.__test_live_allocations());
            }
            Assert.Equal(baselineBytes, Interop.__test_live_bytes());
            Assert.Equal(baselineAllocations, Interop.__test_live_allocations());
        }
    }

    [Fact]
    public void moving_and_disposing_owned_variants_transfers_all_active_fields_once()
    {
        AllocationProbe.Settle();
        var baselineBytes = Interop.__test_live_bytes();
        var baselineAllocations = Interop.__test_live_allocations();
        var factories = new Func<EnumMultiOwned>[]
        {
            () => EnumMultiOwned.Unit,
            () => EnumMultiOwned.Single("solo 🌍".Utf8()),
            () => EnumMultiOwned.Tuple("first 🌍".Utf8(), "second 世界".Utf8(), 37),
            () => EnumMultiOwned.Named("first 🌍".Utf8(), "second 世界".Utf8(), 37)
        };

        foreach (var factory in factories)
        {
            using (var source = factory())
            {
                var expected = DescribeOwned(source);
                var expectedBorrow = Interop.enums_multi_owned_borrow(in source);
                using var result = Interop.enums_multi_owned_echo(source);

                // The moved source no longer owns the buffers; repeated disposal must leave
                // both active result strings alive and must ignore every inactive variant.
                source.Dispose();
                source.Dispose();
                Assert.Equal(expected, DescribeOwned(result));
                Assert.Equal(expectedBorrow, Interop.enums_multi_owned_borrow(in result));
                result.Dispose();
                result.Dispose();
            }
            Assert.Equal(baselineBytes, Interop.__test_live_bytes());
            Assert.Equal(baselineAllocations, Interop.__test_live_allocations());
        }
    }

    [Fact]
    public void owned_cases_expose_views_without_transferring_or_duplicating_ownership()
    {
        Assert.True(typeof(EnumMultiOwned).IsClass);
        Assert.Null(default(EnumMultiOwned));
        Assert.Throws<InvalidOperationException>(() => Interop.enums_multi_owned_echo(null!));
        Assert.Throws<InvalidOperationException>(() => Interop.enums_multi_owned_borrow(null!));

        using var tuple = EnumMultiOwned.Tuple("first".Utf8(), "second".Utf8(), 9);
        Assert.True(tuple.HasValue);
        Assert.True(tuple.TryGetValue(out EnumMultiOwned.TupleCase selected));
        Assert.Equal("first", selected.Item1.String);
        Assert.Equal("second", selected.Item2.String);
        Assert.Equal(9UL, selected.Item3);
        Assert.Equal(selected, tuple.Value);
        Assert.False(tuple.TryGetValue(out EnumMultiOwned.NamedCase _));
        var tupleError = Assert.Throws<EnumException<EnumMultiOwned.TupleCase>>(() => tuple.AsNamed());
        Assert.Same(selected.Item1, tupleError.Value.Item1);
        Assert.Same(selected.Item2, tupleError.Value.Item2);

        using var single = EnumMultiOwned.Single("single".Utf8());
        Utf8String singlePayload = single.AsSingle();
        var singleError = Assert.Throws<EnumException<Utf8String>>(() => single.AsTuple());
        Assert.Same(singlePayload, singleError.Value);
        Assert.Equal("<empty>", DescribeOwned(null));
    }

    private static string DescribeOwned(EnumMultiOwned? value) => value switch
    {
        EnumMultiOwned.UnitCase => "unit",
        EnumMultiOwned.SingleCase(var text) => $"single:{text.String}",
        EnumMultiOwned.TupleCase(var first, var second, var count) => $"tuple:{first.String}:{second.String}:{count}",
        EnumMultiOwned.NamedCase(var first, var second, var count) => $"named:{first.String}:{second.String}:{count}",
        null => "<empty>"
    };

    [Fact]
    public void owned_wire_round_trip_copies_and_disposes_both_active_strings()
    {
        AllocationProbe.Settle();
        var baselineBytes = Interop.__test_live_bytes();
        var baselineAllocations = Interop.__test_live_allocations();

        foreach (var named in new[] { false, true })
        {
            using (var source = named
                ? EnumMultiOwned.Named("héllo 🌍".Utf8(), "世界\0tail".Utf8(), 37)
                : EnumMultiOwned.Tuple("héllo 🌍".Utf8(), "世界\0tail".Utf8(), 37))
            {
                var expected = DescribeOwned(source);
                var expectedBorrow = Interop.enums_multi_owned_borrow(in source);
                using var wire = source.Wire();
                using var returned = Interop.enums_multi_owned_wire_echo(wire);
                using var result = returned.Unwire();

                Assert.Equal(expected, DescribeOwned(source));
                Assert.Equal(expected, DescribeOwned(result));
                source.Dispose();
                Assert.Equal(expected, DescribeOwned(result));
                Assert.Equal(expectedBorrow, Interop.enums_multi_owned_borrow(in result));
                result.Dispose();
                result.Dispose();
            }
            Assert.Equal(baselineBytes, Interop.__test_live_bytes());
            Assert.Equal(baselineAllocations, Interop.__test_live_allocations());
        }
    }

    [Fact]
    public void malformed_later_owned_wire_field_releases_already_decoded_strings()
    {
        AllocationProbe.Settle();
        var baselineBytes = Interop.__test_live_bytes();
        var baselineAllocations = Interop.__test_live_allocations();

        using (var source = EnumMultiOwned.Tuple("first 🌍".Utf8(), "second 世界".Utf8(), 37))
        using (var wire = source.Wire())
        {
            // WireBuffer is deliberately internal. Reflection is confined to corrupting this
            // test-owned buffer, without widening the generated consumer API.
            var bufferField = typeof(WireOfEnumMultiOwned).GetField("Buffer", BindingFlags.Instance | BindingFlags.NonPublic);
            Assert.NotNull(bufferField);
            var buffer = bufferField!.GetValue(wire);
            Assert.NotNull(buffer);
            var writerMethod = buffer!.GetType().GetMethod("Writer", BindingFlags.Instance | BindingFlags.NonPublic);
            Assert.NotNull(writerMethod);
            using (var writer = Assert.IsType<BinaryWriter>(writerMethod!.Invoke(buffer, null)))
            {
                writer.BaseStream.Position = sizeof(byte) + sizeof(uint) + Encoding.UTF8.GetByteCount("first 🌍");
                writer.Write(uint.MaxValue);
            }

            var ownedBytes = Interop.__test_live_bytes();
            var ownedAllocations = Interop.__test_live_allocations();
            for (var i = 0; i < 16; i++)
            {
                Assert.ThrowsAny<Exception>(() => wire.Unwire());
                Assert.Equal(ownedBytes, Interop.__test_live_bytes());
                Assert.Equal(ownedAllocations, Interop.__test_live_allocations());
            }
            Assert.Equal("tuple:first 🌍:second 世界:37", DescribeOwned(source));
        }

        Assert.Equal(baselineBytes, Interop.__test_live_bytes());
        Assert.Equal(baselineAllocations, Interop.__test_live_allocations());
    }

    [Fact]
    public void wire_round_trip_reencodes_every_raw_enum_variant_in_rust()
    {
        foreach (var value in new[]
        {
            EnumMultiPayload.Unit,
            EnumMultiPayload.Single(0xDEADBEEF),
            EnumMultiPayload.Tuple(0xA5, 0x8877665544332211UL, 0xCAFE),
            EnumMultiPayload.Named(0x5A, 0x1122334455667788UL, 0xFECA),
            EnumMultiPayload.NamedSingle(0xAABBCCDD),
            EnumMultiPayload.EmptyTuple,
            EnumMultiPayload.EmptyNamed,
            EnumMultiPayload.Collisions(11, 22, 33, 44)
        })
        {
            using var wire = value.Wire();
            using var returned = Interop.enums_multi_wire_echo(wire);
            var result = returned.Unwire();
            Assert.Equal(value.Value, result.Value);
        }
    }

    [Fact]
    public void wire_round_trip_reencodes_managed_strings_and_vectors_in_rust()
    {
        using (var wire = WireMultiPayload.Unit.Wire())
        using (var returned = Interop.wire_multi_echo(wire))
        {
            var result = returned.Unwire();
            Assert.True(result.IsUnit);
            Assert.True(result.HasValue);
            Assert.IsType<WireMultiPayload.UnitCase>(result.Value);
        }

        using (var wire = WireMultiPayload.Single("single 🌍").Wire())
        using (var returned = Interop.wire_multi_echo(wire))
            Assert.Equal("single 🌍", returned.Unwire().AsSingle());

        using (var wire = WireMultiPayload.Tuple(0xA5, "héllo\0世界", [0x1111, 0xABCD, 0xFFFF]).Wire())
        using (var returned = Interop.wire_multi_echo(wire))
        {
            var result = returned.Unwire();
            var fields = result.AsTuple();
            Assert.Equal((byte)0xA5, fields.Item1);
            Assert.Equal("héllo\0世界", fields.Item2);
            Assert.Equal(new ushort[] { 0x1111, 0xABCD, 0xFFFF }, fields.Item3);
            Assert.True(result.TryGetValue(out WireMultiPayload.TupleCase selected));
            Assert.Equal(fields, selected);
            Assert.False(result.TryGetValue(out WireMultiPayload.NamedCase _));
        }

        using (var wire = WireMultiPayload.Named(0x8877665544332211UL, "named 🌍", [0, 2, 0xFFFF]).Wire())
        using (var returned = Interop.wire_multi_echo(wire))
        {
            var fields = returned.Unwire().AsNamed();
            Assert.Equal(0x8877665544332211UL, fields.code);
            Assert.Equal("named 🌍", fields.text);
            Assert.Equal(new ushort[] { 0, 2, 0xFFFF }, fields.values);
        }
    }
}
