mod postorder;
mod range;

pub(crate) use postorder::PostorderIter;
pub use range::Range;
pub(crate) use range::RangeIter;
pub(crate) use range::Unbound;

use core::marker::PhantomData;
use core::ops::ControlFlow;
use core::ops::RangeFull;
use core::ptr::NonNull;

use ribbit::Atomic;

use crate::raw::cursor::path;
use crate::raw::key;
use crate::raw::key::Read as _;
use crate::raw::Cursor;
use crate::raw::Edge;
use crate::raw::Key;
use crate::Order;

pub(crate) struct Prefix<'k, 'g, K: Key, R = RangeFull> {
    root: NonNull<Atomic<Edge<K::Edge>>>,
    prefix: K::Read<'k>,
    range: R,
    _global: PhantomData<&'g Atomic<Edge<K::Edge>>>,
}

impl<'k, 'g, K, R> Prefix<'k, 'g, K, R>
where
    K: Key,
    R: Range<'k, K>,
{
    #[inline]
    pub(crate) unsafe fn new_all(root: &'g Atomic<Edge<K::Edge>>) -> Prefix<'k, 'g, K, RangeFull> {
        Prefix::new(root, K::Read::default(), ..)
    }

    pub(crate) unsafe fn new_prefix(
        root: &'g Atomic<Edge<K::Edge>>,
        prefix: K::Read<'k>,
    ) -> Option<Prefix<'k, 'g, K, RangeFull>> {
        let mut cursor = unsafe { Cursor::<K, path::Discard>::new(root, prefix) };
        cursor.traverse_prefix()?;
        let root = cursor.edge();
        let bits = cursor.bits();
        let prefix = prefix.prefix(bits);
        Some(unsafe { Prefix::new(root, prefix, ..) })
    }

    pub(crate) unsafe fn new_range(
        root: &'g Atomic<Edge<K::Edge>>,
        range: R,
    ) -> Option<Prefix<'k, 'g, K, R>>
    where
        R: Range<'k, K>,
    {
        let prefix = range.common_prefix();
        let mut cursor = unsafe { Cursor::<K, path::Discard>::new(root, prefix) };
        cursor.traverse_prefix()?;

        let root = cursor.edge();
        let bits = cursor.bits();
        let prefix = prefix.prefix(bits);

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
        EntryIter(unsafe { RangeIter::new_unchecked(self.root, self.prefix, self.range.clone()) })
    }

    #[inline]
    pub(crate) fn values<O: Order>(&self) -> ValueIter<'k, 'g, K, R, O> {
        ValueIter(unsafe { RangeIter::new_unchecked(self.root, self.prefix, self.range.clone()) })
    }
}

pub(crate) struct EntryIter<'k, 'g, K: Key, R: Range<'k, K>, O>(
    RangeIter<'k, 'g, K, K::Write, R, O>,
);

impl<'k, 'g, K, R, O> EntryIter<'k, 'g, K, R, O>
where
    K: Key,
    R: Range<'k, K>,
    O: Order,
{
    #[inline]
    pub(crate) fn lend(&mut self) -> Option<(K::Borrow<'_>, u64)> {
        self.0
            .lend()
            .map(|(key, value)| (unsafe { K::borrow_writer_unchecked(key) }, value))
    }

    #[inline]
    pub(crate) fn for_each_internal<F: FnMut((K::Borrow<'_>, u64)) -> ControlFlow<()>>(
        self,
        mut apply: F,
    ) {
        self.0.for_each_internal(|(key, value)| {
            apply((unsafe { K::borrow_writer_unchecked(key) }, value))
        })
    }
}

impl<'k, 'g, K, R, O> Iterator for EntryIter<'k, 'g, K, R, O>
where
    K: Key,
    R: Range<'k, K>,
    O: Order,
{
    type Item = (K, u64);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.0
            .lend()
            .map(|(key, value)| (unsafe { K::from_writer_unchecked(key.clone()) }, value))
    }
}

/// Iterator over raw values only
pub(crate) struct ValueIter<'k, 'g, K: Key, R: Range<'k, K>, O>(
    RangeIter<'k, 'g, K, key::Ignore<K::Edge>, R, O>,
);

impl<'k, 'g, K, R, O> ValueIter<'k, 'g, K, R, O>
where
    K: Key,
    R: Range<'k, K>,
    O: Order,
{
    #[inline]
    pub(crate) fn for_each_internal<F: FnMut(u64) -> ControlFlow<()>>(self, mut apply: F) {
        self.0.for_each_internal(|(_, value)| apply(value))
    }
}

impl<'k, 'g, K, R, O> Iterator for ValueIter<'k, 'g, K, R, O>
where
    K: Key,
    R: crate::raw::iter::Range<'k, K>,
    O: Order,
{
    type Item = u64;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.0.lend().map(|(_, value)| value)
    }
}
