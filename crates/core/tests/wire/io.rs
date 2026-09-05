use interoptopus::lang::types::WireIO;
use interoptopus::wire::SerializationError;
use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};

macro_rules! assert_seq_eq {
        ($container:expr, $($seq:expr),+) => {
            #[allow(unused_assignments)]
            {
                let mut counter = 0;
                $(
                    assert_eq!($container[counter], $seq, "mismatch in byte {counter}");
                    counter += 1;
                )+
            }
        };
    }

#[test]
fn u_roundtrip() -> Result<(), SerializationError> {
    let x = 144_u8;
    let y = 61_233_u16;
    let z = 3_253_534_345_u32;
    let u = 18_442_244_000_709_551_615_u64;

    let mut cursor = std::io::Cursor::new(Vec::new());
    x.write(&mut cursor)?;
    y.write(&mut cursor)?;
    z.write(&mut cursor)?;
    u.write(&mut cursor)?;

    // Check byte repr in the buffer.
    assert_eq!(x.live_size(), 1);
    assert_eq!(y.live_size(), 2);
    assert_eq!(z.live_size(), 4);
    assert_eq!(u.live_size(), 8);

    cursor.seek(SeekFrom::Start(0))?;
    let mut x_repr = [0u8; 1];
    let mut y_repr = [0u8; 2];
    let mut z_repr = [0u8; 4];
    let mut u_repr = [0u8; 8];

    cursor.read_exact(&mut x_repr)?;
    cursor.read_exact(&mut y_repr)?;
    cursor.read_exact(&mut z_repr)?;
    cursor.read_exact(&mut u_repr)?;

    assert_seq_eq!(x_repr, 0x90);
    assert_seq_eq!(y_repr, 0x31, 0xef);
    assert_seq_eq!(z_repr, 0x89, 0xfe, 0xec, 0xc1);
    assert_seq_eq!(u_repr, 0xff, 0x25, 0x5f, 0x1b, 0x35, 0x03, 0xf0, 0xff);

    // Deserialize back.
    cursor.seek(SeekFrom::Start(0))?;

    let nx = u8::read(&mut cursor)?;
    let ny = u16::read(&mut cursor)?;
    let nz = u32::read(&mut cursor)?;
    let nu = u64::read(&mut cursor)?;

    assert_eq!(nx, x);
    assert_eq!(ny, y);
    assert_eq!(nz, z);
    assert_eq!(nu, u);
    Ok(())
}

#[test]
fn i_roundtrip() -> Result<(), SerializationError> {
    let x = -128_i8;
    let y = -32_000_i16;
    let z = -2_100_500_900_i32;
    let u = -9_200_072_000_054_775_808_i64;

    let mut cursor = std::io::Cursor::new(Vec::new());
    x.write(&mut cursor)?;
    y.write(&mut cursor)?;
    z.write(&mut cursor)?;
    u.write(&mut cursor)?;

    // Check byte repr in the buffer.
    assert_eq!(x.live_size(), 1);
    assert_eq!(y.live_size(), 2);
    assert_eq!(z.live_size(), 4);
    assert_eq!(u.live_size(), 8);

    cursor.seek(SeekFrom::Start(0))?;
    let mut x_repr = [0u8; 1];
    let mut y_repr = [0u8; 2];
    let mut z_repr = [0u8; 4];
    let mut u_repr = [0u8; 8];

    cursor.read_exact(&mut x_repr)?;
    cursor.read_exact(&mut y_repr)?;
    cursor.read_exact(&mut z_repr)?;
    cursor.read_exact(&mut u_repr)?;

    assert_seq_eq!(x_repr, 0x80);
    assert_seq_eq!(y_repr, 0x00, 0x83);
    assert_seq_eq!(z_repr, 0x5c, 0xe6, 0xcc, 0x82);
    assert_seq_eq!(u_repr, 0x00, 0xb0, 0xb7, 0x90, 0x42, 0xc7, 0x52, 0x80);

    // Deserialize back.
    cursor.seek(SeekFrom::Start(0))?;

    let nx = i8::read(&mut cursor)?;
    let ny = i16::read(&mut cursor)?;
    let nz = i32::read(&mut cursor)?;
    let nu = i64::read(&mut cursor)?;

    assert_eq!(nx, x);
    assert_eq!(ny, y);
    assert_eq!(nz, z);
    assert_eq!(nu, u);
    Ok(())
}

