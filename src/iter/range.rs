use core::cmp;
use core::sync::atomic::Ordering;

use ribbit::atomic::Atomic128;

use crate::cursor;
use crate::iter::Scan;
use crate::key::Read as _;
use crate::key::Write as _;
use crate::node;
use crate::Cursor;
use crate::Edge;
use crate::Key;
use crate::Value;

pub(crate) enum RangeIter<'g, 'l, K: Key, V> {
    Root { key: K::Write, next: Option<u64> },
    Node(NodeIter<'g, 'l, K, V>),
}

impl<'g, 'l, K, V> RangeIter<'g, 'l, K, V>
where
    K: Key,
    V: Value,
{
    pub(crate) fn new<'k: 'l>(
        cursor: &'l cursor::Prefix<'g, 'l, K::Read<'k>, V, cursor::Hybrid<'g, K::Read<'k>, V>>,
        min: K::Read<'k>,
        max: K::Read<'k>,
    ) -> Self {
        unsafe {
            Self::new_unchecked(
                cursor.root(),
                <K::Write>::from(cursor.prefix()),
                K::reborrow(min),
                K::reborrow(max),
            )
        }
    }
}

impl<'g, 'l, K, V> RangeIter<'g, 'l, K, V>
where
    K: Key,
{
    pub(crate) unsafe fn new_unchecked(
        root: &'g Atomic128<Edge<V>>,
        mut key: K::Write,
        min: K::Read<'l>,
        max: K::Read<'l>,
    ) -> Self {
        let edge = root.load_packed(Ordering::Acquire);
        let meta = edge.meta();
        let data = edge.data();
        key.extend(edge.meta().key());

        if meta.leaf() {
            let reader = K::Read::from(&key);
            if reader < K::reborrow(min) || reader > K::reborrow(max) {
                return Self::Root { key, next: None };
            }

            Self::Root {
                key,
                next: Some(data.into_leaf()),
            }
        } else if data.is_null() {
            Self::Root { key, next: None }
        } else {
            let node = unsafe { data.into_node_unchecked() };

            let reader = K::Read::from(&key);
            validate!(reader >= K::reborrow(min.slice(key.bits())));
            validate!(reader <= K::reborrow(max.slice(key.bits())));

            let first = (reader == K::reborrow(min.slice(key.bits()))).then(|| min.get(key.bits()));
            let last = (reader == K::reborrow(max.slice(key.bits()))).then(|| max.get(key.bits()));

            let mut stack = Vec::with_capacity(7);
            stack.push((key.bits(), first, last, node.iter_range(first, last)));

            Self::Node(NodeIter {
                stack,
                key,
                min,
                max,
            })
        }
    }

    #[inline]
    pub(crate) fn for_each<F: FnMut(&K::Write, u64)>(self, mut apply: F) {
        match self {
            RangeIter::Root { key, mut next } => {
                crate::cold();
                if let Some(value) = next.take() {
                    apply(&key, value);
                }
            }
            RangeIter::Node(mut iter) => iter.for_each(apply),
        }
    }

    #[inline]
    pub(crate) fn lend(&mut self) -> Option<(&K::Write, u64)> {
        match self {
            RangeIter::Root { key, next } => {
                crate::cold();
                let value = next.take()?;
                Some((key, value))
            }
            RangeIter::Node(iter) => iter.lend(),
        }
    }
}

impl<K: Key, V> Clone for RangeIter<'_, '_, K, V> {
    fn clone(&self) -> Self {
        match self {
            Self::Root { key, next } => Self::Root {
                key: key.clone(),
                next: *next,
            },
            Self::Node(iter) => Self::Node(iter.clone()),
        }
    }
}

impl<'g, 'k, 'l, K, V> Scan<'g, 'k, 'l, (K::Read<'k>, K::Read<'k>), K, V>
    for RangeIter<'g, 'l, K, V>
