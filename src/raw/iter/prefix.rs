use core::marker::PhantomData;
use core::sync::atomic::Ordering;

use ribbit::atomic::Atomic128;

use crate::iter::Sort;
use crate::key;
use crate::raw::edge;
use crate::raw::Edge;

pub(crate) enum PrefixIter<'g, 'l, W: key::Write, C: 'g, S: Sort> {
    Root {
        key: W,
        next: Option<u64>,
    },
    Node {
        key: W,
        frontier: Vec<(W::Len, S::PrefixIter<'g, C>)>,
        _cursor: PhantomData<&'l ()>,
    },
}

impl<'g, 'l, W, C, S> PrefixIter<'g, 'l, W, C, S>
where
    W: key::Write,
    C: 'g,
    S: Sort,
{
    #[inline]
    pub(crate) unsafe fn new_unchecked(root: &'g Atomic128<Edge<C>>, mut key: W) -> Self {
        let edge = root.load_packed(Ordering::Acquire);

        key.extend(edge.meta().key());

        match edge.child() {
            None => Self::Root { key, next: None },
            Some(edge::Child::Value(value)) => Self::Root {
                key,
                next: Some(value),
            },
            Some(edge::Child::Node(node)) => {
                let node = unsafe { node.into_ref_unchecked() };
                Self::Node {
                    frontier: vec![(key.bits(), S::prefix(node))],
                    key,
                    _cursor: PhantomData,
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
            Self::Node {
                key,
                frontier,
                _cursor,
            } => (key, frontier),
        };

        'vertical: loop {
            let (len, iter) = frontier.last_mut()?;

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

                key.truncate(*len);
                key.push(byte);
                unsafe {
                    // SAFETY: we pushed to `key` above
                    key.extend_nonempty_unchecked(meta.key());
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
                        frontier.push((key.bits(), unsafe { S::prefix(node) }));
                        continue 'vertical;
                    }
                }
            }
        }
    }
}
