use core::iter;
use core::mem;
use core::sync::atomic::Ordering;
use std::rc::Rc;

use crate::cursor;
use crate::cursor::Cursor;
use crate::cursor::Op;
use crate::edge;
use crate::node;
use crate::Edge;

pub struct Raw {
    root: Edge,
}

impl Default for Raw {
    fn default() -> Self {
        Raw {
            root: Edge::default(),
        }
    }
}

impl Raw {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&self, key: &[u8], value: u64) -> Option<u64> {
        match self.insert_optimistic(key, value) {
            Ok(old) => old,
            Err(()) => self.insert_pessimistic(key, value),
        }
    }

    #[inline]
    fn insert_optimistic(&self, key: &[u8], value: u64) -> Result<Option<u64>, ()> {
        self.insert_impl::<cursor::Optimistic>(key, value)
    }

    #[cold]
    fn insert_pessimistic(&self, key: &[u8], value: u64) -> Option<u64> {
        self.insert_impl::<cursor::Pessimistic>(key, value).unwrap()
    }

    fn insert_impl<'a, P: cursor::History<'a>>(
        &'a self,
        key: &[u8],
        value: u64,
    ) -> Result<Option<u64>, P::PopError> {
        let mut cursor = Cursor::<P>::new(&self.root, key);

        loop {
            let (op, (old_meta, old_data), (new_meta, new_data)) = cursor.traverse_strong(value);

            let meta = match cursor.here().compare_exchange(
                (old_meta.unfreeze(), old_data),
                (new_meta, new_data),
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok((meta, data)) => {
                    crate::stat::increment(&op);
                    match (op, meta.kind) {
                        (Op::Edge(edge::Op::Insert), node::Kind::None) => return Ok(None),
                        (Op::Edge(edge::Op::Insert), node::Kind::Leaf) => {
                            return Ok(Some(data.to_leaf()))
                        }
                        // FIXME: retire old allocation with SMR
                        _ => continue,
                    }
                }
                Err((meta, _)) => meta,
            };

            match op {
                Op::Node(node::Op::Destroy | node::Op::Compress)
                | Op::Edge(edge::Op::Insert | edge::Op::Remove) => (),

                Op::Node(node::Op::Grow | node::Op::Replace | node::Op::Shrink)
                | Op::Edge(edge::Op::Create | edge::Op::Expand) => unsafe {
                    new_data.deallocate(new_meta.kind)
                },
            }

            if meta.frozen {
                cursor.pop()?;
            }
        }
    }

    pub fn get(&self, key: &[u8]) -> Option<u64> {
        let mut cursor = Cursor::<cursor::Optimistic>::new(&self.root, key);
        let (_, data) = cursor.traverse_weak()?;
        Some(data.to_leaf())
    }

    pub fn remove(&self, key: &[u8]) -> Option<u64> {
        let mut cursor = Cursor::<cursor::Optimistic>::new(&self.root, key);
        let (mut old_meta, mut old_data) = cursor.traverse_weak()?;
        old_meta = old_meta.unfreeze();
        let edge = cursor.here();

        loop {
            match edge.compare_exchange(
                (old_meta, old_data),
                (
                    edge::Meta {
                        key: old_meta.key,
                        frozen: false,
                        kind: node::Kind::None,
                    },
                    old_data,
                ),
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => break,
                Err((meta, _)) if matches!(meta.kind, node::Kind::None) => return None,
                Err((meta, _)) if meta != old_meta => todo!(
                    "Handle metadata conflict in remove: expected {:?} but found {:?}",
                    old_meta,
                    meta
                ),
                Err((meta, data)) => {
                    old_meta = meta;
                    old_data = data;
                }
            }
        }

        Some(old_data.to_leaf())
    }

    pub fn update(&self, key: &[u8], value: u64) -> Option<u64> {
        let mut cursor = Cursor::<cursor::Optimistic>::new(&self.root, key);
        let (mut old_meta, mut old_data) = cursor.traverse_weak()?;
        old_meta = old_meta.unfreeze();
        let edge = cursor.here();

        loop {
            match edge.compare_exchange(
                (old_meta, old_data),
                (
                    edge::Meta {
                        key: old_meta.key,
                        frozen: false,
                        kind: node::Kind::Leaf,
                    },
                    edge::Data::new_leaf(value),
                ),
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => break,

                Err((meta, _))
                    if meta.frozen
                        || meta.key != old_meta.key
                        || !matches!(meta.kind, node::Kind::None | node::Kind::Leaf) =>
                {
                    todo!(
                        "Handle metadata conflict in update: expected {:?} but found {:?}",
                        old_meta,
                        meta
                    )
                }
                Err((meta, data)) => {
                    old_meta = meta;
                    old_data = data;
                }
            }
        }

        Some(old_data.to_leaf())
    }

    pub fn iter(&mut self) -> impl Iterator<Item = (Rc<Vec<u8>>, u64)> + '_ {
        self.preorder()
            .filter_map(|(_, key, meta, data)| match meta.child()? {
                edge::Child::Leaf => Some((key, data.to_leaf())),
                edge::Child::Node(_) => None,
            })
    }

    pub fn keys(&mut self) -> impl Iterator<Item = Rc<Vec<u8>>> + '_ {
        self.iter().map(|(key, _)| key)
    }

    pub fn values(&mut self) -> impl Iterator<Item = u64> + '_ {
        self.iter().map(|(_, value)| value)
    }

    pub(crate) fn preorder(
        &mut self,
    ) -> impl Iterator<Item = (usize, Rc<Vec<u8>>, edge::Meta, edge::Data)> + '_ {
        Iter::new(&mut self.root)
    }
}

