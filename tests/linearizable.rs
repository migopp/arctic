use core::marker::PhantomData;
use std::collections::HashMap;

use lincheck::ConcurrentSpec;
use lincheck::Lincheck;
use lincheck::SequentialSpec;
use proptest::prelude::Arbitrary;
use proptest::prelude::BoxedStrategy;
use proptest::prelude::Strategy;
use proptest::prop_oneof;

#[derive(Clone, Debug)]
enum Op<K, V> {
    Insert { key: K, value: V },
    Get { key: K },
}

#[derive(Clone, Debug, PartialEq)]
enum Ret<V> {
    Insert(Option<V>),
    Get(Option<V>),
}

impl<K: Arbitrary + 'static> Arbitrary for Op<K, u32> {
    type Parameters = ();
    type Strategy = BoxedStrategy<Self>;

    fn arbitrary_with((): Self::Parameters) -> Self::Strategy {
        prop_oneof![
            (K::arbitrary(), u32::arbitrary()).prop_map(|(key, value)| Op::Insert { key, value }),
            K::arbitrary().prop_map(|key| Op::Get { key }),
        ]
        .boxed()
    }
}

struct Sequential<K: art::Key, V> {
    inner: HashMap<K::ByteArray, V>,
    _key: PhantomData<fn() -> K>,
}

impl<K: art::Key, V> Default for Sequential<K, V> {
    fn default() -> Self {
        Self {
            inner: HashMap::default(),
            _key: PhantomData,
        }
    }
}

trait Value: Arbitrary + Clone {}

impl Value for u32 {}

impl<K: art::Key, V: Value> SequentialSpec for Sequential<K, V>
where
    K::ByteArray: core::hash::Hash + Eq,
{
    type Op = Op<K, V>;
    type Ret = Ret<V>;

    fn exec(&mut self, op: Self::Op) -> Self::Ret {
        match op {
            Op::Insert { key, value } => {
                let key = key.to_byte_array();
                let old = self.inner.insert(key, value);
                Ret::Insert(old)
            }
            Op::Get { key } => {
                let key = key.to_byte_array();
                Ret::Get(self.inner.get(&key).cloned())
            }
        }
    }
}

struct Concurrent<K, V> {
    inner: art::Map<K, V>,
    _key: PhantomData<fn() -> K>,
    _value: PhantomData<fn() -> V>,
}

impl<K, V> Default for Concurrent<K, V> {
    fn default() -> Self {
        Self {
            inner: art::Map::default(),
            _key: PhantomData,
            _value: PhantomData,
        }
    }
}

impl<K: art::Key> ConcurrentSpec for Concurrent<K, u32>
where
    K::ByteArray: core::hash::Hash + Eq,
{
    type Seq = Sequential<K, u32>;
    fn exec(&self, op: lincheck::ConcOp<Self>) -> lincheck::ConcRet<Self> {
        match op {
            Op::Insert { key, value } => Ret::Insert(self.inner.insert(key, value)),
            Op::Get { key } => Ret::Get(self.inner.get(key)),
        }
    }
}

#[test]
fn two_threads() {
    Lincheck {
        num_ops: 2,
        num_threads: 2,
    }
    .verify_or_panic::<Concurrent<u8, u32>>();
}
