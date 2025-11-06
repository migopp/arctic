pub(crate) mod cursor;
pub(crate) mod edge;
pub(crate) mod node;

pub(crate) use edge::Edge;
pub(crate) use node::Node;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum Op {
    Node(node::Op),
    Edge(edge::Op),
}

impl Op {
    /// Whether this operation allocates a new node.
    #[inline]
    pub fn is_allocate(self) -> bool {
        match self {
            Self::Node(node) => node.is_allocate(),
            Self::Edge(edge) => edge.is_allocate(),
        }
    }

    /// Whether this operation retires an old node.
    #[inline]
    pub fn is_retire(self) -> bool {
        matches!(self, Self::Node(_))
    }
}
