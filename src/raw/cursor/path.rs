use core::convert::Infallible;
use core::ptr::NonNull;

use ribbit::Atomic;

use crate::raw::Edge;
use crate::raw::Key;
use crate::raw::edge;
use crate::raw::key::Read as _;
use crate::raw::node;

/// A path along the tree is composed of 0 or more path segments.
pub(crate) struct Segment<'k, K: Key> {
    /// Key before matching on `edge`
    pub(super) key: K::Read<'k>,

    /// Edge to match
    pub(super) edge: NonNull<Atomic<Edge<K::Edge>>>,

    /// Number of bytes matched along `edge`
    pub(super) len: <<<K::Edge as ribbit::Pack>::Packed as edge::Meta>::Key as edge::Key>::Len,

    /// Node underneath `edge`
    pub(super) node: ribbit::Packed<node::Ptr<K::Edge>>,
}

pub(crate) trait History<'k, K>: Default
where
    K: Key,
{
    type PopError;

    fn trim(&mut self, bits: usize);

    fn push(&mut self, segment: Segment<'k, K>);
    fn pop(&mut self) -> Result<Option<Segment<'k, K>>, Self::PopError>;
}

#[derive(Default)]
pub(crate) struct Discard;

impl<'k, K> History<'k, K> for Discard
where
    K: Key,
{
    type PopError = ();

    #[inline]
    fn trim(&mut self, _: usize) {}

    #[inline]
    fn push(&mut self, _segment: Segment<'k, K>) {}

    #[inline]
    fn pop(&mut self) -> Result<Option<Segment<'k, K>>, Self::PopError> {
        Err(())
    }
}

pub(crate) struct Retain<'k, K: Key>(Vec<Segment<'k, K>>);

impl<'k, K> History<'k, K> for Retain<'k, K>
where
    K: Key,
{
    type PopError = Infallible;

    #[inline]
    fn trim(&mut self, bits: usize) {
        self.0.iter_mut().for_each(|segment| segment.key.trim(bits))
    }

    #[inline]
    fn push(&mut self, segment: Segment<'k, K>) {
        self.0.push(segment);
    }

    #[inline]
    fn pop(&mut self) -> Result<Option<Segment<'k, K>>, Self::PopError> {
        Ok(self.0.pop())
    }
}

impl<'k, K: Key> Default for Retain<'k, K> {
    fn default() -> Self {
        Self(Vec::new())
    }
}
