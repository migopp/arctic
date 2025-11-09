use ribbit::u56;
use ribbit::u6;

use crate::raw::edge::Meta;

#[derive(Copy, Clone, ribbit::Pack)]
#[ribbit(size = 64)]
pub struct Be {
    bits: u6,
    value: bool,
    frozen: bool,
    prefix: u56,
}

impl Be {}

impl Meta for Be {
    const DEFAULT: ribbit::Packed<Self> =
        ribbit::Packed::<Self>::new(u6::new(0), false, false, u56::new(0));

    fn bits(meta: ribbit::Packed<Self>) -> usize {
        todo!()
    }

    fn equal(left: ribbit::Packed<Self>, right: ribbit::Packed<Self>) -> bool {
        todo!()
    }

    fn cmp(left: ribbit::Packed<Self>, right: ribbit::Packed<Self>) -> core::cmp::Ordering {
        todo!()
    }

    fn is_value(meta: ribbit::Packed<Self>) -> bool {
        todo!()
    }

    fn is_frozen(meta: ribbit::Packed<Self>) -> bool {
        todo!()
    }

    fn with_frozen(meta: ribbit::Packed<Self>, frozen: bool) -> ribbit::Packed<Self> {
        todo!()
    }

    fn with_value(meta: ribbit::Packed<Self>, value: bool) -> ribbit::Packed<Self> {
        todo!()
    }

    fn expand(
        old: ribbit::Packed<Self>,
        new: ribbit::Packed<Self>,
    ) -> Result<(ribbit::Packed<Self>, u8, ribbit::Packed<Self>), usize> {
        todo!()
    }

    fn compress(
        parent: ribbit::Packed<Self>,
        byte: u8,
        child: ribbit::Packed<Self>,
    ) -> Option<ribbit::Packed<Self>> {
        todo!()
    }
}
