//! We divide all value types into two categories. A value type is **inline**
//! if is (1) [`Copy`], and (2) can be packed into 8 bytes. Otherwise, it is
//! **indirect**, and must be encapsulated in a smart pointer type like
//! [`Box`] or [`std::sync::Arc`].

pub unsafe trait Value {
    type Borrow<'l>: Copy
    where
        Self: 'l;

    // type BorrowMut<'l>
    // where
    //     Self: 'l;

    fn into_raw(self) -> u64;

    /// # Safety
    ///
    /// Caller must guarantee that:
    /// 1. `raw` was created by a previous [`Value::into_raw`] call.
    /// 2. There are no live borrows from [`Value::borrow_mut_raw`] during lifetime `'l`.
    /// 3. There is no call to [`Value::from_raw`] during lifetime `'l`.
    unsafe fn borrow_from_raw<'l>(raw: u64) -> Self::Borrow<'l>;

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

unsafe impl<T> Value for Box<T> {
    type Borrow<'l>
        = &'l T
    where
        Self: 'l;

    #[inline]
    unsafe fn from_raw(raw: u64) -> Self {
        Box::from_raw(raw as *mut T)
    }

    #[inline]
    fn into_raw(self) -> u64 {
        Box::into_raw(self) as u64
    }

    unsafe fn borrow_from_raw<'l>(raw: u64) -> Self::Borrow<'l> {
        let borrow = (raw as *const T).as_ref();
        if cfg!(feature = "validate") {
            borrow.unwrap()
        } else {
            unsafe { borrow.unwrap_unchecked() }
        }
    }
}

macro_rules! impl_trivial {
    ($($ty:ty),*) => {
        $(
            unsafe impl Value for $ty {
                type Borrow<'l> = Self;

                #[inline]
                unsafe fn from_raw(raw: u64) -> Self {
                    raw as $ty
                }

                #[inline]
                fn into_raw(self) -> u64 {
                    self as u64
                }

                #[inline]
                unsafe fn borrow_from_raw<'l>(raw: u64) -> Self::Borrow<'l> {
                    raw as $ty
                }
            }
        )*
    };
}

impl_trivial!(u64, u32);
