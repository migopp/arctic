use core::ptr::NonNull;

use crate::iter::postorder;
use crate::smr;
use crate::Edge;

pub type Owned<'g, 'l, V> = <V as Value>::Guard<'g, 'l, true>;
pub type Shared<'g, 'l, V> = <V as Value>::Guard<'g, 'l, false>;

pub unsafe trait Value: Sized {
    type SelectDrop: postorder::Selector<Item<Self> = ribbit::Packed<Edge<Self>>>;

    type Guard<'g, 'l, const OWNED: bool>: Sized
    where
        Self: 'g + 'l,
        'g: 'l;

    type Borrow<'l>: Copy
    where
        Self: 'l;

    type Target;

    type Clone;

    unsafe fn guard<'g, 'l, const OWNED: bool>(
        smr: smr::TraverseGuard<'g, 'l, Self>,
        value: u64,
    ) -> Self::Guard<'g, 'l, OWNED>;

    unsafe fn from_u64(value: u64) -> Self;

    fn into_u64(self) -> u64;

    unsafe fn borrow_from_u64<'g, 'l>(
        smr: &'l smr::TraverseGuard<'g, 'l, Self>,
        value: u64,
    ) -> Self::Borrow<'l>;

    fn borrow_into_u64<'l>(borrow: Self::Borrow<'l>) -> u64
    where
        Self: 'l;
}

unsafe impl<T> Value for Box<T> {
    type SelectDrop = postorder::SelectNonNull;

    type Guard<'g, 'l, const OWNED: bool>
        = smr::ValueGuard<'g, 'l, OWNED, Self>
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
    unsafe fn guard<'g, 'l, const OWNED: bool>(
        smr: smr::TraverseGuard<'g, 'l, Self>,
        value: u64,
    ) -> Self::Guard<'g, 'l, OWNED> {
        let pointer = if cfg!(feature = "validate") {
            NonNull::new(value as *mut T).unwrap()
        } else {
            NonNull::new_unchecked(value as *mut T)
        };
        unsafe { smr.scope::<OWNED>(pointer) }
    }

    #[inline]
    unsafe fn from_u64(value: u64) -> Self {
        Box::from_raw(value as *mut T)
    }

    #[inline]
    fn into_u64(self) -> u64 {
        Box::into_raw(self) as u64
    }

    #[inline]
    unsafe fn borrow_from_u64<'g, 'l>(
        _smr: &'l smr::TraverseGuard<'g, 'l, Self>,
        value: u64,
    ) -> Self::Borrow<'l> {
        let borrow = (value as *const T).as_ref();
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

#[derive(Copy, Clone)]
pub struct Inline<T>(pub T);

unsafe impl<T> Value for Inline<T>
where
    T: Copy + From<u64> + Into<u64>,
{
    type SelectDrop = postorder::SelectNode;

    type Guard<'g, 'l, const OWNED: bool>
        = Self
    where
        Self: 'g + 'l,
        'g: 'l;

    type Borrow<'l>
        = Self
    where
        Self: 'l;

    type Target = Self;

    type Clone = Self;

    #[inline]
    unsafe fn guard<'g, 'l, const OWNED: bool>(
        _smr: smr::TraverseGuard<'g, 'l, Self>,
        value: u64,
    ) -> Self::Guard<'g, 'l, OWNED> {
        Self(T::from(value))
    }

    #[inline]
    unsafe fn from_u64(value: u64) -> Self {
        Inline(T::from(value))
    }

    #[inline]
    fn into_u64(self) -> u64 {
        self.0.into()
    }

    #[inline]
    unsafe fn borrow_from_u64<'g, 'l>(
        _smr: &smr::TraverseGuard<'g, 'l, Self>,
        value: u64,
    ) -> Self::Borrow<'l> {
        Self(T::from(value))
    }

    #[inline]
    fn borrow_into_u64<'l>(borrow: Self::Borrow<'l>) -> u64
    where
        Self: 'l,
    {
        borrow.0.into()
    }
}

macro_rules! impl_trivial {
    ($($ty:ty),*) => {
        $(
            unsafe impl Value for $ty {
                type SelectDrop = postorder::SelectNode;

                type Guard<'g, 'l, const OWNED: bool>
                    = Self
                where
                    'g: 'l;

                type Borrow<'g> = Self;

                type Target = Self;

                type Clone = Self;

                #[inline]
                unsafe fn guard<'g, 'l, const OWNED: bool>(
                    _smr: smr::TraverseGuard<'g, 'l, Self>,
                    value: u64,
                ) -> Self::Guard<'g, 'l, OWNED> {
                    value as $ty
                }

                #[inline]
                unsafe fn from_u64(value: u64) -> Self {
                    value as $ty
                }

                #[inline]
                fn into_u64(self) -> u64 {
                    self as u64
                }

                #[inline]
                unsafe fn borrow_from_u64<'g, 'l>(
                    _smr: &smr::TraverseGuard<'g, 'l, Self>,
                    value: u64,
                ) -> Self::Borrow<'l> {
                    value as $ty
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
