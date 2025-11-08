use core::sync::atomic::Ordering;

use ribbit::atomic::Atomic128;

use crate::iter::Unsorted;
use crate::raw::edge;
use crate::raw::iter::Unbound;
use crate::raw::node;
use crate::raw::Edge;

pub(crate) struct PostorderIter<'g, C> {
    stack: Vec<RepeatIter<'g, C>>,
}

impl<'g, C> PostorderIter<'g, C> {
    #[inline]
    pub(crate) unsafe fn new(root: &'g Atomic128<Edge<C>>) -> Self {
        // HACK: we're masquerading as a node here--this is okay
        // since this iterator doesn't keep track of the key state,
        // so we can use an arbitrary byte.
        Self {
            stack: vec![RepeatIter::new(unsafe {
                node::NodeIter::new(
                    Unbound,
                    Unbound,
                    node::KeyIter::ROOT,
                    core::slice::from_ref(root),
                )
            })],
        }
    }

    #[inline]
    pub(crate) fn for_each<F: FnMut(ribbit::Packed<Edge<C>>, usize)>(mut self, mut apply: F) {
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

                if first {
                    match edge.child() {
                        // Fall through for non-nodes
                        None | Some(edge::Child::Value(_)) => {
                            iter.skip();
                        }
                        // Visit children before node
                        Some(edge::Child::Node(node)) => {
                            let node = unsafe { node.into_ref_unchecked() };
                            self.stack.push(RepeatIter::new(
                                node.iter::<Unsorted, _, _>(Unbound, Unbound),
                            ));
                            continue 'vertical;
                        }
                    }
                }

                apply(edge, depth);
            }
        }
    }
}

struct RepeatIter<'g, C> {
    first: bool,
    edge: ribbit::Packed<Edge<C>>,
    iter: node::NodeIter<'g, Unbound, Unbound, C>,
}

impl<'g, C> RepeatIter<'g, C> {
    #[inline]
    fn new(iter: node::NodeIter<'g, Unbound, Unbound, C>) -> Self {
        Self {
            first: true,
            edge: Edge::DEFAULT,
            iter,
        }
    }
}

impl<C> RepeatIter<'_, C> {
    #[inline]
    fn next(&mut self) -> Option<(bool, ribbit::Packed<Edge<C>>)> {
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
