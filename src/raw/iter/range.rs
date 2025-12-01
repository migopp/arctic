use core::ops::ControlFlow;
use core::sync::atomic::Ordering;

use ribbit::Atomic;

use crate::raw;
use crate::raw::edge;
use crate::raw::iter::Lower as _;
use crate::raw::iter::Upper as _;
use crate::raw::key;
use crate::raw::node::Lower as _;
use crate::raw::node::Upper as _;
use crate::raw::Edge;

pub(crate) enum RangeIter<
    'g,
    const REVERSE: bool,
    R: key::Read,
    W: key::Write,
    M: ribbit::Pack<Packed: edge::Meta>,
    B: raw::iter::Range<R>,
> {
    Root { writer: W, next: Option<u64> },
    Node(NodeIter<'g, REVERSE, R, W, M, B>),
}

impl<'g, const REVERSE: bool, R, W, M, B> Default for RangeIter<'g, REVERSE, R, W, M, B>
where
    R: key::Read<Edge = M>,
    W: key::Write<Edge = M>,
    W: From<R>,
    M: ribbit::Pack<Packed: edge::Meta>,
    B: raw::iter::Range<R>,
{
    fn default() -> Self {
        Self::Root {
            writer: W::default(),
            next: None,
        }
    }
}

impl<'g, const REVERSE: bool, R, W, M, B> RangeIter<'g, REVERSE, R, W, M, B>
where
    R: key::Read<Edge = M>,
    W: key::Write<Edge = M>,
    W: From<R>,
    M: ribbit::Pack<Packed: edge::Meta>,
    B: raw::iter::Range<R>,
{
    pub(crate) unsafe fn new_unchecked(root: &'g Atomic<Edge<M>>, prefix: R, range: B) -> Self {
        let edge = root.load_packed(Ordering::Acquire);

        let Some(child) = edge.child() else {
            return Self::default();
        };

        let key = edge.meta();
        let bits = prefix.bits();
        let mut writer = W::from(prefix);
        let len = writer.write(W::len_from_bits(bits), key);

        let mut lower = range.lower();
        let mut upper = range.upper();

        let Some((lower_byte, upper_byte)) = lower.check(key).zip(upper.check(key)) else {
            return Self::default();
        };

        match child {
            edge::Child::Value(value) => Self::Root {
                writer,
                next: Some(value),
            },
            edge::Child::Node(node) => {
                let mut stack = Vec::with_capacity(7);
                stack.push((len, unsafe {
                    node.entries_unchecked(lower_byte, upper_byte)
                }));

                Self::Node(NodeIter {
                    lower,
                    upper,
                    stack,
                    writer,
                })
            }
        }
    }

    #[inline]
    pub(crate) fn for_each<F: FnMut(&W, u64) -> ControlFlow<()>>(self, mut apply: F) {
        match self {
            RangeIter::Root { writer, mut next } => {
                crate::cold();
                if let Some(value) = next.take() {
                    let _ = apply(&writer, value);
                }
            }
            RangeIter::Node(mut iter) => iter.for_each(apply),
        }
    }

    #[inline]
    pub(crate) fn lend(&mut self) -> Option<(&W, u64)> {
        match self {
            RangeIter::Root { writer: key, next } => {
                crate::cold();
                let value = next.take()?;
                Some((key, value))
            }
            RangeIter::Node(iter) => iter.lend(),
        }
    }
}

pub(crate) struct NodeIter<
    'g,
    const REVERSE: bool,
    R: key::Read,
    W: key::Write,
    M: ribbit::Pack<Packed: edge::Meta> + 'g,
    B: raw::iter::Range<R>,
> {
    lower: B::Lower,
    upper: B::Upper,
    writer: W,
    stack: Vec<(
        W::Len,
        raw::node::NodeIter<
            'g,
            <B::Lower as raw::iter::Lower<R>>::Bound,
            <B::Upper as raw::iter::Upper<R>>::Bound,
            M,
        >,
    )>,
}

impl<'g, const REVERSE: bool, R, W, M, B> NodeIter<'g, REVERSE, R, W, M, B>
where
    R: key::Read<Edge = M>,
    W: key::Write<Edge = M>,
    M: ribbit::Pack<Packed: edge::Meta> + 'g,
    B: raw::iter::Range<R>,
{
    #[inline]
    fn lend(&mut self) -> Option<(&W, u64)> {
        self.walk::<true, _>(|_, _| ControlFlow::Continue(()))
    }

    #[inline]
    fn for_each<F: FnMut(&W, u64) -> ControlFlow<()>>(&mut self, apply: F) {
        self.walk::<false, _>(apply);
    }

    #[inline]
    fn walk<const YIELD: bool, F: FnMut(&W, u64) -> ControlFlow<()>>(
        &mut self,
        mut apply: F,
    ) -> Option<(&W, u64)> {
        'vertical: loop {
            let (len, iter) = self.stack.last_mut()?;
            let len = *len;

            'horizontal: loop {
                let Some((mut byte, mut edge)) = (if REVERSE {
                    iter.next_back()
                } else {
                    iter.next()
                }) else {
                    self.stack.pop();
                    continue 'vertical;
                };

                let mut len = len;
                let mut check_lower = iter.lower().check(byte);
                let mut check_upper = iter.upper().check(byte);

                'compress: loop {
                    let (meta, child) = {
                        let edge = edge.load_packed(Ordering::Acquire);
                        let Some(child) = edge.child() else {
                            continue 'horizontal;
                        };
                        let meta = edge.meta();
                        (meta, child)
                    };

                    len = self.writer.replace(len, byte, meta);

                    let lower = if check_lower {
                        match self.lower.check(meta) {
                            Some(lower) => lower,
                            None if REVERSE => {
                                self.stack.clear();
                                return None;
                            }
                            None => continue 'horizontal,
                        }
                    } else {
                        Default::default()
                    };

                    let upper = if check_upper {
                        match self.upper.check(meta) {
                            Some(upper) => upper,
                            None if REVERSE => continue 'horizontal,
                            None => {
                                self.stack.clear();
                                return None;
                            }
                        }
                    } else {
                        Default::default()
                    };

                    match child {
                        edge::Child::Value(value) if YIELD => {
                            return Some((&self.writer, value));
                        }
                        edge::Child::Value(value) => match apply(&self.writer, value) {
                            ControlFlow::Continue(()) => continue 'horizontal,
                            ControlFlow::Break(()) => {
                                self.stack.clear();
                                return None;
                            }
                        },
                        edge::Child::Node(node) => {
                            // Avoid pushing and popping iterators with only one child
                            match unsafe { node.entries_unchecked(lower, upper) }.try_into_single()
                            {
                                Ok((check_lower_, check_upper_, byte_, edge_)) => {
                                    check_lower = check_lower_;
                                    check_upper = check_upper_;
                                    byte = byte_;
                                    edge = edge_;
                                    continue 'compress;
                                }
                                Err(iter) => {
                                    self.stack.push((len, iter));
                                    continue 'vertical;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
