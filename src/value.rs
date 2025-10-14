pub trait Value: Eq {
    fn from_u64(value: u64) -> Self;
    fn into_u64(self) -> u64;
}

impl Value for u32 {
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
    #[inline]
    fn from_u64(_: u64) -> Self {}

    #[inline]
    fn into_u64(self) -> u64 {
        0
    }
}
