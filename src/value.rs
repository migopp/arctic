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

    type Ref<'g>: Copy
    where
        Self: 'g;

    unsafe fn new_owned<'g, 'l>(
        smr: smr::PathGuard<'g, 'l, Self>,
        value: u64,
    ) -> Self::Owned<'g, 'l>;

    unsafe fn new_shared<'g, 'l>(
        smr: smr::PathGuard<'g, 'l, Self>,
        value: u64,
    ) -> Self::Shared<'g, 'l>;

    fn from_u64(value: u64) -> Self;
    fn into_u64(self) -> u64;
}

impl<T: Eq> Value for Box<T> {
    type Owned<'g, 'l>
        = smr::LeafGuard<'g, 'l, true, Self>
    where
        Self: 'g + 'l,
        'g: 'l;

    type Shared<'g, 'l>
        = smr::LeafGuard<'g, 'l, false, Self>
    where
        Self: 'g + 'l,
        'g: 'l;

    type Ref<'g>
        = &'g T
    where
        Self: 'g;

    #[inline]
    unsafe fn new_owned<'g, 'l>(
        smr: smr::PathGuard<'g, 'l, Self>,
        value: u64,
    ) -> Self::Owned<'g, 'l> {
        unsafe { smr.own((value as *const T).as_ref().unwrap()) }
    }

    #[inline]
    unsafe fn new_shared<'g, 'l>(
        smr: smr::PathGuard<'g, 'l, Self>,
        value: u64,
    ) -> Self::Shared<'g, 'l> {
        unsafe { smr.share((value as *const T).as_ref().unwrap()) }
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

impl Value for u64 {
    type Owned<'g, 'l>
        = Self
    where
        'g: 'l;

    type Shared<'g, 'l>
        = Self
    where
        'g: 'l;

    type Ref<'g> = Self;

    #[inline]
    unsafe fn new_owned<'g, 'l>(
        _smr: smr::PathGuard<'g, 'l, Self>,
        value: u64,
    ) -> Self::Shared<'g, 'l> {
        value
    }

    #[inline]
    unsafe fn new_shared<'g, 'l>(
        _smr: smr::PathGuard<'g, 'l, Self>,
        value: u64,
    ) -> Self::Shared<'g, 'l> {
        value
    }

    #[inline]
    fn from_u64(value: u64) -> Self {
        value
    }

    #[inline]
    fn into_u64(self) -> u64 {
        self
    }
}

impl Value for u32 {
    type Owned<'g, 'l>
        = Self
    where
        'g: 'l;

    type Shared<'g, 'l>
        = Self
    where
        'g: 'l;

    type Ref<'g> = Self;

    #[inline]
    unsafe fn new_owned<'g, 'l>(
        _smr: smr::PathGuard<'g, 'l, Self>,
        value: u64,
    ) -> Self::Shared<'g, 'l> {
        value as u32
    }

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
        'g: 'l;

    type Shared<'g, 'l>
        = Self
    where
        'g: 'l;

    type Ref<'g> = Self;

    #[inline]
    fn from_u64(value: u64) -> Self {
        validate_eq!(value, 0);
    }

    #[inline]
    fn into_u64(self) -> u64 {
        0
    }

    #[inline]
    unsafe fn new_owned<'g, 'l>(
        _smr: smr::PathGuard<'g, 'l, Self>,
        value: u64,
    ) -> Self::Shared<'g, 'l> {
        validate_eq!(value, 0);
    }

    #[inline]
    unsafe fn new_shared<'g, 'l>(
        _smr: smr::PathGuard<'g, 'l, Self>,
        value: u64,
    ) -> Self::Shared<'g, 'l> {
        validate_eq!(value, 0);
    }
}
