//! We divide all value types into two categories. A value type is **inline**
//! if is (1) [`Copy`], and (2) can be packed into 8 bytes. Otherwise, it is
//! **indirect**, and must be encapsulated in a smart pointer type like
//! [`Box`] or [`std::sync::Arc`].

mod inline;

pub trait Value {
    type Borrow<'l>
    where
        Self: 'l;

    fn into_raw(self) -> u64;

    unsafe fn borrow_raw<'l>(raw: u64) -> Self::Borrow<'l>;

    /// # Safety
    ///
    /// Caller must guarantee that `raw` was created by a previous [`Value::into_raw`] call.
    unsafe fn from_raw(raw: u64) -> Self;
}

impl<T> Value for Box<T> {
    type Borrow<'l>
        = &'l T
    where
        Self: 'l;

    #[inline]
    fn into_raw(self) -> u64 {
        Box::into_raw(self) as u64
    }

    #[inline]
    unsafe fn borrow_raw<'l>(raw: u64) -> Self::Borrow<'l> {
        let borrow = (raw as *const T).as_ref();
        if cfg!(feature = "validate") {
            borrow.unwrap()
        } else {
            unsafe { borrow.unwrap_unchecked() }
        }
    }

    #[inline]
    unsafe fn from_raw(raw: u64) -> Self {
        Box::from_raw(raw as *mut T)
    }
}
