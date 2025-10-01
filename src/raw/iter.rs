use core::iter;
use core::marker::PhantomData;
use core::sync::atomic::Ordering;

use ribbit::atomic::Atomic128;

use crate::key;
use crate::node;
use crate::Edge;

pub struct Iter<'a, K, V, O, S>
where
    S: Sort<'a>,
{
    key: K,
    _select: PhantomData<V>,
    _order: PhantomData<O>,
    _sort: PhantomData<S>,
    frontier: Vec<(usize, core::iter::Peekable<TreeIter<'a, S>>)>,
}

impl<'a, K: key::Stack, V: Selector, O: Order, S: Sort<'a>> Iter<'a, K, V, O, S> {
    #[inline]
    pub(crate) unsafe fn new(root: &'a Atomic128<Edge>) -> Self {
        Self {
            key: K::default(),
            _select: PhantomData,
            _order: PhantomData,
            _sort: PhantomData,
            frontier: vec![(0, TreeIter::from_root(root))],
        }
    }

    #[inline]
    pub fn next(&mut self) -> Option<(&K, V::Item)> {
        'vertical: loop {
            // NOTE: we use `saturating_sub` to avoid underflow.
            //
            // If `self.frontier.len()` == 0, we will immediately return at `last_mut()`.
            // We can't move the len call after because `self.frontier` is mutably borrowed.
            let depth = self.frontier.len().saturating_sub(1);
            let (len, iter) = self.frontier.last_mut()?;

            'horizontal: loop {
                let Some((visit, byte, edge)) = iter.peek_mut() else {
                    self.key.truncate(*len);
                    self.frontier.pop();
                    continue 'vertical;
                };

                let edge = *edge;
                let meta = edge.meta();
                let kind = meta.kind();

                if kind == node::Kind::NONE {
                    iter.next();
                    continue 'horizontal;
                }

                macro_rules! visit {
                    ($condition:expr) => {
                        if $condition {
                            match V::select(depth, edge) {
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

pub(crate) trait Selector {
    type Item;
    fn select(depth: usize, edge: ribbit::Packed<Edge>) -> Select<Self::Item>;
}

pub(crate) enum Select<T> {
    Yield(T),
    Continue,
    #[expect(dead_code)]
    Break,
}

pub(crate) struct SelectLeaf;

impl Selector for SelectLeaf {
    type Item = u64;

    #[inline]
    fn select(_depth: usize, edge: ribbit::Packed<Edge>) -> Select<Self::Item> {
        if edge.meta().kind() == node::Kind::LEAF {
            Select::Yield(edge.data())
        } else {
            Select::Continue
        }
    }
}

pub(crate) struct SelectNode;

impl Selector for SelectNode {
    type Item = ribbit::Packed<Edge>;

    #[inline]
    fn select(_: usize, edge: ribbit::Packed<Edge>) -> Select<Self::Item> {
        if edge.meta().kind() >= node::Kind::NODE_3 {
            Select::Yield(edge)
        } else {
            Select::Continue
        }
    }
}

pub(crate) struct SelectAll;

impl Selector for SelectAll {
    type Item = (usize, ribbit::Packed<Edge>);

    #[inline]
    fn select(depth: usize, edge: ribbit::Packed<Edge>) -> Select<Self::Item> {
        Select::Yield((depth, edge))
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
