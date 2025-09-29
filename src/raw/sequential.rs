use ribbit::atomic::Atomic128;

use crate::key;
use crate::node;
use crate::raw::iter;
use crate::stat;
use crate::Edge;

#[derive(Default)]
pub(crate) struct Map {
    root: Atomic128<Edge>,
}

impl Map {
    pub(crate) fn root(&self) -> &Atomic128<Edge> {
        &self.root
    }

    #[expect(unused_variables)]
    #[inline]
    pub(crate) fn get<K: key::Iterator>(&mut self, key: K) -> Option<u64> {
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

    pub(crate) fn iter<'a, K: key::Stack, V: iter::Selector, O: iter::Order, S: iter::Sort<'a>>(
        &'a self,
    ) -> iter::Iter<'a, K, V, O, S> {
        unsafe { iter::Iter::new(&self.root) }
    }
}

impl Drop for Map {
    fn drop(&mut self) {
        let mut iter =
            self.iter::<key::Ignore, iter::SelectNode, iter::Postorder, node::UnsortedIter>();
        while let Some((key::Ignore, edge)) = iter.next() {
            unsafe {
                Edge::deallocate(edge, stat::Counter::FreeDrop);
            }
        }
    }
}
