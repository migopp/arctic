use core::fmt::Debug;
use core::ops::Shr;
use core::sync::atomic::AtomicU64;
use core::sync::atomic::Ordering;

use ribbit::u6;
use ribbit::Atomic;

use crate::raw::edge;
use crate::raw::node;
use crate::raw::node::iter::KeyIndex;
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

unsafe impl<M> Node<M> for Node47<M>
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

    #[inline]
    fn edges(&self) -> &[Atomic<Edge<M>>] {
        &self.edges
    }

    #[inline]
    fn edges_mut(&mut self) -> &mut [Atomic<Edge<M>>] {
        &mut self.edges
    }

    #[inline]
    fn get_key(&self, key: u8) -> Option<u8> {
        self.header.get(key)
    }

    #[inline]
    fn get_or_insert_key(&self, key: u8) -> Option<u8> {
        self.header.get_or_insert(key)
    }

    #[inline]
    fn insert_key(&mut self, key: u8) -> Option<u8> {
        self.header.insert(key)
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
struct Header {
    data: [Atomic<u128>; 16],
    meta: Atomic<Meta>,
}

impl Default for Header {
    fn default() -> Self {
        Self {
            data: [const { Atomic::new_packed(0x7F7F_7F7F_7F7F_7F7F_7F7F_7F7F_7F7F_7F7F) }; 16],
            meta: Atomic::new_packed(Meta::DEFAULT),
        }
    }
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

    #[inline]
    fn get(&self, key: u8) -> Option<u8> {
        let (row, col) = Self::key_to_row_col(key);
        let data = unsafe { self.data_unchecked(row) };
        validate!(col < 64);
        unsafe {
            core::hint::assert_unchecked(col < 64);
        }
        let index = data.load(Ordering::Relaxed).shr(col) as u8;
        (index < 47).then_some(index)
    }

    fn get_or_insert(&self, key: u8) -> Option<u8> {
        loop {
            if let Some(index) = self.get(key) {
                return Some(index);
            }

            let old = self.meta_consistent();
            let len = old.len().value();

            // NOTE: tricky edge case here, where the above `get`
            // call returns `None` between another thread updating
            // the metadata and the data array being updated.
            if key == old.last() {
                let index = len.checked_sub(1);
                validate!(index.is_some());
                return index;
            }

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
                Err(_) => continue,
            }
        }
    }

    fn insert(&mut self, key: u8) -> Option<u8> {
        let old_meta = self.meta.get_packed();
        let len = old_meta.len().value();

        validate!(!old_meta.frozen());
        validate!(len <= 47);

        if len == 47 {
            return None;
        }

        let new_meta = old_meta.with_len(u6::new(len + 1)).with_last(key);
        self.meta.set_packed(new_meta);

        let (row, col) = Self::key_to_row_col(key);

        let data = unsafe { self.data_unchecked_mut(row) };

        let old_data = *data.get_mut();
        let hole = !(0xFFu64 << col);
        let new_data = old_data & hole | ((len as u64) << col);

        *data.get_mut() = new_data;
        Some(len)
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

        let len = self.meta_consistent().len().value();
        let mut entries = [KeyIndex::DEFAULT; 64];
        let mut index = 0;
        let mut keys = node::simd::add(node::simd::U8_SEQ, node::simd::mul(node::simd::U8_16, i));

        for k in i..=j {
            let indices = self.data[k as usize].load(Ordering::Relaxed);
            let valid = node::simd::mask_lt(indices, len as i8)
                & node::simd::mask_range(keys, lower.get(), upper.get());
            unsafe {
                node::simd::compress_into(
                    keys,
                    indices,
                    valid,
                    entries[index as usize..].as_mut_ptr(),
                )
            };
            index += node::simd::mask_byte_to_bit(valid).count_ones() as u8;
            keys = node::simd::add(keys, node::simd::U8_16);
        }

        node::KeyIter::new_47(linear::KeyIter::new_47(
            core::array::from_fn(|i| entries[i]),
            index,
        ))
    }

    #[inline]
    fn keys(&self) -> node::KeyIter {
        let len = self.meta_consistent().len().value();

        let mut entries = [KeyIndex::DEFAULT; 64];
        let mut index = 0;
        let mut keys = node::simd::U8_SEQ;

        for i in 0..16 {
            let indices = self.data[i].load(Ordering::Relaxed);
            let valid = node::simd::mask_lt(indices, len as i8);
            unsafe {
                node::simd::compress_into(
                    keys,
                    indices,
                    valid,
                    entries[index as usize..].as_mut_ptr(),
                )
            };
            index += node::simd::mask_byte_to_bit(valid).count_ones() as u8;
            keys = node::simd::add(keys, node::simd::U8_16);
        }

        validate_eq!(index, len);
        let entries = core::array::from_fn(|i| entries[i]);
        node::KeyIter::new_47(linear::KeyIter::new_47(entries, index))
    }

    fn meta_consistent(&self) -> ribbit::Packed<Meta> {
        let meta = self.meta.load_packed(Ordering::Relaxed);
        self.ensure_meta_consistent(meta);
        meta
    }

    fn ensure_meta_consistent(&self, meta: ribbit::Packed<Meta>) {
        let len = meta.len().value();
        validate!((15..=47).contains(&len));
        let index = len - 1;

        let key = meta.last();
        let (row, col) = Self::key_to_row_col(key);

        let data = unsafe { self.data_unchecked(row) };
        let old = data.load(Ordering::Relaxed);

        if (old >> col) as u8 == index {
            stat::increment(stat::Counter::Node47Consistent);
            return;
        }

        let hole = !(0xFFu64 << col);
        let new = old & hole | ((index as u64) << col);

        match data.compare_exchange(old, new, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => {
                stat::increment(stat::Counter::Node47CasSuccess);
            }
            Err(_) => stat::increment(stat::Counter::Node47CasFailure),
        }
    }

    unsafe fn data_unchecked(&self, row: u8) -> &AtomicU64 {
        let data = unsafe {
            self.data
                .as_ptr()
                .cast::<AtomicU64>()
                .add(row as usize)
                .as_ref()
        };
        if cfg!(feature = "validate") {
            data.unwrap()
        } else {
            unsafe { data.unwrap_unchecked() }
        }
    }

    unsafe fn data_unchecked_mut(&mut self, row: u8) -> &mut AtomicU64 {
        let data = unsafe {
            self.data
                .as_mut_ptr()
                .cast::<AtomicU64>()
                .add(row as usize)
                .as_mut()
        };
        if cfg!(feature = "validate") {
            data.unwrap()
        } else {
            unsafe { data.unwrap_unchecked() }
        }
    }

    fn key_to_row_col(key: u8) -> (u8, u8) {
        let row = key / 8;
        let col = (key % 8) * 8;
        (row, col)
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
            .for_each(|(out, KeyIndex { key, .. })| *out = key);

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
