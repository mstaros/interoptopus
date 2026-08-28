use interoptopus::ffi;
use std::alloc::{GlobalAlloc, Layout, System};
use std::cmp::Ordering;
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

struct CountingAllocator;

static LIVE_BYTES: AtomicUsize = AtomicUsize::new(0);
static LIVE_ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: The request is forwarded unchanged to the system allocator.
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            record_allocation(layout.size());
        }
        ptr
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: The request is forwarded unchanged to the system allocator.
        let ptr = unsafe { System.alloc_zeroed(layout) };
        if !ptr.is_null() {
            record_allocation(layout.size());
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: The pointer and layout are forwarded unchanged to the system allocator.
        unsafe { System.dealloc(ptr, layout) };
        record_deallocation(layout.size());
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: The pointer, original layout, and new size are forwarded unchanged.
        let new_ptr = unsafe { System.realloc(ptr, layout, new_size) };
        if !new_ptr.is_null() {
            record_reallocation(layout.size(), new_size);
        }
        new_ptr
    }
}

fn record_allocation(size: usize) {
    let _ = LIVE_ALLOCATIONS.fetch_add(1, AtomicOrdering::Relaxed);
    let _ = LIVE_BYTES.fetch_add(size, AtomicOrdering::Relaxed);
}

fn record_deallocation(size: usize) {
    let _ = LIVE_ALLOCATIONS.fetch_sub(1, AtomicOrdering::Relaxed);
    let _ = LIVE_BYTES.fetch_sub(size, AtomicOrdering::Relaxed);
}

fn record_reallocation(old_size: usize, new_size: usize) {
    match new_size.cmp(&old_size) {
        Ordering::Less => {
            let _ = LIVE_BYTES.fetch_sub(old_size - new_size, AtomicOrdering::Relaxed);
        }
        Ordering::Equal => {}
        Ordering::Greater => {
            let _ = LIVE_BYTES.fetch_add(new_size - old_size, AtomicOrdering::Relaxed);
        }
    }
}

#[ffi]
pub fn __test_live_bytes() -> u64 {
    u64::try_from(LIVE_BYTES.load(AtomicOrdering::Relaxed)).expect("live byte count exceeds u64")
}

#[ffi]
pub fn __test_live_allocations() -> u64 {
    u64::try_from(LIVE_ALLOCATIONS.load(AtomicOrdering::Relaxed)).expect("live allocation count exceeds u64")
}