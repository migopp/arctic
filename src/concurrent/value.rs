use crate::concurrent::hazard;
use crate::sequential::Value as _;

pub unsafe trait Value: Sized + crate::sequential::Value {
    type OwnedGuard<'g, 'l, P>: Sized
    where
        Self: 'g,
        P: 'g + ribbit::Pack<Packed: hazard::Prefix>,
        'g: 'l;

    type SharedGuard<'g, 'l, P>: Sized
    where
        Self: 'g,
        P: 'g + ribbit::Pack<Packed: hazard::Prefix>,
        'g: 'l;

    unsafe fn guard_borrow<'g, 'l, P: ribbit::Pack<Packed: hazard::Prefix>>(
        smr: &'l hazard::guard::Traverse<'g, 'l, P, Self>,
        raw: u64,
    ) -> Self::Borrow<'l>;

    unsafe fn guard_owned<'g, 'l, P: ribbit::Pack<Packed: hazard::Prefix>>(
        smr: hazard::guard::Traverse<'g, 'l, P, Self>,
        raw: u64,
    ) -> Self::OwnedGuard<'g, 'l, P>;

    unsafe fn guard_shared<'g, 'l, P: ribbit::Pack<Packed: hazard::Prefix>>(
        smr: hazard::guard::Traverse<'g, 'l, P, Self>,
        raw: u64,
    ) -> Self::SharedGuard<'g, 'l, P>;

    fn borrow_into_raw<'l>(borrow: Self::Borrow<'l>) -> u64
    where
        Self: 'l;

    fn borrow_owned<'g, 'l, 'a, P: ribbit::Pack<Packed: hazard::Prefix>>(
        guard: &'a Self::OwnedGuard<'g, 'l, P>,
    ) -> Self::Borrow<'a>;

    fn borrow_shared<'g, 'l, 'a, P: ribbit::Pack<Packed: hazard::Prefix>>(
        guard: &'a Self::SharedGuard<'g, 'l, P>,
    ) -> Self::Borrow<'a>;
}

unsafe impl<T> Value for Box<T> {
    type OwnedGuard<'g, 'l, P>
        = hazard::guard::Value<'g, 'l, true, P, Self>
    where
        Self: 'g + 'l,
        P: 'g + ribbit::Pack<Packed: hazard::Prefix>,
        'g: 'l;

    type SharedGuard<'g, 'l, P>
        = hazard::guard::Value<'g, 'l, false, P, Self>
    where
        Self: 'g + 'l,
        P: 'g + ribbit::Pack<Packed: hazard::Prefix>,
        'g: 'l;

    #[inline]
    unsafe fn guard_owned<'g, 'l, P: ribbit::Pack<Packed: hazard::Prefix>>(
        smr: hazard::guard::Traverse<'g, 'l, P, Self>,
        raw: u64,
    ) -> Self::OwnedGuard<'g, 'l, P> {
        let borrow = Self::borrow_from_raw(raw);
        unsafe { smr.guard_owned(borrow) }
    }

    #[inline]
    unsafe fn guard_shared<'g, 'l, P: ribbit::Pack<Packed: hazard::Prefix>>(
        smr: hazard::guard::Traverse<'g, 'l, P, Self>,
        raw: u64,
    ) -> Self::SharedGuard<'g, 'l, P> {
        let borrow = Self::borrow_from_raw(raw);
        smr.guard_shared(borrow)
    }

    #[inline]
    unsafe fn guard_borrow<'g, 'l, P: ribbit::Pack<Packed: hazard::Prefix>>(
        _smr: &'l hazard::guard::Traverse<'g, 'l, P, Self>,
        raw: u64,
    ) -> Self::Borrow<'l> {
        Self::borrow_from_raw(raw)
    }

    #[inline]
    fn borrow_into_raw<'l>(borrow: Self::Borrow<'l>) -> u64
    where
        Self: 'l,
    {
        borrow as *const T as u64
    }

    #[inline]
    fn borrow_owned<'g, 'l, 'a, P: ribbit::Pack<Packed: hazard::Prefix>>(
        guard: &'a Self::OwnedGuard<'g, 'l, P>,
    ) -> Self::Borrow<'a> {
        guard
    }

    #[inline]
    fn borrow_shared<'g, 'l, 'a, P: ribbit::Pack<Packed: hazard::Prefix>>(
        guard: &'a Self::SharedGuard<'g, 'l, P>,
    ) -> Self::Borrow<'a> {
        guard
    }
}

macro_rules! impl_trivial {
    ($($ty:ty),*) => {
        $(
            unsafe impl Value for $ty {
                type OwnedGuard<'g, 'l, P>
                    = Self
                where
                    P: 'g + ribbit::Pack<Packed: hazard::Prefix>,
                    'g: 'l;

                type SharedGuard<'g, 'l, P>
                    = Self
                where
                    P: 'g + ribbit::Pack<Packed: hazard::Prefix>,
                    'g: 'l;

                #[inline]
                unsafe fn guard_owned<'g, 'l, P>(_smr: hazard::guard::Traverse<'g, 'l, P, Self>, raw: u64) -> Self
                where
                    P: ribbit::Pack<Packed: hazard::Prefix>,
                {
                    raw as $ty
                }

                #[inline]
                unsafe fn guard_shared<'g, 'l, P>(_smr: hazard::guard::Traverse<'g, 'l, P, Self>, raw: u64) -> Self
                where
                    P: ribbit::Pack<Packed: hazard::Prefix>,
                {
                    raw as $ty
                }

                #[inline]
                unsafe fn guard_borrow<'g, 'l, P>(
                    _smr: &'l hazard::guard::Traverse<'g, 'l, P, Self>,
                    raw: u64,
                ) -> Self::Borrow<'l>
                where
                    P: ribbit::Pack<Packed: hazard::Prefix>,
                {
                    raw as $ty
                }

                #[inline]
                fn borrow_into_raw<'l>(borrow: Self::Borrow<'l>) -> u64 where Self: 'l {
                    borrow as u64
                }

                #[inline]
                fn borrow_owned<'g, 'l, 'a, P: ribbit::Pack<Packed: hazard::Prefix>>(
                    guard: &'a Self,
                ) -> Self::Borrow<'a> {
                    *guard
                }

                #[inline]
                fn borrow_shared<'g, 'l, 'a, P: ribbit::Pack<Packed: hazard::Prefix>>(
                    guard: &'a Self,
                ) -> Self::Borrow<'a> {
                    *guard
                }
            }
        )*
    };
}

impl_trivial!(u64, u32);
