use core::convert::Infallible;

use ribbit::atomic::Atomic128;

use crate::raw::edge;
use crate::raw::node;
use crate::raw::Edge;

/// A path along the tree is composed of 0 or more path segments.
pub(crate) struct Segment<'g, R, M: edge::Meta> {
    /// Key before matching on `edge`
    pub(super) key: R,

    /// Edge to match
    pub(super) edge: &'g Atomic128<Edge<M>>,

    /// Number of bytes matched along `edge`
    pub(super) len: M::Len,

    /// Node underneath `edge`
    pub(super) node: node::Ref<'g, M>,
}

pub(crate) trait History<'g, R, M>
where
    M: edge::Meta,
{
    type PopError;

    fn new(root: &'g Atomic128<Edge<M>>, key: R) -> Self;
    fn push(&mut self, segment: Segment<'g, R, M>);
    fn pop(&mut self) -> Result<Option<Segment<'g, R, M>>, Self::PopError>;
}

pub(crate) struct Discard;

impl<'g, R, M> History<'g, R, M> for Discard
where
    M: edge::Meta,
{
    type PopError = ();

    fn new(_root: &'g Atomic128<Edge<M>>, _key: R) -> Self {
        Self
    }

    #[inline]
    fn push(&mut self, _segment: Segment<'g, R, M>) {}

    #[inline]
    fn pop(&mut self) -> Result<Option<Segment<'g, R, M>>, Self::PopError> {
        Err(())
    }
}

pub(crate) struct Retain<'g, R, M: edge::Meta> {
    path: Vec<Segment<'g, R, M>>,
}

impl<'g, R, M> History<'g, R, M> for Retain<'g, R, M>
where
    M: edge::Meta,
{
    type PopError = Infallible;

    fn new(_root: &'g Atomic128<Edge<M>>, _key: R) -> Self {
        Self { path: Vec::new() }
    }

    #[inline]
    fn push(&mut self, segment: Segment<'g, R, M>) {
        self.path.push(segment);
    }

    #[inline]
    fn pop(&mut self) -> Result<Option<Segment<'g, R, M>>, Self::PopError> {
        Ok(self.path.pop())
    }
}

pub(crate) enum Hybrid<'g, R, M: edge::Meta> {
    Discard { root: &'g Atomic128<Edge<M>> },
    Retain(Retain<'g, R, M>),
}

impl<'g, R: Copy, M> History<'g, R, M> for Hybrid<'g, R, M>
where
    M: edge::Meta,
{
    type PopError = ();

    fn new(root: &'g Atomic128<Edge<M>>, _key: R) -> Self {
        Self::Discard { root }
    }

    #[inline]
    fn push(&mut self, segment: Segment<'g, R, M>) {
        match self {
            Self::Discard { .. } => (),
            Self::Retain(retain) => retain.push(segment),
        }
    }

    #[inline]
    fn pop(&mut self) -> Result<Option<Segment<'g, R, M>>, Self::PopError> {
        match self {
            Self::Discard { .. } => Err(()),
            Self::Retain(retain) => Ok(retain.pop().unwrap()),
        }
    }
}
