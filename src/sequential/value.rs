//! We divide all value types into two categories. A value type is **inline**
//! if is (1) [`Copy`], and (2) can be packed into 8 bytes. Otherwise, it is
//! **indirect**, and must be encapsulated in a smart pointer type like
//! [`Box`] or [`std::sync::Arc`].

pub unsafe trait Value {
    type Borrow<'l>
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

unsafe impl<T> Value for T
where
    T: crate::concurrent::Value,
{
    type Borrow<'l>
        = <T as crate::concurrent::Value>::Borrow<'l>
    where
        Self: 'l;

    // FIXME
    // type BorrowMut<'l>
    // where
    //     Self: 'l;

    fn into_raw(self) -> u64 {
        <T as crate::concurrent::Value>::into_raw(self)
    }

    unsafe fn borrow_from_raw<'l>(raw: u64) -> Self::Borrow<'l> {
        <T as crate::concurrent::Value>::borrow_from_raw(raw)
    }

    // FIXME
    // unsafe fn borrow_mut_raw<'l>(raw: &mut u64) -> Self::BorrowMut<'l> {
    //     todo!()
    // }

    unsafe fn from_raw(raw: u64) -> Self {
        <T as crate::concurrent::Value>::from_raw(raw)
    }
}
