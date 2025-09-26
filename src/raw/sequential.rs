use ribbit::atomic::Atomic128;

use crate::byte;
use crate::raw::iter;
use crate::Edge;

#[derive(Default)]
pub(crate) struct Map {
    root: Atomic128<Edge>,
}

impl Map {
    pub(crate) fn root(&self) -> &Atomic128<Edge> {
        &self.root
    }

    #[expect(dead_code, unused_variables)]
    #[inline]
    pub(crate) fn get<K: byte::Iterator>(&mut self, key: K) -> Option<u64> {
        todo!()
    }

    #[expect(dead_code, unused_variables)]
    #[inline]
    pub(crate) fn insert<K: byte::Iterator>(&mut self, key: K, value: u64) -> Option<u64> {
        todo!()
    }

    #[expect(dead_code, unused_variables)]
    #[inline]
    pub(crate) fn remove<K: byte::Iterator>(&mut self, key: K) -> Option<u64> {
        todo!()
    }

    #[expect(dead_code, unused_variables)]
    #[inline]
    pub(crate) fn update<K: byte::Iterator>(&mut self, key: K, value: u64) -> Option<u64> {
        todo!()
    }

    pub(crate) fn preorder<K: byte::Stack, S: iter::Selector>(&mut self) -> iter::EntryIter<K, S> {
        iter::EntryIter::new(&mut self.root)
    }
}
