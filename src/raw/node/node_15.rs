use ribbit::u120;
use ribbit::u4;

use crate::raw::edge;
use crate::raw::node;
use crate::raw::node::linear;
use crate::raw::node::Node3;
use crate::raw::node::Node47;

pub(crate) type Node15<C> = super::Linear<15, Header, C>;

const _: () = assert!(core::mem::size_of::<Node15<()>>() == 256);
const _: () = assert!(core::mem::align_of::<Node15<()>>() == 64);

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
    const KIND: node::Kind = node::Kind::Node15;
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
        let index = node::simd::mask_byte_to_bit(node::simd::mask_eq(self.value, key))
            .trailing_zeros() as u8;
        (index < self.len().value()).then_some(index)
    }

    #[inline]
    fn get_or_insert(self, key: u8) -> Result<u8, Option<Self>> {
        let len = self.len().value();
        validate!(len <= Self::LEN as u8);
        match self.get(key) {
            Some(index) => Ok(index),
            _ if len == Self::LEN as u8 || self.is_frozen() => Err(None),
            _ => Err(Some(Self::new(
                u120::new(self.keys().value() | ((key as u128) << (len * 8))),
                false,
                u4::new(len + 1),
            ))),
        }
    }

    fn keys<L: crate::raw::node::Lower, U: crate::raw::node::Upper>(
        self,
        lower: L,
        upper: U,
    ) -> node::KeyIter {
        let len = self.len().value();

        let mask_len = node::simd::mask_len(len);
        let mask_valid = if lower.get() == 0 && upper.get() == 255 {
            mask_len
        } else {
            mask_len & node::simd::mask_range(self.value, lower.get(), upper.get())
        };
        let len = node::simd::mask_byte_to_bit(mask_valid).count_ones() as u8;
        let out = node::simd::compress(self.value, node::simd::U8_SEQ, mask_valid);

        // TODO: SIMD sorting network?
        let entries = core::array::from_fn(|i| out[i]);
        node::KeyIter::new_15(linear::KeyIter::new_15(entries, len))
    }
}
