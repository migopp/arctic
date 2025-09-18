use core::fmt::Debug;
use core::mem::ManuallyDrop;
use core::sync::atomic::Ordering;

use ribbit::atomic::Atomic128;
use ribbit::atomic::Atomic64;
use ribbit::Unpack as _;

#[repr(C, align(16))]
pub(crate) union Split<L, H> {
    whole: ManuallyDrop<Atomic128<Whole<L, H>>>,
    pair: ManuallyDrop<Pair<L, H>>,
}

impl<L, H> Default for Split<L, H>
where
    L: ribbit::Pack + Default,
    H: ribbit::Pack + Default,
{
    fn default() -> Self {
        Self {
            pair: ManuallyDrop::new(Pair {
                low: Atomic64::new(L::default()),
                high: Atomic64::new(H::default()),
            }),
        }
    }
}

impl<L, H> Debug for Split<L, H>
where
    L: ribbit::Pack + Debug,
    H: ribbit::Pack + Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Split")
            .field(unsafe { &self.pair.low })
            .field(unsafe { &self.pair.high })
            .finish()
    }
}

#[derive(Copy, Clone)]
#[ribbit::pack(size = 128)]
struct Whole<L, H> {
    #[ribbit(size = 64)]
    low: L,
    #[ribbit(size = 64)]
    high: H,
}

#[repr(C)]
struct Pair<L, H> {
    low: Atomic64<L>,
    high: Atomic64<H>,
}

impl<L, H> Split<L, H>
where
    L: ribbit::Pack,
    H: ribbit::Pack,
{
    const INVARIANT: () = {
        assert!(core::mem::size_of::<Self>() == 16);
        assert!(core::mem::size_of::<ManuallyDrop<Atomic128<Whole<L, H>>>>() == 16);
        assert!(core::mem::size_of::<ManuallyDrop<Pair<L, H>>>() == 16);
    };

    #[inline]
    pub(crate) fn load_low(&self, ordering: Ordering) -> L {
        const { Self::INVARIANT }
        self.load_low_packed(ordering).unpack()
    }

    #[inline]
    pub(crate) fn load_low_packed(&self, ordering: Ordering) -> ribbit::Packed<L> {
        const { Self::INVARIANT }
        unsafe { self.pair.low.load_packed(ordering) }
    }

    #[inline]
    pub(crate) fn load_high(&self, ordering: Ordering) -> H {
        const { Self::INVARIANT }
        self.load_high_packed(ordering).unpack()
    }

    #[inline]
    pub(crate) fn load_high_packed(&self, ordering: Ordering) -> ribbit::Packed<H> {
        const { Self::INVARIANT }
        unsafe { self.pair.high.load_packed(ordering) }
    }

    #[inline]
    pub(crate) fn load(&self, ordering: Ordering) -> (L, H) {
        const { Self::INVARIANT }
        let (low, high) = self.load_packed(ordering);
        (low.unpack(), high.unpack())
    }

    #[inline]
    pub(crate) fn load_packed(&self, ordering: Ordering) -> (ribbit::Packed<L>, ribbit::Packed<H>) {
        const { Self::INVARIANT }
        let whole = unsafe { self.whole.load_packed(ordering) };
        (whole.low(), whole.high())
    }

    #[inline]
    pub(crate) fn set_low(&mut self, low: L) {
        const { Self::INVARIANT }
        unsafe { self.pair.low.set(low) }
    }

    #[inline]
    pub(crate) fn set_high(&mut self, high: H) {
        const { Self::INVARIANT }
        unsafe { self.pair.high.set(high) }
    }

    #[inline]
    pub(crate) fn compare_exchange(
        &self,
        (old_low, old_high): (L, H),
        (new_low, new_high): (L, H),
        success: Ordering,
        failure: Ordering,
    ) -> Result<(L, H), (L, H)> {
        const { Self::INVARIANT }
        self.compare_exchange_packed(
            (old_low.pack(), old_high.pack()),
            (new_low.pack(), new_high.pack()),
            success,
            failure,
        )
        .map(|(low, high)| (low.unpack(), high.unpack()))
        .map_err(|(low, high)| (low.unpack(), high.unpack()))
    }

    #[inline]
    pub(crate) fn compare_exchange_packed(
        &self,
        (old_low, old_high): (ribbit::Packed<L>, ribbit::Packed<H>),
        (new_low, new_high): (ribbit::Packed<L>, ribbit::Packed<H>),
        success: Ordering,
        failure: Ordering,
    ) -> Result<(ribbit::Packed<L>, ribbit::Packed<H>), (ribbit::Packed<L>, ribbit::Packed<H>)>
    {
        const { Self::INVARIANT }
        unsafe {
            self.whole
                .compare_exchange_packed(
                    ribbit::Packed::<Whole<L, H>>::new(old_low, old_high),
                    ribbit::Packed::<Whole<L, H>>::new(new_low, new_high),
                    success,
                    failure,
                )
                .map(|old| (old.low(), old.high()))
                .map_err(|old| (old.low(), old.high()))
        }
    }
}
