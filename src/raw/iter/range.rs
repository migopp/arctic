use core::cmp;
use core::sync::atomic::Ordering;

use crate::key;
use crate::node;
use crate::Edge;

pub(crate) enum RangeIter<'a, R, W> {
    Root {
        key: W,
        next: Option<u64>,
    },
    Node {
        key: W,
        min: R,
        max: R,
        frontier: Vec<(usize, node::RangeIter<'a>)>,
    },
}

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

    #[inline]
    pub(crate) unsafe fn new(root: ribbit::Packed<Edge>, mut key: W, min: R, max: R) -> Self {
        let meta = root.meta();
        let data = root.data();

        key.extend(root.meta().key());

        if meta.leaf() {
            if key < min || key > max {
                return Self::Root { key, next: None };
            }

            Self::Root {
                key,
                next: Some(root.data()),
            }
        } else if data == 0 {
            Self::Root { key, next: None }
        } else {
            let node = unsafe { Edge::next_node_unchecked(data) };

            validate!(key >= min.slice(key.bits()));
            validate!(key <= max.slice(key.bits()));

            let first = (key == min.slice(key.bits())).then(|| min.get(key.bits()));
            let last = (key == max.slice(key.bits())).then(|| max.get(key.bits()));

            let mut frontier = Vec::with_capacity(7);
            frontier.push((key.bits(), node.iter_range(first, last)));

            Self::Node {
                frontier,
                key,
                min,
                max,
            }
        }
    }

    #[inline]
    pub fn lend(&mut self) -> Option<(&W, u64)> {
        let (key, min, max, frontier) = match self {
            RangeIter::Root { key, next } => {
                crate::cold();
                let value = next.take()?;
                return Some((key, value));
            }
            RangeIter::Node {
                key,
                min,
                max,
                frontier,
            } => (key, min, max, frontier),
        };

        'vertical: loop {
            let (len, iter) = frontier.last_mut()?;

            loop {
                let Some((byte, edge)) = iter.next() else {
                    frontier.pop();
                    continue 'vertical;
                };

                let edge = edge.load_packed(Ordering::Acquire);
                let meta = edge.meta();
                let data = edge.data();

                if !meta.leaf() && data == 0 {
                    continue;
                }

                key.truncate(*len);
                key.push(byte);
                key.extend(meta.key());

                let check_first = Some(byte) == node::RangeIter::min(iter);
                let check_last = Some(byte) == node::RangeIter::max(iter);

                if !check_first && !check_last {
                    if meta.leaf() {
                        return Some((key, edge.data()));
                    } else {
                        let node = unsafe { Edge::next_node_unchecked(data) };
                        frontier.push((key.bits(), unsafe { node.iter_range(None, None) }));
                        continue 'vertical;
                    }
                }

                crate::cold();

                if meta.leaf() {
                    if check_first && *key < *min {
                        continue;
                    }

                    if check_last && *key > *max {
                        frontier.clear();
                        return None;
                    }

                    return Some((key, edge.data()));
                } else {
                    let min = if check_first {
                        match (*key).partial_cmp(&min.slice(key.bits())) {
                            None => unreachable!(),
                            Some(cmp::Ordering::Less) => continue,
                            Some(cmp::Ordering::Equal) => Some(min.get(key.bits())),
                            Some(cmp::Ordering::Greater) => None,
                        }
                    } else {
                        None
                    };

                    let max = if check_last {
                        match (*key).partial_cmp(&max.slice(key.bits())) {
                            None => unreachable!(),
                            Some(cmp::Ordering::Less) => None,
                            Some(cmp::Ordering::Equal) => Some(max.get(key.bits())),
                            Some(cmp::Ordering::Greater) => continue,
                        }
                    } else {
                        None
                    };

                    let node = unsafe { Edge::next_node_unchecked(data) };
                    frontier.push((key.bits(), unsafe { node.iter_range(min, max) }));
                    continue 'vertical;
                }
            }
        }
    }
}
