//! Owned, single-pass Rust iterators exposed to foreign query APIs.
//!
//! Wrap an owning traversal with `ffi::Iterator::new(iter)`. The iterator must
//! be `Send + 'static`: borrowed collections must first provide an owning
//! traversal (for example one retaining an `Arc` to its source).
//!
//! C# generates consuming `Where`, `Take`, and `Any` extensions. Every stage
//! transfers ownership; the previous wrapper becomes unusable. Evaluation is
//! deferred until `Any`, which short-circuits and releases the entire pipeline.
//! Dispose an unfinished query. This is a single-pass iterator, not a replayable
//! `IEnumerable`. The initial C# projection supports scalars, plain enums, and
//! structs composed of those values. Borrowed and owning elements are rejected.
//!
//! ```
//! use interoptopus::ffi;
//! #[ffi]
//! pub fn numbers() -> ffi::Iterator<u32> {
//!     ffi::Iterator::new(vec![1, 2, 3].into_iter())
//! }
//! ```

use crate::inventory::{Inventory, TypeId};
use crate::lang::meta::{Docs, Visibility, common_or_module_emission};
use crate::lang::types::{Type, TypeInfo, TypeKind, TypePattern, WireIO};
use crate::wire::SerializationError;
use std::ffi::c_void;
use std::io::{Read, Write};
use std::marker::PhantomData;
use std::panic::{AssertUnwindSafe, catch_unwind};

type Predicate = unsafe extern "C" fn(*const c_void, *const c_void) -> i32;
type Release = unsafe extern "C" fn(*const c_void);
type Items<T> = Box<dyn std::iter::Iterator<Item = Result<T, usize>> + Send>;

struct State<T> {
    items: Option<Items<T>>,
}

struct Callback {
    predicate: Predicate,
    context: *const c_void,
    release: Release,
}

// Foreign callbacks must permit invocation and destruction on the consuming thread.
unsafe impl Send for Callback {}

impl Callback {
    fn test<T>(&self, item: &T) -> i32 {
        unsafe { (self.predicate)(std::ptr::from_ref(item).cast(), self.context) }
    }
}

impl Drop for Callback {
    fn drop(&mut self) {
        unsafe { (self.release)(self.context) };
    }
}

/// An owned FFI iterator. Its function pointers are specialized for its element type.
///
/// The ABI is five pointer-sized fields, in the order below. Foreign code must
/// move the descriptor, never copy ownership, and serialize access to its state.
/// Predicates receive a borrowed element pointer valid only during the callback.
/// Return 0 to reject, 1 to accept, or -1 to stop with a managed exception.
/// The error context returned by `any_fn` remains rooted until `drop_fn`.
#[repr(C)]
pub struct Iterator<T> {
    data: *mut c_void,
    where_fn: unsafe extern "C" fn(*mut c_void, Predicate, *const c_void, Release),
    take_fn: unsafe extern "C" fn(*mut c_void, u64),
    any_fn: unsafe extern "C" fn(*mut c_void, *mut *const c_void) -> i32,
    drop_fn: unsafe extern "C" fn(*mut c_void),
    element: PhantomData<T>,
}

// Construction requires an owning Send iterator; no shared mutation is exposed.
unsafe impl<T: Send> Send for Iterator<T> {}

impl<T: Send + 'static> Iterator<T> {
    /// A borrowed traversal cannot escape its source:
    ///
    /// ```compile_fail
    /// use interoptopus::ffi;
    /// let source = vec![1_u32, 2, 3];
    /// let query = ffi::Iterator::new(source.iter().copied());
    /// ```
    /// Retains an owning traversal without enumerating or copying its collection.
    #[must_use]
    pub fn new<I: std::iter::Iterator<Item = T> + Send + 'static>(iter: I) -> Self {
        let state = State { items: Some(Box::new(iter.map(Ok)) as Items<T>) };
        Self {
            data: Box::into_raw(Box::new(state)).cast(),
            where_fn: filter::<T>,
            take_fn: take::<T>,
            any_fn: any::<T>,
            drop_fn: destroy::<T>,
            element: PhantomData,
        }
    }
}

unsafe extern "C" fn filter<T: Send + 'static>(data: *mut c_void, predicate: Predicate, context: *const c_void, release: Release) {
    let state = unsafe { &mut *data.cast::<State<T>>() };
    let callback = Callback { predicate, context, release };
    let source = state.items.take().expect("owned iterator state");
    state.items = Some(Box::new(source.filter_map(move |item| match item {
        Err(error) => Some(Err(error)),
        Ok(item) => match callback.test(&item) {
            0 => None,
            1 => Some(Ok(item)),
            _ => Some(Err(callback.context as usize)),
        },
    })));
}

unsafe extern "C" fn take<T: Send + 'static>(data: *mut c_void, count: u64) {
    let state = unsafe { &mut *data.cast::<State<T>>() };
    let source = state.items.take().expect("owned iterator state");
    // C# exposes an int count, hence this conversion fits on every supported target.
    state.items = Some(Box::new(source.take(usize::try_from(count).unwrap_or(usize::MAX))));
}

