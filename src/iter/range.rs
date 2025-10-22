use core::cmp;
use core::sync::atomic::Ordering;

use ribbit::atomic::Atomic128;

use crate::key;
use crate::node;
use crate::Edge;

pub(crate) enum RangeIter<'a, R, W, V> {
    Root { key: W, next: Option<u64> },
    Node(NodeIter<'a, R, W, V>),
}

impl<'a, R, W, V> RangeIter<'a, R, W, V>
where
    R: key::Read,
    W: key::Write<Len = usize> + PartialOrd<R>,
{
    #[inline]
    pub(crate) fn empty() -> Self {
        Self::Root {
            key: W::default(),
            next: None,
        }
    }

    pub(crate) unsafe fn new(root: &'a Atomic128<Edge<V>>, mut key: W, min: R, max: R) -> Self {
        let edge = root.load_packed(Ordering::Acquire);
        let meta = edge.meta();
        let data = edge.data();
        key.extend(edge.meta().key());

        if meta.leaf() {
            if key < min || key > max {
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

            validate!(key >= min.slice(key.bits()));
            validate!(key <= max.slice(key.bits()));

            let first = (key == min.slice(key.bits())).then(|| min.get(key.bits()));
            let last = (key == max.slice(key.bits())).then(|| max.get(key.bits()));

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
    pub(crate) fn for_each<F: FnMut(&W, u64)>(&mut self, mut apply: F) {
        match self {
            RangeIter::Root { key, next } => {
                crate::cold();
                if let Some(value) = next.take() {
                    apply(key, value);
                }
            }
            RangeIter::Node(iter) => iter.for_each(apply),
        }
    }

    #[inline]
    pub(crate) fn lend(&mut self) -> Option<(&W, u64)> {
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

pub(crate) struct NodeIter<'a, R, W, V> {
    min: R,
    max: R,
    key: W,
    stack: Vec<(usize, Option<u8>, Option<u8>, node::SortedIter<'a, V>)>,
}

impl<'a, R, W, V> NodeIter<'a, R, W, V>
where
    R: key::Read,
    W: key::Write<Len = usize> + PartialOrd<R>,
{
    #[inline]
    fn lend(&mut self) -> Option<(&W, u64)> {
        self.walk::<true, _>(|_, _| ())
    }

    #[inline]
    fn for_each<F: FnMut(&W, u64)>(&mut self, apply: F) {
        self.walk::<false, _>(apply);
    }

    #[inline]
    fn walk<const YIELD: bool, F: FnMut(&W, u64)>(&mut self, mut apply: F) -> Option<(&W, u64)> {
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
                    if check_first && self.key < self.min {
                        continue 'horizontal;
                    }

                    if check_last && self.key > self.max {
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
                        match self.key.partial_cmp(&self.min.slice(self.key.bits())) {
                            None => unreachable!(),
                            Some(cmp::Ordering::Less) => continue 'horizontal,
                            Some(cmp::Ordering::Equal) => Some(self.min.get(self.key.bits())),
                            Some(cmp::Ordering::Greater) => None,
                        }
                    } else {
                        None
                    };

                    let max = if check_last {
                        match self.key.partial_cmp(&self.max.slice(self.key.bits())) {
                            None => unreachable!(),
                            Some(cmp::Ordering::Less) => None,
                            Some(cmp::Ordering::Equal) => Some(self.max.get(self.key.bits())),
                            Some(cmp::Ordering::Greater) => {
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
