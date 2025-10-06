use core::marker::PhantomData;
use core::ops::RangeBounds;
use core::sync::atomic::Ordering;

use ribbit::atomic::Atomic128;

use crate::key;
use crate::node;
use crate::Edge;

pub enum LeafIter<'a, R, K, W, S>
where
    S: Sort<'a>,
{
    Root {
        key: W,
        next: Option<u64>,
    },
    Node {
        key: W,
        range: R,
        frontier: Vec<(usize, S)>,
        _key: PhantomData<K>,
        _sort: PhantomData<&'a ()>,
    },
}

impl<'a, R: RangeBounds<K>, K, W: key::Write + PartialOrd<K>, S: Sort<'a>>
    LeafIter<'a, R, K, W, S>
{
    #[inline]
    pub(crate) unsafe fn new(root: &Atomic128<Edge>, mut key: W, range: R) -> Self {
        let edge = root.load_packed(Ordering::Acquire);
        let meta = edge.meta();
        let data = edge.data();

        key.extend(edge.meta().key());

        match range.end_bound() {
            core::ops::Bound::Included(end) if key > *end => {
                return Self::Root { key, next: None };
            }
            core::ops::Bound::Excluded(end) if key >= *end => {
                return Self::Root { key, next: None };
            }
            _ => (),
        }

        if meta.leaf() {
            match range.start_bound() {
                core::ops::Bound::Included(start) if key < *start => {
                    return Self::Root { key, next: None };
                }
                core::ops::Bound::Excluded(start) if key <= *start => {
                    return Self::Root { key, next: None };
                }
                _ => (),
            }

            Self::Root {
                key,
                next: Some(edge.data()),
            }
        } else if data == 0 {
            Self::Root { key, next: None }
        } else {
            let node = unsafe { Edge::next_node_unchecked(data) };
            Self::Node {
                range,
                frontier: vec![(key.bits(), S::new(node))],
                key,
                _key: PhantomData,
                _sort: PhantomData,
            }
        }
    }

    #[inline]
    pub fn lend(&mut self) -> Option<(&W, u64)> {
        let (key, range, frontier) = match self {
            LeafIter::Root { key, next } => {
                crate::cold();
                let value = next.take()?;
                return Some((key, value));
            }
            LeafIter::Node {
                key,
                range,
                frontier,
                _sort,
                ..
            } => (key, range, frontier),
        };

        'vertical: loop {
            let (len, iter) = frontier.last_mut()?;
            let len = *len;

            loop {
                let Some((byte, edge)) = iter.next() else {
                    frontier.pop();
                    continue 'vertical;
                };

                let edge = edge.load_packed(Ordering::Acquire);
                let meta = edge.meta();
                let data = edge.data();

                if !meta.leaf() && data == 0 {
                    continue;
                }

                key.truncate(len);
                key.push(byte);
                key.extend(meta.key());

                match range.end_bound() {
                    core::ops::Bound::Included(end) if *key > *end => {
                        frontier.clear();
                        return None;
                    }
                    core::ops::Bound::Excluded(end) if *key >= *end => {
                        frontier.clear();
                        return None;
                    }
                    _ => (),
                }

                if meta.leaf() {
                    match range.start_bound() {
                        core::ops::Bound::Included(start) if *key < *start => continue,
                        core::ops::Bound::Excluded(start) if *key <= *start => continue,
                        _ => (),
                    }

                    return Some((key, edge.data()));
                } else {
                    let node = unsafe { Edge::next_node_unchecked(data) };
                    frontier.push((key.bits(), S::new(node)));
                    continue 'vertical;
                }
            }
        }
    }
}

pub enum PostorderIter<'a, W, V, S>
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
        frontier: Vec<(usize, RepeatIter<S>)>,
        _sort: PhantomData<&'a ()>,
    },
}

impl<'a, W: key::Write, V: Selector<W>, S: Sort<'a>> PostorderIter<'a, W, V, S> {
    #[inline]
    pub(crate) unsafe fn new(root: &Atomic128<Edge>, mut key: W, selector: V) -> Self {
        let edge = root.load_packed(Ordering::Acquire);
        let meta = edge.meta();
        let data = edge.data();

        key.extend(edge.meta().key());

        if meta.leaf() {
            let next = selector.select(edge, &key, 0);
            Self::Root { key, next }
        } else if data == 0 {
            Self::Root { key, next: None }
        } else {
            let node = unsafe { Edge::next_node_unchecked(data) };
            Self::Node {
                selector,
                frontier: vec![(key.bits(), RepeatIter::new(node))],
                key,
                _sort: PhantomData,
            }
        }
    }

    #[inline]
    pub fn lend(&mut self) -> Option<(&W, V::Item)> {
        let (key, selector, frontier) = match self {
            PostorderIter::Root { key, next } => {
                crate::cold();
                let value = next.take()?;
                return Some((key, value));
            }
            PostorderIter::Node {
                key,
                selector,
                frontier,
                _sort,
            } => (key, selector, frontier),
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
                        frontier.push((key.bits(), RepeatIter::new(node)));
                        continue 'vertical;
                    }
                }

                // Second visit (or fallthrough)
                iter.skip();

                if let Some(item) = selector.select(edge, key, depth) {
                    return Some((key, item));
                }
            }
        }
    }
}

pub(crate) trait Selector<W> {
    type Item;
    fn select(&self, edge: ribbit::Packed<Edge>, key: &W, depth: usize) -> Option<Self::Item>;
}

pub(crate) struct SelectNode;

impl<W: key::Write> Selector<W> for SelectNode {
    type Item = ribbit::Packed<Edge>;

    #[inline]
    fn select(&self, edge: ribbit::Packed<Edge>, _key: &W, _depth: usize) -> Option<Self::Item> {
        (!edge.meta().leaf() && edge.data() > 0).then_some(edge)
    }
}

pub(crate) struct SelectAll;

impl<W: key::Write> Selector<W> for SelectAll {
    type Item = (ribbit::Packed<Edge>, usize);

    #[inline]
    fn select(&self, edge: ribbit::Packed<Edge>, _key: &W, depth: usize) -> Option<Self::Item> {
        (edge.meta().leaf() || edge.data() > 0).then_some((edge, depth))
    }
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

struct RepeatIter<N> {
    first: bool,
    key: u8,
    edge: ribbit::Packed<Edge>,
    iter: N,
}

impl<'a, N: Sort<'a>> RepeatIter<N> {
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

impl<'a, S> RepeatIter<S>
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
