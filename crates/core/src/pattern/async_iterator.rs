//! Owned native asynchronous streams for foreign async enumeration.
//!
//! Wrap any standard `futures_core::Stream` with `AsyncIterator::new(stream, runtime)`.
//! The stream is pinned, `Send`, and owned; the runtime implements [`AsyncRuntime`].
//! Each foreign request spawns one next-item operation. No collection is buffered.

use crate::inventory::{Inventory, TypeId};
use crate::lang::meta::{Docs, Visibility, common_or_module_emission};
use crate::lang::types::{Type, TypeInfo, TypeKind, TypePattern, WireIO};
use crate::pattern::asynk::{AsyncRuntime, TaskHandle};
use crate::wire::SerializationError;
use futures_core::Stream;
use std::ffi::c_void;
use std::future::Future;
use std::io::{Read, Write};
use std::marker::PhantomData;
use std::mem::MaybeUninit;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

#[cfg(test)]
#[path = "async_iterator_tests.rs"]
mod tests;

type Complete = unsafe extern "C" fn(*const c_void, i32, *const c_void);
type Items<T> = Pin<Box<dyn Stream<Item = T> + Send>>;
type Spawn = Box<dyn Fn(Pin<Box<dyn Future<Output = ()> + Send>>) -> TaskHandle + Send + Sync>;

struct Slot<T> {
    stream: Option<Items<T>>,
    closed: bool,
}

struct State<T> {
    slot: Mutex<Slot<T>>,
    spawn: Spawn,
}

struct Completion {
    callback: Option<Complete>,
    context: *const c_void,
}

// The foreign context remains rooted until exactly one completion, on any worker.
unsafe impl Send for Completion {}

impl Completion {
    fn finish(&mut self, status: i32, value: *const c_void) {
        if let Some(callback) = self.callback.take() {
            unsafe { callback(self.context, status, value) };
        }
    }
}

/// An owning, single-pass asynchronous FFI iterator.
///
/// Three pointer-sized fields: state, next, drop. The next function returns an
/// existing [`TaskHandle`] and completes exactly once, possibly before returning.
/// Completion status is 1 (item), 0 (end), -1 (panic/protocol failure), or -2
/// (cancelled). An item's borrowed pointer is valid only during completion.
/// Backends must copy the value and restrict elements to non-owning value layouts.
/// Cancellation ends the traversal. Abort an active task before dropping its owner;
/// keep the callback context alive until completion acknowledges the abort.
#[repr(C)]
pub struct AsyncIterator<T> {
    data: *mut c_void,
    next_fn: unsafe extern "C" fn(*mut c_void, Complete, *const c_void) -> TaskHandle,
    drop_fn: unsafe extern "C" fn(*mut c_void),
    element: PhantomData<T>,
}

// Construction requires an owning Send stream; access is serialized by the slot.
unsafe impl<T: Send> Send for AsyncIterator<T> {}

impl<T: Send + 'static> AsyncIterator<T> {
    /// Wraps a stream without polling it. The supplied runtime is retained until
    /// the traversal and its final pending request have been released.
    #[must_use]
    pub fn new<S, R>(stream: S, runtime: R) -> Self
    where
        S: Stream<Item = T> + Send + 'static,
        R: AsyncRuntime + Send + Sync + 'static,
    {
        let state = State {
            slot: Mutex::new(Slot { stream: Some(Box::pin(stream) as Items<T>), closed: false }),
            spawn: Box::new(move |future| runtime.spawn(move |_| future)) as Spawn,
        };
        Self {
            data: Arc::into_raw(Arc::new(state)).cast_mut().cast(),
            next_fn: next::<T>,
            drop_fn: destroy::<T>,
            element: PhantomData,
        }
    }
}

struct Advance<T> {
    state: Arc<State<T>>,
    stream: Option<Items<T>>,
    completion: Completion,
}

impl<T> Advance<T> {
    // Drop user stream code before acknowledging cancellation, end, or failure.
    fn close(&mut self, status: i32) {
        self.state.slot.lock().unwrap_or_else(std::sync::PoisonError::into_inner).closed = true;
        let stream = self.stream.take();
        let dropped = catch_unwind(AssertUnwindSafe(|| drop(stream))).is_ok();
        self.completion.finish(if dropped { status } else { -1 }, std::ptr::null());
    }
}

impl<T> Drop for Advance<T> {
    fn drop(&mut self) {
        if self.completion.callback.is_some() {
            self.close(if std::thread::panicking() { -1 } else { -2 });
        }
    }
}

