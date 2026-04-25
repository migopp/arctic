mod postorder;
pub(crate) mod range;

pub(crate) use postorder::PostorderIter;
pub use range::Range;
pub(crate) use range::RangeIter;
pub(crate) use range::Unbound;

use core::marker::PhantomData;
use core::ops::ControlFlow;
use core::ops::RangeFull;
use core::ptr::NonNull;

use ribbit::Atomic;

use crate::Order;
use crate::raw;
use crate::raw::Cursor;
use crate::raw::Edge;
use crate::raw::Key;
use crate::raw::cursor::path;
use crate::raw::key;
use crate::raw::key::Read as _;

pub(crate) struct Prefix<'k, 'g, K, R = RangeFull>
where
    K: Key,
{
    root: NonNull<Atomic<Edge<K::Edge>>>,
    prefix: K::Read<'k>,
    range: R,
    _global: PhantomData<&'g Atomic<Edge<K::Edge>>>,
}

impl<'k, 'g, K, R> Prefix<'k, 'g, K, R>
where
    K: Key,
    R: raw::iter::Range<K::Read<'k>>,
{
    #[inline]
    pub(crate) unsafe fn new_all(root: &'g Atomic<Edge<K::Edge>>) -> Prefix<'k, 'g, K, RangeFull> {
        unsafe { Prefix::new(root, K::Read::default(), ..) }
    }

    pub(crate) unsafe fn new_prefix(
        root: &'g Atomic<Edge<K::Edge>>,
        prefix: K::Read<'k>,
    ) -> Option<Prefix<'k, 'g, K, RangeFull>> {
        let mut cursor = unsafe { Cursor::<_, path::Discard>::new(root, prefix) };
        cursor.traverse_prefix()?;
        let root = cursor.edge();
        let len = cursor.len();
        let prefix = prefix.prefix(len);
        Some(unsafe { Prefix::new(root, prefix, ..) })
    }

    pub(crate) unsafe fn new_range(
        root: &'g Atomic<Edge<K::Edge>>,
        range: R,
    ) -> Option<Prefix<'k, 'g, K, R>>
    where
        R: Range<K::Read<'k>>,
    {
        let prefix = range.common_prefix();
        let mut cursor = unsafe { Cursor::<_, path::Discard>::new(root, prefix) };
        cursor.traverse_prefix()?;

        let root = cursor.edge();
        let len = cursor.len();
        let prefix = prefix.prefix(len);

        Some(unsafe { Prefix::new(root, prefix, range) })
    }

    #[inline]
    unsafe fn new(
        root: &'g Atomic<Edge<K::Edge>>,
        prefix: K::Read<'k>,
        range: R,
    ) -> Prefix<'k, 'g, K, R> {
        Prefix {
            root: NonNull::from(root),
            prefix,
            range,
            _global: PhantomData,
        }
    }

    #[inline]
    pub(crate) fn entries<O: Order>(&self) -> EntryIter<'k, 'g, K, R, O> {
        EntryIter(unsafe { RangeIter::new_unchecked(self.root, self.prefix, &self.range) })
    }

    #[inline]
    pub(crate) fn values<O: Order>(&self) -> ValueIter<'k, 'g, K, R, O> {
        ValueIter(unsafe { RangeIter::new_unchecked(self.root, self.prefix, &self.range) })
    }
}

pub(crate) struct EntryIter<'k, 'g, K: Key, R: Range<K::Read<'k>>, O>(
    RangeIter<'g, K::Read<'k>, K::Write, R, O>,
);

impl<'k, 'g, K, R, O> EntryIter<'k, 'g, K, R, O>
where
    K: Key,
    R: Range<K::Read<'k>>,
    O: Order,
{
    #[inline]
    pub(crate) fn lend(&mut self) -> Option<(&K::Borrowed, u64, NonNull<Atomic<Edge<K::Edge>>>)> {
        self.0.lend().map(|(writer, value, edge)| {
            (unsafe { K::borrow_writer_unchecked(writer) }, value, edge)
        })
    }

    #[inline]
    pub(crate) fn for_each_internal<
        F: FnMut((&K::Borrowed, u64, NonNull<Atomic<Edge<K::Edge>>>)) -> ControlFlow<()>,
    >(
        self,
        mut apply: F,
    ) {
        self.0.for_each_internal(|(writer, value, edge)| {
            apply((unsafe { K::borrow_writer_unchecked(writer) }, value, edge))
        })
    }
}

/// Iterator over raw values only
pub(crate) struct ValueIter<'k, 'g, K: Key, R: Range<K::Read<'k>>, O>(
    RangeIter<'g, K::Read<'k>, key::Discard<K::Read<'k>>, R, O>,
);

impl<'k, 'g, K, R, O> ValueIter<'k, 'g, K, R, O>
where
    K: Key,
    R: Range<K::Read<'k>>,
    O: Order,
{
    #[inline]
    pub(crate) fn lend(&mut self) -> Option<(u64, NonNull<Atomic<Edge<K::Edge>>>)> {
        self.0.lend().map(|(_, value, edge)| (value, edge))
    }

    #[inline]
    pub(crate) fn for_each_internal<
        F: FnMut((u64, NonNull<Atomic<Edge<K::Edge>>>)) -> ControlFlow<()>,
    >(
        self,
        mut apply: F,
    ) {
        self.0
            .for_each_internal(|(_, value, edge)| apply((value, edge)))
    }
}
