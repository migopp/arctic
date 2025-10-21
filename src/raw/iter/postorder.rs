use core::sync::atomic::Ordering;

use ribbit::atomic::Atomic128;

use crate::node;
use crate::Edge;

pub(crate) enum PostorderIter<'a, V, S>
where
    S: Selector<V>,
{
    Root(Option<S::Item>),
    Node(#[allow(private_interfaces)] Vec<RepeatIter<'a, V>>),
}

impl<'a, V: 'a, S: Selector<V>> PostorderIter<'a, V, S> {
    #[inline]
    pub(crate) unsafe fn new(root: &Atomic128<Edge<V>>) -> Self {
        let edge = root.load_packed(Ordering::Acquire);
        let meta = edge.meta();
        let data = edge.data();

        if meta.leaf() {
            let next = S::select(edge, 0);
            Self::Root(next)
        } else if data.is_null() {
            Self::Root(None)
        } else {
            let node = unsafe { data.into_node_unchecked() };
            Self::Node(vec![RepeatIter::new(node)])
        }
    }
}

impl<'a, V: 'a, S: Selector<V>> Iterator for PostorderIter<'a, V, S> {
    type Item = S::Item;

    #[inline]
    fn next(&mut self) -> Option<S::Item> {
        let frontier = match self {
            PostorderIter::Root(next) => {
                crate::cold();
                let value = next.take()?;
                return Some(value);
            }
            PostorderIter::Node(frontier) => frontier,
        };

        'vertical: loop {
            let depth = frontier.len();
            let iter = frontier.last_mut()?;

            loop {
                let Some((first, edge)) = iter.next() else {
                    frontier.pop();
                    continue 'vertical;
                };

                if edge.is_null() {
                    iter.skip();
                    continue;
                }

                let meta = edge.meta();
                let data = edge.data();

                if first {
                    // Fall through for leaf
                    if meta.leaf() {
                        iter.skip();
                    } else {
                        // Visit children before node
                        let node = unsafe { data.into_node_unchecked() };
                        frontier.push(unsafe { RepeatIter::new(node) });
                        continue 'vertical;
                    }
                }

                if let Some(item) = S::select(edge, depth) {
                    return Some(item);
                }
            }
        }
    }
}

pub(crate) trait Selector<V> {
    type Item;
    fn select(edge: ribbit::Packed<Edge<V>>, depth: usize) -> Option<Self::Item>;
}

pub(crate) struct SelectNode;

impl<V> Selector<V> for SelectNode {
    type Item = ribbit::Packed<Edge<V>>;

    #[inline]
    fn select(edge: ribbit::Packed<Edge<V>>, _depth: usize) -> Option<Self::Item> {
        edge.is_node().then_some(edge)
    }
}

pub(crate) struct SelectAll;

impl<V> Selector<V> for SelectAll {
    type Item = (ribbit::Packed<Edge<V>>, usize);

    #[inline]
    fn select(edge: ribbit::Packed<Edge<V>>, depth: usize) -> Option<Self::Item> {
        (!edge.is_null()).then_some((edge, depth))
    }
}

struct RepeatIter<'a, V> {
    first: bool,
    edge: ribbit::Packed<Edge<V>>,
    iter: node::UnsortedIter<'a, V>,
}

impl<'a, V> RepeatIter<'a, V> {
    #[inline]
    unsafe fn new(node: node::Ref<'a, V>) -> Self {
        Self {
            first: true,
            edge: Edge::DEFAULT,
            iter: node.iter_unsorted(),
        }
    }
}

impl<'a, V> RepeatIter<'a, V> {
    #[inline]
    fn next(&mut self) -> Option<(bool, ribbit::Packed<Edge<V>>)> {
        let first = self.first;
        self.first ^= true;

        if first {
            let (_, edge) = self.iter.next()?;
            let edge = edge.load_packed(Ordering::Acquire);
            self.edge = edge;
        }

        Some((first, self.edge))
    }

    #[inline]
    fn skip(&mut self) {
        self.first = true;
    }
}
