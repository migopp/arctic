pub(crate) mod concurrent;
pub(crate) mod iter;
pub(crate) mod sequential;

use crate::edge;
use crate::node;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum Op {
    Node(node::Op),
    Edge(edge::Op),
}
