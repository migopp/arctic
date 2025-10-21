use core::cell::Cell;
use core::marker::PhantomData;

use ribbit::atomic::Atomic128;

use crate::key;
use crate::raw::iter;
use crate::stat;
use crate::Edge;

#[repr(transparent)]
pub(crate) struct Map<V> {
    root: Atomic128<Edge>,
    _not_sync: PhantomData<Cell<()>>,
    _value: PhantomData<V>,
}

impl<V> Default for Map<V> {
    fn default() -> Self {
        Self {
            root: Atomic128::default(),
            _not_sync: PhantomData,
            _value: PhantomData,
        }
    }
}

impl<V> Map<V> {
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
        unsafe { iter::LeafIter::new(&self.root, W::default()) }
    }

    pub(crate) fn postorder<'a, S: iter::postorder::Selector>(
        &'a self,
    ) -> iter::PostorderIter<'a, S> {
        unsafe { iter::PostorderIter::new(&self.root) }
    }
}

impl<V> Drop for Map<V> {
    fn drop(&mut self) {
        self.postorder::<iter::postorder::SelectNode>()
            .for_each(|edge| unsafe {
                edge.data().deallocate_unchecked(stat::Counter::FreeDrop);
            })
    }
}
