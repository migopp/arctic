use core::sync::atomic::Ordering;

use ribbit::atomic::Atomic128;

use crate::iter::Sort;
use crate::key;
use crate::raw::edge;
use crate::raw::Edge;

pub(crate) enum PrefixIter<'g, W: key::Write, C: 'g, S: Sort> {
    Root {
        key: W,
        next: Option<u64>,
    },
    Node {
        key: W,
        frontier: Vec<(usize, S::PrefixIter<'g, C>)>,
    },
}

impl<'g, W, C, S> PrefixIter<'g, W, C, S>
where
    W: key::Write,
    S: Sort,
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
        writer.extend(bits, key);

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
                    frontier: vec![(bits + key.len().bits() as usize, S::prefix(node))],
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

                key.truncate(bits);
                key.push(bits, byte);
                unsafe {
                    // SAFETY: we pushed to `key` above
                    key.extend_nonempty_unchecked(bits + 8, meta.key());
                }

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
                        frontier.push((bits + 8 + meta.key().len().bits() as usize, unsafe {
                            S::prefix(node)
                        }));
                        continue 'vertical;
                    }
                }
            }
        }
    }
}
