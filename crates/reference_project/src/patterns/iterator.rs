use interoptopus::ffi;
use std::sync::atomic::{AtomicU32, Ordering};


#[ffi(service)]
pub struct ServiceIterator {
    count: u32,
}

#[ffi]
impl ServiceIterator {
    pub fn create(count: u32) -> Self {
        Self { count }
    }

    pub fn values(&self) -> ffi::Iterator<u32> {
        tracked(self.count, false)
    }

    pub fn try_values(&self) -> ffi::Result<ffi::Iterator<u32>, crate::patterns::result::Error> {
        ffi::Result::Ok(tracked(self.count, false))
    }

    pub fn bools(&self) -> ffi::Iterator<ffi::Bool> {
        ffi::Iterator::new((0..self.count).map(|x| ffi::Bool::from(x != 0)))
    }
}

static LIVE: AtomicU32 = AtomicU32::new(0);
static VISITS: AtomicU32 = AtomicU32::new(0);

struct Tracked {
    items: std::vec::IntoIter<u32>,
    panic_on_next: bool,
}

impl Iterator for Tracked {
    type Item = u32;
    fn next(&mut self) -> Option<Self::Item> {
        VISITS.fetch_add(1, Ordering::SeqCst);
        assert!(!self.panic_on_next, "iterator fixture panic");
        self.items.next()
    }
}

impl Drop for Tracked {
    fn drop(&mut self) { LIVE.fetch_sub(1, Ordering::SeqCst); }
}

fn tracked(count: u32, panic_on_next: bool) -> ffi::Iterator<u32> {
    LIVE.fetch_add(1, Ordering::SeqCst);
    ffi::Iterator::new(Tracked { items: (0..count).collect::<Vec<_>>().into_iter(), panic_on_next })
}

#[ffi]
pub fn pattern_iterator_create(count: u32) -> ffi::Iterator<u32> { tracked(count, false) }

#[ffi]
pub fn pattern_iterator_panic() -> ffi::Iterator<u32> { tracked(1, true) }

#[ffi]
pub fn pattern_iterator_live() -> u32 { LIVE.load(Ordering::SeqCst) }

#[ffi]
pub fn pattern_iterator_visits() -> u32 { VISITS.load(Ordering::SeqCst) }

#[ffi]
pub fn pattern_iterator_echo(iter: ffi::Iterator<u32>) -> ffi::Iterator<u32> { iter }

#[ffi]
pub fn pattern_iterator_points() -> ffi::Iterator<crate::types::basic::Vec3f32> {
    ffi::Iterator::new(vec![crate::types::basic::Vec3f32 { x: 1.0, y: 2.0, z: 3.0 }].into_iter())
}

#[ffi]
pub fn pattern_iterator_bools() -> ffi::Iterator<ffi::Bool> {
    ffi::Iterator::new([ffi::Bool::from(false), ffi::Bool::from(true)].into_iter())
}
