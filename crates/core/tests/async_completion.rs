use interoptopus::pattern::asynk::{AsyncCallback, AsyncCallbackGuard, AsyncOutcome};
use std::ffi::c_void;
use std::future::{Future, poll_fn};
use std::pin::pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::task::{Context, Poll, Waker};

#[derive(Default)]
struct State {
    calls: AtomicUsize,
    tag: AtomicUsize,
    dropped: AtomicBool,
    dropped_at_callback: AtomicBool,
}

extern "C" fn record(outcome: *const AsyncOutcome<u32>, context: *const c_void) {
    // SAFETY: Each test keeps State alive through the one completion.
    let state = unsafe { &*context.cast::<State>() };
    let tag = match unsafe { &*outcome } {
        AsyncOutcome::Ok(_) => 0,
        AsyncOutcome::Cancelled => 1,
        AsyncOutcome::Panicked => 2,
    };
    state.tag.store(tag, Ordering::SeqCst);
    state.dropped_at_callback.store(state.dropped.load(Ordering::SeqCst), Ordering::SeqCst);
    state.calls.fetch_add(1, Ordering::SeqCst);
}

fn callback(state: &State) -> AsyncCallback<u32> {
    AsyncCallback::with_context(record, std::ptr::from_ref(state).cast())
}

#[test]
fn dropping_unpolled_future_cancels_once() {
    let state = State::default();
    let guard = AsyncCallbackGuard::new(callback(&state));
    let future = async move {
        let _guard = guard;
        std::future::pending::<()>().await;
    };
    drop(future);
    assert_eq!(state.calls.load(Ordering::SeqCst), 1);
    assert_eq!(state.tag.load(Ordering::SeqCst), 1);
}

#[test]
fn panic_after_suspension_faults_once_and_releases_future_before_callback() {
    struct Probe<'a>(&'a State);
    impl Drop for Probe<'_> {
        fn drop(&mut self) {
            self.0.dropped.store(true, Ordering::SeqCst);
        }
    }
    let state = State::default();
    let guard = AsyncCallbackGuard::new(callback(&state));
    let probe = Probe(&state);
    let mut first = true;
    let native = poll_fn(move |_| -> Poll<u32> {
        let _ = &probe;
        if first {
            first = false;
            Poll::Pending
        } else {
            panic!("after suspension")
        }
    });
    {
        let mut caught = pin!(guard.catch_unwind(native));
        let mut context = Context::from_waker(Waker::noop());
        assert!(caught.as_mut().poll(&mut context).is_pending());
        assert_eq!(caught.as_mut().poll(&mut context), Poll::Ready(None));
    }
    drop(guard);
    assert_eq!(state.calls.load(Ordering::SeqCst), 1);
    assert_eq!(state.tag.load(Ordering::SeqCst), 2);
    assert!(state.dropped_at_callback.load(Ordering::SeqCst));
}

#[test]
fn successful_future_does_not_send_cancellation_on_drop() {
    let state = State::default();
    let callback = callback(&state);
    let guard = AsyncCallbackGuard::new(callback);
    {
        let mut caught = pin!(guard.catch_unwind(async { 7 }));
        assert_eq!(caught.as_mut().poll(&mut Context::from_waker(Waker::noop())), Poll::Ready(Some(7)));
    }
    assert!(guard.mark_completed());
    assert!(!guard.mark_completed());
    // SAFETY: State is alive and this is the callback's sole invocation.
    let value = 7;
    unsafe { callback.call_ok(&raw const value); }
    drop(guard);
    assert_eq!(state.calls.load(Ordering::SeqCst), 1);
    assert_eq!(state.tag.load(Ordering::SeqCst), 0);
}
