use core::marker::PhantomData;
use core::sync::atomic::Ordering;

use ribbit::atomic::Atomic128;

use crate::cursor;
use crate::iter::Sort;
use crate::key;
use crate::raw::edge;
use crate::raw::Edge;
use crate::Value;

pub(crate) enum PrefixIter<'g, 'l, W: key::Write, V: 'g, S: Sort> {
    Root {
        key: W,
        next: Option<ribbit::Packed<edge::Value<V>>>,
    },
    Node {
        key: W,
        frontier: Vec<(W::Len, S::PrefixIter<'g, V>)>,
        _cursor: PhantomData<&'l ()>,
    },
}

impl<'g, 'c, W, V, S> PrefixIter<'g, 'c, W, V, S>
where
    W: key::Write,
    V: Value,
    S: Sort,
{
    pub(crate) fn new<'l, R>(
        cursor: &'c cursor::Prefix<'g, 'l, R, V, cursor::path::Hybrid<'g, R, V>>,
    ) -> Self
    where
        R: key::Read,
        W: From<R>,
    {
        unsafe { Self::new_unchecked(cursor.edge(), W::from(cursor.prefix())) }
    }
}

impl<'g, 'l, W, V, S> PrefixIter<'g, 'l, W, V, S>
where
    W: key::Write,
    V: 'g,
    S: Sort,
{
    #[inline]
    pub(crate) unsafe fn new_unchecked(root: &'g Atomic128<Edge<V>>, mut key: W) -> Self {
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
    pub(crate) fn lend(&mut self) -> Option<(&W, ribbit::Packed<edge::Value<V>>)> {
        self.walk::<true, _>(|_, _| ())
    }

    #[inline]
    pub(crate) fn for_each<F: FnMut(&W, ribbit::Packed<edge::Value<V>>)>(mut self, apply: F) {
        self.walk::<false, _>(apply);
    }

    #[inline]
    fn walk<const YIELD: bool, F: FnMut(&W, ribbit::Packed<edge::Value<V>>)>(
        &mut self,
        mut apply: F,
    ) -> Option<(&W, ribbit::Packed<edge::Value<V>>)> {
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
                key.extend(meta.key());

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

impl<'g, 'l, W, V, S> Clone for PrefixIter<'g, 'l, W, V, S>
where
    W: key::Write,
    S: Sort,
{
    fn clone(&self) -> Self {
        match self {
            Self::Root { key, next } => Self::Root {
                key: key.clone(),
                next: *next,
            },
            Self::Node {
                key,
                frontier,
                _cursor,
            } => Self::Node {
                key: key.clone(),
                frontier: frontier.clone(),
                _cursor: PhantomData,
            },
        }
    }
}
