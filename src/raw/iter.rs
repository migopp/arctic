use core::marker::PhantomData;
use core::ops::Bound;
use core::sync::atomic::Ordering;

use ribbit::atomic::Atomic128;

use crate::key;
use crate::node;
use crate::Edge;

pub enum Iter<'a, W, V, O, S>
where
    S: Sort<'a>,
    V: Selector<W>,
{
    Root {
        key: W,
        next: Option<V::Item>,
    },
    Node {
        key: W,
        selector: V,
        #[allow(private_interfaces)]
        frontier: Vec<(usize, NodeIter<S>)>,
        _order: PhantomData<O>,
        _sort: PhantomData<&'a ()>,
    },
}

impl<'a, W: key::Write, V: Selector<W>, O: Order, S: Sort<'a>> Iter<'a, W, V, O, S> {
    #[inline]
    pub(crate) unsafe fn new(root: &Atomic128<Edge>, mut key: W, selector: V) -> Self {
        let edge = root.load_packed(Ordering::Acquire);
        let meta = edge.meta();
        let kind = meta.kind();

        key.extend(edge.meta().key());

        if kind == node::Kind::NONE {
            Self::Root { key, next: None }
        } else if kind == node::Kind::LEAF {
            let next = match selector.select(edge, &key, 0) {
                Select::Yield(value) => Some(value),
                Select::Continue | Select::Break => None,
            };
            Self::Root { key, next }
        } else {
            let node = unsafe { Edge::next_node_unchecked(edge.data(), kind) };
            Self::Node {
                selector,
                frontier: vec![(key.len(), NodeIter::new(node))],
                key,
                _order: PhantomData,
                _sort: PhantomData,
            }
        }
    }

    #[inline]
    pub fn lend(&mut self) -> Option<(&W, V::Item)> {
        let (key, selector, frontier) = match self {
            Iter::Root { key, next } => {
                crate::cold();
                let value = next.take()?;
                return Some((key, value));
            }
            Iter::Node {
                key,
                selector,
                frontier,
                _order,
                _sort,
            } => (key, selector, frontier),
        };

        'vertical: loop {
            let depth = frontier.len();
            let (len, iter) = frontier.last_mut()?;

            loop {
                let Some((first, byte, edge)) = iter.next() else {
                    key.truncate(*len);
                    frontier.pop();
                    continue 'vertical;
                };

                let meta = edge.meta();
                let kind = meta.kind();
                if kind == node::Kind::NONE {
                    iter.skip();
                    continue;
                }

                macro_rules! visit {
                    ($condition:expr) => {
                        if $condition {
                            match selector.select(edge, &key, depth) {
                                Select::Yield(item) => return Some((key, item)),
                                Select::Continue => (),
                                Select::Break => {
                                    frontier.clear();
                                    return None;
                                }
                            }
                        } else if kind >= node::Kind::NODE_3 {
                            let node = unsafe { Edge::next_node_unchecked(edge.data(), kind) };
                            frontier.push((key.len(), NodeIter::new(node)));
                            continue 'vertical;
                        }
                    };
                }

                if first {
                    key.truncate(*len);
                    key.push(byte);
                    key.extend(meta.key());
                    visit!(O::PREORDER);
                }

                // Second visit (or fallthrough)
                iter.skip();

                visit!(!O::PREORDER);
            }
        }
    }
}

pub(crate) trait Selector<W> {
    type Item;
    fn select(&self, edge: ribbit::Packed<Edge>, key: &W, depth: usize) -> Select<Self::Item>;
}

pub(crate) enum Select<T> {
    Yield(T),
    Continue,
    Break,
}

pub(crate) struct SelectLeaf;

impl<W: key::Write> Selector<W> for SelectLeaf {
    type Item = u64;

    #[inline]
    fn select(&self, edge: ribbit::Packed<Edge>, _key: &W, _depth: usize) -> Select<Self::Item> {
        if edge.meta().kind() == node::Kind::LEAF {
            Select::Yield(edge.data())
        } else {
            Select::Continue
        }
    }
}

pub(crate) struct SelectNode;

impl<W: key::Write> Selector<W> for SelectNode {
    type Item = ribbit::Packed<Edge>;

    #[inline]
    fn select(&self, edge: ribbit::Packed<Edge>, _key: &W, _depth: usize) -> Select<Self::Item> {
        if edge.meta().kind() >= node::Kind::NODE_3 {
            Select::Yield(edge)
        } else {
            Select::Continue
        }
    }
}

pub(crate) struct SelectAll;

impl<W: key::Write> Selector<W> for SelectAll {
    type Item = (ribbit::Packed<Edge>, usize);

    #[inline]
    fn select(&self, edge: ribbit::Packed<Edge>, _key: &W, depth: usize) -> Select<Self::Item> {
        if edge.meta().kind() > node::Kind::NONE {
            Select::Yield((edge, depth))
        } else {
            Select::Continue
        }
    }
}

pub(crate) struct SelectRange<R, W> {
    start: Bound<R>,
    end: Bound<R>,
    _stack: PhantomData<W>,
}

impl<R, W> SelectRange<R, W> {
    #[inline]
    pub(crate) fn new(start: Bound<R>, end: Bound<R>) -> Self {
        Self {
            start,
            end,
            _stack: PhantomData,
        }
    }
}

impl<R, W> Selector<W> for SelectRange<R, W>
where
    W: key::Write + PartialOrd<R>,
{
    type Item = u64;
    #[inline]
    fn select(&self, edge: ribbit::Packed<Edge>, key: &W, _depth: usize) -> Select<Self::Item> {
        match &self.end {
            core::ops::Bound::Included(end) if key > end => return Select::Break,
            core::ops::Bound::Excluded(end) if key >= end => return Select::Break,
            _ => (),
        }

        match &self.start {
            core::ops::Bound::Included(start) if key < start => return Select::Continue,
            core::ops::Bound::Excluded(start) if key <= start => return Select::Continue,
            _ => (),
        }

        if edge.meta().kind() == node::Kind::LEAF {
            Select::Yield(edge.data())
        } else {
            Select::Continue
        }
    }
}

pub(crate) trait Order {
    const PREORDER: bool;
}

pub(crate) struct Preorder;
impl Order for Preorder {
    const PREORDER: bool = true;
}

pub(crate) struct Postorder;
impl Order for Postorder {
    const PREORDER: bool = false;
}

pub(crate) trait Sort<'a>: Iterator<Item = (u8, &'a Atomic128<Edge>)> {
    fn new(node: node::Ref<'a>) -> Self;
}

impl<'a> Sort<'a> for node::SortedIter<'a> {
    #[inline]
    fn new(node: node::Ref) -> node::SortedIter {
        unsafe { node.iter_sorted() }
    }
}

impl<'a> Sort<'a> for node::UnsortedIter<'a> {
    #[inline]
    fn new(node: node::Ref) -> node::UnsortedIter {
        unsafe { node.iter_unsorted() }
    }
}

struct NodeIter<N> {
    first: bool,
    key: u8,
    edge: ribbit::Packed<Edge>,
    iter: N,
}

impl<'a, N: Sort<'a>> NodeIter<N> {
    #[inline]
    fn new(node: node::Ref<'a>) -> Self {
        Self {
            first: true,
            key: 0,
            edge: Edge::DEFAULT,
            iter: N::new(node),
        }
    }
}

impl<'a, S> NodeIter<S>
where
    S: Sort<'a>,
{
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
