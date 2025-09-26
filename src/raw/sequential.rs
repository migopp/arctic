use ribbit::atomic::Atomic128;

use crate::byte;
use crate::raw::iter;
use crate::Edge;

#[derive(Default)]
pub(crate) struct Raw {
    root: Atomic128<Edge>,
}

impl Raw {
    pub(crate) fn root(&self) -> &Atomic128<Edge> {
        &self.root
    }

    pub(crate) fn preorder<K: byte::Stack, S: iter::Selector>(&mut self) -> iter::EntryIter<K, S> {
        iter::EntryIter::new(&mut self.root)
    }
}
