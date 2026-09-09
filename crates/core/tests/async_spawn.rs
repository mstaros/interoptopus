use interoptopus::ffi;
use interoptopus::pattern::asynk::{Async, AsyncCallback, AsyncOutcome, AsyncRuntime, TaskHandle};
use std::ffi::c_void;
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

#[ffi(service)]
pub struct RefuseRuntime {
    number_value: u32,
}

impl AsyncRuntime for RefuseRuntime {
    type T = ();
    fn spawn<C, F>(&self, _factory: C) -> TaskHandle
    where
        C: FnOnce(()) -> F + Send + 'static,
        F: Future<Output = ()> + Send + 'static,
    {
        panic!("runtime rejected task")
    }
}

#[ffi]
impl RefuseRuntime {
    pub fn create() -> Self { Self { number_value: 7 } }
    pub async fn number(this: Async<Self>) -> u32 { this.number_value }
}

extern "C" fn complete(value: *const AsyncOutcome<u32>, context: *const c_void) {
    // SAFETY: The test keeps both the result and state alive through this call.
    let result = unsafe { &*value };
    let state = unsafe { &*context.cast::<AtomicUsize>() };
    let tag = match result {
        AsyncOutcome::Ok(_) => 1,
        AsyncOutcome::Cancelled => 2,
        AsyncOutcome::Panicked => 3,
    };
    state.fetch_add(tag, Ordering::SeqCst);
}

#[test]
fn runtime_spawn_panic_faults_once_without_crossing_the_export() {
    let service = Arc::new(RefuseRuntime::create());
    let raw = Arc::into_raw(Arc::clone(&service));
    let state = AtomicUsize::new(0);
    let callback = AsyncCallback::with_context(complete, std::ptr::from_ref(&state).cast());
    // SAFETY: Both the service and callback context outlive the exported call.
    let handle = unsafe { refuse_runtime_number(raw, callback) };
    handle.abort();
    drop(handle);
    assert_eq!(state.load(Ordering::SeqCst), 3);
    // SAFETY: Balance the reference placed in the raw FFI handle.
    drop(unsafe { Arc::from_raw(raw) });
    assert_eq!(Arc::strong_count(&service), 1);
}
