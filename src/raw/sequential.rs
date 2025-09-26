use ribbit::atomic::Atomic128;

use crate::byte;
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

    pub(crate) fn iter<K: byte::Stack, S: iter::Selector, O: iter::Order>(
        &mut self,
    ) -> iter::Iter<K, S, O> {
        iter::Iter::new(&mut self.root)
    }
}

impl Drop for Map {
    fn drop(&mut self) {
        let mut iter = self.iter::<byte::Ignore, iter::SelectNode, iter::Postorder>();
        while let Some((byte::Ignore, edge)) = iter.next() {
            unsafe {
                Edge::deallocate(edge, stat::Counter::FreeDrop);
            }
        }
    }
}
