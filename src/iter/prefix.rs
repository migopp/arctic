use core::marker::PhantomData;
use core::sync::atomic::Ordering;

use ribbit::atomic::Atomic128;

use crate::cursor;
use crate::edge;
use crate::iter::Sort;
use crate::key;
use crate::Edge;
use crate::Value;

pub(crate) enum PrefixIter<'g, 'l, W: key::Write, V: 'g, S: Sort> {
    Root {
        key: W,
        next: Option<ribbit::Packed<edge::Data<V>>>,
    },
    Node {
        key: W,
        frontier: Vec<(W::Len, S::Iter<'g, V>)>,
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
        let meta = edge.meta();
        let data = edge.data();

        key.extend(edge.meta().key());

        if meta.leaf() {
            Self::Root {
                key,
                next: Some(data),
            }
        } else if data.is_null() {
            Self::Root { key, next: None }
        } else {
            let node = unsafe { data.into_node_unchecked() };
            Self::Node {
                frontier: vec![(key.bits(), S::new(node))],
                key,
                _cursor: PhantomData,
            }
        }
    }

    #[inline]
    pub(crate) fn lend(&mut self) -> Option<(&W, ribbit::Packed<edge::Data<V>>)> {
        self.walk::<true, _>(|_, _| ())
    }

    #[inline]
    pub(crate) fn for_each<F: FnMut(&W, ribbit::Packed<edge::Data<V>>)>(mut self, apply: F) {
        self.walk::<false, _>(apply);
    }

    #[inline]
    fn walk<const YIELD: bool, F: FnMut(&W, ribbit::Packed<edge::Data<V>>)>(
        &mut self,
        mut apply: F,
    ) -> Option<(&W, ribbit::Packed<edge::Data<V>>)> {
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

            loop {
                let Some((byte, edge)) = iter.next() else {
                    frontier.pop();
                    continue 'vertical;
                };

                let edge = edge.load_packed(Ordering::Acquire);
                if edge.is_null() {
                    continue;
                }

                let meta = edge.meta();
                let data = edge.data();

                key.truncate(*len);
                key.push(byte);
                key.extend(meta.key());

                if meta.leaf() {
                    if YIELD {
                        return Some((key, data));
                    } else {
                        apply(key, data);
                    }
                } else {
                    let node = unsafe { data.into_node_unchecked() };
                    frontier.push((key.bits(), unsafe { S::new(node) }));
                    continue 'vertical;
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
