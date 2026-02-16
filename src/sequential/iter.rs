use core::marker::PhantomData;
use core::ops::ControlFlow;
use core::ops::Deref;

use crate::raw;
use crate::raw::Key;
use crate::sequential::Value;

pub struct Prefix<'k, 'g, K: Key, V, R> {
    inner: raw::iter::Prefix<'k, 'g, K, R>,
    _value: PhantomData<&'g V>,
}

impl<'k, 'g, K: Key, V: Value, R: raw::iter::Range<'k, K>> Prefix<'k, 'g, K, V, R> {
    #[inline]
    pub(crate) unsafe fn new(prefix: raw::iter::Prefix<'k, 'g, K, R>) -> Self {
        Self {
            inner: prefix,
            _value: PhantomData,
        }
    }

    #[inline]
    pub fn entries<const REVERSE: bool>(&self) -> EntryIter<'k, 'g, REVERSE, K, V, R> {
        EntryIter {
            inner: self.inner.entries::<REVERSE>(),
            _value: PhantomData,
        }
    }

    #[inline]
    pub fn values<const REVERSE: bool>(&self) -> ValueIter<'k, 'g, REVERSE, K, V, R> {
        ValueIter {
            inner: self.inner.values::<REVERSE>(),
            _value: PhantomData,
        }
    }
}

pub struct PrefixMut<'k, 'g, K: Key, V, R>(Prefix<'k, 'g, K, V, R>);

impl<'k, 'g, K: Key, V: Value, R: raw::iter::Range<'k, K>> PrefixMut<'k, 'g, K, V, R> {
    #[inline]
    pub(crate) unsafe fn new(prefix: Prefix<'k, 'g, K, V, R>) -> Self {
        Self(prefix)
    }

    #[inline]
    pub fn entries_mut<const REVERSE: bool>(&mut self) -> EntryIterMut<'k, 'g, REVERSE, K, V, R> {
        EntryIterMut {
            inner: self.0.inner.entries::<REVERSE>(),
            _value: PhantomData,
        }
    }

    #[inline]
    pub fn values_mut<const REVERSE: bool>(&mut self) -> ValueIterMut<'k, 'g, REVERSE, K, V, R> {
        ValueIterMut {
            inner: self.0.inner.values::<REVERSE>(),
            _value: PhantomData,
        }
    }
}

impl<'k, 'g, K: Key, V: Value, R: raw::iter::Range<'k, K>> Deref for PrefixMut<'k, 'g, K, V, R> {
    type Target = Prefix<'k, 'g, K, V, R>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// Iterator over keys and values
pub struct EntryIter<'k, 'g, const REVERSE: bool, K: Key, V, R: raw::iter::Range<'k, K>> {
    inner: raw::iter::EntryIter<'k, 'g, REVERSE, K, R>,
    _value: PhantomData<&'g V>,
}

impl<'k, 'g, const REVERSE: bool, K, V, R> EntryIter<'k, 'g, REVERSE, K, V, R>
where
    K: Key,
    V: Value,
    R: raw::iter::Range<'k, K>,
{
    #[inline]
    pub fn lend(&mut self) -> Option<(K::Borrow<'_>, V::Borrow<'g>)> {
        self.inner
            .lend()
            .map(|(key, value)| (key, unsafe { V::borrow_from_raw(value) }))
    }

    #[inline]
    pub fn for_each<F: FnMut((K::Borrow<'_>, V::Borrow<'g>)) -> ControlFlow<()>>(
        self,
        mut apply: F,
    ) {
        self.inner
            .for_each(|(key, value)| apply((key, unsafe { V::borrow_from_raw(value) })))
    }
}

impl<'k, 'g, const REVERSE: bool, K, V, R> Iterator for EntryIter<'k, 'g, REVERSE, K, V, R>
where
    K: Key,
    V: Value,
    R: raw::iter::Range<'k, K>,
{
    type Item = (K, V::Borrow<'g>);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.inner
            .next()
            .map(|(key, value)| (key, unsafe { V::borrow_from_raw(value) }))
    }
}

pub struct EntryIterMut<'k, 'g, const REVERSE: bool, K: Key, V, R: raw::iter::Range<'k, K>> {
    inner: raw::iter::EntryIter<'k, 'g, REVERSE, K, R>,
    _value: PhantomData<&'g mut V>,
}

