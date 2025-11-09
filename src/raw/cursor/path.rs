use core::convert::Infallible;

use ribbit::atomic::Atomic128;

use crate::raw::edge;
use crate::raw::node;
use crate::raw::Edge;
use crate::raw::Key;

/// A path along the tree is composed of 0 or more path segments.
pub(crate) struct Segment<'g, 'k, K: Key> {
    /// Key before matching on `edge`
    pub(super) key: K::Read<'k>,

    /// Edge to match
    pub(super) edge: &'g Atomic128<Edge<K::Edge>>,

    /// Number of bytes matched along `edge`
    pub(super) len: <K::Edge as edge::Meta>::Len,

    /// Node underneath `edge`
    pub(super) node: node::Ref<'g, K::Edge>,
}

pub(crate) trait History<'g, 'k, K>
where
    K: Key,
{
    type PopError;

    fn new(root: &'g Atomic128<Edge<K::Edge>>, key: K::Read<'k>) -> Self;
    fn push(&mut self, segment: Segment<'g, 'k, K>);
    fn pop(&mut self) -> Result<Option<Segment<'g, 'k, K>>, Self::PopError>;
}

pub(crate) struct Discard;

impl<'g, 'k, K> History<'g, 'k, K> for Discard
where
    K: Key,
{
    type PopError = ();

    fn new(_root: &'g Atomic128<Edge<K::Edge>>, _key: K::Read<'k>) -> Self {
        Self
    }

    #[inline]
    fn push(&mut self, _segment: Segment<'g, 'k, K>) {}

    #[inline]
    fn pop(&mut self) -> Result<Option<Segment<'g, 'k, K>>, Self::PopError> {
        Err(())
    }
}

pub(crate) struct Retain<'g, 'k, K: Key> {
    path: Vec<Segment<'g, 'k, K>>,
}

impl<'g, 'k, K> History<'g, 'k, K> for Retain<'g, 'k, K>
where
    K: Key,
{
    type PopError = Infallible;

    fn new(_root: &'g Atomic128<Edge<K::Edge>>, _key: K::Read<'k>) -> Self {
        Self { path: Vec::new() }
    }

    #[inline]
    fn push(&mut self, segment: Segment<'g, 'k, K>) {
        self.path.push(segment);
    }

    #[inline]
    fn pop(&mut self) -> Result<Option<Segment<'g, 'k, K>>, Self::PopError> {
        Ok(self.path.pop())
    }
}

pub(crate) enum Hybrid<'g, 'k, K: Key> {
    Discard { root: &'g Atomic128<Edge<K::Edge>> },
    Retain(Retain<'g, 'k, K>),
}

impl<'g, 'k, K> History<'g, 'k, K> for Hybrid<'g, 'k, K>
where
    K: Key,
{
    type PopError = ();

    fn new(root: &'g Atomic128<Edge<K::Edge>>, _key: K::Read<'k>) -> Self {
        Self::Discard { root }
    }

    #[inline]
    fn push(&mut self, segment: Segment<'g, 'k, K>) {
        match self {
            Self::Discard { .. } => (),
            Self::Retain(retain) => retain.push(segment),
        }
    }

    #[inline]
    fn pop(&mut self) -> Result<Option<Segment<'g, 'k, K>>, Self::PopError> {
        match self {
            Self::Discard { .. } => Err(()),
            Self::Retain(retain) => Ok(retain.pop().unwrap()),
        }
    }
}
