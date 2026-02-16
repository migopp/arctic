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
    pub(crate) fn entries<const REVERSE: bool>(&self) -> EntryIter<'k, 'g, REVERSE, K, R> {
        EntryIter(unsafe { RangeIter::new_unchecked(self.root, self.prefix, self.range.clone()) })
    }

    #[inline]
    pub(crate) fn values<const REVERSE: bool>(&self) -> ValueIter<'k, 'g, REVERSE, K, R> {
        ValueIter(unsafe { RangeIter::new_unchecked(self.root, self.prefix, self.range.clone()) })
    }
}

pub(crate) struct EntryIter<'k, 'g, const REVERSE: bool, K: Key, R: Range<'k, K>>(
    RangeIter<'k, 'g, REVERSE, K, R, K::Write>,
);

impl<'k, 'g, const REVERSE: bool, K, R> EntryIter<'k, 'g, REVERSE, K, R>
where
    K: Key,
    R: Range<'k, K>,
{
    #[inline]
    pub(crate) fn lend(&mut self) -> Option<(K::Borrow<'_>, u64)> {
        self.0
            .lend()
            .map(|(key, value)| (unsafe { K::borrow_writer_unchecked(key) }, value))
    }

    #[inline]
    pub(crate) fn for_each<F: FnMut((K::Borrow<'_>, u64)) -> ControlFlow<()>>(self, mut apply: F) {
        self.0
            .for_each(|(key, value)| apply((unsafe { K::borrow_writer_unchecked(key) }, value)))
    }
}

impl<'k, 'g, const REVERSE: bool, K, R> Iterator for EntryIter<'k, 'g, REVERSE, K, R>
where
    K: Key,
    R: Range<'k, K>,
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
pub(crate) struct ValueIter<'k, 'g, const REVERSE: bool, K: Key, R: Range<'k, K>>(
    RangeIter<'k, 'g, REVERSE, K, R, key::Ignore<K::Edge>>,
);

impl<'k, 'g, const REVERSE: bool, K, R> ValueIter<'k, 'g, REVERSE, K, R>
where
    K: Key,
    R: Range<'k, K>,
{
    #[inline]
    pub(crate) fn for_each<F: FnMut(u64) -> ControlFlow<()>>(self, mut apply: F) {
        self.0.for_each(|(_, value)| apply(value))
    }
}

impl<'k, 'g, const REVERSE: bool, K, R> Iterator for ValueIter<'k, 'g, REVERSE, K, R>
where
    K: Key,
    R: crate::raw::iter::Range<'k, K>,
{
    type Item = u64;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.0.lend().map(|(_, value)| value)
    }
}
