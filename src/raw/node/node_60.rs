use core::fmt::Debug;
use core::ops::BitAnd as _;
use core::ops::Shr as _;
use core::sync::atomic::Ordering;

use ribbit::u112;
use ribbit::u6;
use ribbit::Atomic;

use crate::raw::edge;
use crate::raw::node;
use crate::raw::node::linear;
use crate::raw::node::Node15;
use crate::raw::node::Node256;
use crate::raw::Edge;
use crate::raw::Node;

#[repr(C, align(1024))]
pub(crate) struct Node60<M: ribbit::Pack> {
    header: Header,
    edges: [Atomic<Edge<M>>; 60],
}

const _: () = assert!(core::mem::size_of::<Node60<()>>() == 1024);
const _: () = assert!(core::mem::align_of::<Node60<()>>() == 1024);

impl<M> Default for Node60<M>
where
    M: ribbit::Pack<Packed: edge::Meta>,
{
    fn default() -> Self {
        Self {
            header: Header::default(),
            edges: core::array::from_fn(|_| Atomic::new_packed(Edge::DEFAULT)),
        }
    }
}

impl<M> Node<M> for Node60<M>
where
    M: ribbit::Pack<Packed: edge::Meta>,
{
    const KIND: node::Kind = node::Kind::Node60;
    const LEN: usize = 60;

    type Grow = Node256<M>;
    type Shrink = Node15<M>;

    fn keys<L: node::iter::Lower, U: node::iter::Upper>(
        &self,
        lower: L,
        upper: U,
    ) -> node::KeyIter {
        todo!()
    }

    fn edges(&self) -> &[Atomic<Edge<M>>] {
        &self.edges
    }

    fn get(&self, key: u8) -> Option<&Atomic<Edge<M>>> {
        let index = self.header.get(key)?;
        validate!((index as usize) < self.edges.len());
        Some(unsafe { self.edges.get_unchecked(index as usize) })
    }

    fn get_or_insert(&self, key: u8) -> Option<&Atomic<Edge<M>>> {
        let index = self.header.get_or_insert(key)?;
        validate!((index as usize) < self.edges.len());
        Some(unsafe { self.edges.get_unchecked(index as usize) })
    }

    fn insert(&mut self, key: u8) -> Option<&mut Atomic<Edge<M>>> {
        let index = self.header.insert(key)?;
        validate!((index as usize) < self.edges.len());
        Some(unsafe { self.edges.get_unchecked_mut(index as usize) })
    }

    fn freeze(
        &self,
    ) -> (
        impl Iterator<Item = u8>,
        impl Iterator<Item = ribbit::Packed<Edge<M>>>,
    ) {
        self.header.freeze();
        self.edges.iter().for_each(Edge::freeze);
        (
            self.header.keys_unsorted().map(|(key, _)| key),
            self.edges
                .iter()
                .map(|edge| edge.load_packed(Ordering::Relaxed)),
        )
    }
}

impl<M> Debug for Node60<M>
where
    M: ribbit::Pack<Packed: edge::Meta + Debug>,
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Node60")
            .field("header", &self.header)
            .field("edges", &self.edges)
            .finish()
    }
}

#[derive(Default)]
struct Header {
    meta: Atomic<Meta>,
    data: [Atomic<u128>; 3],
}

impl Header {
    fn freeze(&self) {
        let mut old = self.meta.load_packed(Ordering::Relaxed);
        while !old.frozen() {
            match self.meta.compare_exchange_packed(
                old,
                old.with_frozen(true),
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(conflict) => old = conflict,
            }
        }
    }

    fn get(&self, key: u8) -> Option<u8> {
        self.get_impl(key).ok()
    }

