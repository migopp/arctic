use core::sync::atomic::Ordering;

use ribbit::atomic::Atomic128;

use crate::key;
use crate::node;
use crate::Edge;

pub enum PostorderIter<'a, W, V>
where
    V: Selector<W>,
{
    Root {
        key: W,
        next: Option<V::Item>,
    },
    Node {
        key: W,
        #[allow(private_interfaces)]
        frontier: Vec<(usize, RepeatIter<'a>)>,
    },
}

impl<'a, W: key::Write, V: Selector<W>> PostorderIter<'a, W, V> {
    #[inline]
    pub(crate) unsafe fn new(root: &Atomic128<Edge>, mut key: W) -> Self {
        let edge = root.load_packed(Ordering::Acquire);
        let meta = edge.meta();
        let data = edge.data();

        key.extend(edge.meta().key());

        if meta.leaf() {
            let next = V::select(edge, &key, 0);
            Self::Root { key, next }
        } else if data == 0 {
            Self::Root { key, next: None }
        } else {
            let node = unsafe { Edge::next_node_unchecked(data) };
            Self::Node {
                frontier: vec![(key.bits(), RepeatIter::new(node))],
                key,
            }
        }
    }

    #[inline]
    pub fn lend(&mut self) -> Option<(&W, V::Item)> {
        let (key, frontier) = match self {
            PostorderIter::Root { key, next } => {
                crate::cold();
                let value = next.take()?;
                return Some((key, value));
            }
            PostorderIter::Node { key, frontier } => (key, frontier),
        };

        'vertical: loop {
            let depth = frontier.len();
            let (len, iter) = frontier.last_mut()?;

            loop {
                let Some((first, byte, edge)) = iter.next() else {
                    frontier.pop();
                    continue 'vertical;
                };

                let meta = edge.meta();
                let data = edge.data();

                if !meta.leaf() && data == 0 {
                    iter.skip();
                    continue;
                }

                if first {
                    key.truncate(*len);
                    key.push(byte);
                    key.extend(meta.key());

                    if !meta.leaf() {
                        let node = unsafe { Edge::next_node_unchecked(data) };
                        frontier.push((key.bits(), unsafe { RepeatIter::new(node) }));
                        continue 'vertical;
                    }
                }

                // Second visit (or fallthrough)
                iter.skip();

                if let Some(item) = V::select(edge, key, depth) {
                    return Some((key, item));
                }
            }
        }
    }
}

pub(crate) trait Selector<W> {
    type Item;
    fn select(edge: ribbit::Packed<Edge>, key: &W, depth: usize) -> Option<Self::Item>;
}

pub(crate) struct SelectNode;

impl<W: key::Write> Selector<W> for SelectNode {
    type Item = ribbit::Packed<Edge>;

    #[inline]
    fn select(edge: ribbit::Packed<Edge>, _key: &W, _depth: usize) -> Option<Self::Item> {
        (!edge.meta().leaf() && edge.data() > 0).then_some(edge)
    }
}

pub(crate) struct SelectAll;

impl<W: key::Write> Selector<W> for SelectAll {
    type Item = (ribbit::Packed<Edge>, usize);

    #[inline]
    fn select(edge: ribbit::Packed<Edge>, _key: &W, depth: usize) -> Option<Self::Item> {
        (edge.meta().leaf() || edge.data() > 0).then_some((edge, depth))
    }
}

struct RepeatIter<'a> {
    first: bool,
    key: u8,
    edge: ribbit::Packed<Edge>,
    iter: node::UnsortedIter<'a>,
}

impl<'a> RepeatIter<'a> {
    #[inline]
    unsafe fn new(node: node::Ref<'a>) -> Self {
        Self {
            first: true,
            key: 0,
            edge: Edge::DEFAULT,
            iter: node.iter_unsorted(),
        }
    }
}

impl<'a> RepeatIter<'a> {
    #[inline]
    fn next(&mut self) -> Option<(bool, u8, ribbit::Packed<Edge>)> {
        let first = self.first;
        self.first ^= true;

        if first {
            let (key, edge) = self.iter.next()?;
            let edge = edge.load_packed(Ordering::Acquire);
            self.key = key;
            self.edge = edge;
        }

        Some((first, self.key, self.edge))
    }

    #[inline]
    fn skip(&mut self) {
        self.first = true;
    }
}
