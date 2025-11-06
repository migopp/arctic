pub(crate) trait Inline: Copy {
    fn into_raw(self) -> u64;

    /// # Safety
    ///
    /// Caller must guarantee that `raw` was created by a previous [`Inline::into_raw`] call.
    unsafe fn from_raw(raw: u64) -> Self;
}
