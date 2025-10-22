use crate::iter::postorder;
use crate::smr;
use crate::Edge;

pub type Owned<'g, 'l, V> = <V as Value>::Guard<'g, 'l, true>;
pub type Shared<'g, 'l, V> = <V as Value>::Guard<'g, 'l, false>;

pub trait Value: Sized + Eq {
    type SelectDrop: postorder::Selector<Item<Self> = ribbit::Packed<Edge<Self>>>;

    type Guard<'g, 'l, const RETIRE: bool>: Sized
    where
        Self: 'g + 'l,
        'g: 'l;

    type Borrow<'l>: Copy
    where
        Self: 'l;

    unsafe fn protect<'g, 'l, const RETIRE: bool>(
        smr: smr::PathGuard<'g, 'l, Self>,
        value: u64,
    ) -> Self::Guard<'g, 'l, RETIRE>;

    unsafe fn from_u64(value: u64) -> Self;

    fn into_u64(self) -> u64;

    unsafe fn borrow_from_u64<'g, 'l>(
        smr: &smr::PathGuard<'g, 'l, Self>,
        value: u64,
    ) -> Self::Borrow<'l>;

    fn borrow_into_u64(borrow: Self::Borrow<'_>) -> u64;
}

impl<T: Eq> Value for Box<T> {
    type SelectDrop = postorder::SelectNonNull;

    type Guard<'g, 'l, const RETIRE: bool>
        = smr::LeafGuard<'g, 'l, RETIRE, Self>
    where
        Self: 'g + 'l,
        'g: 'l;

    type Borrow<'g>
        = &'g T
    where
        Self: 'g;

    #[inline]
    unsafe fn protect<'g, 'l, const RETIRE: bool>(
        smr: smr::PathGuard<'g, 'l, Self>,
        value: u64,
    ) -> Self::Guard<'g, 'l, RETIRE> {
        let borrow = Self::borrow_from_u64(&smr, value);
        unsafe { smr.scope::<RETIRE>(borrow) }
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
        _smr: &smr::PathGuard<'g, 'l, Self>,
        value: u64,
    ) -> Self::Borrow<'l> {
        let pointer = (value as *const T).as_ref();
        if cfg!(feature = "validate") {
            pointer.unwrap()
        } else {
            pointer.unwrap_unchecked()
        }
    }

    #[inline]
    fn borrow_into_u64(borrow: Self::Borrow<'_>) -> u64 {
        borrow as *const T as u64
    }
}

macro_rules! impl_trivial {
    ($($ty:ty),*) => {
        $(
            impl Value for $ty {
                type SelectDrop = postorder::SelectNode;

                type Guard<'g, 'l, const RETIRE: bool>
                    = Self
                where
                    'g: 'l;

                type Borrow<'g> = Self;

                #[inline]
                unsafe fn protect<'g, 'l, const RETIRE: bool>(
                    _smr: smr::PathGuard<'g, 'l, Self>,
                    value: u64,
                ) -> Self::Guard<'g, 'l, RETIRE> {
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
                    _smr: &smr::PathGuard<'g, 'l, Self>,
                    value: u64,
                ) -> Self::Borrow<'l> {
                    value as $ty
                }

                #[inline]
                fn borrow_into_u64(borrow: Self::Borrow<'_>) -> u64 {
                    borrow as u64
                }
            }
        )*
    };
}

impl_trivial!(u64, u32);
