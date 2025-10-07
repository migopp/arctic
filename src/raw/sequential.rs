use core::cell::Cell;
use core::marker::PhantomData;

use ribbit::atomic::Atomic128;

use crate::key;
use crate::node;
use crate::raw::iter;
use crate::stat;
use crate::Edge;

#[repr(transparent)]
#[derive(Default)]
pub(crate) struct Map {
    root: Atomic128<Edge>,
    _not_sync: PhantomData<Cell<()>>,
}

impl Map {
    pub(crate) fn root(&self) -> &Atomic128<Edge> {
        &self.root
    }

    #[expect(unused_variables)]
    #[inline]
    pub(crate) fn get<R: key::Read>(&self, key: R) -> Option<u64> {
        todo!()
    }

    #[expect(unused_variables)]
    #[inline]
    pub(crate) fn insert<R: key::Read>(&mut self, key: R, value: u64) -> Option<u64> {
        todo!()
    }

    #[expect(unused_variables)]
    #[inline]
    pub(crate) fn remove<R: key::Read>(&mut self, key: R) -> Option<u64> {
        todo!()
    }

    #[expect(unused_variables)]
    #[inline]
    pub(crate) fn update<R: key::Read>(&mut self, key: R, value: u64) -> Option<u64> {
        todo!()
    }

    pub(crate) fn iter_leaves<'a, K: key::Borrow, W: key::Write + PartialOrd<K>>(
        &'a self,
        min: K,
        max: K,
    ) -> iter::LeafIter<'a, K, W> {
        unsafe { iter::LeafIter::new(&self.root, W::default(), min, max) }
    }

    pub(crate) fn iter_postorder<'a, W: key::Write, V: iter::Selector<W>, S: iter::Sort<'a>>(
        &'a self,
        selector: V,
    ) -> iter::PostorderIter<'a, W, V, S> {
        unsafe { iter::PostorderIter::new(&self.root, W::default(), selector) }
    }
}

impl Drop for Map {
    fn drop(&mut self) {
        let mut iter = self
            .iter_postorder::<key::Ignore, iter::SelectNode, node::UnsortedIter>(iter::SelectNode);
        while let Some((key::Ignore, edge)) = iter.lend() {
            unsafe {
                Edge::deallocate(edge, stat::Counter::FreeDrop);
            }
        }
    }
}
