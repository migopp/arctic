use core::cell::Cell;
use core::marker::PhantomData;

use ribbit::atomic::Atomic128;

use crate::key;
use crate::raw::cursor;
use crate::raw::iter;
use crate::raw::Cursor;
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
        unsafe { iter::LeafIter::new(&self.root, W::default()) }
    }

    pub(crate) fn postorder<'a, S: iter::postorder::Selector>(
        &'a self,
    ) -> iter::PostorderIter<'a, S> {
        unsafe { iter::PostorderIter::new(&self.root) }
    }

    pub(super) fn traverse_prefix<'a, R: key::Read>(
        &'a self,
        prefix: R,
    ) -> Cursor<'a, R, cursor::Optimistic<R>> {
        let mut cursor = Cursor::new(prefix, &self.root);
        cursor.traverse_prefix();
        cursor
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
