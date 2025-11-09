use core::convert::Infallible;

use ribbit::atomic::Atomic128;

use crate::raw::node;
use crate::raw::Edge;

/// A path along the tree is composed of 0 or more path segments.
pub(crate) struct Segment<'g, R, C> {
    /// Key before matching on `edge`
    pub(super) key: R,

    /// Edge to match
    pub(super) edge: &'g Atomic128<Edge<C>>,

    /// Number of bytes matched along `edge`
    pub(super) bits: usize,

    /// Node underneath `edge`
    pub(super) node: node::Ref<'g, C>,
}

pub(crate) trait History<'g, R, C> {
    type PopError;

    fn new(root: &'g Atomic128<Edge<C>>, key: R) -> Self;
    fn push(&mut self, segment: Segment<'g, R, C>);
    fn pop(&mut self) -> Result<Option<Segment<'g, R, C>>, Self::PopError>;
}

pub(crate) struct Discard;

impl<'g, R, C> History<'g, R, C> for Discard {
    type PopError = ();

    fn new(_root: &'g Atomic128<Edge<C>>, _key: R) -> Self {
        Self
    }

    #[inline]
    fn push(&mut self, _segment: Segment<'g, R, C>) {}

    #[inline]
    fn pop(&mut self) -> Result<Option<Segment<'g, R, C>>, Self::PopError> {
        Err(())
    }
}

pub(crate) struct Retain<'g, R, C> {
    path: Vec<Segment<'g, R, C>>,
}

impl<'g, R, C> History<'g, R, C> for Retain<'g, R, C> {
    type PopError = Infallible;

    fn new(_root: &'g Atomic128<Edge<C>>, _key: R) -> Self {
        Self { path: Vec::new() }
    }

    #[inline]
    fn push(&mut self, segment: Segment<'g, R, C>) {
        self.path.push(segment);
    }

    #[inline]
    fn pop(&mut self) -> Result<Option<Segment<'g, R, C>>, Self::PopError> {
        Ok(self.path.pop())
    }
}

pub(crate) enum Hybrid<'g, R, C> {
    Discard { root: &'g Atomic128<Edge<C>> },
    Retain(Retain<'g, R, C>),
}

impl<'g, R: Copy, C> History<'g, R, C> for Hybrid<'g, R, C> {
    type PopError = ();

    fn new(root: &'g Atomic128<Edge<C>>, _key: R) -> Self {
        Self::Discard { root }
    }

    #[inline]
    fn push(&mut self, segment: Segment<'g, R, C>) {
        match self {
            Self::Discard { .. } => (),
            Self::Retain(retain) => retain.push(segment),
        }
    }

    #[inline]
    fn pop(&mut self) -> Result<Option<Segment<'g, R, C>>, Self::PopError> {
        match self {
            Self::Discard { .. } => Err(()),
            Self::Retain(retain) => Ok(retain.pop().unwrap()),
        }
    }
}
