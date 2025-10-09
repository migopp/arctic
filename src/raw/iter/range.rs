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
        frontier: Vec<(usize, bool, bool, node::RangeIter<'a>)>,
    },
}

impl<'a, R, W> RangeIter<'a, R, W>
where
    R: key::Read,
    W: key::Write<Len = usize> + PartialOrd<R>,
{
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

            validate_eq!(key, min.slice(key.bits()));
            validate_eq!(key, max.slice(key.bits()));

            Self::Node {
                frontier: vec![(
                    key.bits(),
                    true,
                    true,
                    node.iter_range(min.get(key.bits()), max.get(key.bits())),
                )],
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
            let (len, check_first, check_last, iter) = frontier.last_mut()?;

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

                let check_first = *check_first && byte == node::RangeIter::min(iter);
                let check_last = *check_last && byte == node::RangeIter::max(iter);

                if check_last && *key > *max {
                    frontier.clear();
                    return None;
                }

                if meta.leaf() {
                    if check_first && *key < *min {
                        continue;
                    }

                    return Some((key, edge.data()));
                } else {
                    if check_first && *key < min.slice(key.bits()) {
                        continue;
                    }

                    let min = if check_first { min.get(key.bits()) } else { 0 };
                    let max = if check_last { max.get(key.bits()) } else { 255 };

                    let node = unsafe { Edge::next_node_unchecked(data) };
                    frontier.push((key.bits(), check_first, check_last, unsafe {
                        node.iter_range(min, max)
                    }));
                    continue 'vertical;
                }
            }
        }
    }
}
