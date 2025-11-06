use crate::concurrent::hazard;
use crate::sequential::Value as _;

pub unsafe trait Value: Sized + crate::sequential::Value {
    type OwnedGuard<'g, 'l>: Sized
    where
        Self: 'g + 'l,
        'g: 'l;

    type SharedGuard<'g, 'l>: Sized
    where
        Self: 'g + 'l,
        'g: 'l;

    type LinearizableGuard<'g, 'l>: Sized
    where
        Self: 'g + 'l,
        'g: 'l;

    unsafe fn guard_borrow<'g, 'l>(
        smr: &'l hazard::TraverseGuard<'g, 'l, Self>,
        raw: u64,
    ) -> Self::Borrow<'l>;

    unsafe fn guard_owned<'g, 'l>(
        smr: hazard::TraverseGuard<'g, 'l, Self>,
        raw: u64,
    ) -> Self::OwnedGuard<'g, 'l>;

    unsafe fn guard_shared<'g, 'l>(
        smr: hazard::TraverseGuard<'g, 'l, Self>,
        raw: u64,
    ) -> Self::SharedGuard<'g, 'l>;

    unsafe fn downgrade_guard<'g, 'l>(
        smr: hazard::TraverseGuard<'g, 'l, Self>,
    ) -> Self::LinearizableGuard<'g, 'l>;

    unsafe fn guard_linearizable<'g, 'l>(
        smr: &Self::LinearizableGuard<'g, 'l>,
        raw: u64,
    ) -> Self::Borrow<'l>;

    fn borrow_into_raw<'l>(borrow: Self::Borrow<'l>) -> u64
    where
        Self: 'l;
}

unsafe impl<T> Value for Box<T> {
    type OwnedGuard<'g, 'l>
        = hazard::ValueGuard<'g, 'l, true, Self>
    where
        Self: 'g + 'l,
        'g: 'l;

    type SharedGuard<'g, 'l>
        = hazard::ValueGuard<'g, 'l, false, Self>
    where
        Self: 'g + 'l,
        'g: 'l;

    type LinearizableGuard<'g, 'l>
        = hazard::LinearizableGuard<'g, 'l, Self>
    where
        Self: 'g + 'l,
        'g: 'l;

    #[inline]
    unsafe fn guard_owned<'g, 'l>(
        smr: hazard::TraverseGuard<'g, 'l, Self>,
        raw: u64,
    ) -> Self::OwnedGuard<'g, 'l> {
        let borrow = Self::borrow_from_raw(raw);
        unsafe { smr.guard_owned(borrow) }
    }

    #[inline]
    unsafe fn guard_shared<'g, 'l>(
        smr: hazard::TraverseGuard<'g, 'l, Self>,
        raw: u64,
    ) -> Self::SharedGuard<'g, 'l> {
        let borrow = Self::borrow_from_raw(raw);
        smr.guard_shared(borrow)
    }

    #[inline]
    unsafe fn guard_borrow<'g, 'l>(
        _smr: &'l hazard::TraverseGuard<'g, 'l, Self>,
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

    unsafe fn downgrade_guard<'g, 'l>(
        smr: hazard::TraverseGuard<'g, 'l, Self>,
    ) -> Self::LinearizableGuard<'g, 'l> {
        todo!()
    }

    unsafe fn guard_linearizable<'g, 'l>(
        smr: &Self::LinearizableGuard<'g, 'l>,
        raw: u64,
    ) -> Self::Borrow<'l> {
        todo!()
    }
}

// #[derive(Copy, Clone)]
// pub struct Inline<T>(pub T);
//
// unsafe impl<T> Value for Inline<T>
// where
//     T: Copy + From<u64> + Into<u64>,
// {
//     type SelectDrop = postorder::SelectNode;
//
//     type OwnedGuard<'g, 'l>
//         = Self
//     where
//         Self: 'g + 'l,
//         'g: 'l;
//
//     type SharedGuard<'g, 'l>
//         = Self
//     where
//         Self: 'g + 'l,
//         'g: 'l;
//
//     type Borrow<'l>
//         = Self
//     where
//         Self: 'l;
//
//     type Target = Self;
//
//     type Clone = Self;
//
//     #[inline]
//     unsafe fn from_u64(value: u64) -> Self {
//         Inline(T::from(value))
//     }
//
//     #[inline]
//     fn into_u64(self) -> u64 {
//         self.0.into()
//     }
//
//     #[inline]
//     unsafe fn borrow_from_u64<'g, 'l>(
//         _smr: &hazard::TraverseGuard<'g, 'l, Self>,
//         value: u64,
//     ) -> Self::Borrow<'l> {
//         Self(T::from(value))
//     }
//
//     #[inline]
//     fn borrow_into_u64<'l>(borrow: Self::Borrow<'l>) -> u64
//     where
//         Self: 'l,
//     {
//         borrow.0.into()
//     }
// }

macro_rules! impl_trivial {
    ($($ty:ty),*) => {
        $(
            unsafe impl Value for $ty {
                type OwnedGuard<'g, 'l>
                    = Self
                where
                    'g: 'l;

                type SharedGuard<'g, 'l>
                    = Self
                where
                    'g: 'l;

                type LinearizableGuard<'g, 'l>
                    = ()
                where
                    'g: 'l;

                #[inline]
                unsafe fn guard_owned<'g, 'l>(_smr: hazard::TraverseGuard<'g, 'l, Self>, raw: u64) -> Self {
                    raw as $ty
                }

                #[inline]
                unsafe fn guard_shared<'g, 'l>(_smr: hazard::TraverseGuard<'g, 'l, Self>, raw: u64) -> Self {
                    raw as $ty
                }

                #[inline]
                unsafe fn guard_borrow<'g, 'l>(
                    _smr: &hazard::TraverseGuard<'g, 'l, Self>,
                    raw: u64,
                ) -> Self::Borrow<'l> {
                    raw as $ty
                }

                #[inline]
                fn borrow_into_raw<'l>(borrow: Self::Borrow<'l>) -> u64 where Self: 'l {
                    borrow as u64
                }

                unsafe fn downgrade_guard<'g, 'l>(
                    _smr: hazard::TraverseGuard<'g, 'l, Self>,
                ) -> Self::LinearizableGuard<'g, 'l> {
                }

                unsafe fn guard_linearizable<'g, 'l>(
                    (): &(),
                    raw: u64,
                ) -> Self::Borrow<'l> {
                    raw as $ty
                }
            }
        )*
    };
}

impl_trivial!(u64, u32);