impl<T> Future for Advance<T> {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let this = self.get_mut();
        let polled = catch_unwind(AssertUnwindSafe(|| {
            this.stream.as_mut().expect("active stream").as_mut().poll_next(cx)
        }));
        match polled {
            Ok(Poll::Pending) => Poll::Pending,
            Ok(Poll::Ready(Some(item))) => {
                // Drop the original before invoking foreign code, containing custom
                // destructors as well as poll panics. MaybeUninit preserves alignment
                // and never drops the copied representation.
                let mut copied = MaybeUninit::<T>::uninit();
                let copied_ok = catch_unwind(AssertUnwindSafe(|| {
                    unsafe { std::ptr::copy_nonoverlapping(std::ptr::from_ref(&item), copied.as_mut_ptr(), 1) };
                    drop(item);
                })).is_ok();
                if !copied_ok {
                    this.close(-1);
                    return Poll::Ready(());
                }
                let closed = {
                    let mut slot = this.state.slot.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                    if slot.closed {
                        true
                    } else {
                        slot.stream = this.stream.take();
                        false
                    }
                };
                if closed {
                    this.close(-2);
                } else {
                    this.completion.finish(1, copied.as_ptr().cast());
                }
                Poll::Ready(())
            }
            Ok(Poll::Ready(None)) => {
                this.close(0);
                Poll::Ready(())
            }
            Err(_) => {
                this.close(-1);
                Poll::Ready(())
            }
        }
    }
}

unsafe extern "C" fn next<T: Send + 'static>(data: *mut c_void, callback: Complete, context: *const c_void) -> TaskHandle {
    // The descriptor owns one Arc. The caller serializes next/drop while this
    // function clones it; the spawned request then owns its independent reference.
    let pointer = data.cast::<State<T>>();
    let state = unsafe {
        Arc::increment_strong_count(pointer);
        Arc::from_raw(pointer)
    };
    let mut completion = Completion { callback: Some(callback), context };
    let (stream, closed) = {
        let mut slot = state.slot.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        (slot.stream.take(), slot.closed)
    };
    let Some(stream) = stream else {
        completion.finish(if closed { 0 } else { -1 }, std::ptr::null());
        return TaskHandle::dummy();
    };
    let advance = Advance { state: Arc::clone(&state), stream: Some(stream), completion };
    match catch_unwind(AssertUnwindSafe(|| (state.spawn)(Box::pin(advance)))) {
        Ok(handle) => handle,
        Err(_) => TaskHandle::dummy(), // Advance::drop has already completed the request.
    }
}

unsafe extern "C" fn destroy<T>(data: *mut c_void) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        let state = unsafe { Arc::from_raw(data.cast::<State<T>>()) };
        let stream = {
            let mut slot = state.slot.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            slot.closed = true;
            slot.stream.take()
        };
        // Pending requests own their stream separately and close it before callback.
        drop(stream);
    }));
}

impl<T> Drop for AsyncIterator<T> {
    fn drop(&mut self) {
        if !self.data.is_null() {
            unsafe { (self.drop_fn)(self.data) };
        }
    }
}

unsafe impl<T: TypeInfo> TypeInfo for AsyncIterator<T> {
    const WIRE_SAFE: bool = false;
    const RAW_SAFE: bool = T::RAW_SAFE;
    const ASYNC_SAFE: bool = false;
    const SERVICE_SAFE: bool = false;
    const SERVICE_CTOR_SAFE: bool = false;

    fn id() -> TypeId {
        TypeId::new(0x6A86AC433B9E45779302E26E811847FA).derive_id(T::id())
    }

    fn kind() -> TypeKind { TypeKind::TypePattern(TypePattern::AsyncIterator(T::id())) }

    fn ty() -> Type {
        let item = T::ty();
        Type {
            name: format!("AsyncIterator<{}>", item.name),
            visibility: Visibility::Public,
            docs: Docs::from_line("Owned, single-pass native asynchronous stream."),
            emission: common_or_module_emission(&[item.emission]),
            kind: Self::kind(),
        }
    }

    fn register(inventory: &mut impl Inventory) {
        T::register(inventory);
        <*mut c_void>::register(inventory);
        TaskHandle::register(inventory);
        inventory.register_type(Self::id(), Self::ty());
    }
}

unsafe impl<T: TypeInfo> WireIO for AsyncIterator<T> {
    fn write(&self, _: &mut impl Write) -> Result<(), SerializationError> { crate::bad_wire!() }
    fn read(_: &mut impl Read) -> Result<Self, SerializationError> { crate::bad_wire!() }
    fn live_size(&self) -> usize { crate::bad_wire!() }
}
