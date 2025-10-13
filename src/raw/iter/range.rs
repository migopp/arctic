use core::cmp;
use core::sync::atomic::Ordering;

use ribbit::atomic::Atomic128;

use crate::key;
use crate::node;
use crate::Edge;

pub(crate) enum RangeIter<'a, R, W> {
    Root(RootIter<W>),
    Node(NodeIter<'a, R, W>),
}

const _: [(); 32] = [(); size_of::<(usize, node::RangeIter<'static>)>()];

impl<'a, R, W> RangeIter<'a, R, W>
where
    R: key::Read,
    W: key::Write<Len = usize> + PartialOrd<R>,
{
    #[inline]
    pub(crate) fn empty() -> Self {
        Self::Root(RootIter {
            key: W::default(),
            next: None,
        })
    }

    pub(crate) unsafe fn new(root: &'a Atomic128<Edge>, mut key: W, min: R, max: R) -> Self {
        let edge = root.load_packed(Ordering::Acquire);
        let meta = edge.meta();
        let data = edge.data();
        key.extend(edge.meta().key());

        if meta.leaf() {
            if key < min || key > max {
                return Self::Root(RootIter { key, next: None });
            }

            Self::Root(RootIter {
                key,
                next: Some(data),
            })
        } else if data == 0 {
            Self::Root(RootIter { key, next: None })
        } else {
            let node = unsafe { Edge::next_node_unchecked(data) };

            validate!(key >= min.slice(key.bits()));
            validate!(key <= max.slice(key.bits()));

            let first = (key == min.slice(key.bits())).then(|| min.get(key.bits()));
            let last = (key == max.slice(key.bits())).then(|| max.get(key.bits()));

            let mut stack = Vec::with_capacity(7);
            stack.push((key.bits(), node.iter_range(first, last)));

            Self::Node(NodeIter {
                stack,
                key,
                min,
                max,
            })
        }
    }

    #[inline]
    pub fn lend(&mut self) -> Option<(&W, u64)> {
        match self {
            RangeIter::Root(iter) => iter.lend(),
            RangeIter::Node(iter) => iter.lend(),
        }
    }
}

pub(crate) struct RootIter<W> {
    key: W,
    next: Option<u64>,
}

impl<W: Clone> RootIter<W> {
    pub(crate) fn collect<K: From<W>>(&mut self) -> Vec<(K, u64)> {
        self.next
            .take()
            .map(|value| (K::from(self.key.clone()), value))
            .into_iter()
            .collect()
    }

    #[inline]
    pub(crate) fn lend(&mut self) -> Option<(&W, u64)> {
        let value = self.next.take()?;
        Some((&self.key, value))
    }
}

pub(crate) struct NodeIter<'a, R, W> {
    min: R,
    max: R,
    key: W,
    stack: Vec<(usize, node::RangeIter<'a>)>,
}

impl<'a, R, W> NodeIter<'a, R, W>
where
    R: key::Read,
    W: key::Write<Len = usize> + PartialOrd<R>,
{
    pub(crate) fn collect<K: From<W>>(&mut self) -> Vec<(K, u64)> {
        let mut buffer = Vec::new();
        self.for_each(|key, value| {
            buffer.push((K::from(key.clone()), value));
        });
        buffer
    }

    pub(crate) fn for_each<F: FnMut(&W, u64)>(&mut self, mut apply: F) {
        'vertical: loop {
            let Some((len, iter)) = self.stack.last_mut() else {
                return;
            };

            'horizontal: loop {
                let Some((byte, edge)) = iter.next() else {
                    self.stack.pop();
                    continue 'vertical;
                };

                let edge = edge.load_packed(Ordering::Acquire);
                let meta = edge.meta();
                let data = edge.data();

                if !meta.leaf() && data == 0 {
                    continue 'horizontal;
                }

                self.key.truncate(*len);
                self.key.push(byte);

                unsafe {
                    // SAFETY: we just pushed `byte` onto `key`
                    self.key.extend_nonempty_unchecked(meta.key());
                }

                let check_first = Some(byte) == node::RangeIter::min(iter);
                let check_last = Some(byte) == node::RangeIter::max(iter);

                if !check_first && !check_last {
                    if meta.leaf() {
                        apply(&self.key, edge.data());
                        continue 'horizontal;
                    } else {
                        let node = unsafe { Edge::next_node_unchecked(data) };
                        self.stack
                            .push((self.key.bits(), unsafe { node.iter_range(None, None) }));
                        continue 'vertical;
                    }
                }

                crate::cold();

                if meta.leaf() {
                    if check_first && self.key < self.min {
                        continue 'horizontal;
                    }

                    if check_last && self.key > self.max {
                        return;
                    }

                    apply(&self.key, edge.data());
                    continue 'horizontal;
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
                            Some(cmp::Ordering::Greater) => return,
                        }
                    } else {
                        None
                    };

                    let node = unsafe { Edge::next_node_unchecked(data) };
                    self.stack
                        .push((self.key.bits(), unsafe { node.iter_range(min, max) }));
                    continue 'vertical;
                }
            }
        }
    }

    pub(crate) fn lend(&mut self) -> Option<(&W, u64)> {
        'vertical: loop {
            let (len, iter) = self.stack.last_mut()?;

            loop {
                let Some((byte, edge)) = iter.next() else {
                    self.stack.pop();
                    continue 'vertical;
                };

                let edge = edge.load_packed(Ordering::Acquire);
                let meta = edge.meta();
                let data = edge.data();

                if !meta.leaf() && data == 0 {
                    continue;
                }

                self.key.truncate(*len);
                self.key.push(byte);

                unsafe {
                    // SAFETY: we just pushed `byte` onto `key`
                    self.key.extend_nonempty_unchecked(meta.key());
                }

                let check_first = Some(byte) == node::RangeIter::min(iter);
                let check_last = Some(byte) == node::RangeIter::max(iter);

                if !check_first && !check_last {
                    if meta.leaf() {
                        return Some((&self.key, edge.data()));
                    } else {
                        let node = unsafe { Edge::next_node_unchecked(data) };
                        self.stack
                            .push((self.key.bits(), unsafe { node.iter_range(None, None) }));
                        continue 'vertical;
                    }
                }

                crate::cold();

                if meta.leaf() {
                    if check_first && self.key < self.min {
                        continue;
                    }

                    if check_last && self.key > self.max {
                        self.stack.clear();
                        return None;
                    }

                    return Some((&self.key, edge.data()));
                } else {
                    let min = if check_first {
                        match self.key.partial_cmp(&self.min.slice(self.key.bits())) {
                            None => unreachable!(),
                            Some(cmp::Ordering::Less) => continue,
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
                            Some(cmp::Ordering::Greater) => continue,
                        }
                    } else {
                        None
                    };

                    let node = unsafe { Edge::next_node_unchecked(data) };
                    self.stack
                        .push((self.key.bits(), unsafe { node.iter_range(min, max) }));
                    continue 'vertical;
                }
            }
        }
    }
}
