use core::iter;
use core::marker::PhantomData;
use core::ops::Bound;
use core::sync::atomic::Ordering;

use ribbit::atomic::Atomic128;

use crate::key;
use crate::node;
use crate::Edge;

pub struct Iter<'a, W, V, O, S>
where
    S: Sort<'a>,
{
    key: W,
    selector: V,
    _order: PhantomData<O>,
    frontier: Vec<(usize, core::iter::Peekable<TreeIter<'a, S>>)>,
}

impl<'a, W: key::Write, V: Selector<W>, O: Order, S: Sort<'a>> Iter<'a, W, V, O, S> {
    #[inline]
    pub(crate) unsafe fn new(root: &'a Atomic128<Edge>, key: W, selector: V) -> Self {
        let len = key.len();
        Self {
            key,
            selector,
            _order: PhantomData,
            frontier: vec![(len, TreeIter::from_root(root))],
        }
    }

    #[inline]
    pub fn lend(&mut self) -> Option<(&W, V::Item)> {
        'vertical: loop {
            // NOTE: we use `saturating_sub` to avoid underflow.
            //
            // If `self.frontier.len()` == 0, we will immediately return at `last_mut()`.
            // We can't move the len call after because `self.frontier` is mutably borrowed.
            let depth = self.frontier.len().saturating_sub(1);
            let (len, iter) = self.frontier.last_mut()?;

            loop {
                let Some((visit, byte, edge)) = iter.peek_mut() else {
                    self.key.truncate(*len);
                    self.frontier.pop();
                    continue 'vertical;
                };

                let edge = *edge;
                let meta = edge.meta();
                let kind = meta.kind();

                macro_rules! visit {
                    ($condition:expr) => {
                        if $condition {
                            match self.selector.select(edge, &self.key, depth) {
                                Select::Yield(item) => return Some((&self.key, item)),
                                Select::Continue => (),
                                Select::Break => {
                                    self.frontier.clear();
                                    return None;
                                }
                            }
                        } else if kind >= node::Kind::NODE_3 {
                            let node = unsafe { Edge::next_node_unchecked(edge.data(), kind) };
                            self.frontier
                                .push((self.key.len(), TreeIter::from_node(node)));
                            continue 'vertical;
                        }
                    };
                }

                // First visit
                if core::mem::take(visit) {
                    self.key.truncate(*len);

                    if let Some(byte) = byte {
                        self.key.push(*byte);
                    }

                    self.key.extend(meta.key());

                    visit!(O::PREORDER);
                }

                // Second visit (or fallthrough)
                iter.next();

                visit!(!O::PREORDER);
            }
        }
    }
}

pub(crate) trait Selector<W: key::Write> {
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
    R: PartialOrd<W>,
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

enum TreeIter<'a, N> {
    Root(iter::Once<(bool, &'a Atomic128<Edge>)>),
    Node(iter::Zip<iter::Repeat<bool>, N>),
}

impl<'a, N> TreeIter<'a, N>
where
    N: Sort<'a>,
{
    #[inline]
    fn from_root(root: &'a Atomic128<Edge>) -> core::iter::Peekable<Self> {
        Self::Root(core::iter::once((true, root))).peekable()
    }

    #[inline]
    fn from_node(node: node::Ref<'a>) -> core::iter::Peekable<Self> {
        Self::Node(iter::repeat(true).zip(N::new(node))).peekable()
    }
}

impl<'a, S> Iterator for TreeIter<'a, S>
where
    S: Sort<'a>,
{
    type Item = (bool, Option<u8>, ribbit::Packed<Edge>);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        match self {
            TreeIter::Root(iter) => iter
                .next()
                .map(|(visit, edge)| (visit, None, edge.load_packed(Ordering::Acquire))),
            TreeIter::Node(iter) => iter.next().map(|(visit, (byte, edge))| {
                (visit, Some(byte), edge.load_packed(Ordering::Acquire))
            }),
        }
    }
}
