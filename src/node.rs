use core::fmt::Debug;

mod linear;
mod node15;
mod node256;
mod node3;

use linear::Linear;
pub(crate) use node15::Node15;
pub(crate) use node256::Node256;
pub(crate) use node3::Node3;
use ribbit::atomic::Atomic128;

use crate::edge;
use crate::iter::Or;
use crate::Edge;

pub(crate) trait Node {
    fn get(&self, key: u8) -> Option<&Atomic128<Edge>>;

    fn get_or_reserve(&self, key: u8) -> Option<&Atomic128<Edge>>;

    fn reserve(&mut self, key: u8) -> Option<&mut Atomic128<Edge>>;

    fn replace(&self, parent: ribbit::Packed<edge::Meta>) -> (Op, ribbit::Packed<Edge>);
}

pub(crate) trait Info: Node + Default + core::fmt::Debug + 'static {
    const KIND: Kind;
    const GROW: usize;
    const REF: for<'a> fn(&'a Self) -> Ref<'a>;

    type Grow: Info;
    type Shrink: Info;
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum Op {
    /// Node shrink (smaller size)
    #[expect(dead_code)]
    Shrink,

    /// Node replacement (same size)
    Replace,

    /// Node growth (larger size)
    Grow,

    /// Node elimination
    Destroy,

    /// Path compression
    Compress,
}

#[derive(Copy, Clone)]
pub(crate) enum Ref<'a> {
    Node3(&'a Node3),
    Node15(&'a Node15),
    Node256(&'a Node256),
}

impl<'a> Ref<'a> {
    #[inline]
    pub(crate) unsafe fn iter(&self) -> Iter<'a> {
        match self {
            Ref::Node3(node) => Or::L(node.iter()),
            Ref::Node15(node) => Or::L(node.iter()),
            Ref::Node256(node) => Or::R(node.into_iter()),
        }
    }

    #[inline]
    pub(crate) unsafe fn iter_unsorted(&self) -> UnsortedIter<'a> {
        match self {
            Ref::Node3(node) => Or::L(node.iter_unsorted()),
            Ref::Node15(node) => Or::L(node.iter_unsorted()),
            Ref::Node256(node) => Or::R(node.into_iter()),
        }
    }

    #[inline]
    pub(crate) unsafe fn iter_range(&self, min: Option<u8>, max: Option<u8>) -> RangeIter<'a> {
        match self {
            Ref::Node3(node) if min.is_none() && max.is_none() => RangeIter::Linear {
                iter: node.iter(),
                min,
                max,
            },
            Ref::Node3(node) => RangeIter::Linear {
                iter: node.iter_range(min.unwrap_or(0), max.unwrap_or(255)),
                min,
                max,
            },

            Ref::Node15(node) if min.is_none() && max.is_none() => RangeIter::Linear {
                iter: node.iter(),
                min,
                max,
            },
            Ref::Node15(node) => RangeIter::Linear {
                iter: node.iter_range(min.unwrap_or(0), max.unwrap_or(255)),
                min,
                max,
            },

            Ref::Node256(node) => RangeIter::Node256(node.iter_range(min, max)),
        }
    }
}

impl<'a> Ref<'a> {
    #[inline]
    pub(crate) fn get(&self, key: u8) -> Option<&'a Atomic128<Edge>> {
        match self {
            Ref::Node3(node) => node.get(key),
            Ref::Node15(node) => node.get(key),
            Ref::Node256(node) => node.get(key),
        }
    }

    #[inline]
    pub(crate) fn get_or_reserve(&self, key: u8) -> Option<&'a Atomic128<Edge>> {
        match self {
            Ref::Node3(node) => node.get_or_reserve(key),
            Ref::Node15(node) => node.get_or_reserve(key),
            Ref::Node256(node) => node.get_or_reserve(key),
        }
    }

    #[cold]
    pub(crate) fn replace(&self, meta: ribbit::Packed<edge::Meta>) -> (Op, ribbit::Packed<Edge>) {
        match self {
            Ref::Node3(node) => node.replace(meta),
            Ref::Node15(node) => node.replace(meta),
            Ref::Node256(node) => node.replace(meta),
        }
    }

    #[inline]
    pub(crate) fn as_data(&self) -> u64 {
        match *self {
            Ref::Node3(node) => node as *const _ as u64 | Kind::Node3 as u64,
            Ref::Node15(node) => node as *const _ as u64 | Kind::Node15 as u64,
            Ref::Node256(node) => node as *const _ as u64 | Kind::Node256 as u64,
        }
    }
}

impl Debug for Ref<'_> {
    fn fmt(&self, fmt: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Ref::Node3(node) => node.fmt(fmt),
            Ref::Node15(node) => node.fmt(fmt),
            Ref::Node256(node) => node.fmt(fmt),
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ribbit::Pack)]
#[ribbit(size = 2, eq, debug)]
pub(crate) enum Kind {
    #[ribbit(size = 0)]
    Node3 = 0,
    #[ribbit(size = 0)]
    Node15 = 1,
    #[ribbit(size = 0)]
    Node256 = 2,
}

impl Default for Kind {
    fn default() -> Self {
        Self::Node3
    }
}

impl Kind {
    pub(crate) const NODE_3: ribbit::Packed<Kind> = ribbit::Packed::<Kind>::new_node3();
    pub(crate) const NODE_15: ribbit::Packed<Kind> = ribbit::Packed::<Kind>::new_node15();
    pub(crate) const NODE_256: ribbit::Packed<Kind> = ribbit::Packed::<Kind>::new_node256();
}

pub(crate) type Iter<'a> = Or<linear::Iter<'a>, node256::Iter<'a>>;
pub(crate) type UnsortedIter<'a> = Or<linear::UnsortedIter<'a>, node256::Iter<'a>>;

pub(crate) enum RangeIter<'a> {
    Linear {
        iter: linear::Iter<'a>,
        min: Option<u8>,
        max: Option<u8>,
    },
    Node256(node256::Iter<'a>),
}

impl<'a> RangeIter<'a> {
    #[inline]
    pub(crate) fn min(&self) -> Option<u8> {
        match self {
            RangeIter::Linear { min, .. } => *min,
            RangeIter::Node256(iter) => iter.min(),
        }
    }

    #[inline]
    pub(crate) fn max(&self) -> Option<u8> {
        match self {
            RangeIter::Linear { max, .. } => *max,
            RangeIter::Node256(iter) => iter.max(),
        }
    }
}

impl<'a> Iterator for RangeIter<'a> {
    type Item = (u8, &'a Atomic128<Edge>);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        match self {
            RangeIter::Linear { iter, .. } => iter.next(),
            RangeIter::Node256(iter) => iter.next(),
        }
    }
}

impl DoubleEndedIterator for RangeIter<'_> {
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        match self {
            Self::Linear { iter, .. } => iter.next_back(),
            Self::Node256(iter) => iter.next_back(),
        }
    }
}

impl ExactSizeIterator for RangeIter<'_> {
    #[inline]
    fn len(&self) -> usize {
        let (lower, upper) = self.size_hint();
        validate_eq!(upper, Some(lower));
        lower
    }
}
