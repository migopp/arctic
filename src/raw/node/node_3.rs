use ribbit::u2;
use ribbit::u48;

use crate::raw::Edge;
use crate::raw::edge;
use crate::raw::node;
use crate::raw::node::Linear;
use crate::raw::node::linear;
use crate::raw::node::simd;

pub(crate) type Node3<M> = Linear<3, Header, M>;

const_assert_size_align!(Node3::<()>, 64, 64);

impl<M: ribbit::Pack<Packed: edge::Meta>> Linear<3, Header, M> {
    pub(crate) fn insert(&mut self, key: u8) -> Option<&mut ribbit::Atomic<Edge<M>>> {
        node::Node::insert(self, key)
    }
}

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
    const TYPE: node::Type = node::Type::Node3;
    const CAPACITY: usize = 3;

    #[expect(clippy::get_first)]
    fn new(keys: &[u8]) -> Self {
        let mut buffer = 0u64;
        buffer |= keys.get(0).copied().unwrap_or(0) as u64;
        buffer |= (keys.get(1).copied().unwrap_or(0) as u64) << 16;
        buffer |= (keys.get(2).copied().unwrap_or(0) as u64) << 32;
        Self::new(u48::new(buffer), false, u2::new(keys.len() as u8))
    }

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

        if len >= Self::CAPACITY as u8 || self.is_frozen() {
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
