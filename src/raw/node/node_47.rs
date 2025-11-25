use core::fmt::Debug;
use core::ops::Shr;
use core::sync::atomic::Ordering;

use ribbit::u6;
use ribbit::Atomic;

use crate::raw::edge;
use crate::raw::node;
use crate::raw::node::linear;
use crate::raw::node::Node15;
use crate::raw::node::Node256;
use crate::raw::Edge;
use crate::raw::Node;
use crate::stat;

#[repr(C, align(1024))]
pub(crate) struct Node47<M: ribbit::Pack> {
    header: Header,
    edges: [Atomic<Edge<M>>; 47],
}

const _: () = assert!(core::mem::size_of::<Node47<()>>() == 1024);
const _: () = assert!(core::mem::align_of::<Node47<()>>() == 1024);

impl<M> Default for Node47<M>
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

impl<M> Node<M> for Node47<M>
where
    M: ribbit::Pack<Packed: edge::Meta>,
{
    const KIND: node::Kind = node::Kind::Node47;
    const LEN: usize = 47;

    type Grow = Node256<M>;
    type Shrink = Node15<M>;

    fn keys<L: node::iter::Lower, U: node::iter::Upper>(
        &self,
        lower: L,
        upper: U,
    ) -> node::KeyIter {
        self.header.keys_range(lower, upper)
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

    fn freeze(&self) {
        self.header.freeze();
        self.edges.iter().for_each(Edge::freeze);
    }
}

impl<M> Debug for Node47<M>
where
    M: ribbit::Pack<Packed: edge::Meta + Debug>,
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Node47")
            .field("header", &self.header)
            .field("edges", &self.edges)
            .finish()
    }
}

#[repr(C, align(16))]
#[derive(Default)]
struct Header {
    data: [Atomic<u128>; 16],
    meta: Atomic<Meta>,
}

const _: [(); 272] = [(); core::mem::size_of::<Header>()];

impl Header {
    fn freeze(&self) {
        let mut old = self.meta.load_packed(Ordering::Relaxed);
        while !old.frozen() {
            self.ensure_meta_consistent(old);
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
        let i = key / 16;
        let j = key % 16;
        let index = (unsafe { self.data.get_unchecked(i as usize) }
            .load(Ordering::Relaxed)
            .shr(j << 3) as u8)
            .wrapping_sub(1);
        (index < 47).then_some(index)
    }

    fn get_or_insert(&self, key: u8) -> Option<u8> {
        loop {
            if let Some(index) = self.get(key) {
                return Some(index);
            }

            let old = self.meta();

            let len = old.len().value();
            if len == 47 || old.frozen() {
                return None;
            }

            let new = old.with_len(u6::new(len + 1)).with_last(key);

            match self
                .meta
                .compare_exchange_packed(old, new, Ordering::Relaxed, Ordering::Relaxed)
            {
                Ok(_) => {
                    self.ensure_meta_consistent(new);
                    return Some(len);
                }
                Err(conflict) if conflict.frozen() => return None,
                Err(_) => continue,
            }
        }
    }

    fn insert(&mut self, key: u8) -> Option<u8> {
        let old_meta = self.meta.get_packed();
        let old_len = old_meta.len().value();

        validate!(!old_meta.frozen());
        validate!(old_len <= 47);

        if old_len == 47 {
            return None;
        }

        let new_len = old_len + 1;
        let new_meta = old_meta.with_len(u6::new(new_len)).with_last(key);
        self.meta.set_packed(new_meta);

        let i = key / 16;
        let j = key % 16;

        let data = unsafe { self.data.get_unchecked_mut(i as usize) };
        let old_data = data.get();
        let new_data = old_data | ((new_len as u128) << (j << 3));
        data.set(new_data);
        Some(old_len)
    }

    fn keys_range<L: node::iter::Lower, U: node::iter::Upper>(
        &self,
        lower: L,
        upper: U,
    ) -> node::KeyIter {
        if lower.get() == 0 && upper.get() == 255 {
            return self.keys();
        }

        let i = lower.get() / 16;
        let j = upper.get() / 16;

        let mut entries = [(0u8, 0u8); 64];
        let mut len = 0;
        let mut keys = node::simd::add(node::simd::U8_SEQ, node::simd::mul(node::simd::U8_16, i));

        for k in i..=j {
            let indices = self.data[k as usize].load(Ordering::Relaxed);
            let valid = node::simd::mask_nonzero(indices)
                & node::simd::mask_range(keys, lower.get(), upper.get());
            let chunk = node::simd::compress(keys, node::simd::sub_one(indices), valid);
            unsafe {
                entries
                    .as_mut_ptr()
                    .add(len as usize)
                    .copy_from_nonoverlapping(chunk.as_ptr(), 16)
            };
            len += (valid.count_ones() >> 3) as u8;
            keys = node::simd::add(keys, node::simd::U8_16);
        }

        node::KeyIter::from_node_47(linear::KeyIter::new(
            core::array::from_fn(|i| entries[i]),
            len,
        ))
    }

    #[inline]
    fn keys(&self) -> node::KeyIter {
        let mut entries = [(0u8, 0u8); 64];
        let mut len = 0;
        let mut keys = node::simd::U8_SEQ;

        for i in 0..16 {
            let indices = self.data[i].load(Ordering::Relaxed);
            let valid = node::simd::mask_nonzero(indices);
            let chunk = node::simd::compress(keys, node::simd::sub_one(indices), valid);
            unsafe {
                entries
                    .as_mut_ptr()
                    .add(len as usize)
                    .copy_from_nonoverlapping(chunk.as_ptr(), 16)
            };
            len += (valid.count_ones() >> 3) as u8;
            keys = node::simd::add(keys, node::simd::U8_16);
        }

        let entries = core::array::from_fn(|i| entries[i]);
        node::KeyIter::from_node_47(linear::KeyIter::new(entries, len))
    }

    fn meta(&self) -> ribbit::Packed<Meta> {
        let meta = self.meta.load_packed(Ordering::Relaxed);
        self.ensure_meta_consistent(meta);
        meta
    }

    fn ensure_meta_consistent(&self, meta: ribbit::Packed<Meta>) {
        let len = meta.len().value();

        let key = meta.last();
        let i = key / 16;
        let j = key % 16;

        let data = unsafe { self.data.get_unchecked(i as usize) };
        let old = data.load(Ordering::Relaxed);

        if (old >> (j << 3)) as u8 == len {
            stat::increment(stat::Counter::Node47Consistent);
            return;
        }

        match data.compare_exchange(
            old,
            old | ((len as u128) << (j << 3)),
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => {
                stat::increment(stat::Counter::Node47CasSuccess);
            }
            Err(_) => stat::increment(stat::Counter::Node47CasFailure),
        }
    }
}

impl Debug for Header {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let meta = self.meta.load_packed(Ordering::Relaxed);
        let iter = self.keys();

        let len = meta.len().value();
        let mut keys = [0u8; 47];
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
#[ribbit(size = 16, packed(rename = "MetaPacked"))]
struct Meta {
    last: u8,
    frozen: bool,
    len: u6,
}

impl Meta {
    const DEFAULT: ribbit::Packed<Self> = ribbit::Packed::<Self>::new(0, false, u6::new(0));
}

impl Default for MetaPacked {
    fn default() -> Self {
        Meta::DEFAULT
    }
}
