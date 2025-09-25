use core::iter;
use core::marker::PhantomData;
use core::mem;
use core::sync::atomic::Ordering;

use ribbit::atomic::Atomic128;

use crate::byte;
use crate::node;
use crate::Edge;

pub struct EntryIter<'a, K, S> {
    key: K,
    _select: PhantomData<S>,
    frontier: Vec<(usize, Or<RootIter<'a>, NodeIter<'a>>)>,
}

type RootIter<'a> = iter::Peekable<iter::Zip<iter::Once<bool>, node::EdgeIter<'a>>>;
type NodeIter<'a> = iter::Peekable<iter::Zip<iter::Repeat<bool>, node::Iter<'a>>>;

impl<'a, K: byte::Stack, S: Selector> EntryIter<'a, K, S> {
    pub(super) fn new(root: &'a mut Atomic128<Edge>) -> Self {
        Self {
            key: K::default(),
            _select: PhantomData,
            frontier: vec![(
                0,
                Or::L(iter::zip(iter::once(true), core::slice::from_ref(root).iter()).peekable()),
            )],
        }
    }

    pub fn next(&mut self) -> Option<(&K, S::Item)> {
        'vertical: loop {
            // NOTE: we use `saturating_sub` to avoid underflow.
            //
            // If `self.frontier.len()` == 0, we will immediately return at `last_mut()`.
            // We can't move the len call after because `self.frontier` is mutably borrowed.
            let depth = self.frontier.len().saturating_sub(1);
            let (len, iter) = self.frontier.last_mut()?;

            'horizontal: loop {
                let Some((emit, byte, edge)) = (match iter {
                    Or::L(iter_root) => iter_root.peek_mut().map(|(emit, edge)| (emit, None, edge)),
                    Or::R(iter_node) => iter_node
                        .peek_mut()
                        .map(|(emit, (key, edge))| (emit, Some(*key), edge)),
                }) else {
                    self.key.pop(*len);
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

                let key = meta.key();

                // First time seeing edge
                if mem::replace(emit, false) {
                    if let Some(byte) = byte {
                        self.key.push_byte(byte);
                    }

                    let key = meta.key();
                    self.key.push_array(key);

                    if let Some(item) = S::select(depth, edge) {
                        return Some((&self.key, item));
                    }
                }

                // Second time seeing edge
                iter.skip();
                let len = byte::Array::len(key) + byte.is_some() as usize;
                if kind == node::Kind::LEAF {
                    self.key.pop(len);
                    continue 'horizontal;
                } else {
                    let node = unsafe { Edge::next_node_unchecked(edge.data(), kind) };
                    self.frontier.push((
                        len,
                        Or::R(iter::repeat(true).zip(unsafe { node.iter() }).peekable()),
                    ));
                    continue 'vertical;
                }
            }
        }
    }
}

pub(crate) trait Selector {
    type Item;
    fn select(depth: usize, edge: ribbit::Packed<Edge>) -> Option<Self::Item>;
}

pub(crate) struct SelectAll;

impl Selector for SelectAll {
    type Item = (usize, ribbit::Packed<Edge>);

    #[inline]
    fn select(depth: usize, edge: ribbit::Packed<Edge>) -> Option<Self::Item> {
        Some((depth, edge))
    }
}

#[derive(Debug)]
enum Or<L, R> {
    L(L),
    R(R),
}

impl<L, R, T> Iterator for Or<L, R>
where
    L: Iterator<Item = T>,
    R: Iterator<Item = T>,
{
    type Item = T;
    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Or::L(left) => left.next(),
            Or::R(right) => right.next(),
        }
    }
}

impl<L, R> Or<L, R>
where
    L: Iterator,
    R: Iterator,
{
    fn skip(&mut self) {
        match self {
            Or::L(left) => {
                left.next();
            }
            Or::R(right) => {
                right.next();
            }
        }
    }
}
