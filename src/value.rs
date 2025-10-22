use crate::smr;

pub trait Value: Sized + Eq {
    type Owned<'g, 'l>: Sized
    where
        Self: 'g + 'l,
        'g: 'l;

    type Shared<'g, 'l>: Sized
    where
        Self: 'g + 'l,
        'g: 'l;

    unsafe fn new_shared<'g, 'l>(
        smr: smr::PathGuard<'g, 'l, Self>,
        value: u64,
    ) -> Self::Shared<'g, 'l>;

    fn from_u64(value: u64) -> Self;
    fn into_u64(self) -> u64;
}

impl<T: Eq> Value for Box<T> {
    type Owned<'g, 'l>
        = smr::Owned<'g, 'l, Self>
    where
        Self: 'g + 'l,
        'g: 'l;

    type Shared<'g, 'l>
        = smr::Shared<'g, 'l, Self, T>
    where
        Self: 'g + 'l,
        'g: 'l;

    #[inline]
    unsafe fn new_shared<'g, 'l>(
        smr: smr::PathGuard<'g, 'l, Self>,
        value: u64,
    ) -> Self::Shared<'g, 'l> {
        unsafe { smr.share::<T>((value as *const T).as_ref().unwrap()) }
    }

    #[inline]
    fn from_u64(value: u64) -> Self {
        todo!()
    }

    #[inline]
    fn into_u64(self) -> u64 {
        Box::into_raw(self) as u64
    }
}

impl Value for u32 {
    type Owned<'g, 'l>
        = Self
    where
        Self: 'g + 'l,
        'g: 'l;

    type Shared<'g, 'l>
        = Self
    where
        Self: 'g + 'l,
        'g: 'l;

    #[inline]
    unsafe fn new_shared<'g, 'l>(
        _smr: smr::PathGuard<'g, 'l, Self>,
        value: u64,
    ) -> Self::Shared<'g, 'l> {
        value as u32
    }

    #[inline]
    fn from_u64(value: u64) -> Self {
        value as u32
    }

    #[inline]
    fn into_u64(self) -> u64 {
        self as u64
    }
}

impl Value for () {
    type Owned<'g, 'l>
        = Self
    where
        Self: 'g + 'l,
        'g: 'l;

    type Shared<'g, 'l>
        = Self
    where
        Self: 'g + 'l,
        'g: 'l;

    #[inline]
    fn from_u64(_: u64) -> Self {}

    #[inline]
    fn into_u64(self) -> u64 {
        0
    }

    #[inline]
    unsafe fn new_shared<'g, 'l>(
        _smr: smr::PathGuard<'g, 'l, Self>,
        value: u64,
    ) -> Self::Shared<'g, 'l> {
        validate_eq!(value, 0);
    }
}
