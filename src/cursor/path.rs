use core::convert::Infallible;

use ribbit::atomic::Atomic128;

use crate::byte;
use crate::node;
use crate::Edge;

/// A path along the tree is composed of 0 or more path segments.
pub(crate) struct Segment<'g, R, V> {
    /// Key before matching on `edge`
    pub(super) key: R,

    /// Edge to match
    pub(super) edge: &'g Atomic128<Edge<V>>,

    /// Number of bytes matched along `edge`
    pub(super) len: byte::Len,

    /// Node underneath `edge`
    pub(super) node: node::Ref<'g, V>,
}

pub(crate) trait History<'g, R, V> {
    type PopError;

    fn new(root: &'g Atomic128<Edge<V>>, key: R) -> Self;
    fn push(&mut self, segment: Segment<'g, R, V>);
    fn pop(&mut self) -> Result<Option<Segment<'g, R, V>>, Self::PopError>;
}

pub(crate) struct Discard;

impl<'g, R, V> History<'g, R, V> for Discard {
    type PopError = ();

    fn new(_root: &'g Atomic128<Edge<V>>, _key: R) -> Self {
        Self
    }

    #[inline]
    fn push(&mut self, _segment: Segment<'g, R, V>) {}

    #[inline]
    fn pop(&mut self) -> Result<Option<Segment<'g, R, V>>, Self::PopError> {
        Err(())
    }
}

pub(crate) struct Retain<'g, R, V> {
    path: Vec<Segment<'g, R, V>>,
}

impl<'g, R, V> History<'g, R, V> for Retain<'g, R, V> {
    type PopError = Infallible;

    fn new(_root: &'g Atomic128<Edge<V>>, _key: R) -> Self {
        Self { path: Vec::new() }
    }

    #[inline]
    fn push(&mut self, segment: Segment<'g, R, V>) {
        self.path.push(segment);
    }

    #[inline]
    fn pop(&mut self) -> Result<Option<Segment<'g, R, V>>, Self::PopError> {
        Ok(self.path.pop())
    }
}

pub(crate) enum Hybrid<'g, R, V> {
    Discard {
        key: R,
        root: &'g Atomic128<Edge<V>>,
    },
    Retain {
        key: R,
        retain: Retain<'g, R, V>,
    },
}

impl<'g, R: Copy, V> History<'g, R, V> for Hybrid<'g, R, V> {
    type PopError = ();

    fn new(root: &'g Atomic128<Edge<V>>, key: R) -> Self {
        Self::Discard { key, root }
    }

    #[inline]
    fn push(&mut self, segment: Segment<'g, R, V>) {
        match self {
            Self::Discard { .. } => (),
            Self::Retain { retain, .. } => retain.push(segment),
        }
    }

    #[inline]
    fn pop(&mut self) -> Result<Option<Segment<'g, R, V>>, Self::PopError> {
        match self {
            Self::Discard { .. } => Err(()),
            Self::Retain { retain, .. } => Ok(retain.pop().unwrap()),
        }
    }
}
