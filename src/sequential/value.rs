pub unsafe trait Value {
    fn into_raw(self) -> u64;

    /// # Safety
    ///
    /// Caller must guarantee that:
    /// 1. `raw` was created by a previous [`Value::into_raw`] call.
    /// 2. `from_raw` is called at most once for each [`Value::into_raw`] call.
    /// 3. There are no live borrows from [`Value::ref_from_raw`] or [`Value::ref_mut_from_raw`] when [`Value::from_raw`] is called.
    unsafe fn from_raw(raw: u64) -> Self;
}

unsafe impl<T: Sized> Value for Box<T> {
    #[inline]
    unsafe fn from_raw(raw: u64) -> Self {
        unsafe { Box::from_raw(raw as *mut T) }
    }

    #[inline]
    fn into_raw(self) -> u64 {
        Box::into_raw(self) as u64
    }
}

unsafe impl<'v, T: 'v + Sized> Value for &'v T {
    #[inline]
    fn into_raw(self) -> u64 {
        // FIXME: strict provenance
        (self as *const T) as u64
    }

    #[inline]
    unsafe fn from_raw(raw: u64) -> Self {
        let borrow = unsafe { (raw as *const T).as_ref() };
        if_validate!(borrow.unwrap(), unsafe { borrow.unwrap_unchecked() })
    }
}

macro_rules! impl_integer {
    ($($ty:ty),*) => {
        $(
            unsafe impl Value for $ty {
                #[inline]
                unsafe fn from_raw(raw: u64) -> Self {
                    raw as $ty
                }

                #[inline]
                fn into_raw(self) -> u64 {
                    self as u64
                }
            }
        )*
    };
}

impl_integer!(u64, i64);
