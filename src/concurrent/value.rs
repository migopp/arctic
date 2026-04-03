use core::ops::Deref;

use crate::concurrent::smr;
use crate::concurrent::smr::Guard as _;

pub unsafe trait Value: Sized + crate::sequential::Value {
    type Guard<G>: smr::Guard<Self>
    where
        G: smr::Guard<Self>;

    unsafe fn own<G: smr::Guard<Self>>(guard: G, raw: u64) -> Owned<G, Self>;

    unsafe fn share<G: smr::Guard<Self>>(guard: G, raw: u64) -> Shared<G, Self>;
}

unsafe impl<T> Value for Box<T> {
    type Guard<G>
        = G
    where
        G: smr::Guard<Self>;

    #[inline]
    unsafe fn own<G: smr::Guard<Self>>(guard: G, raw: u64) -> Owned<G, Self> {
        Owned { guard, raw }
    }

    #[inline]
    unsafe fn share<G: smr::Guard<Self>>(guard: G, raw: u64) -> Shared<G, Self> {
        Shared { _guard: guard, raw }
    }
}

unsafe impl<'v, T: 'v + Sized> Value for &'v T {
    type Guard<G>
        = smr::NoOp
    where
        G: smr::Guard<Self>;

    #[inline]
    unsafe fn own<G: smr::Guard<Self>>(_guard: G, raw: u64) -> Owned<G, Self> {
        Owned {
            guard: smr::NoOp,
            raw,
        }
    }

    #[inline]
    unsafe fn share<G: smr::Guard<Self>>(_guard: G, raw: u64) -> Shared<G, Self> {
        Shared {
            _guard: smr::NoOp,
            raw,
        }
    }
}

macro_rules! impl_trivial {
    ($($ty:ty),*) => {
        $(
            unsafe impl Value for $ty {
                type Guard<G>
                    = smr::NoOp
                where
                    G: smr::Guard<Self>;

                #[inline]
                unsafe fn own<G: smr::Guard<Self>>(_guard: G, raw: u64) -> Owned<G, Self> {
                    Owned {
                        guard: smr::NoOp,
                        raw,
                    }
                }

                #[inline]
                unsafe fn share<G: smr::Guard<Self>>(_guard: G, raw: u64) -> Shared<G, Self> {
                    Shared {
                        _guard: smr::NoOp,
                        raw,
                    }
                }
            }
        )*
    };
}

impl_trivial!(u64, u32);

pub struct Owned<G: smr::Guard<V>, V: Value> {
    guard: V::Guard<G>,
    raw: u64,
}

impl<G, V> Deref for Owned<G, V>
where
    G: smr::Guard<V>,
    V: Value,
{
    type Target = V::Target;

    #[inline]
    fn deref(&self) -> &Self::Target {
        unsafe { V::target_from_raw(&self.raw) }
    }
}

impl<G: smr::Guard<V>, V: Value> Drop for Owned<G, V> {
    #[inline]
    fn drop(&mut self) {
        unsafe { self.guard.retire_value(self.raw) }
    }
}

pub struct Shared<G: smr::Guard<V>, V: Value> {
    _guard: V::Guard<G>,
    raw: u64,
}

impl<G, V> Deref for Shared<G, V>
where
    G: smr::Guard<V>,
    V: Value,
{
    type Target = V::Target;

    #[inline]
    fn deref(&self) -> &Self::Target {
        unsafe { V::target_from_raw(&self.raw) }
    }
}
