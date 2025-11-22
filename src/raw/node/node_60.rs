use core::fmt::Debug;
use core::ops::BitAnd as _;
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
use crate::stat;

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

    fn freeze(
        &self,
    ) -> (
        impl Iterator<Item = u8>,
        impl Iterator<Item = ribbit::Packed<Edge<M>>>,
    ) {
        self.header.freeze();
        self.edges.iter().for_each(Edge::freeze);
        (
            self.header.keys_unsorted().0.map(|(key, _)| key),
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

#[repr(C, align(64))]
#[derive(Default)]
struct Header {
    data: [Atomic<u128>; 3],
    meta: Atomic<Meta>,
}

const _: [(); 64] = [(); core::mem::size_of::<Header>()];

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
            let old = match self.get_impl(key) {
                Ok(index) => return Some(index),
                Err(meta) => meta,
            };

            let len = old.len().value();
            if len == 60 || old.frozen() {
                return None;
            }

            let mut new = old.with_len(u6::new(len + 1));
            if len >= 48 {
                let key = (key as u128) << ((len - 48) << 3);
                new = unsafe { ribbit::Packed::<Meta>::new_unchecked(new.value | key) }
            } else {
                new = new.with_last(key);
            }

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
        let len = old_meta.len().value();

        validate!(!old_meta.frozen());
        validate!(len <= 60);

        if len == 60 {
            return None;
        }

        let mut new_meta = old_meta.with_len(u6::new(len + 1));
        let i = len / 16;
        let j = len % 16;
        let key_data = (key as u128) << (j << 3);

        if i < 3 {
            new_meta = new_meta.with_last(key);

            let old_data = &mut self.data[i as usize];
            let new_data = old_data.get() | key_data;
            old_data.set(new_data);
        } else {
            new_meta = unsafe { ribbit::Packed::<Meta>::new_unchecked(new_meta.value | key_data) }
        }

        self.meta.set_packed(new_meta);
        Some(len)
    }

    #[inline]
    fn get_impl(&self, key: u8) -> Result<u8, ribbit::Packed<Meta>> {
        let meta = self.meta();
        let data = self.data();

        let mut r#match = 0;
        for (i, chunk) in data
            .into_iter()
            .chain(core::iter::once(meta.value))
            .enumerate()
        {
            let local = node::simd::mask_eq(chunk, key) as u64;
            let global = local << (i * 16);
            r#match |= global;
        }

        let len = meta.len().value();
        let index = r#match
            // Mask against node length
            .bitand((1u64 << len) - 1)
            .trailing_zeros() as u8;

        if index < len {
            Ok(index)
        } else {
            Err(meta)
        }
    }

    fn keys_range<L: node::iter::Lower, U: node::iter::Upper>(
        &self,
        lower: L,
        upper: U,
    ) -> node::KeyIter {
        if L::UNBOUND && U::UNBOUND {
            return self.keys_unsorted().0;
        }

        let meta = self.meta();
        let data = self.data();

        #[inline(always)]
        const fn mask(len: u8) -> u128 {
            match (1u128).checked_shl((len as u32) << 3) {
                None => u128::MAX,
                Some(mask) => mask - 1,
            }
        }

        let len_total = meta.len().value();
        let mut len_valid = 0;
        let mut keys = [0u128; 4];

        for (i, chunk) in data
            .into_iter()
            .chain(core::iter::once(meta.value))
            .enumerate()
        {
            let mask_len = len_total
                .checked_sub(i as u8 * 16)
                .map(mask)
                .unwrap_or(0u128);

            let mask_range = node::simd::mask_range(chunk, lower.get(), upper.get());
            let mask_valid = mask_len & mask_range;
            len_valid += (mask_valid.count_ones() >> 3) as u8;
            keys[i] = chunk & mask_valid | !mask_valid;
        }

        let keys = unsafe { core::mem::transmute::<[u128; 4], [u8; 64]>(keys) };
        let entries = core::array::from_fn(|index| (keys[index], index as u8));
        node::KeyIter::from_node_60(linear::KeyIter::new(entries, len_valid))
    }

    #[inline]
    fn keys_unsorted(&self) -> (node::KeyIter, ribbit::Packed<Meta>) {
        let meta = self.meta();
        let data = self.data();

        let len = meta.len().value();
        let mut keys = unsafe {
            core::mem::transmute::<[u128; 4], [u8; 64]>([data[0], data[1], data[2], meta.value])
        };
        keys[len as usize..].fill(0xFF);
        let entries = core::array::from_fn(|index| (keys[index], index as u8));
        (
            node::KeyIter::from_node_60(linear::KeyIter::new(entries, len)),
            meta,
        )
    }

    fn meta(&self) -> ribbit::Packed<Meta> {
        let meta = self.meta.load_packed(Ordering::Relaxed);
        self.ensure_meta_consistent(meta);
        meta
    }

    fn ensure_meta_consistent(&self, meta: ribbit::Packed<Meta>) {
        validate!((15..=60).contains(&meta.len().value()));

        let index = meta.len().value() - 1;
        let i = index / 16;
        let j = (index % 16) << 3;

        // `get_or_insert` atomically maintains consistency
        // when len > 48, so helping is not necessary here
        if i == 3 {
            stat::increment(stat::Counter::Node60Consistent);
            return;
        }

        let keys = &self.data[i as usize];
        let old = keys.load(Ordering::Relaxed);
        let last = meta.last();

        // Consistent state
        if (old >> j) as u8 == last {
            stat::increment(stat::Counter::Node60Consistent);
            return;
        }

        let new = old | ((last as u128) << j);

        // Failed CAS is okay, means someone else helped
        match keys.compare_exchange_packed(old, new, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => stat::increment(stat::Counter::Node60CasSuccess),
            Err(_) => stat::increment(stat::Counter::Node60CasFailure),
        }
    }

    fn data(&self) -> [u128; 3] {
        core::array::from_fn(|i| self.data[i].load(Ordering::Relaxed))
    }
}

impl Debug for Header {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (iter, meta) = self.keys_unsorted();
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
    keys: u112,
    last: u8,
    frozen: bool,
    len: u6,
}

impl Meta {
    const DEFAULT: ribbit::Packed<Self> =
        ribbit::Packed::<Self>::new(u112::new(0), 0, false, u6::new(0));
}

impl Default for MetaPacked {
    fn default() -> Self {
        Meta::DEFAULT
    }
}
