use core::fmt::Debug;
use core::mem::ManuallyDrop;

mod iter;
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

pub(crate) trait Node<V> {
    fn edges(&self) -> &[Atomic128<Edge<V>>];

    fn get(&self, key: u8) -> Option<&Atomic128<Edge<V>>>;

    fn get_or_reserve(&self, key: u8) -> Option<&Atomic128<Edge<V>>>;

    fn reserve(&mut self, key: u8) -> Option<&mut Atomic128<Edge<V>>>;

    fn try_freeze(&self) -> Result<(), ()>;

    fn freeze(&self);

    fn replace(&self, parent: ribbit::Packed<edge::Meta>) -> (Op, ribbit::Packed<Edge<V>>);
}

pub(crate) trait Info<V>: Node<V> + Default + core::fmt::Debug {
    const KIND: Kind;
    const GROW: usize;
    const REF: for<'g> fn(&'g Self) -> Ref<'g, V>;

    type Grow: Info<V>;
    type Shrink: Info<V>;
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

impl Op {
    /// Whether this operation allocates a new node.
    #[inline]
    pub(crate) fn is_allocate(self) -> bool {
        match self {
            Self::Destroy | Self::Compress => false,
            Self::Grow | Self::Replace | Self::Shrink => true,
        }
    }
}

pub(crate) enum Ref<'g, V> {
    Node3(&'g Node3<V>),
    Node15(&'g Node15<V>),
    Node256(&'g Node256<V>),
}

impl<'g, V> Copy for Ref<'g, V> {}
impl<'g, V> Clone for Ref<'g, V> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<'g, V> Ref<'g, V> {
    #[inline]
    pub(crate) fn iter_sorted(&self) -> SortedIter<'g, V> {
        let (keys, edges) = match self {
            Ref::Node3(node) => (SortedKeyIter::from_linear(node.keys_sorted()), node.edges()),
            Ref::Node15(node) => (SortedKeyIter::from_linear(node.keys_sorted()), node.edges()),
            Ref::Node256(node) => (
                SortedKeyIter::from_node_256(node.keys_sorted()),
                node.edges(),
            ),
        };

        unsafe { SortedIter::new(keys, edges) }
    }

    #[inline]
    pub(crate) fn iter_unsorted(&self) -> UnsortedIter<'g, V> {
        let (keys, edges) = match self {
            Ref::Node3(node) => (Or::L(node.keys_unsorted()), node.edges()),
            Ref::Node15(node) => (Or::L(node.keys_unsorted()), node.edges()),
            Ref::Node256(node) => (Or::R(node.keys_sorted()), node.edges()),
        };

        unsafe { UnsortedIter::new(keys, edges) }
    }

    #[inline]
    pub(crate) fn iter_range(&self, min: Option<u8>, max: Option<u8>) -> SortedIter<'g, V> {
        if min.is_none() && max.is_none() {
            return self.iter_sorted();
        }

        let (keys, edges) = match self {
            Ref::Node3(node) => (
                SortedKeyIter::from_linear(node.keys_range(min.unwrap_or(0), max.unwrap_or(255))),
                node.edges(),
            ),
            Ref::Node15(node) => (
                SortedKeyIter::from_linear(node.keys_range(min.unwrap_or(0), max.unwrap_or(255))),
                node.edges(),
            ),
            Ref::Node256(node) => (
                SortedKeyIter::from_node_256(node.keys_range(min, max)),
                node.edges(),
            ),
        };

        unsafe { SortedIter::new(keys, edges) }
    }
}

impl<'g, V> Ref<'g, V> {
    #[inline]
    pub(crate) fn get(&self, key: u8) -> Option<&'g Atomic128<Edge<V>>> {
        match self {
            Ref::Node3(node) => node.get(key),
            Ref::Node15(node) => node.get(key),
            Ref::Node256(node) => node.get(key),
        }
    }

    #[inline]
    pub(crate) fn get_or_reserve(&self, key: u8) -> Option<&'g Atomic128<Edge<V>>> {
        match self {
            Ref::Node3(node) => node.get_or_reserve(key),
            Ref::Node15(node) => node.get_or_reserve(key),
            Ref::Node256(node) => node.get_or_reserve(key),
        }
    }

    #[cold]
    pub(crate) fn try_freeze(&self) -> Result<(), ()> {
        match self {
            Ref::Node3(node) => node.try_freeze(),
            Ref::Node15(node) => node.try_freeze(),
            Ref::Node256(node) => node.try_freeze(),
        }
    }

    #[cold]
    pub(crate) fn freeze(&self) {
        match self {
            Ref::Node3(node) => node.freeze(),
            Ref::Node15(node) => node.freeze(),
            Ref::Node256(node) => node.freeze(),
        }
    }

    #[cold]
    pub(crate) fn replace(
        &self,
        parent: ribbit::Packed<edge::Meta>,
    ) -> (Op, ribbit::Packed<Edge<V>>) {
        match self {
            Ref::Node3(node) => node.replace(parent),
            Ref::Node15(node) => node.replace(parent),
            Ref::Node256(node) => node.replace(parent),
        }
    }
}

impl<V> Debug for Ref<'_, V> {
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

pub(crate) type SortedIter<'g, V> = iter::SortedIter<'g, SortedKeyIter, V>;

pub(crate) type UnsortedIter<'g, V> = iter::UnsortedIter<'g, UnsortedKeyIter, V>;

pub(crate) type UnsortedKeyIter = Or<linear::UnsortedKeyIter, node256::KeyIter>;

pub(crate) union SortedKeyIter {
    linear: ManuallyDrop<linear::SortedKeyIter>,
    node_256: node256::KeyIter,
    raw: usize,
}

impl SortedKeyIter {
    const MASK_TAG: usize = 1usize.rotate_right(1);

    #[inline]
    fn from_linear(iter: linear::SortedKeyIter) -> Self {
        let iter = Self {
            linear: ManuallyDrop::new(iter),
        };
        validate_eq!(unsafe { iter.raw } & Self::MASK_TAG, 0);
        iter
    }

    #[inline]
    fn from_node_256(iter: node256::KeyIter) -> Self {
        let iter = Self { node_256: iter };
        validate_eq!(unsafe { iter.raw } & Self::MASK_TAG, Self::MASK_TAG);
        iter
    }

    fn is_node_256(&self) -> bool {
        (unsafe { self.raw } & Self::MASK_TAG) > 0
    }
}

impl Clone for SortedKeyIter {
    fn clone(&self) -> Self {
        if self.is_node_256() {
            Self {
                node_256: unsafe { self.node_256 },
            }
        } else {
            Self {
                linear: unsafe { self.linear.clone() },
            }
        }
    }
}

impl Iterator for SortedKeyIter {
    type Item = (u8, u8);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.is_node_256() {
            unsafe { &mut self.node_256 }.next().map(|key| (key, key))
        } else {
            unsafe { &mut self.linear }.next()
        }
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        if self.is_node_256() {
            unsafe { &self.node_256 }.size_hint()
        } else {
            unsafe { &self.linear }.size_hint()
        }
    }
}

impl DoubleEndedIterator for SortedKeyIter {
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.is_node_256() {
            unsafe { &mut self.node_256 }
                .next_back()
                .map(|key| (key, key))
        } else {
            unsafe { &mut self.linear }.next_back()
        }
    }
}

impl ExactSizeIterator for SortedKeyIter {
    #[inline]
    fn len(&self) -> usize {
        let (lower, upper) = self.size_hint();
        validate_eq!(upper, Some(lower));
        lower
    }
}
