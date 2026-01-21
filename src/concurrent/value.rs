use core::ops::Deref;

use crate::concurrent::smr;
use crate::concurrent::smr::Guard as _;
use crate::sequential::Value as _;

pub unsafe trait Value<'v>: Sized + crate::sequential::Value<'v> {
    type Guard<G>: smr::Guard<'v, Self>
    where
        G: smr::Guard<'v, Self>;

    unsafe fn own<G: smr::Guard<'v, Self>>(guard: G, raw: u64) -> Owned<'v, G, Self>;

    unsafe fn share<G: smr::Guard<'v, Self>>(guard: G, raw: u64) -> Shared<'v, G, Self>;
}

unsafe impl<'v, T: 'v> Value<'v> for Box<T> {
    type Guard<G>
        = G
    where
        G: smr::Guard<'v, Self>;

    #[inline]
    unsafe fn own<G: smr::Guard<'v, Self>>(guard: G, raw: u64) -> Owned<'v, G, Self> {
        Owned {
            guard,
            value: Self::borrow_from_raw(raw),
        }
    }

    #[inline]
    unsafe fn share<G: smr::Guard<'v, Self>>(guard: G, raw: u64) -> Shared<'v, G, Self> {
        Shared {
            _guard: guard,
            value: Self::borrow_from_raw(raw),
        }
    }
}

macro_rules! impl_trivial {
    ($($ty:ty),*) => {
        $(
            unsafe impl Value<'static> for $ty {
                type Guard<G>
                    = smr::NoOp
                where
                    G: smr::Guard<'static, Self>;

                #[inline]
                unsafe fn own<G: smr::Guard<'static, Self>>(_guard: G, raw: u64) -> Owned<'static, G, Self> {
                    Owned {
                        guard: smr::NoOp,
                        value: Self::borrow_from_raw(raw),
                    }
                }

                #[inline]
                unsafe fn share<G: smr::Guard<'static, Self>>(_guard: G, raw: u64) -> Shared<'static, G, Self> {
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

pub struct Owned<'v, G: smr::Guard<'v, V>, V: Value<'v>> {
    guard: V::Guard<G>,
    value: V::Borrow<'v>,
}

impl<'v, G, V> Deref for Owned<'v, G, V>
where
    G: smr::Guard<'v, V>,
    V: Value<'v>,
{
    type Target = V::Borrow<'v>;

    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

impl<'v, G: smr::Guard<'v, V>, V: Value<'v>> Drop for Owned<'v, G, V> {
    fn drop(&mut self) {
        unsafe { self.guard.retire_value(self.value) }
    }
}

pub struct Shared<'v, G: smr::Guard<'v, V>, V: Value<'v>> {
    _guard: V::Guard<G>,
    value: V::Borrow<'v>,
}

impl<'v, G, V> Deref for Shared<'v, G, V>
where
    G: smr::Guard<'v, V>,
    V: Value<'v>,
{
    type Target = V::Borrow<'v>;

    fn deref(&self) -> &Self::Target {
        &self.value
    }
}
