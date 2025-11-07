use core::marker::PhantomData;

use ribbit::atomic::Atomic128;

use crate::concurrent::cursor;
use crate::concurrent::hazard;
use crate::concurrent::Value;
use crate::iter::Order;
use crate::key;
use crate::raw;
use crate::raw::Edge;

pub(crate) use crate::raw::iter::Prefix;
pub(crate) use crate::raw::iter::Range;
use crate::Key;

/// Provide safe memory reclamation and strongly-typed values over
/// scan iterators in [`crate::raw::iter`].
pub(crate) trait Scan: raw::iter::Scan + Sized {
    #[expect(private_interfaces)]
    fn guard<'g, 'l, K, V, H>(
        cursor: cursor::Prefix<'g, 'l, K::Read<'l>, (), V, H>,
        input: Self::Input<'l, K::Read<'l>>,
    ) -> PrefixGuard<'g, 'l, K, V, Self>
    where
        K: Key,
        V: Value,
        H: cursor::path::History<'g, K::Read<'l>, ()>;
}

impl<T> Scan for T
where
    T: raw::iter::Scan,
{
    #[expect(private_interfaces)]
    fn guard<'g, 'l, K, V, H>(
        cursor: cursor::Prefix<'g, 'l, K::Read<'l>, (), V, H>,
        input: Self::Input<'l, K::Read<'l>>,
    ) -> PrefixGuard<'g, 'l, K, V, Self>
    where
        K: Key,
        V: Value,
        H: cursor::path::History<'g, K::Read<'l>, ()>,
    {
        PrefixGuard {
            root: cursor.edge(),
            guard: cursor.into_guard().guard_prefix(),
            input,
        }
    }
}

/// Guard all nodes and values below this prefix from memory reclamation.
#[expect(private_bounds)]
pub struct PrefixGuard<'g, 'l, K: Key, V: Value, S: Scan> {
    guard: hazard::PrefixGuard<'g, 'l, V>,
    root: &'g Atomic128<Edge<()>>,
    input: S::Input<'l, K::Read<'l>>,
}

#[expect(private_bounds)]
impl<'g, 'l, K, V, S> PrefixGuard<'g, 'l, K, V, S>
where
    K: Key,
    V: Value,
    S: Scan,
{
    #[inline]
    #[expect(private_interfaces)]
    #[expect(clippy::type_complexity)]
    pub fn entries<O>(
        &self,
    ) -> EntryIter<'g, 'l, '_, K, V, O, S::Iter<'g, K::Read<'l>, K::Write, (), O>>
    where
        O: Order,
    {
        EntryIter {
            guard: &self.guard,
            iter: unsafe { S::new_unchecked(self.root, self.input) },
            _type: PhantomData,
        }
    }

    #[inline]
    #[expect(private_interfaces)]
    #[expect(clippy::type_complexity)]
    pub fn values<O>(
        &self,
    ) -> ValueIter<'g, 'l, '_, K::Read<'l>, V, O, S::Iter<'g, K::Read<'l>, key::Ignore, (), O>>
    where
        O: Order,
    {
        ValueIter {
            guard: &self.guard,
            iter: unsafe { S::new_unchecked(self.root, self.input) },
            _type: PhantomData,
        }
    }

    pub(crate) fn guard_value(self) -> V::LinearizableGuard<'g, 'l> {
        unsafe { V::downgrade_guard(self.guard) }
    }
}

/// Iterator over keys and values
pub struct EntryIter<'g, 'l, 'guard, K: Key, V: Value, O, I> {
    guard: &'guard hazard::PrefixGuard<'g, 'l, V>,
    iter: I,
    _type: PhantomData<(K, O)>,
}

#[expect(private_bounds)]
impl<'g, 'l, 'guard, K, V, O, I> EntryIter<'g, 'l, 'guard, K, V, O, I>
where
    K: Key,
    V: Value,
    I: raw::iter::ScanIter<'g, K::Read<'l>, K::Write, (), O>,
{
    #[inline]
    pub fn lend(&mut self) -> Option<(K::Borrow<'_>, V::Borrow<'guard>)> {
        self.iter.lend().map(|(key, value)| {
            (unsafe { K::borrow_writer_unchecked(key) }, unsafe {
                V::guard_borrow(self.guard, value)
            })
        })
    }

    #[inline]
    pub fn for_each<F: FnMut(K::Borrow<'_>, V::Borrow<'guard>)>(self, mut apply: F) {
        raw::iter::ScanIter::for_each(self.iter, |key, value| {
            apply(unsafe { K::borrow_writer_unchecked(key) }, unsafe {
                V::guard_borrow(self.guard, value)
            })
        })
    }

    #[inline]
    pub fn for_each_raw<F: FnMut(&K::Write, u64)>(self, apply: F) {
        raw::iter::ScanIter::for_each(self.iter, apply)
    }
}

impl<'g, 'l, 'guard, K, V, O, I> Iterator for EntryIter<'g, 'l, 'guard, K, V, O, I>
where
    K: Key,
    V: Value,
    I: raw::iter::ScanIter<'g, K::Read<'l>, K::Write, (), O>,
{
    type Item = (K, V::Borrow<'guard>);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.iter.lend().map(|(key, value)| {
            (unsafe { K::from_writer_unchecked(key.clone()) }, unsafe {
                V::guard_borrow(self.guard, value)
            })
        })
    }
}

/// Iterator over values only
pub struct ValueIter<'g, 'l, 'guard, R, V: Value, O, I> {
    guard: &'guard hazard::PrefixGuard<'g, 'l, V>,
    iter: I,
    _type: PhantomData<(R, O)>,
}

#[expect(private_bounds)]
impl<'g, 'l, 'guard, R, V, O, I> ValueIter<'g, 'l, 'guard, R, V, O, I>
where
    V: Value,
    I: raw::iter::ScanIter<'g, R, key::Ignore, (), O>,
{
    #[inline]
    pub fn lend(&mut self) -> Option<V::Borrow<'guard>> {
        self.iter
            .lend()
            .map(|(key::Ignore, value)| unsafe { V::guard_borrow(self.guard, value) })
    }

    #[inline]
    pub fn for_each<F: FnMut(V::Borrow<'guard>)>(self, mut apply: F) {
        raw::iter::ScanIter::for_each(self.iter, |key::Ignore, value| {
            apply(unsafe { V::guard_borrow(self.guard, value) })
        })
    }
}

impl<'g, 'l, 'guard, R, V, O, I> Iterator for ValueIter<'g, 'l, 'guard, R, V, O, I>
where
    V: Value,
    I: raw::iter::ScanIter<'g, R, key::Ignore, (), O>,
{
    type Item = V::Borrow<'guard>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.lend()
    }
}