    fn get_or_insert(&self, key: u8) -> Option<u8> {
        loop {
            let meta = match self.get_impl(key) {
                Ok(index) => return Some(index),
                Err(meta) => meta,
            };

            let len = meta.len().value();
            if len == 60 || meta.frozen() {
                return None;
            }

            match self.meta.compare_exchange_packed(
                meta,
                meta.with_len(u6::new(len + 1)).with_last(key),
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    // Must ensure new key is visible to readers before returning
                    self.help(len + 1, key);
                    return Some(len);
                }
                Err(conflict) if conflict.frozen() => return None,
                Err(_) => continue,
            }
        }
    }

    fn insert(&mut self, key: u8) -> Option<u8> {
        let old = self.meta.get_packed();
        let len = old.len().value();

        validate!(!old.frozen());
        validate!(len <= 60);

        if len == 60 {
            return None;
        }

        let mut new = old.with_len(u6::new(len + 1)).with_last(key);

        let i = (len + 2) / 16;
        let j = (len + 2) % 16;
        let key = (key as u128) << (j << 3);

        match i.checked_sub(1) {
            None => new.value |= key,
            Some(i) => {
                let old = &mut self.data[i as usize];
                let new = old.get() | key;
                old.set(new);
            }
        }

        self.meta.set_packed(new);
        Some(len)
    }

    #[inline]
    fn get_impl(&self, key: u8) -> Result<u8, ribbit::Packed<Meta>> {
        use core::arch::x86_64::_mm_cmpeq_epi8;
        use core::arch::x86_64::_mm_movemask_epi8;
        use core::arch::x86_64::_mm_set1_epi8;
        use std::arch::x86_64::__m128i;

        unsafe {
            let key = _mm_set1_epi8(key as i8);

            let meta = self.meta.load_packed(Ordering::Relaxed);
            let len = meta.len().value();
            self.help(len, meta.last());

            let data_0 = self.data[0].load(Ordering::Relaxed);
            let data_1 = self.data[1].load(Ordering::Relaxed);
            let data_2 = self.data[2].load(Ordering::Relaxed);

            let match_0 = _mm_movemask_epi8(_mm_cmpeq_epi8(
                key,
                core::mem::transmute::<u128, __m128i>(meta.value),
            )) as u64;

            let match_1 = _mm_movemask_epi8(_mm_cmpeq_epi8(
                key,
                core::mem::transmute::<u128, __m128i>(data_0),
            )) as u64;

            let match_2 = _mm_movemask_epi8(_mm_cmpeq_epi8(
                key,
                core::mem::transmute::<u128, __m128i>(data_1),
            )) as u64;

            let match_3 = _mm_movemask_epi8(_mm_cmpeq_epi8(
                key,
                core::mem::transmute::<u128, __m128i>(data_2),
            )) as u64;

            let r#match = match_0 | (match_1 << 16) | (match_2 << 32) | (match_3 << 48);
            let len = meta.len().value();
            let index = r#match
                // Skip bottom two metadata bytes
                .shr(2u8)
                // Filter by node length
                .bitand((1u64 << len) - 1)
                .trailing_zeros() as u8;

            if index < len {
                Ok(index)
            } else {
                Err(meta)
            }
        }
    }

    fn keys_unsorted(&self) -> node::KeyIter {
        self.keys_inner().0
    }

    fn keys_range<L: node::iter::Lower, U: node::iter::Upper>(
        &self,
        lower: L,
        upper: U,
    ) -> node::KeyIter {
        todo!()
        // // https://stackoverflow.com/a/28383095
        // // https://talkchess.com/viewtopic.php?t=78804
        // let (keys, len) = unsafe {
        //     use core::arch::x86_64::_mm_and_si128;
        //     use core::arch::x86_64::_mm_cmpeq_epi8;
        //     use core::arch::x86_64::_mm_max_epu8;
        //     use core::arch::x86_64::_mm_min_epu8;
        //     use core::arch::x86_64::_mm_set1_epi8;
        //
        //     let len = self.len().value() as usize;
        //
        //     let mask_len = core::mem::transmute::<u128, core::arch::x86_64::__m128i>(
        //         (1u128 << (len << 3)) - 1,
        //     );
        //
        //     let min = lower.get();
        //     let max = upper.get();
        //
        //     let min = _mm_set1_epi8(min as i8);
        //     let max = _mm_set1_epi8(max as i8);
        //     let mask_range = _mm_cmpeq_epi8(_mm_min_epu8(_mm_max_epu8(min, keys), max), keys);
        //
        //     let mask_valid = core::mem::transmute::<core::arch::x86_64::__m128i, u128>(
        //         _mm_and_si128(mask_len, mask_range),
        //     );
        //     let len = (mask_valid.count_ones() >> 3) as u8;
        //
        //     (self.value & mask_valid | !mask_valid, len)
        // };
    }

    #[inline]
    fn keys_inner(&self) -> (node::KeyIter, ribbit::Packed<Meta>) {
        let meta = self.meta.load_packed(Ordering::Relaxed);
        let len = meta.len().value();
        self.help(len, meta.last());

        let mut entries = [(0u8, 0u8); 60];

        meta.value
            .to_le_bytes()
            .into_iter()
            .skip(2)
            .chain(
                self.data
                    .iter()
                    .flat_map(|keys| keys.load(Ordering::Relaxed).to_ne_bytes()),
            )
            .zip(&mut entries)
            .take(len as usize)
            .enumerate()
            .for_each(|(index_old, (key_old, (key_new, index_new)))| {
                *key_new = key_old;
                *index_new = index_old as u8;
            });

        (
            node::KeyIter::from_node_60(linear::KeyIter::new(entries, len)),
            meta,
        )
    }

    fn help(&self, len: u8, last: u8) {
        validate!((15..=60).contains(&len));

        let i = (len - 14 - 1) / 16;
        let j = ((len - 14 - 1) % 16) << 3;

        let keys = &self.data[i as usize];
        let old = keys.load(Ordering::Relaxed);

        if (old >> j) as u8 == last {
            return;
        }

        let new = old | ((last as u128) << j);

        // Safe to ignore failure: someone must have helped
        let _ = keys.compare_exchange_packed(old, new, Ordering::Relaxed, Ordering::Relaxed);
    }
}

impl Debug for Header {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (iter, meta) = self.keys_inner();
        let len = meta.len().value();
        let mut keys = [0u8; 60];
        keys.iter_mut()
            .zip(iter)
            .for_each(|(out, (key, _))| *out = key);

        f.debug_struct("Header")
            .field("len", &len)
            .field("frozen", &meta.frozen())
            .field("last", &meta.last())
            .field("keys", &&keys[..len as usize])
            .finish()
    }
}

#[derive(Copy, Clone, ribbit::Pack)]
#[ribbit(size = 128, packed(rename = "MetaPacked"))]
struct Meta {
    len: u6,
    frozen: bool,
    #[ribbit(offset = 8)]
    last: u8,
    #[ribbit(offset = 16)]
    keys: u112,
}

impl Meta {
    const DEFAULT: ribbit::Packed<Self> =
        ribbit::Packed::<Self>::new(u6::new(0), false, 0, u112::new(0));
}

impl Default for MetaPacked {
    fn default() -> Self {
        Meta::DEFAULT
    }
}