struct Iter<'a> {
    // Workaround for lending iterator
    // https://users.rust-lang.org/t/how-to-write-an-iterator-that-returns-references-to-itself/72386/5
    key: Rc<Vec<u8>>,

    // TODO: allow starting traversal at a given prefix?
    frontier: Vec<(usize, Or<IterRoot<'a>, IterNode<'a>>)>,
}

type IterRoot<'a> = iter::Peekable<iter::Zip<iter::Once<bool>, node::EdgeIter<'a>>>;
type IterNode<'a> = iter::Peekable<iter::Zip<iter::Repeat<bool>, node::Iter<'a>>>;

impl<'a> Iter<'a> {
    fn new(root: &'a mut Edge) -> Self {
        Self {
            key: Rc::new(Vec::new()),
            frontier: vec![(
                0,
                Or::L(
                    iter::zip(
                        iter::once(false),
                        node::EdgeIter::new(core::slice::from_ref(root)),
                    )
                    .peekable(),
                ),
            )],
        }
    }
}

impl<'a> Iterator for Iter<'a> {
    type Item = (usize, Rc<Vec<u8>>, edge::Meta, edge::Data);

    fn next(&mut self) -> Option<Self::Item> {
        'vertical: loop {
            // NOTE: we use `saturating_sub` to avoid underflow.
            //
            // If `self.frontier.len()` == 0, we will immediately return at `last_mut()`.
            // We can't move the len call after because `self.frontier` is mutably borrowed.
            let depth = self.frontier.len().saturating_sub(1);
            let (len, iter) = self.frontier.last_mut()?;

            'horizontal: loop {
                let Some((descend, byte, edge)) = (match iter {
                    Or::L(iter_root) => iter_root
                        .peek_mut()
                        .map(|(descend, edge)| (descend, None, edge)),
                    Or::R(iter_node) => iter_node
                        .peek_mut()
                        .map(|(descend, (key, edge))| (descend, Some(*key), edge)),
                }) else {
                    Rc::make_mut(&mut self.key).truncate(*len);
                    self.frontier.pop();
                    continue 'vertical;
                };

                let meta = edge.load_low(Ordering::Relaxed);

                // Skip empty edges
                let Some(child) = meta.child() else {
                    iter.skip();
                    continue 'horizontal;
                };

                let data = edge.load_high(Ordering::Acquire);

                // Update key for current edge
                let key = Rc::make_mut(&mut self.key);

                // Produce edge before traversing for preorder traversal
                if !mem::replace(descend, true) {
                    key.extend(byte.into_iter().chain(meta.key.bytes()));
                    return Some((depth, Rc::clone(&self.key), meta, data));
                }

                iter.skip();
                let len = key.len() - meta.key.len.to_usize() - byte.is_some() as usize;

                match child {
                    edge::Child::Leaf => {
                        key.truncate(len);
                        continue 'horizontal;
                    }
                    edge::Child::Node(kind) => {
                        let node = unsafe { data.to_node(kind) };
                        self.frontier.push((
                            len,
                            Or::R(iter::repeat(false).zip(unsafe { node.iter() }).peekable()),
                        ));
                        continue 'vertical;
                    }
                }
            }
        }
    }
}

enum Or<L, R> {
    L(L),
    R(R),
}

impl<L, R> Or<L, R>
where
    L: Iterator,
    R: Iterator,
{
    fn skip(&mut self) {
        match self {
            Or::L(left) => {
                left.next();
            }
            Or::R(right) => {
                right.next();
            }
        }
    }
}