unsafe extern "C" fn any<T>(data: *mut c_void, error: *mut *const c_void) -> i32 {
    unsafe { error.write(std::ptr::null()) };
    match catch_unwind(AssertUnwindSafe(|| {
        let state = unsafe { &mut *data.cast::<State<T>>() };
        state.items.as_mut().expect("owned iterator state").next().map(|item| item.map(|_| ()))
    })) {
        Ok(None) => 0,
        Ok(Some(Ok(()))) => 1,
        Ok(Some(Err(context))) => {
            unsafe { error.write(context as *const c_void) };
            -1
        }
        Err(_) => -2,
    }
}

unsafe extern "C" fn destroy<T>(data: *mut c_void) {
    // User iterator destructors must never unwind through the foreign caller.
    let _ = catch_unwind(AssertUnwindSafe(|| {
        drop(unsafe { Box::from_raw(data.cast::<State<T>>()) });
    }));
}

impl<T> Drop for Iterator<T> {
    fn drop(&mut self) {
        if !self.data.is_null() {
            unsafe { (self.drop_fn)(self.data) };
        }
    }
}

unsafe impl<T: TypeInfo> TypeInfo for Iterator<T> {
    const WIRE_SAFE: bool = false;
    const RAW_SAFE: bool = T::RAW_SAFE;
    const ASYNC_SAFE: bool = false;
    const SERVICE_SAFE: bool = false;
    const SERVICE_CTOR_SAFE: bool = false;

    fn id() -> TypeId {
        TypeId::new(0x6149260D64AC4B71B6D0AD613F9DF28C).derive_id(T::id())
    }

    fn kind() -> TypeKind {
        TypeKind::TypePattern(TypePattern::Iterator(T::id()))
    }

    fn ty() -> Type {
        let item = T::ty();
        Type {
            name: format!("Iterator<{}>", item.name),
            visibility: Visibility::Public,
            docs: Docs::from_line("Owned, single-pass Rust iterator."),
            emission: common_or_module_emission(&[item.emission]),
            kind: Self::kind(),
        }
    }

    fn register(inventory: &mut impl Inventory) {
        T::register(inventory);
        <*mut c_void>::register(inventory);
        inventory.register_type(Self::id(), Self::ty());
    }
}

unsafe impl<T: TypeInfo> WireIO for Iterator<T> {
    fn write(&self, _: &mut impl Write) -> Result<(), SerializationError> { crate::bad_wire!() }
    fn read(_: &mut impl Read) -> Result<Self, SerializationError> { crate::bad_wire!() }
    fn live_size(&self) -> usize { crate::bad_wire!() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};

    #[test]
    fn construction_and_take_zero_are_deferred() {
        let visits = Arc::new(AtomicUsize::new(0));
        let observed = Arc::clone(&visits);
        let iter = Iterator::new((0_u32..10).inspect(move |_| { observed.fetch_add(1, Ordering::SeqCst); }));
        assert_eq!(visits.load(Ordering::SeqCst), 0);
        let mut error = std::ptr::null();
        unsafe {
            (iter.take_fn)(iter.data, 0);
            assert_eq!((iter.any_fn)(iter.data, &mut error), 0);
        }
        assert_eq!(visits.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn callback_failure_stops_and_releases_once() {
        unsafe extern "C" fn reject(_: *const c_void, _: *const c_void) -> i32 { -1 }
        unsafe extern "C" fn release(context: *const c_void) {
            unsafe { &*context.cast::<AtomicUsize>() }.fetch_add(1, Ordering::SeqCst);
        }
        let releases = AtomicUsize::new(0);
        let visits = Arc::new(AtomicUsize::new(0));
        let observed = Arc::clone(&visits);
        let iter = Iterator::new((0_u32..10).inspect(move |_| { observed.fetch_add(1, Ordering::SeqCst); }));
        let context = std::ptr::from_ref(&releases).cast();
        let mut error = std::ptr::null();
        unsafe {
            (iter.where_fn)(iter.data, reject, context, release);
            assert_eq!((iter.any_fn)(iter.data, &mut error), -1);
        }
        assert_eq!(error, context);
        assert_eq!(visits.load(Ordering::SeqCst), 1);
        assert_eq!(releases.load(Ordering::SeqCst), 0);
        drop(iter);
        assert_eq!(releases.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn yielded_item_destructor_panic_does_not_cross_abi() {
        struct PanicOnDrop;
        impl Drop for PanicOnDrop {
            fn drop(&mut self) { panic!("item destructor panic"); }
        }
        let iter = Iterator::new(std::iter::once(PanicOnDrop));
        let mut error = std::ptr::null();
        assert_eq!(unsafe { (iter.any_fn)(iter.data, &mut error) }, -2);
        assert!(error.is_null());
    }

    #[test]
    fn native_panic_does_not_cross_abi() {
        let iter = Iterator::new(std::iter::from_fn(|| -> Option<u32> { panic!("iterator panic") }));
        let mut error = std::ptr::null();
        assert_eq!(unsafe { (iter.any_fn)(iter.data, &mut error) }, -2);
        assert!(error.is_null());
    }
}
