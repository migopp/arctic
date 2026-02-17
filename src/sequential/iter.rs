use core::marker::PhantomData;
use core::ops::ControlFlow;
use core::ops::Deref;

use crate::raw;
use crate::raw::Key;
use crate::sequential::Value;
use crate::Order;

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
    pub fn entries<O: Order>(&self) -> EntryIter<'k, 'g, K, V, R, O> {
        EntryIter {
            inner: self.inner.entries::<O>(),
            _value: PhantomData,
        }
    }

    #[inline]
    pub fn values<O: Order>(&self) -> ValueIter<'k, 'g, K, V, R, O> {
        ValueIter {
            inner: self.inner.values::<O>(),
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
    pub fn entries_mut<O: Order>(&mut self) -> EntryIterMut<'k, 'g, K, V, R, O> {
        EntryIterMut {
            inner: self.0.inner.entries::<O>(),
            _value: PhantomData,
        }
    }

    #[inline]
    pub fn values_mut<O: Order>(&mut self) -> ValueIterMut<'k, 'g, K, V, R, O> {
        ValueIterMut {
            inner: self.0.inner.values::<O>(),
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
pub struct EntryIter<'k, 'g, K: Key, V, R: raw::iter::Range<'k, K>, O> {
    inner: raw::iter::EntryIter<'k, 'g, K, R, O>,
    _value: PhantomData<&'g V>,
}

impl<'k, 'g, K, V, R, O> EntryIter<'k, 'g, K, V, R, O>
where
    K: Key,
    V: Value,
    R: raw::iter::Range<'k, K>,
    O: Order,
{
    #[inline]
    pub fn lend(&mut self) -> Option<(K::Borrow<'_>, V::Borrow<'g>)> {
        self.inner
            .lend()
            .map(|(key, value)| (key, unsafe { V::borrow_from_raw(value) }))
    }

    #[inline]
    pub fn for_each_internal<F: FnMut((K::Borrow<'_>, V::Borrow<'g>)) -> ControlFlow<()>>(
        self,
        mut apply: F,
    ) {
        self.inner
            .for_each_internal(|(key, value)| apply((key, unsafe { V::borrow_from_raw(value) })))
    }
}

impl<'k, 'g, K, V, R, O> Iterator for EntryIter<'k, 'g, K, V, R, O>
where
    K: Key,
    V: Value,
    R: raw::iter::Range<'k, K>,
    O: Order,
{
    type Item = (K, V::Borrow<'g>);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.inner
            .next()
            .map(|(key, value)| (key, unsafe { V::borrow_from_raw(value) }))
    }
}

pub struct EntryIterMut<'k, 'g, K: Key, V, R: raw::iter::Range<'k, K>, O> {
    inner: raw::iter::EntryIter<'k, 'g, K, R, O>,
    _value: PhantomData<&'g mut V>,
}

impl<'k, 'g, K, V, R, O> EntryIterMut<'k, 'g, K, V, R, O>
where
    K: Key,
    V: Value,
    R: raw::iter::Range<'k, K>,
    O: Order,
{
    #[inline]
    pub fn lend(&mut self) -> Option<(K::Borrow<'_>, V::BorrowMut<'g>)> {
        self.inner
            .lend()
            .map(|(key, value)| (key, unsafe { V::borrow_mut_from_raw(value) }))
    }

    #[inline]
    pub fn for_each_internal<F: FnMut((K::Borrow<'_>, V::BorrowMut<'g>)) -> ControlFlow<()>>(
        self,
        mut apply: F,
    ) {
        self.inner.for_each_internal(|(key, value)| {
            apply((key, unsafe { V::borrow_mut_from_raw(value) }))
        })
    }
}

impl<'k, 'g, K, V, R, O> Iterator for EntryIterMut<'k, 'g, K, V, R, O>
where
    K: Key,
    V: Value,
    R: raw::iter::Range<'k, K>,
    O: Order,
{
    type Item = (K, V::BorrowMut<'g>);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.inner
            .next()
            .map(|(key, value)| (key, unsafe { V::borrow_mut_from_raw(value) }))
    }
}

pub struct ValueIter<'k, 'g, K: Key, V, R: raw::iter::Range<'k, K>, O> {
    inner: raw::iter::ValueIter<'k, 'g, K, R, O>,
    _value: PhantomData<&'g V>,
}

impl<'k, 'g, K, V, R, O> ValueIter<'k, 'g, K, V, R, O>
where
    K: Key,
    V: Value,
    R: raw::iter::Range<'k, K>,
    O: Order,
{
    #[inline]
    pub fn for_each_internal<F: FnMut(V::Borrow<'g>) -> ControlFlow<()>>(self, mut apply: F) {
        self.inner
            .for_each_internal(|value| apply(unsafe { V::borrow_from_raw(value) }))
    }
}

impl<'k, 'g, K, V, R, O> Iterator for ValueIter<'k, 'g, K, V, R, O>
where
    K: Key,
    V: Value,
    R: crate::raw::iter::Range<'k, K>,
    O: Order,
{
    type Item = V::Borrow<'g>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.inner
            .next()
            .map(|value| unsafe { V::borrow_from_raw(value) })
    }
}

pub struct ValueIterMut<'k, 'g, K: Key, V, R: raw::iter::Range<'k, K>, O> {
    inner: raw::iter::ValueIter<'k, 'g, K, R, O>,
    _value: PhantomData<&'g mut V>,
}

impl<'k, 'g, K, V, R, O> ValueIterMut<'k, 'g, K, V, R, O>
where
    K: Key,
    V: Value,
    R: raw::iter::Range<'k, K>,
    O: Order,
{
    #[inline]
    pub fn for_each_internal<F: FnMut(V::BorrowMut<'g>) -> ControlFlow<()>>(self, mut apply: F) {
        self.inner
            .for_each_internal(|value| apply(unsafe { V::borrow_mut_from_raw(value) }))
    }
}

impl<'k, 'g, K, V, R, O> Iterator for ValueIterMut<'k, 'g, K, V, R, O>
where
    K: Key,
    V: Value,
    R: crate::raw::iter::Range<'k, K>,
    O: Order,
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
    use crate::Ascend;
    use crate::Descend;

    #[test]
    fn values_mut() {
        let mut map = Map::<u64, _>::default();

        for i in 0..1024 {
            map.upsert(i, Box::new(i));
        }

        map.all_mut()
            .values_mut::<Ascend>()
            .for_each_internal(|value| {
                *value += 1;
                ControlFlow::Continue(())
            });

        map.all()
            .entries::<Descend>()
            .for_each_internal(|(key, value)| {
                assert_eq!(key + 1, *value);
                ControlFlow::Continue(())
            });
    }
}
