use core::marker::PhantomData;
use core::sync::atomic::Ordering;

use ribbit::atomic::Atomic128;

use crate::iter::Or;
use crate::node;
use crate::node::UnsortedIter;
use crate::Edge;

pub(crate) struct PostorderIter<'g, V, S>
where
    S: Selector,
{
    stack: Vec<RepeatIter<'g, V>>,
    _selector: PhantomData<S>,
}

impl<'g, V, S: Selector> PostorderIter<'g, V, S> {
    #[inline]
    pub(crate) unsafe fn new(root: &Atomic128<Edge<V>>) -> Self {
        // HACK: we're masquerading as a node here--this is okay
        // since this iterator doesn't keep track of the key state,
        // so we can use an arbitrary byte.
        let iter = Or::L(Or::L([0u8; 4].into_iter().take(1)));
        Self {
            stack: vec![RepeatIter::new(unsafe {
                UnsortedIter::new(iter, core::slice::from_ref(root))
            })],
            _selector: PhantomData,
        }
    }

    #[inline]
    pub(crate) fn for_each<F: FnMut(S::Item<V>)>(mut self, mut apply: F) {
        'vertical: loop {
            let depth = self.stack.len().saturating_sub(1);

            let Some(iter) = self.stack.last_mut() else {
                return;
            };

            loop {
                let Some((first, edge)) = iter.next() else {
                    self.stack.pop();
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
                        self.stack.push(RepeatIter::new(node.iter_unsorted()));
                        continue 'vertical;
                    }
                }

                if let Some(item) = S::select(edge, depth) {
                    apply(item);
                }
            }
        }
    }
}

pub trait Selector {
    type Item<V>;
    fn select<V>(edge: ribbit::Packed<Edge<V>>, depth: usize) -> Option<Self::Item<V>>;
}

pub struct SelectNode;

impl Selector for SelectNode {
    type Item<V> = ribbit::Packed<Edge<V>>;

    #[inline]
    fn select<V>(edge: ribbit::Packed<Edge<V>>, _depth: usize) -> Option<Self::Item<V>> {
        edge.is_node().then_some(edge)
    }
}

pub struct SelectNonNull;

impl Selector for SelectNonNull {
    type Item<V> = ribbit::Packed<Edge<V>>;

    #[inline]
    fn select<V>(edge: ribbit::Packed<Edge<V>>, _depth: usize) -> Option<Self::Item<V>> {
        (!edge.is_null()).then_some(edge)
    }
}

pub(crate) struct SelectStat;

impl Selector for SelectStat {
    type Item<V> = (ribbit::Packed<Edge<V>>, usize);

    #[inline]
    fn select<V>(edge: ribbit::Packed<Edge<V>>, depth: usize) -> Option<Self::Item<V>> {
        (!edge.is_null()).then_some((edge, depth))
    }
}

struct RepeatIter<'g, V> {
    first: bool,
    edge: ribbit::Packed<Edge<V>>,
    iter: node::UnsortedIter<'g, V>,
}

impl<'g, V> RepeatIter<'g, V> {
    #[inline]
    fn new(iter: node::UnsortedIter<'g, V>) -> Self {
        Self {
            first: true,
            edge: Edge::DEFAULT,
            iter,
        }
    }
}

impl<V> RepeatIter<'_, V> {
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