#[test]
fn vec_roundtrip() -> Result<(), SerializationError> {
    let v1 = vec![0x1u8, 0x2, 0x3];
    let v2 = Vec::<u8>::new();

    let mut cursor = std::io::Cursor::new(Vec::new());
    v1.write(&mut cursor)?;
    v2.write(&mut cursor)?;

    // Check byte repr in the buffer (u32 length prefix = 4 bytes, always).
    cursor.seek(SeekFrom::Start(0))?;

    assert_eq!(v1.live_size(), 4 + 3);
    assert_eq!(v2.live_size(), 4);

    let mut v1_repr = [0u8; 4 + 3];
    let mut v2_repr = [0u8; 4];
    cursor.read_exact(&mut v1_repr)?;
    cursor.read_exact(&mut v2_repr)?;

    #[rustfmt::skip]
    assert_seq_eq!(v1_repr,
        0x03, 0x00, 0x00, 0x00,
        0x01, 0x02, 0x03);

    #[rustfmt::skip]
    assert_seq_eq!(v2_repr,
        0x00, 0x00, 0x00, 0x00);

    // Deserialize back.
    cursor.seek(SeekFrom::Start(0))?;

    let deserialized_v1 = Vec::<u8>::read(&mut cursor)?;
    let deserialized_v2 = Vec::<u8>::read(&mut cursor)?;

    assert_eq!(deserialized_v1, v1);
    assert_eq!(deserialized_v2, v2);
    Ok(())
}

#[test]
#[expect(clippy::cognitive_complexity)]
fn string_roundtrip() -> Result<(), SerializationError> {
    let s1 = String::from("Hello world");
    let s2 = String::from("selâm aleyküm dünya");

    let mut cursor = std::io::Cursor::new(Vec::new());
    s1.write(&mut cursor)?;
    s2.write(&mut cursor)?;

    // Check byte repr in the buffer (u32 length prefix = 4 bytes, always).
    cursor.seek(SeekFrom::Start(0))?;

    assert_eq!(s1.live_size(), 4 + 11);
    assert_eq!(s2.live_size(), 4 + 22);

    let mut s1_repr = [0u8; 4 + 11];
    let mut s2_repr = [0u8; 4 + 22];

    cursor.read_exact(&mut s1_repr)?;
    cursor.read_exact(&mut s2_repr)?;

    #[rustfmt::skip]
    assert_seq_eq!(s1_repr,
        0x0b, 0x00, 0x00, 0x00,
        72, 101, 108, 108, 111, 32, 119, 111, 114, 108, 100);

    #[rustfmt::skip]
    assert_seq_eq!(
        s2_repr,
        0x16, 0x00, 0x00, 0x00,
        115, 101, 108, 195, 162, 109, 32, 97, 108, 101, 121, 107,
        195, 188, 109, 32, 100, 195, 188, 110, 121, 97
    );

    // Deserialize back.
    cursor.seek(SeekFrom::Start(0))?;

    let deserialized_s1 = String::read(&mut cursor)?;
    let deserialized_s2 = String::read(&mut cursor)?;

    assert_eq!(deserialized_s1, s1);
    assert_eq!(deserialized_s2, s2);
    Ok(())
}

#[test]
fn hashmap_roundtrip() -> Result<(), SerializationError> {
    // Build maps and serialize.
    let mut h1 = HashMap::<String, u16>::new();
    let mut h2 = HashMap::<u16, Vec<bool>>::new();

    h1.insert("First".into(), 0x11aa);
    h1.insert("Second".into(), 0x22bb);
    h2.insert(0x22bb, vec![true, true, false]);
    h2.insert(0x11aa, vec![false, true, true]);

    let mut cursor = std::io::Cursor::new(Vec::new());
    h1.write(&mut cursor)?;
    h2.write(&mut cursor)?;

    // Check live_size (u32 length prefix = 4 bytes for maps, strings, vecs).
    // h1: 4(map len) + 4(str len)+5("First")+2(u16) + 4(str len)+6("Second")+2(u16) = 27
    assert_eq!(h1.live_size(), 4 + 4 + 5 + 2 + 4 + 6 + 2);
    // h2: 4(map len) + 2(u16)+4(vec len)+3(bools) + 2(u16)+4(vec len)+3(bools) = 22
    assert_eq!(h2.live_size(), 4 + 2 + 4 + 3 + 2 + 4 + 3);

    // Deserialize back and check values (iteration order is non-deterministic,
    // so we only check roundtrip equality, not byte layout).
    cursor.seek(SeekFrom::Start(0))?;

    let deserialized_h1 = HashMap::<String, u16>::read(&mut cursor)?;
    let deserialized_h2 = HashMap::<u16, Vec<bool>>::read(&mut cursor)?;

    assert_eq!(deserialized_h1, h1);
    assert_eq!(deserialized_h2, h2);
    Ok(())
}

