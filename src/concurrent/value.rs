use core::ops::Deref;

use crate::concurrent::smr;
use crate::concurrent::smr::Guard as _;
use crate::sequential::Value as _;

pub unsafe trait Value: Sized + crate::sequential::Value {
    type Guard<G>: smr::Guard<Self>
    where
        G: smr::Guard<Self>;

    unsafe fn own<'l, G: smr::Guard<Self>>(guard: G, raw: u64) -> Owned<'l, G, Self>;

    unsafe fn share<'l, G: smr::Guard<Self>>(guard: G, raw: u64) -> Shared<'l, G, Self>;
}

unsafe impl<T> Value for Box<T> {
    type Guard<G>
        = G
    where
        G: smr::Guard<Self>;

    #[inline]
    unsafe fn own<'l, G: smr::Guard<Self>>(guard: G, raw: u64) -> Owned<'l, G, Self> {
        Owned {
            guard,
            value: Self::borrow_from_raw(raw),
        }
    }

    #[inline]
    unsafe fn share<'l, G: smr::Guard<Self>>(guard: G, raw: u64) -> Shared<'l, G, Self> {
        Shared {
            _guard: guard,
            value: Self::borrow_from_raw(raw),
        }
    }
}

unsafe impl<'v, T: 'v + Sized> Value for &'v T {
    type Guard<G>
        = smr::NoOp
    where
        G: smr::Guard<Self>;

    #[inline]
    unsafe fn own<'l, G: smr::Guard<Self>>(_guard: G, raw: u64) -> Owned<'l, G, Self> {
        Owned {
            guard: smr::NoOp,
            value: Self::borrow_from_raw(raw),
        }
    }

    #[inline]
    unsafe fn share<'l, G: smr::Guard<Self>>(_guard: G, raw: u64) -> Shared<'l, G, Self> {
        Shared {
            _guard: smr::NoOp,
            value: Self::borrow_from_raw(raw),
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
                unsafe fn own<'l, G: smr::Guard<Self>>(_guard: G, raw: u64) -> Owned<'l, G, Self> {
                    Owned {
                        guard: smr::NoOp,
                        value: Self::borrow_from_raw(raw),
                    }
                }

                #[inline]
                unsafe fn share<'l, G: smr::Guard<Self>>(_guard: G, raw: u64) -> Shared<'l, G, Self> {
                    Shared {
                        _guard: smr::NoOp,
                        value: Self::borrow_from_raw(raw),
                    }
                }
            }
        )*
    };
}

impl_trivial!(u64, u32);

pub struct Owned<'l, G: smr::Guard<V>, V: Value + 'l> {
    guard: V::Guard<G>,
    value: V::Borrow<'l>,
}

impl<'l, G, V> Deref for Owned<'l, G, V>
where
    G: smr::Guard<V>,
    V: Value,
{
    type Target = V::Borrow<'l>;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

impl<'l, G: smr::Guard<V>, V: Value> Drop for Owned<'l, G, V> {
    #[inline]
    fn drop(&mut self) {
        unsafe { self.guard.retire_value(V::borrow_into_raw(self.value)) }
    }
}

pub struct Shared<'l, G: smr::Guard<V>, V: Value + 'l> {
    _guard: V::Guard<G>,
    value: V::Borrow<'l>,
}

impl<'l, G, V> Deref for Shared<'l, G, V>
where
    G: smr::Guard<V>,
    V: Value + 'l,
{
    type Target = V::Borrow<'l>;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.value
    }
}
