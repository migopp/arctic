use core::sync::atomic::Ordering;

use ribbit::atomic::Atomic128;

use crate::node;
use crate::Edge;

pub(crate) enum PostorderIter<'a, S>
where
    S: Selector,
{
    Root(Option<S::Item>),
    Node(#[allow(private_interfaces)] Vec<RepeatIter<'a>>),
}

impl<'a, S: Selector> PostorderIter<'a, S> {
    #[inline]
    pub(crate) unsafe fn new(root: &Atomic128<Edge>) -> Self {
        let edge = root.load_packed(Ordering::Acquire);
        let meta = edge.meta();
        let data = edge.data();

        if meta.leaf() {
            let next = S::select(edge, 0);
            Self::Root(next)
        } else if data == 0 {
            Self::Root(None)
        } else {
            let node = unsafe { Edge::next_node_unchecked(data) };
            Self::Node(vec![RepeatIter::new(node)])
        }
    }
}

impl<'a, S: Selector> Iterator for PostorderIter<'a, S> {
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

                let meta = edge.meta();
                let data = edge.data();

                if !meta.leaf() && data == 0 {
                    iter.skip();
                    continue;
                }

                if first && !meta.leaf() {
                    let node = unsafe { Edge::next_node_unchecked(data) };
                    frontier.push(unsafe { RepeatIter::new(node) });
                    continue 'vertical;
                }

                // Second visit (or fallthrough)
                iter.skip();

                if let Some(item) = S::select(edge, depth) {
                    return Some(item);
                }
            }
        }
    }
}

pub(crate) trait Selector {
    type Item;
    fn select(edge: ribbit::Packed<Edge>, depth: usize) -> Option<Self::Item>;
}

pub(crate) struct SelectNode;

impl Selector for SelectNode {
    type Item = ribbit::Packed<Edge>;

    #[inline]
    fn select(edge: ribbit::Packed<Edge>, _depth: usize) -> Option<Self::Item> {
        (!edge.meta().leaf() && edge.data() > 0).then_some(edge)
    }
}

pub(crate) struct SelectAll;

impl Selector for SelectAll {
    type Item = (ribbit::Packed<Edge>, usize);

    #[inline]
    fn select(edge: ribbit::Packed<Edge>, depth: usize) -> Option<Self::Item> {
        (edge.meta().leaf() || edge.data() > 0).then_some((edge, depth))
    }
}

struct RepeatIter<'a> {
    first: bool,
    edge: ribbit::Packed<Edge>,
    iter: node::UnsortedIter<'a>,
}

impl<'a> RepeatIter<'a> {
    #[inline]
    unsafe fn new(node: node::Ref<'a>) -> Self {
        Self {
            first: true,
            edge: Edge::DEFAULT,
            iter: node.iter_unsorted(),
        }
    }
}

impl<'a> RepeatIter<'a> {
    #[inline]
    fn next(&mut self) -> Option<(bool, ribbit::Packed<Edge>)> {
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