#[test]
#[expect(clippy::cognitive_complexity)]
fn tuple_roundtrip() -> Result<(), SerializationError> {
    let a = (8u32, "Hello world".to_string(), vec![1u32, 2, 3]);

    let mut cursor = std::io::Cursor::new(Vec::new());
    a.write(&mut cursor)?;

    // Check byte repr in the buffer (u32 length prefixes = 4 bytes).
    // u32(4) + string: u32(4)+11 + vec: u32(4)+3*u32(12) = 4+15+16 = 35
    assert_eq!(a.live_size(), 4 + 4 + 11 + 4 + 4 + 4 + 4);

    cursor.seek(SeekFrom::Start(0))?;
    let mut a_repr = [0u8; 35];

    cursor.read_exact(&mut a_repr)?;

    #[rustfmt::skip]
    assert_seq_eq!(a_repr,
        0x08, 0x00, 0x00, 0x00,
        0x0b, 0x00, 0x00, 0x00,
        72, 101, 108, 108, 111, 32, 119, 111, 114, 108, 100,
        0x03, 0x00, 0x00, 0x00,
        0x01, 0x00, 0x00, 0x00,
        0x02, 0x00, 0x00, 0x00,
        0x03, 0x00, 0x00, 0x00);

    // Deserialize back.
    cursor.seek(SeekFrom::Start(0))?;

    let deserialized_a = <(u32, String, Vec<u32>)>::read(&mut cursor)?;

    assert_eq!(deserialized_a, a);

    Ok(())
}

#[interoptopus::ffi]
#[derive(Debug, PartialEq)]
enum WireChoice {
    Empty = -2,
    Number(u16),
    Text(String) = 3,
}

#[test]
fn enum_payloads_preserve_tags_bytes_and_live_size() -> Result<(), SerializationError> {
    for (value, expected) in [
        (WireChoice::Empty, vec![0xfe]),
        (WireChoice::Number(0x1234), vec![0xff, 0x34, 0x12]),
        (WireChoice::Text("aß".to_string()), vec![3, 3, 0, 0, 0, b'a', 0xc3, 0x9f]),
        (WireChoice::Text(String::new()), vec![3, 0, 0, 0, 0]),
    ] {
        let mut bytes = Vec::new();
        value.write(&mut bytes)?;
        assert_eq!(bytes, expected);
        assert_eq!(value.live_size(), bytes.len());

        let mut input = bytes.as_slice();
        assert_eq!(WireChoice::read(&mut input)?, value);
        assert!(input.is_empty(), "reading a variant must consume exactly its tag and payload");
    }
    Ok(())
}

#[test]
fn enum_payloads_reject_unknown_tags_and_truncation() {
    for bytes in [&[0u8][..], &[0xff], &[0xff, 0x34], &[3, 1, 0, 0, 0]] {
        assert!(WireChoice::read(&mut &*bytes).is_err(), "invalid wire enum accepted: {bytes:?}");
    }
}

#[interoptopus::ffi]
#[derive(Debug, PartialEq)]
enum WireFields {
    Unit = -129,
    Tuple(u8, u64, u16),
    Named { small: u8, text: String, last: u16 } = 256,
    EmptyTuple(),
    EmptyNamed {},
}

#[test]
fn multi_field_wire_is_ordered_packed_and_uses_the_declared_tag_width() -> Result<(), SerializationError> {
    for (value, expected) in [
        (WireFields::Unit, vec![0x7f, 0xff]),
        (WireFields::Tuple(0xab, 0x0102_0304_0506_0708, 0xcdef), vec![0x80, 0xff, 0xab, 8, 7, 6, 5, 4, 3, 2, 1, 0xef, 0xcd]),
        (WireFields::Named { small: 7, text: "aß".to_string(), last: 0x1234 }, vec![0, 1, 7, 3, 0, 0, 0, b'a', 0xc3, 0x9f, 0x34, 0x12]),
        (WireFields::EmptyTuple(), vec![1, 1]),
        (WireFields::EmptyNamed {}, vec![2, 1]),
    ] {
        let mut bytes = Vec::new();
        value.write(&mut bytes)?;
        assert_eq!(bytes, expected);
        assert_eq!(value.live_size(), bytes.len());
        let mut input = bytes.as_slice();
        assert_eq!(WireFields::read(&mut input)?, value);
        assert!(input.is_empty());
        for len in 0..bytes.len() {
            assert!(WireFields::read(&mut &bytes[..len]).is_err(), "truncated {value:?} accepted at {len} bytes");
        }
    }
    assert!(WireFields::read(&mut &[0u8, 0][..]).is_err());
    Ok(())
}

#[test]
fn owned_utf8_wire_matches_standard_string_and_rejects_invalid_input() -> Result<(), SerializationError> {
    for text in ["", "aß\0🌍"] {
        let owned = interoptopus::ffi::String::from_string(text.to_string());
        let mut bytes = Vec::new();
        owned.write(&mut bytes)?;
        let mut expected = Vec::new();
        text.to_string().write(&mut expected)?;
        assert_eq!(bytes, expected);
        assert_eq!(owned.live_size(), bytes.len());
        assert_eq!(interoptopus::ffi::String::read(&mut bytes.as_slice())?.as_str(), text);
        for len in 0..bytes.len() {
            assert!(interoptopus::ffi::String::read(&mut &bytes[..len]).is_err());
        }
    }
    assert!(interoptopus::ffi::String::read(&mut &[1, 0, 0, 0, 0xff][..]).is_err());
    Ok(())
}
