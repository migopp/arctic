use core::sync::atomic::Ordering;

use ribbit::atomic::Atomic128;

use crate::key;
use crate::Edge;

pub(crate) enum LeafIter<'a, W: key::Write, S: crate::iter::Sort> {
    Root {
        key: W,
        next: Option<u64>,
    },
    Node {
        key: W,
        frontier: Vec<(W::Len, S::Iter<'a>)>,
    },
}

impl<'a, W, S> LeafIter<'a, W, S>
where
    W: key::Write,
    S: crate::iter::Sort,
{
    #[inline]
    pub(crate) fn empty() -> Self {
        Self::Root {
            key: W::default(),
            next: None,
        }
    }

    #[inline]
    pub(crate) unsafe fn new(root: &'a Atomic128<Edge>, mut key: W) -> Self {
        let edge = root.load_packed(Ordering::Acquire);
        let meta = edge.meta();
        let data = edge.data();

        key.extend(edge.meta().key());

        if meta.leaf() {
            Self::Root {
                key,
                next: Some(data.into_leaf()),
            }
        } else if data.is_null() {
            Self::Root { key, next: None }
        } else {
            let node = unsafe { data.into_node_unchecked() };
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

                if !meta.leaf() && data.is_null() {
                    continue;
                }

                key.truncate(*len);
                key.push(byte);
                key.extend(meta.key());

                if meta.leaf() {
                    return Some((key, data.into_leaf()));
                } else {
                    let node = unsafe { data.into_node_unchecked() };
                    frontier.push((key.bits(), unsafe { S::new(node) }));
                    continue 'vertical;
                }
            }
        }
    }
}
