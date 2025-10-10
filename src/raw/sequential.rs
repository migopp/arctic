use core::cell::Cell;
use core::marker::PhantomData;
use core::sync::atomic::Ordering;

use ribbit::atomic::Atomic128;

use crate::key;
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

    pub(crate) fn iter<'a, W: key::Write, S: crate::iter::Sort>(
        &'a self,
    ) -> iter::LeafIter<'a, W, S> {
        let root = self.root.load_packed(Ordering::Relaxed);
        unsafe { iter::LeafIter::new(root, W::default()) }
    }

    pub(crate) fn postorder<'a, S: iter::postorder::Selector>(
        &'a self,
    ) -> iter::PostorderIter<'a, S> {
        unsafe { iter::PostorderIter::new(&self.root) }
    }
}

impl Drop for Map {
    fn drop(&mut self) {
        self.postorder::<iter::postorder::SelectNode>()
            .for_each(|edge| unsafe {
                Edge::deallocate_unchecked(edge, stat::Counter::FreeDrop);
            })
    }
}