impl<'k, 'g, const REVERSE: bool, K, V, R> EntryIterMut<'k, 'g, REVERSE, K, V, R>
where
    K: Key,
    V: Value,
    R: raw::iter::Range<'k, K>,
{
    #[inline]
    pub fn lend(&mut self) -> Option<(K::Borrow<'_>, V::BorrowMut<'g>)> {
        self.inner
            .lend()
            .map(|(key, value)| (key, unsafe { V::borrow_mut_from_raw(value) }))
    }

    #[inline]
    pub fn for_each<F: FnMut((K::Borrow<'_>, V::BorrowMut<'g>)) -> ControlFlow<()>>(
        self,
        mut apply: F,
    ) {
        self.inner
            .for_each(|(key, value)| apply((key, unsafe { V::borrow_mut_from_raw(value) })))
    }
}

impl<'k, 'g, const REVERSE: bool, K, V, R> Iterator for EntryIterMut<'k, 'g, REVERSE, K, V, R>
where
    K: Key,
    V: Value,
    R: raw::iter::Range<'k, K>,
{
    type Item = (K, V::BorrowMut<'g>);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.inner
            .next()
            .map(|(key, value)| (key, unsafe { V::borrow_mut_from_raw(value) }))
    }
}

pub struct ValueIter<'k, 'g, const REVERSE: bool, K: Key, V, R: raw::iter::Range<'k, K>> {
    inner: raw::iter::ValueIter<'k, 'g, REVERSE, K, R>,
    _value: PhantomData<&'g V>,
}

impl<'k, 'g, const REVERSE: bool, K, V, R> ValueIter<'k, 'g, REVERSE, K, V, R>
where
    K: Key,
    V: Value,
    R: raw::iter::Range<'k, K>,
{
    #[inline]
    pub fn for_each<F: FnMut(V::Borrow<'g>) -> ControlFlow<()>>(self, mut apply: F) {
        self.inner
            .for_each(|value| apply(unsafe { V::borrow_from_raw(value) }))
    }
}

impl<'k, 'g, const REVERSE: bool, K, V, R> Iterator for ValueIter<'k, 'g, REVERSE, K, V, R>
where
    K: Key,
    V: Value,
    R: crate::raw::iter::Range<'k, K>,
{
    type Item = V::Borrow<'g>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.inner
            .next()
            .map(|value| unsafe { V::borrow_from_raw(value) })
    }
}

pub struct ValueIterMut<'k, 'g, const REVERSE: bool, K: Key, V, R: raw::iter::Range<'k, K>> {
    inner: raw::iter::ValueIter<'k, 'g, REVERSE, K, R>,
    _value: PhantomData<&'g mut V>,
}

impl<'k, 'g, const REVERSE: bool, K, V, R> ValueIterMut<'k, 'g, REVERSE, K, V, R>
where
    K: Key,
    V: Value,
    R: raw::iter::Range<'k, K>,
{
    #[inline]
    pub fn for_each<F: FnMut(V::BorrowMut<'g>) -> ControlFlow<()>>(self, mut apply: F) {
        self.inner
            .for_each(|value| apply(unsafe { V::borrow_mut_from_raw(value) }))
    }
}

impl<'k, 'g, const REVERSE: bool, K, V, R> Iterator for ValueIterMut<'k, 'g, REVERSE, K, V, R>
where
    K: Key,
    V: Value,
    R: crate::raw::iter::Range<'k, K>,
{
    type Item = V::BorrowMut<'g>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.inner
            .next()
            .map(|value| unsafe { V::borrow_mut_from_raw(value) })
    }
}

#[cfg(test)]
mod tests {
    use core::ops::ControlFlow;

    use crate::sequential::Map;

    #[test]
    fn values_mut() {
        let mut map = Map::<u64, _>::default();

        for i in 0..1024 {
            map.upsert(i, Box::new(i));
        }

        map.all_mut().values_mut::<false>().for_each(|value| {
            *value += 1;
            ControlFlow::Continue(())
        });

        map.all().entries::<false>().for_each(|(key, value)| {
            assert_eq!(key + 1, *value);
            ControlFlow::Continue(())
        });
    }
}
