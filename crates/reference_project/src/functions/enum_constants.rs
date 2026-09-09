use interoptopus::ffi;
use interoptopus::wire::Wire;

/// Signed, sparse, 64-bit discriminants with interleaved payload variants.
#[ffi]
#[derive(Clone, Debug, PartialEq)]
pub enum EnumConstants {
    Ready = -7,
    Number(i64) = 11,
    End = 1_099_511_627_776,
    Count(i64) = 21,
    EmptyTuple() = 55,
    EmptyNamed {} = 56,
}

#[ffi]
pub enum EnumConstantsOwned {
    Empty = 3,
    Text(ffi::String) = 8,
    End = 99,
}

/// Names intentionally collide with the proposed nested enum and its backing member.
#[ffi]
#[allow(non_camel_case_types, non_snake_case)]
pub enum EnumConstantsNames {
    Constants = 4_294_967_296,
    Constants2,
    value__,
    value__Constant,
    Payload { Constants3: u32 },
}

#[ffi]
pub fn enum_constants_echo(x: EnumConstants) -> EnumConstants { x }

#[ffi]
pub fn enum_constants_wire_echo(mut x: Wire<EnumConstants>) -> Wire<EnumConstants> {
    Wire::from(x.unwire())
}

#[ffi]
pub fn enum_constants_owned_echo(x: EnumConstantsOwned) -> EnumConstantsOwned { x }

#[ffi]
pub fn enum_constants_owned_wire_echo(mut x: Wire<EnumConstantsOwned>) -> Wire<EnumConstantsOwned> {
    Wire::from(x.unwire())
}
