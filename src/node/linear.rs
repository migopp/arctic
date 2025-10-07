use core::arch::x86_64::_mm_extract_epi64;
use core::arch::x86_64::_pext_u64;
use core::fmt::Debug;
use core::sync::atomic::Ordering;

use ribbit::atomic::Atomic128;

use crate::edge;
use crate::node;
use crate::node::Edge;
use crate::node::Op;
use crate::Node;

#[repr(C, align(64))]
#[derive(Debug)]
pub(crate) struct Linear<const LEN: usize, H> {
    pub(super) header: H,
    pub(super) edges: [Atomic128<Edge>; LEN],
}

impl<const LEN: usize, H: Default> Default for Linear<LEN, H> {
    fn default() -> Self {
        Self {
            header: H::default(),
            edges: core::array::from_fn(|_| Atomic128::default()),
        }
    }
}

impl<const LEN: usize, H> Node for Linear<LEN, H>
where
    H: Header,
    Self: node::Info,
{
    #[inline]
    fn get(&self, key: u8) -> Option<&Atomic128<Edge>> {
        let index = self.header.get(key)?;
        Some(unsafe { self.edges.get_unchecked(index as usize) })
    }

    #[inline]
    fn get_or_reserve(&self, key: u8) -> Option<&Atomic128<Edge>> {
        let index = self.header.get_or_reserve(key)?;
        Some(unsafe { self.edges.get_unchecked(index as usize) })
    }

    #[inline]
    fn reserve(&mut self, key: u8) -> Option<&mut Atomic128<Edge>> {
        let index = self.header.get_or_reserve(key)?;
        Some(unsafe { self.edges.get_unchecked_mut(index as usize) })
    }

    fn replace(&self, parent: ribbit::Packed<edge::Meta>) -> (Op, ribbit::Packed<Edge>) {
        let len = self.header.freeze();
        self.edges.iter().take(len).for_each(Edge::freeze);

        let mut edges: [(u8, ribbit::Packed<Edge>); LEN] =
            core::array::from_fn(|_| (0, Edge::DEFAULT));
        let mut len = 0;

        core::iter::zip(
            self.header.keys_unsorted(),
            self.edges
                .iter()
                .map(|edge| edge.load_packed(Ordering::Relaxed)),
        )
        .filter(|(_, edge)| edge.meta().leaf() || edge.data() != 0)
        .map(|(key, edge)| {
            validate!(
                edge.meta().frozen(),
                "{} edge must be frozen before replace",
                core::any::type_name::<Self>(),
            );
            (key, edge.with_meta(edge.meta().with_frozen(false)))
        })
        .zip(&mut edges)
        .for_each(|(edge, save)| {
            *save = edge;
            len += 1;
        });

        match &edges[..len] {
            _ if len == <Self as node::Info>::GROW => {
                return (
                    node::Op::Grow,
                    Edge::new_node::<<Self as node::Info>::Grow, _>(
                        parent.key(),
                        edges.into_iter().take(len),
                    ),
                )
            }
            [] => return (Op::Destroy, Edge::DEFAULT),
            [(key, edge)] => {
                if let Some(compress) = parent.key().compress(*key, edge.meta().key()) {
                    return (Op::Compress, edge.with_meta(edge.meta().with_key(compress)));
                }
            }

            _ => (),
        }

        // Catch-all:
        (
            node::Op::Replace,
            Edge::new_node::<Self, _>(parent.key(), edges.into_iter().take(len)),
        )
    }
}

#[expect(private_bounds)]
impl<const LEN: usize, H: Header> Linear<LEN, H> {
    #[inline]
    pub(super) fn iter_range(&self, min: u8, max: u8) -> SortedIter {
        SortedIter {
            keys: self.header.keys_range(min, max),
            edges: self.edges.as_slice(),
        }
    }

    #[inline]
    pub(super) fn iter_sorted(&self) -> SortedIter {
        SortedIter {
            keys: self.header.keys_sorted(),
            edges: self.edges.as_slice(),
        }
    }

    #[inline]
    pub(super) fn iter_unsorted(&self) -> UnsortedIter {
        UnsortedIter {
            keys: self.header.keys_unsorted(),
            edges: self.edges.iter(),
        }
    }
}

pub(super) trait Header {
    fn freeze(&self) -> usize;
    fn get(&self, key: u8) -> Option<u8>;
    fn get_or_reserve(&self, key: u8) -> Option<u8>;

    fn keys_range(&self, min: u8, max: u8) -> RangeKeyIter;
    fn keys_sorted(&self) -> SortedKeyIter;
    fn keys_unsorted(&self) -> UnsortedKeyIter;
}

pub(crate) struct SortedIter<'a> {
    keys: SortedKeyIter,
    edges: &'a [Atomic128<Edge>],
}

impl<'a> Iterator for SortedIter<'a> {
    type Item = (u8, &'a Atomic128<Edge>);
    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        let (key, index) = self.keys.next()?;
        let edge = unsafe { self.edges.get_unchecked(index as usize) };
        Some((key, edge))
    }
}

