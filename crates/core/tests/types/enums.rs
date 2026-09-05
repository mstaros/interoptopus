use interoptopus::ffi;
use interoptopus::inventory::RustInventory;
use interoptopus::lang::types::{TypeInfo, TypeKind, VariantKind, WireIO};

#[ffi]
#[derive(Debug, PartialEq)]
enum MultiLayout {
    Unit,
    Single(u32),
    Tuple(u8, u64, u16),
    Named {
        small: u8,
        /// The middle payload must retain its metadata and alignment.
        wide: u64,
        tail: u16,
    },
    EmptyTuple(),
    EmptyNamed {},
}

#[ffi]
enum WireOnlyLast {
    Tuple(u8, String),
}

#[ffi]
enum NonWireLast {
    Named { lead: u8, text: ffi::Vec<u8> },
}

#[ffi]
enum NonAsyncLast {
    Tuple(u8, *const u8),
}

#[ffi]
#[derive(Debug, PartialEq)]
enum GenericMulti<T: TypeInfo, U>
where
    U: TypeInfo
{
    Tuple(u8, T, U),
    Named { first: T, second: U },
}

#[test]
fn multi_field_metadata_retains_order_names_and_registration() {
    let TypeKind::Enum(e) = MultiLayout::kind() else { panic!("expected an enum") };
    assert!(matches!(&e.variants[2].kind, VariantKind::Tuple(fields) if fields == &[u8::id(), u64::id(), u16::id()]));
    let VariantKind::Struct(fields) = &e.variants[3].kind else { panic!("expected named fields") };
    assert_eq!(fields.iter().map(|field| (field.name.as_str(), field.ty)).collect::<Vec<_>>(), [("small", u8::id()), ("wide", u64::id()), ("tail", u16::id())]);
    assert_eq!(fields[1].docs.lines, ["The middle payload must retain its metadata and alignment."]);
    assert_eq!(e.variants[3].payloads().map(|field| field.name).collect::<Vec<_>>(), [Some("small"), Some("wide"), Some("tail")]);
    assert_eq!(e.variants.iter().map(|variant| variant.tag).collect::<Vec<_>>(), [0, 1, 2, 3, 4, 5]);
    assert!(!e.variants[0].has_payload());
    assert!(e.variants[4].has_payload());
    assert!(e.variants[5].has_payload());
    assert_eq!(e.variants[4].payloads().count(), 0);
    assert_eq!(e.variants[5].payloads().count(), 0);

    let mut inventory = RustInventory::new();
    MultiLayout::register(&mut inventory);
    for id in [MultiLayout::id(), u8::id(), u32::id(), u64::id(), u16::id()] {
        assert!(inventory.types.contains_key(&id), "every payload must be registered");
    }
}

#[test]
fn safety_flags_include_later_tuple_and_named_fields() {
    let raw = std::hint::black_box(MultiLayout::RAW_SAFE);
    assert!(raw);
    let flags = std::hint::black_box((WireOnlyLast::WIRE_SAFE, WireOnlyLast::RAW_SAFE, NonWireLast::WIRE_SAFE, NonWireLast::RAW_SAFE, NonAsyncLast::ASYNC_SAFE));
    assert_eq!(flags, (true, false, false, true, false));
}

#[test]
fn generic_multi_field_wire_bounds_cover_both_parameters() {
    for value in [GenericMulti::Tuple(7, 0x1234_u16, "payload".to_string()), GenericMulti::Named { first: 0x5678, second: String::new() }] {
        let mut bytes = Vec::new();
        value.write(&mut bytes).unwrap();
        assert_eq!(value.live_size(), bytes.len());
        assert_eq!(GenericMulti::<u16, String>::read(&mut bytes.as_slice()).unwrap(), value);
    }
    let mut inventory = RustInventory::new();
    GenericMulti::<u16, String>::register(&mut inventory);
    assert!(inventory.types.contains_key(&String::id()));
    assert!(inventory.types.contains_key(&u16::id()));
}

#[test]
fn multi_field_native_layout_matches_a_tagged_sequential_helper() {
    #[repr(C)]
    struct Helper {
        tag: u8,
        small: u8,
        wide: u64,
        tail: u16,
    }
    assert_eq!(std::mem::size_of::<MultiLayout>(), std::mem::size_of::<Helper>());
    assert_eq!(std::mem::align_of::<MultiLayout>(), std::mem::align_of::<Helper>());
    for value in [MultiLayout::Tuple(0xab, 0x0102_0304_0506_0708, 0xcdef), MultiLayout::Named { small: 0xab, wide: 0x0102_0304_0506_0708, tail: 0xcdef }] {
        let base = std::ptr::from_ref(&value).addr();
        let (MultiLayout::Tuple(small, wide, tail) | MultiLayout::Named { small, wide, tail }) = &value else { unreachable!() };
        assert_eq!(std::ptr::from_ref(small).addr() - base, std::mem::offset_of!(Helper, small));
        assert_eq!(std::ptr::from_ref(wide).addr() - base, std::mem::offset_of!(Helper, wide));
        assert_eq!(std::ptr::from_ref(tail).addr() - base, std::mem::offset_of!(Helper, tail));
    }
}