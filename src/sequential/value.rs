//! We divide all value types into two categories. A value type is **inline**
//! if is (1) [`Copy`], and (2) can be packed into 8 bytes. Otherwise, it is
//! **indirect**, and must be encapsulated in a smart pointer type like
//! [`Box`] or [`std::sync::Arc`].

pub unsafe trait Value {
    type Borrow<'l>: Copy
    where
        Self: 'l;

    fn borrow<'l>(&'l self) -> Self::Borrow<'l>;

    // type BorrowMut<'l>
    // where
    //     Self: 'l;

    fn into_raw(self) -> u64;

    fn borrow_into_raw<'l>(borrow: Self::Borrow<'l>) -> u64
    where
        Self: 'l;

    /// # Safety
    ///
    /// Caller must guarantee that:
    /// 1. `raw` was created by a previous [`Value::into_raw`] call.
    /// 2. There are no live borrows from [`Value::borrow_mut_raw`] during lifetime `'l`.
    /// 3. There is no call to [`Value::from_raw`] during lifetime `'l`.
    unsafe fn borrow_from_raw<'l>(raw: u64) -> Self::Borrow<'l>
    where
        Self: 'l;

    // FIXME
    // /// # Safety
    // ///
    // /// Caller must guarantee that:
    // /// 1. `raw` was created by a previous [`Value::into_raw`] call.
    // /// 2. There are no live borrows from [`Value::borrow_raw`] or [`Value::borrow_mut_raw`] during lifetime `'l`.
    // /// 3. There is no call to [`Value::from_raw`] during lifetime `'l`.
    // unsafe fn borrow_mut_raw<'l>(raw: &mut u64) -> Self::BorrowMut<'l>;

    /// # Safety
    ///
    /// Caller must guarantee that:
    /// 1. `raw` was created by a previous [`Value::into_raw`] call.
    /// 2. `from_raw` is called exactly once for each [`Value::into_raw`] call.
    /// 3. There are no live borrows from [`Value::borrow_raw`] when [`Value::from_raw`] is called.
    unsafe fn from_raw(raw: u64) -> Self;
}

unsafe impl<T: Sized> Value for Box<T> {
    type Borrow<'l>
        = &'l T
    where
        Self: 'l;

    #[inline]
    fn borrow<'l>(&'l self) -> Self::Borrow<'l> {
        self
    }

    #[inline]
    unsafe fn from_raw(raw: u64) -> Self {
        Box::from_raw(raw as *mut T)
    }

    #[inline]
    fn into_raw(self) -> u64 {
        Box::into_raw(self) as u64
    }

    #[inline]
    fn borrow_into_raw<'l>(borrow: Self::Borrow<'l>) -> u64
    where
        Self: 'l,
    {
        // FIXME: strict provenance
        (borrow as *const T) as u64
    }

    #[inline]
    unsafe fn borrow_from_raw<'l>(raw: u64) -> Self::Borrow<'l> {
        let borrow = (raw as *const T).as_ref();
        if cfg!(feature = "validate") {
            borrow.unwrap()
        } else {
            unsafe { borrow.unwrap_unchecked() }
        }
    }
}

unsafe impl<'v, T: 'v + Sized> Value for &'v T {
    type Borrow<'l>
        = &'l T
    where
        Self: 'l;

    #[inline]
    fn borrow<'l>(&'l self) -> Self::Borrow<'l> {
        self
    }

    #[inline]
    unsafe fn from_raw(raw: u64) -> Self {
        let borrow = (raw as *const T).as_ref();
        if cfg!(feature = "validate") {
            borrow.unwrap()
        } else {
            unsafe { borrow.unwrap_unchecked() }
        }
    }

    #[inline]
    fn into_raw(self) -> u64 {
        // FIXME: strict provenance
        (self as *const T) as u64
    }

    #[inline]
    fn borrow_into_raw<'l>(borrow: Self::Borrow<'l>) -> u64
    where
        Self: 'l,
    {
        // FIXME: strict provenance
        (borrow as *const T) as u64
    }

    #[inline]
    unsafe fn borrow_from_raw<'l>(raw: u64) -> Self::Borrow<'l>
    where
        Self: 'l,
    {
        let borrow = (raw as *const T).as_ref();
        if cfg!(feature = "validate") {
            borrow.unwrap()
        } else {
            unsafe { borrow.unwrap_unchecked() }
        }
    }
}

macro_rules! impl_integer {
    ($($ty:ty),*) => {
        $(
            unsafe impl Value for $ty {
                type Borrow<'l> = Self;

                #[inline]
                fn borrow<'l>(&'l self) -> Self::Borrow<'l> {
                    *self
                }

                #[inline]
                unsafe fn from_raw(raw: u64) -> Self {
                    raw as $ty
                }

                #[inline]
                fn into_raw(self) -> u64 {
                    self as u64
                }

                #[inline]
                unsafe fn borrow_from_raw<'l>(raw: u64) -> Self::Borrow<'l>
                where
                    Self: 'l,
                {
                    raw as $ty
                }

                #[inline]
                fn borrow_into_raw<'l>(borrow: Self::Borrow<'l>) -> u64
                where
                    Self: 'l,
                {
                    borrow as u64
                }
            }
        )*
    };
}

impl_integer!(u64, u32, u16, u8, i64, i32, i16, i8);
