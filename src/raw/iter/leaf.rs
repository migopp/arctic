use core::sync::atomic::Ordering;

use ribbit::atomic::Atomic128;

use crate::key;
use crate::node;
use crate::Edge;

pub(crate) enum LeafIter<W: key::Write, S> {
    Root { key: W, next: Option<u64> },
    Node { key: W, frontier: Vec<(W::Len, S)> },
}

impl<'a, W, S> LeafIter<W, S>
where
    W: key::Write,
    S: Sort<'a>,
{
    #[inline]
    pub(crate) unsafe fn new(root: &Atomic128<Edge>, mut key: W) -> Self {
        let edge = root.load_packed(Ordering::Acquire);
        let meta = edge.meta();
        let data = edge.data();

        key.extend(edge.meta().key());

        if meta.leaf() {
            Self::Root {
                key,
                next: Some(edge.data()),
            }
        } else if data == 0 {
            Self::Root { key, next: None }
        } else {
            let node = unsafe { Edge::next_node_unchecked(data) };
            Self::Node {
                frontier: vec![(key.bits(), S::new(node))],
                key,
            }
        }
    }

    #[inline]
    pub(crate) fn lend(&mut self) -> Option<(&W, u64)> {
        let (key, frontier) = match self {
            Self::Root { key, next } => {
                crate::cold();
                let value = next.take()?;
                return Some((key, value));
            }
            Self::Node { key, frontier } => (key, frontier),
        };

        'vertical: loop {
            let (len, iter) = frontier.last_mut()?;

            loop {
                let Some((byte, edge)) = iter.next() else {
                    frontier.pop();
                    continue 'vertical;
                };

                let edge = edge.load_packed(Ordering::Acquire);
                let meta = edge.meta();
                let data = edge.data();

                if !meta.leaf() && data == 0 {
                    continue;
                }

                key.truncate(*len);
                key.push(byte);
                key.extend(meta.key());

                if meta.leaf() {
                    return Some((key, edge.data()));
                } else {
                    let node = unsafe { Edge::next_node_unchecked(data) };
                    frontier.push((key.bits(), unsafe { S::new(node) }));
                    continue 'vertical;
                }
            }
        }
    }
}

pub(crate) trait Sort<'a>: Iterator<Item = (u8, &'a Atomic128<Edge>)> {
    unsafe fn new(node: node::Ref<'a>) -> Self;
}

impl<'a> Sort<'a> for node::SortedIter<'a> {
    unsafe fn new(node: node::Ref<'a>) -> Self {
        node.iter_sorted()
    }
}

impl<'a> Sort<'a> for node::UnsortedIter<'a> {
    unsafe fn new(node: node::Ref<'a>) -> Self {
        node.iter_unsorted()
    }
}
