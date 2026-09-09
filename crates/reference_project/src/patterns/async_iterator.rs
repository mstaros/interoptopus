use interoptopus::ffi;
use interoptopus::rt::Tokio;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU32, Ordering};
use std::task::{Context, Poll};

static LIVE: AtomicU32 = AtomicU32::new(0);
static PRODUCED: AtomicU32 = AtomicU32::new(0);

struct Delayed {
    next: u32,
    count: u32,
    ms: u64,
    sleep: Option<Pin<Box<tokio::time::Sleep>>>,
    mode: u32,
}

impl futures_core::Stream for Delayed {
    type Item = u32;
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<u32>> {
        let this = self.get_mut();
        assert!(this.mode != 1, "async stream fixture panic");
        if this.mode == 2 { return Poll::Pending; }
        if this.next == this.count { return Poll::Ready(None); }
        if this.ms > 0 {
            let sleep = this.sleep.get_or_insert_with(|| Box::pin(tokio::time::sleep(std::time::Duration::from_millis(this.ms))));
            if sleep.as_mut().poll(cx).is_pending() { return Poll::Pending; }
            this.sleep = None;
        }
        let value = this.next;
        this.next += 1;
        PRODUCED.fetch_add(1, Ordering::SeqCst);
        Poll::Ready(Some(value))
    }
}

impl Drop for Delayed {
    fn drop(&mut self) {
        LIVE.fetch_sub(1, Ordering::SeqCst);
        assert!(self.mode != 3, "async stream destructor fixture panic");
    }
}

fn tracked(count: u32, ms: u64, mode: u32) -> Delayed {
    LIVE.fetch_add(1, Ordering::SeqCst);
    Delayed { next: 0, count, ms, sleep: None, mode }
}

struct ReadyItems<T>(std::vec::IntoIter<T>);
impl<T: Unpin> futures_core::Stream for ReadyItems<T> {
    type Item = T;
    fn poll_next(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Option<T>> {
        Poll::Ready(self.get_mut().0.next())
    }
}

#[ffi]
pub fn pattern_async_iterator_create(count: u32, ms: u64, mode: u32) -> ffi::AsyncIterator<u32> {
    ffi::AsyncIterator::new(tracked(count, ms, mode), Tokio::new())
}

#[ffi]
pub fn pattern_async_iterator_live() -> u32 { LIVE.load(Ordering::SeqCst) }

#[ffi]
pub fn pattern_async_iterator_produced() -> u32 { PRODUCED.load(Ordering::SeqCst) }

#[ffi]
pub fn pattern_async_iterator_echo(source: ffi::AsyncIterator<u32>) -> ffi::AsyncIterator<u32> { source }

#[ffi]
pub fn pattern_async_iterator_bools() -> ffi::AsyncIterator<ffi::Bool> {
    ffi::AsyncIterator::new(ReadyItems(vec![ffi::Bool::from(false), ffi::Bool::from(true)].into_iter()), Tokio::new())
}

#[ffi]
pub fn pattern_async_iterator_points() -> ffi::AsyncIterator<crate::types::basic::Vec3f32> {
    ffi::AsyncIterator::new(ReadyItems(vec![crate::types::basic::Vec3f32 { x: 1.0, y: 2.0, z: 3.0 }].into_iter()), Tokio::new())
}

#[ffi(service)]
pub struct ServiceAsyncStream { runtime: Tokio }

#[ffi]
impl ServiceAsyncStream {
    pub fn create() -> Self { Self { runtime: Tokio::new() } }

    pub fn values(&self, count: u32, ms: u64) -> ffi::AsyncIterator<u32> {
        ffi::AsyncIterator::new(tracked(count, ms, 0), self.runtime.clone())
    }

    pub fn try_values(&self, count: u32) -> ffi::Result<ffi::AsyncIterator<u32>, crate::patterns::result::Error> {
        ffi::Ok(self.values(count, 0))
    }
}
