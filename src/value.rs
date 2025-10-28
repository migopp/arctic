use crate::edge;
use crate::iter::postorder;
use crate::smr;
use crate::Edge;

pub unsafe trait Value: Sized {
    type SelectDrop: postorder::Selector<Item<Self> = ribbit::Packed<Edge<Self>>>;

    type OwnedGuard<'g, 'l>: Sized
    where
        Self: 'g + 'l,
        'g: 'l;

    type SharedGuard<'g, 'l>: Sized
    where
        Self: 'g + 'l,
        'g: 'l;

    type Borrow<'l>: Copy
    where
        Self: 'l;

    type Target;

    type Clone;

    unsafe fn guard_borrow<'g, 'l>(
        smr: &'l smr::TraverseGuard<'g, 'l, Self>,
        data: ribbit::Packed<edge::Data<Self>>,
    ) -> Self::Borrow<'l>;

    unsafe fn guard_owned<'g, 'l>(
        smr: smr::TraverseGuard<'g, 'l, Self>,
        data: ribbit::Packed<edge::Data<Self>>,
    ) -> Self::OwnedGuard<'g, 'l>;

    unsafe fn guard_shared<'g, 'l>(
        smr: smr::TraverseGuard<'g, 'l, Self>,
        data: ribbit::Packed<edge::Data<Self>>,
    ) -> Self::SharedGuard<'g, 'l>;

    unsafe fn from_data(data: ribbit::Packed<edge::Data<Self>>) -> Self;

    fn into_u64(self) -> u64;

    fn borrow_into_u64<'l>(borrow: Self::Borrow<'l>) -> u64
    where
        Self: 'l;
}

unsafe impl<T> Value for Box<T> {
    type SelectDrop = postorder::SelectNonNull;

    type OwnedGuard<'g, 'l>
        = smr::ValueGuard<'g, 'l, true, Self>
    where
        Self: 'g + 'l,
        'g: 'l;

    type SharedGuard<'g, 'l>
        = smr::ValueGuard<'g, 'l, false, Self>
    where
        Self: 'g + 'l,
        'g: 'l;

    type Borrow<'l>
        = &'l T
    where
        Self: 'l;

    type Target = T;

    type Clone = T;

    #[inline]
    unsafe fn guard_owned<'g, 'l>(
        smr: smr::TraverseGuard<'g, 'l, Self>,
        data: ribbit::Packed<edge::Data<Self>>,
    ) -> Self::OwnedGuard<'g, 'l> {
        let borrow = (data.value() as *const T).as_ref();
        let borrow = if cfg!(feature = "validate") {
            borrow.unwrap()
        } else {
            unsafe { borrow.unwrap_unchecked() }
        };
        unsafe { smr.guard_owned(borrow) }
    }

    #[inline]
    unsafe fn guard_shared<'g, 'l>(
        smr: smr::TraverseGuard<'g, 'l, Self>,
        data: ribbit::Packed<edge::Data<Self>>,
    ) -> Self::SharedGuard<'g, 'l> {
        let borrow = (data.value() as *const T).as_ref();
        let borrow = if cfg!(feature = "validate") {
            borrow.unwrap()
        } else {
            unsafe { borrow.unwrap_unchecked() }
        };
        smr.guard_shared(borrow)
    }

    #[inline]
    unsafe fn from_data(data: ribbit::Packed<edge::Data<Self>>) -> Self {
        Box::from_raw(data.value() as *mut T)
    }

    #[inline]
    fn into_u64(self) -> u64 {
        Box::into_raw(self) as u64
    }

    #[inline]
    unsafe fn guard_borrow<'g, 'l>(
        _smr: &'l smr::TraverseGuard<'g, 'l, Self>,
        data: ribbit::Packed<edge::Data<Self>>,
    ) -> Self::Borrow<'l> {
        let borrow = (data.value() as *const T).as_ref();
        if cfg!(feature = "validate") {
            borrow.unwrap()
        } else {
            borrow.unwrap_unchecked()
        }
    }

    #[inline]
    fn borrow_into_u64<'l>(borrow: Self::Borrow<'l>) -> u64
    where
        Self: 'l,
    {
        borrow as *const T as u64
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
//         _smr: &smr::TraverseGuard<'g, 'l, Self>,
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
                type SelectDrop = postorder::SelectNode;

                type OwnedGuard<'g, 'l>
                    = Self
                where
                    'g: 'l;

                type SharedGuard<'g, 'l>
                    = Self
                where
                    'g: 'l;

                type Borrow<'g> = Self;

                type Target = Self;

                type Clone = Self;

                #[inline]
                unsafe fn guard_owned<'g, 'l>(_smr: smr::TraverseGuard<'g, 'l, Self>, data: ribbit::Packed<edge::Data<Self>>) -> Self {
                    data.value() as $ty
                }

                #[inline]
                unsafe fn guard_shared<'g, 'l>(_smr: smr::TraverseGuard<'g, 'l, Self>, data: ribbit::Packed<edge::Data<Self>>) -> Self {
                    data.value() as $ty
                }

                #[inline]
                unsafe fn from_data(data: ribbit::Packed<edge::Data<Self>>) -> Self {
                    data.value() as $ty
                }

                #[inline]
                fn into_u64(self) -> u64 {
                    self as u64
                }

                #[inline]
                unsafe fn guard_borrow<'g, 'l>(
                    _smr: &smr::TraverseGuard<'g, 'l, Self>,
                    data: ribbit::Packed<edge::Data<Self>>,
                ) -> Self::Borrow<'l> {
                    data.value() as $ty
                }

                #[inline]
                fn borrow_into_u64<'l>(borrow: Self::Borrow<'l>) -> u64 where Self: 'l {
                    borrow as u64
                }
            }
        )*
    };
}

impl_trivial!(u64, u32);
