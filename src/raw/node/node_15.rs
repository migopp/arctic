use ribbit::u4;
use ribbit::u120;

use crate::raw::edge;
use crate::raw::node;
use crate::raw::node::Node3;
use crate::raw::node::Node47;
use crate::raw::node::linear;

pub(crate) type Node15<C> = super::Linear<15, Header, C>;

const_assert_size_align!(Node15::<()>, 256, 64);

#[derive(Copy, Clone, Debug, ribbit::Pack)]
#[ribbit(size = 128, packed(rename = "HeaderPacked"), debug)]
pub(crate) struct Header {
    keys: u120,
    frozen: bool,
    len: u4,
}

impl Header {
    const DEFAULT: ribbit::Packed<Self> =
        ribbit::Packed::<Self>::new(u120::new(0), false, u4::new(0));
}

impl Default for HeaderPacked {
    fn default() -> Self {
        Header::DEFAULT
    }
}

impl linear::Header for ribbit::Packed<Header> {
    const TYPE: node::Type = node::Type::Node15;
    const LEN: usize = 15;

    type Grow<M>
        = Node47<M>
    where
        M: ribbit::Pack<Packed: edge::Meta>;
    type Shrink<M>
        = Node3<M>
    where
        M: ribbit::Pack<Packed: edge::Meta>;

    #[inline]
    fn freeze(self) -> Self {
        self.with_frozen(true)
    }

    #[inline]
    fn is_frozen(self) -> bool {
        self.frozen()
    }

    #[inline]
    fn len(self) -> u8 {
        self.len().value()
    }

    #[inline]
    fn get(self, key: u8) -> Option<u8> {
        let index = node::simd::get_15(self.value, key);
        (index < self.len().value()).then_some(index)
    }

    #[inline]
    fn get_or_insert(self, key: u8) -> Result<u8, Option<Self>> {
        let index = node::simd::get_15(self.value, key);
        let len = self.len().value();

        if index < len {
            return Ok(index);
        }

        if len >= Self::LEN as u8 || self.is_frozen() {
            return Err(None);
        }

        let key = (key as u128) << (len << 3);
        let value = (self.value | key) + (1u128 << 121);

        // SAFETY: `len < Self::LEN`
        Err(Some(unsafe { Self::new_unchecked(value) }))
    }

    fn keys<L: crate::raw::node::Lower, U: crate::raw::node::Upper>(
        self,
        lower: L,
        upper: U,
    ) -> node::KeyIter {
        let len = self.len();
        let mut iter = Box::new(linear::KeyIter::default());
        node::simd::compress_15(self.value, len, lower, upper, &mut iter);
        node::KeyIter::new_15(iter)
    }
}
