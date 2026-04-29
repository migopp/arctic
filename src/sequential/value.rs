pub unsafe trait Value {
    type Target;

    fn into_raw(self) -> u64;

    /// # Safety
    ///
    /// Caller must guarantee that:
    /// 1. `raw` was created by a previous [`Value::into_raw`] call.
    /// 2. `from_raw` is called exactly once for each [`Value::into_raw`] call.
    /// 3. There are no live borrows from [`Value::borrow_raw`] when [`Value::from_raw`] is called.
    unsafe fn from_raw(raw: u64) -> Self;

    fn as_target(&self) -> &Self::Target;
    unsafe fn target_from_raw(raw: &u64) -> &Self::Target;
    unsafe fn target_mut_from_raw(raw: &mut u64) -> &mut Self::Target;
}

unsafe impl<T: Sized> Value for Box<T> {
    type Target = T;

    #[inline]
    unsafe fn from_raw(raw: u64) -> Self {
        unsafe { Box::from_raw(raw as *mut T) }
    }

    #[inline]
    fn into_raw(self) -> u64 {
        Box::into_raw(self) as u64
    }

    #[inline]
    fn as_target(&self) -> &Self::Target {
        self
    }

    #[inline]
    unsafe fn target_from_raw(raw: &u64) -> &Self::Target {
        let borrow = unsafe { (*raw as *const T).as_ref() };
        if_validate!(borrow.unwrap(), unsafe { borrow.unwrap_unchecked() })
    }

    #[inline]
    unsafe fn target_mut_from_raw(raw: &mut u64) -> &mut Self::Target {
        let borrow = unsafe { (*raw as *mut T).as_mut() };
        if_validate!(borrow.unwrap(), unsafe { borrow.unwrap_unchecked() })
    }
}

unsafe impl<'v, T: 'v + Sized> Value for &'v T {
    type Target = Self;

    #[inline]
    fn into_raw(self) -> u64 {
        // FIXME: strict provenance
        (self as *const T) as u64
    }

    #[inline]
    unsafe fn from_raw(raw: u64) -> Self {
        let borrow = unsafe { (raw as *const T).as_ref() };
        if cfg!(feature = "validate") {
            borrow.unwrap()
        } else {
            unsafe { borrow.unwrap_unchecked() }
        }
    }

    #[inline]
    fn as_target(&self) -> &Self::Target {
        self
    }

    #[inline]
    unsafe fn target_from_raw(raw: &u64) -> &Self::Target {
        unsafe { core::mem::transmute(raw) }
    }

    #[inline]
    unsafe fn target_mut_from_raw(raw: &mut u64) -> &mut Self::Target {
        unsafe { core::mem::transmute(raw) }
    }
}

macro_rules! impl_integer {
    ($($ty:ty),*) => {
        $(
            unsafe impl Value for $ty {
                type Target = Self;

                #[inline]
                unsafe fn from_raw(raw: u64) -> Self {
                    raw as $ty
                }

                #[inline]
                fn into_raw(self) -> u64 {
                    self as u64
                }

                #[inline]
                fn as_target(&self) -> &Self::Target {
                    self
                }

                #[inline]
                unsafe fn target_from_raw(raw: &u64) -> &Self::Target {
                    // TODO: supporting non-8-byte values requires
                    // changes due to endianness
                    unsafe { core::mem::transmute(raw) }
                }

                #[inline]
                unsafe fn target_mut_from_raw(raw: &mut u64) -> &mut Self::Target {
                    unsafe { core::mem::transmute(raw) }
                }
            }
        )*
    };
}

impl_integer!(u64, i64);
