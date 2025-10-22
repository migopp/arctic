use crate::smr;

pub trait Value: Sized + Eq {
    type Owned<'g, 'l>;
    type Shared<'g, 'l>: Sized
    where
        Self: 'g + 'l,
        'g: 'l;

    fn new_shared<'g, 'l>(smr: smr::PathGuard<'g, 'l, Self>, value: u64) -> Self::Shared<'g, 'l>;

    fn from_u64(value: u64) -> Self;
    fn into_u64(self) -> u64;
}

impl Value for u32 {
    type Owned<'g, 'l> = Self;
    type Shared<'g, 'l>
        = Self
    where
        Self: 'g + 'l,
        'g: 'l;

    #[inline]
    fn new_shared<'g, 'l>(_smr: smr::PathGuard<'g, 'l, Self>, value: u64) -> Self::Shared<'g, 'l> {
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
    type Owned<'g, 'l> = Self;
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
    fn new_shared<'g, 'l>(_smr: smr::PathGuard<'g, 'l, Self>, value: u64) -> Self::Shared<'g, 'l> {
        validate_eq!(value, 0);
    }
}
