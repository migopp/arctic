use core::convert::Infallible;

use ribbit::Atomic;

use crate::raw::edge;
use crate::raw::Edge;
use crate::raw::Key;

/// A path along the tree is composed of 0 or more path segments.
pub(crate) struct Segment<'k, 'g, K: Key> {
    /// Key before matching on `edge`
    pub(super) key: K::Read<'k>,

    /// Edge to match
    pub(super) edge: &'g Atomic<Edge<K::Edge>>,

    /// Number of bytes matched along `edge`
    pub(super) len: <<K::Edge as ribbit::Pack>::Packed as edge::Meta>::Len,

    /// Node underneath `edge`
    pub(super) node: ribbit::Packed<edge::Ptr<K::Edge>>,
}

pub(crate) trait History<'k, 'g, K>
where
    K: Key,
{
    type PopError;

    fn new(root: &'g Atomic<Edge<K::Edge>>, key: K::Read<'k>) -> Self;
    fn push(&mut self, segment: Segment<'k, 'g, K>);
    fn pop(&mut self) -> Result<Option<Segment<'k, 'g, K>>, Self::PopError>;
}

pub(crate) struct Discard;

impl<'k, 'g, K> History<'k, 'g, K> for Discard
where
    K: Key,
{
    type PopError = ();

    fn new(_root: &'g Atomic<Edge<K::Edge>>, _key: K::Read<'k>) -> Self {
        Self
    }

    #[inline]
    fn push(&mut self, _segment: Segment<'k, 'g, K>) {}

    #[inline]
    fn pop(&mut self) -> Result<Option<Segment<'k, 'g, K>>, Self::PopError> {
        Err(())
    }
}

pub(crate) struct Retain<'k, 'g, K: Key> {
    path: Vec<Segment<'k, 'g, K>>,
}

impl<'k, 'g, K> History<'k, 'g, K> for Retain<'k, 'g, K>
where
    K: Key,
{
    type PopError = Infallible;

    fn new(_root: &'g Atomic<Edge<K::Edge>>, _key: K::Read<'k>) -> Self {
        Self { path: Vec::new() }
    }

    #[inline]
    fn push(&mut self, segment: Segment<'k, 'g, K>) {
        self.path.push(segment);
    }

    #[inline]
    fn pop(&mut self) -> Result<Option<Segment<'k, 'g, K>>, Self::PopError> {
        Ok(self.path.pop())
    }
}

pub(crate) enum Hybrid<'k, 'g, K: Key> {
    Discard { root: &'g Atomic<Edge<K::Edge>> },
    Retain(Retain<'k, 'g, K>),
}

impl<'k, 'g, K> History<'k, 'g, K> for Hybrid<'k, 'g, K>
where
    K: Key,
{
    type PopError = ();

    fn new(root: &'g Atomic<Edge<K::Edge>>, _key: K::Read<'k>) -> Self {
        Self::Discard { root }
    }

    #[inline]
    fn push(&mut self, segment: Segment<'k, 'g, K>) {
        match self {
            Self::Discard { .. } => (),
            Self::Retain(retain) => retain.push(segment),
        }
    }

    #[inline]
    fn pop(&mut self) -> Result<Option<Segment<'k, 'g, K>>, Self::PopError> {
        match self {
            Self::Discard { .. } => Err(()),
            Self::Retain(retain) => Ok(retain.pop().unwrap()),
        }
    }
}
