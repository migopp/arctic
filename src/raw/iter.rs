use core::iter;
use core::marker::PhantomData;
use core::sync::atomic::Ordering;

use ribbit::atomic::Atomic128;

use crate::key;
use crate::node;
use crate::Edge;
use crate::Or;

pub struct Iter<'a, K, V, O, S>
where
    S: Iterator,
{
    key: K,
    _select: PhantomData<V>,
    _order: PhantomData<O>,
    _sort: PhantomData<S>,
    frontier: Vec<(usize, Or<RootIter<'a>, NodeIter<'a, S>>)>,
}

type RootIter<'a> = iter::Peekable<iter::Once<(Visit, &'a Atomic128<Edge>)>>;
type NodeIter<'a, S> = iter::Peekable<iter::Zip<iter::Repeat<Visit>, S>>;

impl<'a, K: key::Stack, V: Selector, O: Order, S: Sort<'a>> Iter<'a, K, V, O, S> {
    pub(crate) fn new(root: &'a mut Atomic128<Edge>) -> Self {
        Self {
            key: K::default(),
            _select: PhantomData,
            _order: PhantomData,
            _sort: PhantomData,
            frontier: vec![(0, Or::L(iter::once((O::START, &*root)).peekable()))],
        }
    }

    pub fn next(&mut self) -> Option<(&K, V::Item)> {
        'vertical: loop {
            // NOTE: we use `saturating_sub` to avoid underflow.
            //
            // If `self.frontier.len()` == 0, we will immediately return at `last_mut()`.
            // We can't move the len call after because `self.frontier` is mutably borrowed.
            let depth = self.frontier.len().saturating_sub(1);
            let (len, iter) = self.frontier.last_mut()?;

            'horizontal: loop {
                let Some((visit, byte, edge)) = (match iter {
                    Or::L(iter_root) => iter_root
                        .peek_mut()
                        .map(|(visit, edge)| (visit, None, edge)),
                    Or::R(iter_node) => iter_node
                        .peek_mut()
                        .map(|(visit, (key, edge))| (visit, Some(*key), edge)),
                }) else {
                    self.key.truncate(*len);
                    self.frontier.pop();
                    continue 'vertical;
                };

                let edge = edge.load_packed(Ordering::Relaxed);
                let meta = edge.meta();
                let kind = meta.kind();

                if kind == node::Kind::NONE {
                    iter.skip();
                    continue 'horizontal;
                }

                // First visit
                if *visit == O::START {
                    if let Some(byte) = byte {
                        self.key.push(byte);
                    }

                    self.key.extend(meta.key());
                }

                match O::step(visit) {
                    Visit::Yield => {
                        if let Some(item) = V::select(depth, edge) {
                            return Some((&self.key, item));
                        }
                    }
                    Visit::Descend if kind >= node::Kind::NODE_3 => {
                        let node = unsafe { Edge::next_node_unchecked(edge.data(), kind) };
                        self.frontier.push((
                            self.key.len(),
                            Or::R(iter::repeat(O::START).zip(S::new(node)).peekable()),
                        ));
                        continue 'vertical;
                    }
                    Visit::Descend => (),
                    Visit::Done => {
                        self.key.truncate(*len);
                        iter.skip();
                        continue 'horizontal;
                    }
                }
            }
        }
    }
}

pub(crate) trait Selector {
    type Item;
    fn select(depth: usize, edge: ribbit::Packed<Edge>) -> Option<Self::Item>;
}

pub(crate) struct SelectLeaf;

impl Selector for SelectLeaf {
    type Item = u64;

    #[inline]
    fn select(_depth: usize, edge: ribbit::Packed<Edge>) -> Option<Self::Item> {
        (edge.meta().kind() == node::Kind::LEAF).then(|| edge.data())
    }
}

pub(crate) struct SelectNode;

impl Selector for SelectNode {
    type Item = ribbit::Packed<Edge>;

    #[inline]
    fn select(_: usize, edge: ribbit::Packed<Edge>) -> Option<Self::Item> {
        (edge.meta().kind() >= node::Kind::NODE_3).then_some(edge)
    }
}

pub(crate) struct SelectAll;

impl Selector for SelectAll {
    type Item = (usize, ribbit::Packed<Edge>);

    #[inline]
    fn select(depth: usize, edge: ribbit::Packed<Edge>) -> Option<Self::Item> {
        Some((depth, edge))
    }
}

pub(crate) trait Order {
    const START: Visit;
    fn step(state: &mut Visit) -> Visit;
}

pub(crate) struct Preorder;

impl Order for Preorder {
    const START: Visit = Visit::Yield;
    fn step(state: &mut Visit) -> Visit {
        let visit = *state;
        *state = match state {
            Visit::Yield => Visit::Descend,
            Visit::Descend => Visit::Done,
            Visit::Done => Visit::Done,
        };
        visit
    }
}

pub(crate) struct Postorder;

impl Order for Postorder {
    const START: Visit = Visit::Descend;
    fn step(state: &mut Visit) -> Visit {
        let visit = *state;
        *state = match state {
            Visit::Descend => Visit::Yield,
            Visit::Yield => Visit::Done,
            Visit::Done => Visit::Done,
        };
        visit
    }
}

#[derive(Copy, Clone, PartialEq, Eq)]
pub(crate) enum Visit {
    Yield,
    Descend,
    Done,
}

pub(crate) trait Sort<'a>: Iterator<Item = (u8, &'a Atomic128<Edge>)> {
    fn new(node: node::Ref<'a>) -> Self;
}

impl<'a> Sort<'a> for node::SortedIter<'a> {
    fn new(node: node::Ref) -> node::SortedIter {
        unsafe { node.iter_sorted() }
    }
}

impl<'a> Sort<'a> for node::UnsortedIter<'a> {
    fn new(node: node::Ref) -> node::UnsortedIter {
        unsafe { node.iter_unsorted() }
    }
}
