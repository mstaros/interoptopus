use crate::patterns::result::Error;
use crate::types::basic::Vec3f32;
use interoptopus::pattern::asynk::Async;
use interoptopus::rt::Tokio;
use interoptopus::{AsyncRuntime, callback, ffi};
use std::pin::Pin;
use std::task::{Context, Poll};

#[ffi]
#[derive(Clone, Copy)]
pub struct TuplePair(pub u32, pub f32);

#[ffi]
#[derive(Clone, Copy)]
pub struct TupleFlags(pub ffi::Bool, pub ffi::Bool);

#[ffi]
#[derive(Clone, Copy)]
pub struct TupleNested(pub TuplePair, pub TupleFlags, pub Vec3f32);

#[ffi]
#[derive(Clone, Copy)]
pub struct TupleAligned(pub u8, pub u64);

#[ffi]
#[derive(Clone, Copy)]
pub struct TupleEight(pub u8, pub u16, pub u32, pub u64, pub i8, pub i16, pub i32, pub i64);

#[ffi]
#[derive(Clone, Copy)]
pub struct TupleId(pub u64);

#[ffi]
pub struct TupleOwned(pub ffi::String, pub u32);

#[ffi]
pub struct TupleBorrowed<'a>(pub ffi::Slice<'a, u8>, pub u32);

#[ffi]
#[derive(Clone, Copy)]
pub struct TupleContainer {
    pub first: TuplePair,
    pub second: TuplePair,
}

callback!(TupleTransform(x: TuplePair) -> TuplePair);
callback!(TupleVisit(x: TuplePair));

#[ffi]
pub fn tuple_echo(x: TuplePair) -> TuplePair { x }

#[ffi]
pub fn tuple_bare_apply(callback: extern "C" fn(TupleNested) -> TupleNested, x: TupleNested) -> TupleNested { callback(x) }

#[ffi]
pub fn tuple_nested_echo(x: TupleNested) -> TupleNested { x }

#[ffi]
pub fn tuple_aligned_echo(x: TupleAligned) -> TupleAligned { x }

#[ffi]
pub fn tuple_eight_echo(x: TupleEight) -> TupleEight { x }

#[ffi]
pub fn tuple_id_echo(x: TupleId) -> TupleId { x }

#[ffi]
pub fn tuple_owned_echo(x: TupleOwned) -> TupleOwned { x }

#[ffi]
pub fn tuple_container_echo(x: TupleContainer) -> TupleContainer { x }

#[ffi]
pub fn tuple_borrowed_sum(x: TupleBorrowed<'_>) -> u32 {
    x.0.as_slice().iter().map(|x| u32::from(*x)).sum::<u32>() + x.1
}

#[ffi]
pub fn tuple_apply(callback: TupleTransform, x: TuplePair) -> TuplePair { callback.call(x) }

#[ffi]
pub fn tuple_visit(callback: TupleVisit, x: TuplePair) { callback.call(x); }

#[ffi]
pub fn tuple_native_callback() -> TupleTransform {
    TupleTransform::from_fn(|x| TuplePair(x.0 + 1, x.1 + 0.5))
}

#[ffi]
pub fn tuple_borrow(x: &TuplePair) -> TuplePair { *x }

#[ffi]
pub fn tuple_slice_sum(values: ffi::Slice<'_, TuplePair>) -> u32 {
    values.as_slice().iter().map(|x| x.0).sum()
}

#[ffi]
#[derive(Clone, Copy)]
pub enum TupleChoice {
    Left(TuplePair),
    Right(TuplePair),
    Empty,
}

#[ffi]
pub fn tuple_choice_echo(x: TupleChoice) -> TupleChoice { x }

#[ffi]
pub fn tuple_option_echo(x: ffi::Option<TuplePair>) -> ffi::Option<TuplePair> { x }

#[ffi]
pub fn tuple_result_echo(x: ffi::Result<TuplePair, Error>) -> ffi::Result<TuplePair, Error> { x }

struct Pairs(std::ops::Range<u16>);

impl futures_core::Stream for Pairs {
    type Item = TuplePair;
    fn poll_next(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Option<TuplePair>> {
        Poll::Ready(self.0.next().map(|x| TuplePair(u32::from(x), f32::from(x))))
    }
}

#[ffi(service)]
#[derive(AsyncRuntime)]
pub struct ServiceTuple { runtime: Tokio }

#[ffi]
impl ServiceTuple {
    pub fn create() -> Self { Self { runtime: Tokio::new() } }

    pub fn pair(&self, x: TuplePair) -> TuplePair { x }

    pub fn try_pair(&self, x: TuplePair) -> ffi::Result<TuplePair, Error> { ffi::Ok(x) }

    pub fn apply(&self, callback: TupleTransform, x: TuplePair) -> TuplePair { callback.call(x) }

    pub async fn pair_later(_: Async<Self>, x: TuplePair) -> TuplePair {
        tokio::task::yield_now().await;
        x
    }

    pub async fn try_pair_later(_: Async<Self>, x: TuplePair) -> ffi::Result<TuplePair, Error> {
        tokio::task::yield_now().await;
        ffi::Ok(x)
    }

    pub fn pairs(&self) -> ffi::Iterator<TuplePair> {
        ffi::Iterator::new((0_u16..4).map(|x| TuplePair(u32::from(x), f32::from(x))))
    }

    pub fn pair_stream(&self) -> ffi::AsyncIterator<TuplePair> {
        ffi::AsyncIterator::new(Pairs(0..4), self.runtime.clone())
    }
}
