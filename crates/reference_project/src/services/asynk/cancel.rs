use crate::patterns::result::Error;
use crate::patterns::callback::MyCallback;
use interoptopus::pattern::asynk::Async;
use interoptopus::pattern::result::result_to_ffi;
use interoptopus::rt::Tokio;
use interoptopus::{AsyncRuntime, ffi};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

#[ffi(service)]
#[derive(AsyncRuntime)]
pub struct ServiceAsyncCancel {
    runtime: Tokio,
    counter: Arc<AtomicU64>,
    drop_callback: Option<MyCallback>,
}

impl Drop for ServiceAsyncCancel {
    fn drop(&mut self) {
        if let Some(callback) = &self.drop_callback {
            callback.call(0);
        }
    }
}

#[ffi]
impl ServiceAsyncCancel {
    pub fn create() -> ffi::Result<Self, Error> {
        result_to_ffi(|| Ok(Self { runtime: Tokio::new(), counter: Arc::new(AtomicU64::new(0)), drop_callback: None }))
    }

    pub fn create_with_drop_callback(callback: MyCallback) -> ffi::Result<Self, Error> {
        result_to_ffi(|| Ok(Self { runtime: Tokio::new(), counter: Arc::new(AtomicU64::new(0)), drop_callback: Some(callback) }))
    }

    /// The callback is a deterministic gate; cancellation must acknowledge dropping this future.
    pub async fn callback_work(this: Async<Self>, gate: MyCallback, wait_for_cancel: bool) -> ffi::Result<u64, Error> {
        gate.call(0);
        if wait_for_cancel {
            std::future::pending::<()>().await;
        }
        ffi::Ok(this.counter.load(Ordering::Relaxed))
    }

    /// Runs for `iterations` steps, sleeping `step_ms` each. Returns
    /// the number of steps actually completed, which will be less than
    /// `iterations` if the task is aborted.
    pub async fn long_running(_this: Async<Self>, iterations: u64, step_ms: u64) -> ffi::Result<u64, Error> {
        let mut completed = 0u64;
        for _ in 0..iterations {
            tokio::time::sleep(std::time::Duration::from_millis(step_ms)).await;
            completed += 1;
        }
        ffi::Ok(completed)
    }

    /// Increments the service's shared counter each step. The final
    /// counter value is observable via [`counter`](Self::counter) even
    /// after the task is aborted.
    pub async fn counting_work(this: Async<Self>, iterations: u64, step_ms: u64) -> ffi::Result<u64, Error> {
        for _ in 0..iterations {
            tokio::time::sleep(std::time::Duration::from_millis(step_ms)).await;
            this.counter.fetch_add(1, Ordering::Relaxed);
        }
        ffi::Ok(this.counter.load(Ordering::Relaxed))
    }

    /// Returns the shared counter value. Non-async, always available.
    pub fn counter(&self) -> u64 {
        self.counter.load(Ordering::Relaxed)
    }

    /// Sleeps indefinitely (1 hour). Only completes if aborted.
    pub async fn sleep_forever(_this: Async<Self>) -> ffi::Result<(), Error> {
        tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
        ffi::Ok(())
    }
}