where
    K: Key,
    V: Value,
    'k: 'l,
{
    fn new(
        cursor: &'l cursor::Prefix<'g, 'l, K::Read<'k>, V, cursor::Hybrid<'g, K::Read<'k>, V>>,
        (min, max): &(K::Read<'k>, K::Read<'k>),
    ) -> Self {
        Self::new(cursor, *min, *max)
    }

    fn for_each<F: FnMut(&K::Write, u64)>(self, apply: F) {
        Self::for_each(self, apply)
    }
}

pub(crate) struct NodeIter<'g, 'l, K: Key, V> {
    min: K::Read<'l>,
    max: K::Read<'l>,
    key: K::Write,
    stack: Vec<(usize, Option<u8>, Option<u8>, node::SortedIter<'g, V>)>,
}

impl<'g, 'k, K, V> NodeIter<'g, 'k, K, V>
where
    K: Key,
{
    #[inline]
    fn lend(&mut self) -> Option<(&K::Write, u64)> {
        self.walk::<true, _>(|_, _| ())
    }

    #[inline]
    fn for_each<F: FnMut(&K::Write, u64)>(&mut self, apply: F) {
        self.walk::<false, _>(apply);
    }

    #[inline]
    fn walk<const YIELD: bool, F: FnMut(&K::Write, u64)>(
        &mut self,
        mut apply: F,
    ) -> Option<(&K::Write, u64)> {
        'vertical: loop {
            let (len, min, max, iter) = self.stack.last_mut()?;

            'horizontal: loop {
                let Some((byte, edge)) = iter.next() else {
                    self.stack.pop();
                    continue 'vertical;
                };

                let edge = edge.load_packed(Ordering::Acquire);
                if edge.is_null() {
                    continue 'horizontal;
                }

                let meta = edge.meta();
                let data = edge.data();

                self.key.truncate(*len);
                self.key.push(byte);

                unsafe {
                    // SAFETY: we just pushed `byte` onto `key`
                    self.key.extend_nonempty_unchecked(meta.key());
                }

                let check_first = Some(byte) == *min;
                let check_last = Some(byte) == *max;

                if !check_first && !check_last {
                    if meta.leaf() {
                        if YIELD {
                            return Some((&self.key, data.into_leaf()));
                        } else {
                            apply(&self.key, data.into_leaf());
                            continue 'horizontal;
                        }
                    } else {
                        let node = unsafe { data.into_node_unchecked() };
                        self.stack
                            .push((self.key.bits(), None, None, node.iter_range(None, None)));
                        continue 'vertical;
                    }
                }

                crate::cold();

                if meta.leaf() {
                    if check_first && K::Read::from(&self.key) < K::reborrow(self.min) {
                        continue 'horizontal;
                    }

                    if check_last && K::Read::from(&self.key) > K::reborrow(self.max) {
                        self.stack.clear();
                        return None;
                    }

                    if YIELD {
                        return Some((&self.key, data.into_leaf()));
                    } else {
                        apply(&self.key, data.into_leaf());
                    }
                } else {
                    let min = if check_first {
                        match K::Read::from(&self.key)
                            .cmp(&K::reborrow(self.min.slice(self.key.bits())))
                        {
                            cmp::Ordering::Less => continue 'horizontal,
                            cmp::Ordering::Equal => Some(self.min.get(self.key.bits())),
                            cmp::Ordering::Greater => None,
                        }
                    } else {
                        None
                    };

                    let max = if check_last {
                        match K::Read::from(&self.key)
                            .cmp(&K::reborrow(self.max.slice(self.key.bits())))
                        {
                            cmp::Ordering::Less => None,
                            cmp::Ordering::Equal => Some(self.max.get(self.key.bits())),
                            cmp::Ordering::Greater => {
                                self.stack.clear();
                                return None;
                            }
                        }
                    } else {
                        None
                    };

                    let node = unsafe { data.into_node_unchecked() };
                    self.stack
                        .push((self.key.bits(), min, max, node.iter_range(min, max)));
                    continue 'vertical;
                }
            }
        }
    }
}

impl<K: Key, V> Clone for NodeIter<'_, '_, K, V> {
    fn clone(&self) -> Self {
        Self {
            min: self.min,
            max: self.max,
            key: self.key.clone(),
            stack: self.stack.clone(),
        }
    }
}
