use ribbit::u2;
use ribbit::u48;

use crate::raw::edge;
use crate::raw::node;
use crate::raw::node::linear;
use crate::raw::node::simd;

use super::Node15;

pub(crate) type Node3<C> = super::Linear<3, Header, C>;

const _: () = assert!(core::mem::size_of::<Node3<()>>() == 64);
const _: () = assert!(core::mem::align_of::<Node3<()>>() == 64);

#[derive(Copy, Clone, Debug, ribbit::Pack)]
#[ribbit(size = 64, packed(rename = "HeaderPacked"), debug)]
pub(crate) struct Header {
    keys: u48,
    #[ribbit(offset = 48)]
    frozen: bool,
    #[ribbit(offset = 56)]
    len: u2,
}

impl Header {
    const DEFAULT: ribbit::Packed<Self> =
        ribbit::Packed::<Self>::new(u48::new(0), false, u2::new(0));
}

impl Default for HeaderPacked {
    fn default() -> Self {
        Header::DEFAULT
    }
}

impl linear::Header for ribbit::Packed<Header> {
    const KIND: node::Kind = node::Kind::Node3;
    const LEN: usize = 3;

    type Grow<M>
        = Node15<M>
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
        let index = simd::get_3(self.value, key);
        (index < self.len().value()).then_some(index)
    }

    #[inline]
    fn get_or_insert(self, key: u8) -> Result<u8, Option<Self>> {
        let index = simd::get_3(self.value, key);
        let len = self.len().value();

        if index < len {
            return Ok(index);
        }

        if len >= Self::LEN as u8 || self.is_frozen() {
            return Err(None);
        }

        // Insert key byte and increment length
        let key = (key as u64) << (len << 4);
        let value = (self.value | key) + (1u64 << 56);

        // SAFETY: `len < Self::LEN`
        Err(Some(unsafe { Self::new_unchecked(value) }))
    }

    fn keys<L: crate::raw::node::Lower, U: crate::raw::node::Upper>(
        self,
        lower: L,
        upper: U,
    ) -> node::KeyIter {
        let len = self.len();
        let iter = node::simd::compress_3(self.value, len, lower, upper);
        node::KeyIter::new_3(iter)
    }
}