pub(crate) struct UnsortedIter<'a> {
    keys: UnsortedKeyIter,
    edges: core::slice::Iter<'a, Atomic128<Edge>>,
}

impl<'a> Iterator for UnsortedIter<'a> {
    type Item = (u8, &'a Atomic128<Edge>);
    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        let key = self.keys.next()?;
        let edge = unsafe { self.edges.next().unwrap_unchecked() };
        Some((key, edge))
    }
}

pub(crate) type RangeKeyIter = SortedKeyIter;

pub(crate) enum SortedKeyIter {
    K3(core::iter::Take<core::array::IntoIter<(u8, u8), 3>>),
    K15(core::iter::Take<core::array::IntoIter<(u8, u8), 15>>),
}

impl SortedKeyIter {
    #[inline]
    pub(crate) fn new_3(keys: u32, len: usize) -> Self {
        let keys = keys.to_ne_bytes();
        let mut indexes: [(u8, u8); 3] = core::array::from_fn(|index| (keys[index], index as u8));
        indexes[..len].sort_unstable();
        Self::K3(indexes.into_iter().take(len))
    }

    #[inline]
    pub(crate) fn new_15(keys: u128, len: usize) -> Self {
        let keys = keys.to_ne_bytes();
        let mut indexes: [(u8, u8); 15] = core::array::from_fn(|index| (keys[index], index as u8));
        indexes[..len].sort_unstable();
        Self::K15(indexes.into_iter().take(len))
    }

    #[inline]
    pub(crate) fn range_3(keys: u32, mut len: usize, min: u8, max: u8) -> Self {
        let keys = keys.to_ne_bytes();
        let mut indexes: [(u8, u8); 3] = core::array::from_fn(|index| {
            if (min..=max).contains(&keys[index]) {
                (keys[index], index as u8)
            } else {
                len -= 1;
                (255, 3)
            }
        });

        indexes.sort_unstable();
        Self::K3(indexes.into_iter().take(len))
    }

    #[inline]
    pub(crate) fn range_15(keys: u128, len: usize, min: u8, max: u8) -> Self {
        // https://stackoverflow.com/a/28383095
        // https://talkchess.com/viewtopic.php?t=78804
        let (keys, len) = unsafe {
            use core::arch::x86_64::_mm_and_si128;
            use core::arch::x86_64::_mm_cmpeq_epi8;
            use core::arch::x86_64::_mm_cvtsi128_si64x;
            use core::arch::x86_64::_mm_max_epu8;
            use core::arch::x86_64::_mm_min_epu8;
            use core::arch::x86_64::_mm_set1_epi8;

            let keys = core::mem::transmute::<u128, core::arch::x86_64::__m128i>(keys);

            let min = _mm_set1_epi8(min as i8);
            let max = _mm_set1_epi8(max as i8);
            let valid =
                core::mem::transmute::<u128, core::arch::x86_64::__m128i>(1u128 << (len << 3));

            let mask = _mm_and_si128(
                _mm_cmpeq_epi8(_mm_min_epu8(_mm_max_epu8(min, keys), max), keys),
                valid,
            );
            let len = core::mem::transmute::<core::arch::x86_64::__m128i, u128>(mask).count_ones()
                as usize;

            let mask_low = _mm_cvtsi128_si64x(mask) as u64;
            let keys_low = _mm_cvtsi128_si64x(keys) as u64;
            let keys_low = _pext_u64(keys_low, mask_low);

            let mask_high = _mm_extract_epi64::<1>(mask) as u64;
            let keys_high = _mm_extract_epi64::<1>(keys) as u64;
            let keys_high = _pext_u64(keys_high, mask_high);

            let keys = ((keys_high as u128) << mask_low.count_ones()) | (keys_low as u128);
            (keys, len)
        };

        // TODO: SIMD sorting network?
        let keys = keys.to_ne_bytes();
        let mut indexes: [(u8, u8); 15] = core::array::from_fn(|index| (keys[index], index as u8));
        indexes[..len].sort_unstable();
        Self::K15(indexes.into_iter().take(len))
    }
}

impl Iterator for SortedKeyIter {
    type Item = (u8, u8);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        match self {
            SortedKeyIter::K3(iter) => iter.next(),
            SortedKeyIter::K15(iter) => iter.next(),
        }
    }
}

pub(crate) enum UnsortedKeyIter {
    K3(core::iter::Take<core::array::IntoIter<u8, 4>>),
    K15(core::iter::Take<core::array::IntoIter<u8, 16>>),
}

impl UnsortedKeyIter {
    #[inline]
    pub(crate) fn new_3(keys: u32, len: usize) -> Self {
        Self::K3(keys.to_ne_bytes().into_iter().take(len))
    }

    #[inline]
    pub(crate) fn new_15(keys: u128, len: usize) -> Self {
        Self::K15(keys.to_ne_bytes().into_iter().take(len))
    }
}

impl Iterator for UnsortedKeyIter {
    type Item = u8;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::K3(iter) => iter.next(),
            Self::K15(iter) => iter.next(),
        }
    }
}
