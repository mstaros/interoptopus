//! Regression fixtures for generated ownership cleanup.
use crate::patterns::callback::MyCallback;
use crate::types::enums::EnumMultiOwned;
use interoptopus::ffi;
use interoptopus::wire::Wire;
use std::collections::HashMap;

#[ffi]
pub fn disposal_take_strings(_first: ffi::String, _second: ffi::String) {}

#[ffi]
pub fn disposal_take_callback(_callback: MyCallback, _value: EnumMultiOwned) {}

#[ffi]
pub struct DisposalFields {
    pub first: MyCallback,
    pub text: ffi::String,
    pub second: MyCallback,
}

#[ffi]
pub fn disposal_fields_invoke(input: &DisposalFields) {
    input.first.call(1);
    input.second.call(2);
}

#[ffi]
pub enum DisposalCallbacks {
    Unit,
    Values(MyCallback, ffi::String, MyCallback),
}

#[ffi]
pub fn disposal_callbacks_invoke(input: &DisposalCallbacks) {
    if let DisposalCallbacks::Values(first, _, second) = input {
        first.call(1);
        second.call(2);
    }
}

#[ffi]
pub struct DisposalMixedLeaf {
    pub text: ffi::String,
    pub note: String,
}

#[ffi]
pub struct DisposalMixedFields {
    pub child: DisposalMixedLeaf,
    pub label: String,
    pub native: ffi::String,
    pub list: Vec<ffi::String>,
    pub optional: Option<ffi::String>,
    pub array: [u32; 2],
}

#[ffi]
pub enum DisposalMixed {
    Empty,
    End,
    Managed(String),
    Native(ffi::String),
    Many(Vec<ffi::String>),
    Table(HashMap<String, ffi::String>),
    Nested(DisposalMixedFields),
}

#[ffi]
pub fn disposal_mixed_echo(mut input: Wire<DisposalMixed>) -> Wire<DisposalMixed> {
    Wire::from(input.unwire())
}

#[ffi]
pub fn disposal_mixed_fields_echo(mut input: Wire<DisposalMixedFields>) -> Wire<DisposalMixedFields> {
    Wire::from(input.unwire())
}
