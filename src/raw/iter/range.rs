use core::cmp;
use core::sync::atomic::Ordering;

use ribbit::atomic::Atomic128;

use crate::key;
use crate::node;
use crate::Edge;

pub(crate) enum RangeIter<'a, R, W> {
    Root { key: W, next: Option<u64> },
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
        Self::Root {
            key: W::default(),
            next: None,
        }
    }

    pub(crate) unsafe fn new(root: &'a Atomic128<Edge>, mut key: W, min: R, max: R) -> Self {
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
                next: Some(data),
            }
        } else if data == 0 {
            Self::Root { key, next: None }
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

    #[inline]
    pub(crate) fn collect<K: From<W>>(&mut self) -> Vec<(K, u64)> {
        match self {
            RangeIter::Root { key, next } => {
                crate::cold();
                match next.take() {
                    None => Vec::new(),
                    Some(value) => vec![(K::from(key.clone()), value)],
                }
            }
            RangeIter::Node(iter) => iter.collect(),
        }
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
    #[inline]
    fn collect<K: From<W>>(&mut self) -> Vec<(K, u64)> {
        let mut buffer = Vec::new();
        self.for_each(|key, value| {
            buffer.push((K::from(key.clone()), value));
        });
        buffer
    }

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
            let (len, iter) = self.stack.last_mut()?;

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
                        if YIELD {
                            return Some((&self.key, edge.data()));
                        } else {
                            apply(&self.key, edge.data());
                            continue 'horizontal;
                        }
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
                        self.stack.clear();
                        return None;
                    }

                    if YIELD {
                        return Some((&self.key, edge.data()));
                    } else {
                        apply(&self.key, edge.data());
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

                    let node = unsafe { Edge::next_node_unchecked(data) };
                    self.stack
                        .push((self.key.bits(), unsafe { node.iter_range(min, max) }));
                    continue 'vertical;
                }
            }
        }
    }
}
