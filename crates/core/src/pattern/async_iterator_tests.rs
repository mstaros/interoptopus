use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::Waker;

type Log = Mutex<Vec<(i32, Option<u32>)>>;

unsafe extern "C" fn record(context: *const c_void, status: i32, item: *const c_void) {
    let log = unsafe { &*context.cast::<Log>() };
    let value = if status == 1 { Some(unsafe { *item.cast::<u32>() }) } else { None };
    log.lock().unwrap().push((status, value));
}

struct Probe {
    values: std::vec::IntoIter<u32>,
    pending: bool,
    drops: Arc<AtomicUsize>,
}

impl Stream for Probe {
    type Item = u32;
    fn poll_next(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Option<u32>> {
        if self.pending { Poll::Pending } else { Poll::Ready(self.values.next()) }
    }
}

impl Drop for Probe {
    fn drop(&mut self) { self.drops.fetch_add(1, Ordering::SeqCst); }
}

enum Immediate {
    Poll,
    Discard,
    Panic,
}

impl AsyncRuntime for Immediate {
    type T = ();
    fn spawn<Fn, F>(&self, f: Fn) -> TaskHandle
    where
        Fn: FnOnce(()) -> F + Send + 'static,
        F: Future<Output = ()> + Send + 'static,
    {
        match self {
            Self::Poll => {
                let mut future = Box::pin(f(()));
                assert!(future.as_mut().poll(&mut Context::from_waker(Waker::noop())).is_ready());
            }
            Self::Discard => drop(f),
            Self::Panic => panic!("executor rejected spawn"),
        }
        TaskHandle::dummy()
    }
}

type Pending = Arc<Mutex<Option<Pin<Box<dyn Future<Output = ()> + Send>>>>>;

#[derive(Clone)]
struct Queued(Pending);

impl AsyncRuntime for Queued {
    type T = ();
    fn spawn<Fn, F>(&self, f: Fn) -> TaskHandle
    where
        Fn: FnOnce(()) -> F + Send + 'static,
        F: Future<Output = ()> + Send + 'static,
    {
        *self.0.lock().unwrap() = Some(Box::pin(f(())));
        TaskHandle::from_handle(Arc::clone(&self.0), |pending| {
            let future = pending.lock().unwrap().take();
            drop(future);
        })
    }
}

fn probe(pending: bool, drops: &Arc<AtomicUsize>) -> Probe {
    Probe { values: vec![7, 8].into_iter(), pending, drops: Arc::clone(drops) }
}

fn advance(source: &AsyncIterator<u32>, log: &Log) -> TaskHandle {
    unsafe { (source.next_fn)(source.data, record, std::ptr::from_ref(log).cast()) }
}

#[test]
fn inline_completion_copies_items_and_fuses_end() {
    let drops = Arc::new(AtomicUsize::new(0));
    let source = AsyncIterator::new(probe(false, &drops), Immediate::Poll);
    let log = Log::default();
    for _ in 0..4 { drop(advance(&source, &log)); }
    assert_eq!(*log.lock().unwrap(), vec![(1, Some(7)), (1, Some(8)), (0, None), (0, None)]);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    drop(source);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

#[test]
fn executor_discard_before_first_poll_acknowledges_cancellation() {
    let drops = Arc::new(AtomicUsize::new(0));
    let source = AsyncIterator::new(probe(true, &drops), Immediate::Discard);
    let log = Log::default();
    drop(advance(&source, &log));
    assert_eq!(*log.lock().unwrap(), vec![(-2, None)]);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

#[test]
fn executor_spawn_panic_completes_once_after_cleanup() {
    let drops = Arc::new(AtomicUsize::new(0));
    let source = AsyncIterator::new(probe(true, &drops), Immediate::Panic);
    let log = Log::default();
    drop(advance(&source, &log));
    assert_eq!(*log.lock().unwrap(), vec![(-1, None)]);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

#[test]
fn pending_request_retains_state_until_abort_acknowledges_cleanup() {
    let drops = Arc::new(AtomicUsize::new(0));
    let pending = Pending::default();
    let source = AsyncIterator::new(probe(true, &drops), Queued(Arc::clone(&pending)));
    let log = Log::default();
    let task = advance(&source, &log);
    assert!(pending.lock().unwrap().as_mut().unwrap().as_mut().poll(&mut Context::from_waker(Waker::noop())).is_pending());
    assert!(log.lock().unwrap().is_empty());
    drop(source);
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    task.abort();
    task.abort();
    assert_eq!(*log.lock().unwrap(), vec![(-2, None)]);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert!(pending.lock().unwrap().is_none());
}
