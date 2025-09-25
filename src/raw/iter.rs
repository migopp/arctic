use core::iter;
use core::mem;
use core::sync::atomic::Ordering;

use ribbit::atomic::Atomic128;

use crate::byte;
use crate::node;
use crate::Edge;

pub struct EntryIter<'a, K> {
    key: K,
    frontier: Vec<(usize, Or<RootIter<'a>, NodeIter<'a>>)>,
}

type RootIter<'a> = iter::Peekable<iter::Zip<iter::Once<bool>, node::EdgeIter<'a>>>;
type NodeIter<'a> = iter::Peekable<iter::Zip<iter::Repeat<bool>, node::Iter<'a>>>;

impl<'a, K: byte::Stack> EntryIter<'a, K> {
    pub(super) fn new(root: &'a mut Atomic128<Edge>) -> Self {
        Self {
            key: K::default(),
            frontier: vec![(
                0,
                Or::L(iter::zip(iter::once(false), core::slice::from_ref(root).iter()).peekable()),
            )],
        }
    }

    pub fn next(&mut self) -> Option<(usize, &K, ribbit::Packed<Edge>)> {
        'vertical: loop {
            // NOTE: we use `saturating_sub` to avoid underflow.
            //
            // If `self.frontier.len()` == 0, we will immediately return at `last_mut()`.
            // We can't move the len call after because `self.frontier` is mutably borrowed.
            let depth = self.frontier.len().saturating_sub(1);
            let (len, iter) = self.frontier.last_mut()?;

            'horizontal: loop {
                let Some((descend, byte, edge)) = (match iter {
                    Or::L(iter_root) => iter_root
                        .peek_mut()
                        .map(|(descend, edge)| (descend, None, edge)),
                    Or::R(iter_node) => iter_node
                        .peek_mut()
                        .map(|(descend, (key, edge))| (descend, Some(*key), edge)),
                }) else {
                    self.key.pop(*len);
                    self.frontier.pop();
                    continue 'vertical;
                };

                let edge = edge.load_packed(Ordering::Relaxed);
                let meta = edge.meta();
                let kind = meta.kind();

                // Skip empty edges
                if kind == node::Kind::NONE {
                    iter.skip();
                    continue 'horizontal;
                }

                let key = meta.key();

                // Produce edge before traversing for preorder traversal
                if !mem::replace(descend, true) {
                    if let Some(byte) = byte {
                        self.key.push_byte(byte);
                    }

                    self.key.push_array(key);
                    return Some((depth, &self.key, edge));
                }

                iter.skip();
                let len = byte::Array::len(key) + byte.is_some() as usize;

                if kind == node::Kind::LEAF {
                    self.key.pop(len);
                    continue 'horizontal;
                } else {
                    let node = unsafe { Edge::next_node_unchecked(edge.data(), kind) };
                    self.frontier.push((
                        len,
                        Or::R(iter::repeat(false).zip(unsafe { node.iter() }).peekable()),
                    ));
                    continue 'vertical;
                }
            }
        }
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
