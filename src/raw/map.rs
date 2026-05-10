use core::ops::RangeFull;

use ribbit::Atomic;

use crate::raw::Cursor;
use crate::raw::Edge;
use crate::raw::Key;
use crate::raw::cursor;
use crate::raw::iter;
use crate::raw::iter::PostorderIter;

#[repr(transparent)]
pub(crate) struct Map<K: Key>(Atomic<Edge<K::Edge>>);

impl<K: Key> Map<K> {
    pub(crate) fn postorder<'g>(&'g mut self) -> PostorderIter<'g, K::Edge> {
        unsafe { PostorderIter::new(self.root()) }
    }

    #[inline]
    pub(crate) unsafe fn cursor<'k, P: cursor::Path<K::Read<'k>>>(
        &self,
        key: impl Into<K::Read<'k>>,
    ) -> Cursor<K::Read<'k>, P> {
        unsafe { Cursor::<_, P>::new(self.root(), key.into()) }
    }

    #[inline]
    pub unsafe fn all(&self) -> iter::Prefix<'static, '_, K, RangeFull> {
        unsafe { iter::Prefix::<K>::new_all(self.root()) }
    }

    #[inline]
    pub unsafe fn prefix<'k>(
        &self,
        prefix: impl Into<K::Read<'k>>,
    ) -> Option<iter::Prefix<'k, '_, K, RangeFull>> {
        unsafe { iter::Prefix::<K>::new_prefix(self.root(), prefix.into()) }
    }

    #[inline]
    pub fn range<'k, R>(&self, range: R, prefix: K::Read<'k>) -> Option<iter::Prefix<'k, '_, K, R>>
    where
        R: iter::Range<K::Read<'k>>,
    {
        unsafe { iter::Prefix::new_range(self.root(), range, prefix) }
    }

    #[inline]
    fn root(&self) -> &Atomic<Edge<K::Edge>> {
        &self.0
    }
}

impl<K> Default for Map<K>
where
    K: Key,
{
    #[inline]
    fn default() -> Self {
        Self(Atomic::new_packed(Edge::DEFAULT))
    }
}
