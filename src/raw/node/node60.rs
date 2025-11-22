use core::fmt::Debug;
use core::ops::BitAnd as _;
use core::ops::Shr as _;
use core::sync::atomic::Ordering;

use ribbit::u112;
use ribbit::u6;
use ribbit::Atomic;

use crate::raw::edge;
use crate::raw::node;
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
    const GROW: usize = 60;

    type Grow = Node256<M>;
    type Shrink = Node15<M>;

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

    fn replace(&self, parent: ribbit::Packed<M>) -> (super::Smo, ribbit::Packed<Edge<M>>) {
        todo!()
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
            validate!(len <= 60);

            // Must help before initiating freeze
            self.help(len, meta.last());

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
        validate!(self.keys().all(|key_| key != key_));

        let old = self.meta.get_packed();
        validate!(!old.frozen());
        let len = old.len().value();
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

    fn help(&self, len: u8, last: u8) {
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

    #[inline]
    fn get_impl(&self, key: u8) -> Result<u8, ribbit::Packed<Meta>> {
        use core::arch::x86_64::_mm_cmpeq_epi8;
        use core::arch::x86_64::_mm_movemask_epi8;
        use core::arch::x86_64::_mm_set1_epi8;
        use std::arch::x86_64::__m128i;

        unsafe {
            let key = _mm_set1_epi8(key as i8);

            let meta = self.meta.load_packed(Ordering::Relaxed);
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

    fn keys(&self) -> impl Iterator<Item = u8> + '_ {
        let meta = self.meta.load_packed(Ordering::Relaxed);
        let len = meta.len().value() as usize;
        meta.value
            .to_ne_bytes()
            .into_iter()
            .skip(2)
            .chain(
                self.data
                    .iter()
                    .flat_map(|keys| keys.load(Ordering::Relaxed).to_ne_bytes()),
            )
            .take(len)
    }
}

impl Debug for Header {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let meta = self.meta.load_packed(Ordering::Relaxed);
        let len = meta.len();

        let mut keys = [0u8; 60];
        keys.iter_mut()
            .zip(self.keys())
            .for_each(|(out, r#in)| *out = r#in);

        f.debug_struct("Header")
            .field("len", &len)
            .field("frozen", &meta.frozen())
            .field("last", &meta.last())
            .field("keys", &&keys[..len.value() as usize])
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
