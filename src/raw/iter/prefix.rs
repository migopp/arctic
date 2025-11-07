use core::sync::atomic::Ordering;

use ribbit::atomic::Atomic128;

use crate::iter::Order;
use crate::key;
use crate::raw::edge;
use crate::raw::Edge;

pub enum PrefixIter<'g, W: key::Write, C: 'g, O: Order> {
    Root {
        key: W,
        next: Option<u64>,
    },
    Node {
        key: W,
        frontier: Vec<(W::Len, O::PrefixIter<'g, C>)>,
    },
}

impl<'g, W, C, O> PrefixIter<'g, W, C, O>
where
    W: key::Write,
    O: Order,
{
    #[inline]
    pub(crate) unsafe fn new_unchecked<R: key::Read>(
        root: &'g Atomic128<Edge<C>>,
        prefix: R,
    ) -> Self
    where
        W: From<R>,
    {
        let bits = prefix.bits();
        let mut writer = W::from(prefix);

        let edge = root.load_packed(Ordering::Acquire);
        let key = edge.meta().key();
        let bits = writer.write(W::len_from_bits(bits), key);

        match edge.child() {
            None => Self::Root {
                key: writer,
                next: None,
            },
            Some(edge::Child::Value(value)) => Self::Root {
                key: writer,
                next: Some(value),
            },
            Some(edge::Child::Node(node)) => {
                let node = unsafe { node.into_ref_unchecked() };
                Self::Node {
                    frontier: vec![(bits, O::prefix(node))],
                    key: writer,
                }
            }
        }
    }

    #[inline]
    pub(crate) fn lend(&mut self) -> Option<(&W, u64)> {
        self.walk::<true, _>(|_, _| ())
    }

    #[inline]
    pub(crate) fn for_each<F: FnMut(&W, u64)>(mut self, apply: F) {
        self.walk::<false, _>(apply);
    }

    #[inline]
    fn walk<const YIELD: bool, F: FnMut(&W, u64)>(&mut self, mut apply: F) -> Option<(&W, u64)> {
        let (key, frontier) = match self {
            Self::Root { key, next } => {
                crate::cold();
                let value = next.take()?;
                if YIELD {
                    return Some((key, value));
                } else {
                    apply(key, value);
                    return None;
                }
            }
            Self::Node { key, frontier } => (key, frontier),
        };

        'vertical: loop {
            let (bits, iter) = frontier.last_mut()?;
            let bits = *bits;

            'horizontal: loop {
                let Some((byte, edge)) = iter.next() else {
                    frontier.pop();
                    continue 'vertical;
                };

                let edge = edge.load_packed(Ordering::Acquire);

                let Some(child) = edge.child() else {
                    continue 'horizontal;
                };

                let meta = edge.meta();
                let bits = key.replace(bits, byte, meta.key());

                match child {
                    edge::Child::Value(value) => {
                        if YIELD {
                            return Some((key, value));
                        } else {
                            apply(key, value);
                        }
                    }
                    edge::Child::Node(node) => {
                        let node = unsafe { node.into_ref_unchecked() };
                        frontier.push((bits, unsafe { O::prefix(node) }));
                        continue 'vertical;
                    }
                }
            }
        }
    }
}
