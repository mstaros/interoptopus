//! Regression fixtures for generated ownership cleanup.
use crate::patterns::callback::MyCallback;
use crate::types::enums::EnumMultiOwned;
use interoptopus::ffi;
use interoptopus::wire::Wire;
use std::collections::HashMap;
use crate::services::callback::ServiceCallbacks;

#[ffi]
pub fn disposal_call_services(_first: &ServiceCallbacks, _second: &ServiceCallbacks, callback: MyCallback) -> u32 {
    callback.call(0)
}

#[ffi]
pub fn disposal_call_services_mut_shared(_first: &mut ServiceCallbacks, _second: &ServiceCallbacks, callback: MyCallback) -> u32 {
    callback.call(0)
}

#[ffi]
pub fn disposal_call_services_shared_mut(_first: &ServiceCallbacks, _second: &mut ServiceCallbacks, callback: MyCallback) -> u32 {
    callback.call(0)
}

#[ffi]
pub fn disposal_call_services_mut_mut(_first: &mut ServiceCallbacks, _second: &mut ServiceCallbacks, callback: MyCallback) -> u32 {
    callback.call(0)
}

#[ffi]
pub fn disposal_borrow_async_service(_service: &mut crate::services::asynk::cancel::ServiceAsyncCancel) {}

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

#[ffi]
pub fn disposal_borrow_resources(input: &ffi::String, vector: &ffi::Vec<u8>, callback: &MyCallback) -> u32 {
    let result = callback.call(input.as_str().len() as u32 + vector.len() as u32);
    // Read again after callback reentry to exercise the full borrow lifetime.
    result + input.as_str().len() as u32 + vector.len() as u32
}

#[ffi]
pub fn disposal_native_callback(callback: MyCallback) -> MyCallback {
    MyCallback::from_fn(move |input| callback.call(input))
}

#[ffi]
pub fn disposal_borrow_owned_fields(input: crate::types::vec::UseSliceAndVec, callback: MyCallback) -> u32 {
    callback.call(input.s1.len() as u32);
    input.s1.len() as u32
}

#[ffi]
pub fn disposal_empty_callback() -> MyCallback { MyCallback::default() }

#[ffi]
pub fn disposal_native_slice_callback(callback: crate::patterns::slice::CallbackSliceMut) -> crate::patterns::slice::CallbackSliceMut {
    crate::patterns::slice::CallbackSliceMut::from_fn(move |input| callback.call(input))
}
