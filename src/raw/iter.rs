mod postorder;
mod range;

use core::cmp;
use core::ops::RangeInclusive;

pub(crate) use postorder::PostorderIter;
pub(crate) use range::RangeIter;

use crate::raw;
use crate::raw::edge::Key as _;
use crate::raw::edge::Len as _;
use crate::raw::edge::Meta as _;
use crate::raw::key;
use crate::raw::key::Read as _;

#[derive(Copy, Clone, Debug)]
pub struct Include<T>(pub(crate) T);

#[derive(Copy, Clone, Default, Debug)]
pub struct Unbound;

pub trait Range<'k, K: raw::Key>: Clone {
    #[expect(private_bounds)]
    type Lower: Lower<K::Read<'k>>;

    #[expect(private_bounds)]
    type Upper: Upper<K::Read<'k>>;

    fn lower(&self, bits: usize) -> Self::Lower;
    fn upper(&self, bits: usize) -> Self::Upper;

    #[inline]
    fn common_prefix(&self) -> K::Read<'k> {
        K::Read::default()
    }
}

impl<'k, K: raw::Key> Range<'k, K> for RangeInclusive<K::Borrow<'k>> {
    type Lower = Include<K::Read<'k>>;
    type Upper = Include<K::Read<'k>>;

    #[inline]
    fn lower(&self, bits: usize) -> Self::Lower {
        Include(K::Read::from(*self.start()).suffix(bits))
    }

    #[inline]
    fn upper(&self, bits: usize) -> Self::Upper {
        Include(K::Read::from(*self.end()).suffix(bits))
    }

    #[inline]
    fn common_prefix(&self) -> <K as raw::Key>::Read<'k> {
        K::Read::from(*self.start()).common_prefix(K::Read::from(*self.end()))
    }
}

impl<'k, K: raw::Key> Range<'k, K> for core::ops::RangeFrom<K::Borrow<'k>> {
    type Lower = Include<K::Read<'k>>;
    type Upper = Unbound;

    #[inline]
    fn lower(&self, bits: usize) -> Self::Lower {
        Include(K::Read::from(self.start).suffix(bits))
    }

    #[inline]
    fn upper(&self, _bits: usize) -> Self::Upper {
        Unbound
    }
}

impl<'k, K: raw::Key> Range<'k, K> for core::ops::RangeToInclusive<K::Borrow<'k>> {
    type Lower = Unbound;
    type Upper = Include<K::Read<'k>>;

    #[inline]
    fn lower(&self, _bits: usize) -> Self::Lower {
        Unbound
    }

    #[inline]
    fn upper(&self, bits: usize) -> Self::Upper {
        Include(K::Read::from(self.end).suffix(bits))
    }
}

impl<'k, K: raw::Key> Range<'k, K> for core::ops::RangeFull {
    type Lower = Unbound;
    type Upper = Unbound;

    #[inline]
    fn lower(&self, _bits: usize) -> Self::Lower {
        Unbound
    }

    #[inline]
    fn upper(&self, _bits: usize) -> Self::Upper {
        Unbound
    }
}

trait Lower<R: key::Read>: core::fmt::Debug {
    type Bound: raw::node::Lower;

    fn check(&mut self, edge: ribbit::Packed<R::Edge>) -> Option<Self::Bound>;
}

trait Upper<R: key::Read>: core::fmt::Debug {
    type Bound: raw::node::Upper;

    fn check(&mut self, edge: ribbit::Packed<R::Edge>) -> Option<Self::Bound>;
}

impl<R: key::Read> Lower<R> for Include<R> {
    type Bound = Option<u8>;

    fn check(&mut self, edge: ribbit::Packed<R::Edge>) -> Option<Self::Bound> {
        let key = edge.key();
        let len = key.len();

        // Skip check for fixed-length keys
        if R::BITS.is_none() && self.0.bits() < len.bits() {
            return None;
        }

        match self.0.read(len).cmp(&key) {
            cmp::Ordering::Less => Some(None),
            cmp::Ordering::Equal => Some(self.0.next()),
            cmp::Ordering::Greater => None,
        }
    }
}

impl<R: key::Read> Upper<R> for Include<R> {
    type Bound = Option<u8>;

    fn check(&mut self, edge: ribbit::Packed<R::Edge>) -> Option<Self::Bound> {
        let key = edge.key();
        let len = key.len();

        // Skip check for fixed-length keys
        if R::BITS.is_none() && self.0.bits() > len.bits() {
            return None;
        }

        match self.0.read(len).cmp(&key) {
            cmp::Ordering::Less => None,
            cmp::Ordering::Equal => Some(self.0.next()),
            cmp::Ordering::Greater => Some(None),
        }
    }
}

impl<R: key::Read> Lower<R> for Unbound {
    type Bound = Unbound;

    #[inline]
    fn check(&mut self, _edge: ribbit::Packed<R::Edge>) -> Option<Self::Bound> {
        Some(Unbound)
    }
}

impl<R: key::Read> Upper<R> for Unbound {
    type Bound = Unbound;

    #[inline]
    fn check(&mut self, _edge: ribbit::Packed<R::Edge>) -> Option<Self::Bound> {
        Some(Unbound)
    }
}
