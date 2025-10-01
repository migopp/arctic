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
    pub(crate) fn get<K: key::Iterator>(&self, key: K) -> Option<u64> {
        todo!()
    }

    #[expect(unused_variables)]
    #[inline]
    pub(crate) fn insert<K: key::Iterator>(&mut self, key: K, value: u64) -> Option<u64> {
        todo!()
    }

    #[expect(unused_variables)]
    #[inline]
    pub(crate) fn remove<K: key::Iterator>(&mut self, key: K) -> Option<u64> {
        todo!()
    }

    #[expect(unused_variables)]
    #[inline]
    pub(crate) fn update<K: key::Iterator>(&mut self, key: K, value: u64) -> Option<u64> {
        todo!()
    }

    pub(crate) fn iter<
        'a,
        K: key::Stack,
        V: iter::Selector<K>,
        O: iter::Order,
        S: iter::Sort<'a>,
    >(
        &'a self,
        selector: V,
    ) -> iter::Iter<'a, K, V, O, S> {
        unsafe { iter::Iter::new(&self.root, K::default(), selector) }
    }
}

impl Drop for Map {
    fn drop(&mut self) {
        let mut iter = self
            .iter::<key::Ignore, iter::SelectNode, iter::Postorder, node::UnsortedIter>(
                iter::SelectNode,
            );
        while let Some((key::Ignore, edge)) = iter.lend() {
            unsafe {
                Edge::deallocate(edge, stat::Counter::FreeDrop);
            }
        }
    }
}
