use crate::patterns::result::Error;
use crate::types::string::UseString;
use interoptopus::ffi::CStrPtr;
use interoptopus::lang::types::TypeInfo;
use interoptopus::{callback, ffi};
use std::ffi::c_void;

callback!(MyCallback(value: u32) -> u32);
callback!(MyCallbackNamespaced(value: u32) -> u32, namespace = NAMESPACE_COMMON);
callback!(MyCallbackVoid(ptr: *const c_void));
callback!(MyCallbackContextual(context: *const c_void, value: u32));
callback!(SumDelegate1());
callback!(SumDelegate2(x: i32, y: i32) -> i32);
callback!(SumDelegateReturn(x: i32, y: i32) -> ffi::Result<(), Error>);
callback!(SumDelegateReturn2(x: i32, y: i32));
callback!(Pointers(x: &i32, y: &mut i32));
callback!(StringCallback(s: ffi::String));
callback!(NestedStringCallback(s: UseString));
callback!(CStrPassthrough(s: CStrPtr<'static>) -> CStrPtr<'static>);

#[ffi]
pub struct DelegateCallback<C: TypeInfo> {
    pub callback: C,
    pub context: *const c_void,
}

#[ffi]
pub fn pattern_callback_1(callback: MyCallback, x: u32) -> u32 {
    callback.call(x)
}

#[ffi]
pub fn pattern_callback_2(callback: MyCallbackVoid) -> MyCallbackVoid {
    callback
}

// #[ffi]
// pub fn pattern_callback_3(callback: DelegateCallback<MyCallbackContextual>, x: u32) {
//     callback.callback.call(callback.context, x);
// }

#[ffi]
pub fn pattern_callback_4(callback: MyCallbackNamespaced, x: u32) -> u32 {
    callback.call(x)
}

#[ffi]
pub fn pattern_callback_5() -> SumDelegate1 {
    let value = Box::new("hello".to_string());
    SumDelegate1::from_fn(move || {
        dbg!(&value);
    })
}

#[ffi]
pub fn pattern_callback_6() -> SumDelegate2 {
    SumDelegate2::from_fn(|x, y| x + y)
}

#[ffi]
pub fn pattern_callback_7(c1: SumDelegateReturn, c2: SumDelegateReturn2, x: i32, i: i32, o: &mut i32) -> ffi::Result<(), Error> {
    *o = i - 1;

    // Call both callbacks. In C#, if the callback throws an exception, we might not re-enter
    // and the rest of this function won't run (incl. not running `drop()` and doing other
    // cleanup.
    //
    // Callbacks that return an `FFIError` can (if enabled in the C# backend) avoid that issue
    // by doing some exception handling ping-pong; see the interoptopus_backend_csharp
    // config setting `config.work_around_exception_in_callback_no_reentry`.
    //
    _ = c1.call(x, x); // In a real world you'd also want to check the result here.
    c2.call(x, x);

    *o = i + 1;

    ffi::Ok(())
}

#[ffi]
pub fn pattern_callback_8(cb: StringCallback, cb2: NestedStringCallback, s: ffi::String) {
    cb.call(s.clone());
    cb2.call(UseString { s1: s.clone(), s2: s.clone() });
}

#[ffi]
pub fn pattern_callback_9(x: Pointers) -> i32 {
    let a = 1;
    let mut b = 2;
    x.call(&a, &mut b);
    b
}

callback!(BoolCallback(value: ffi::Bool) -> ffi::Bool);
callback!(ValueCallback() -> u32);
callback!(ManyArgsCallback(a0: i32, a1: i32, a2: i32, a3: i32, a4: i32, a5: i32, a6: i32, a7: i32, a8: i32, a9: i32, a10: i32, a11: i32, a12: i32, a13: i32, a14: i32, a15: i32, a16: i32) -> i32);

#[ffi]
pub fn pattern_callback_bool(callback: BoolCallback, x: ffi::Bool) -> ffi::Bool {
    callback.call(x)
}

#[ffi]
pub fn pattern_callback_value(callback: ValueCallback) -> u32 {
    callback.call()
}

#[ffi]
pub fn pattern_callback_many(callback: ManyArgsCallback) -> i32 {
    callback.call(0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16)
}

