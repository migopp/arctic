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

impl<K: Arbitrary + 'static> Arbitrary for Op<K, u64> {
    type Parameters = ();
    type Strategy = BoxedStrategy<Self>;

    fn arbitrary_with((): Self::Parameters) -> Self::Strategy {
        prop_oneof![
            (
                K::arbitrary(),
                u64::arbitrary().prop_filter("Tagged pointer", |value| *value < (1 << 63))
            )
                .prop_map(|(key, value)| Op::Insert { key, value }),
            K::arbitrary().prop_map(|key| Op::Get { key }),
        ]
        .boxed()
    }
}

struct Sequential<K, V> {
    inner: HashMap<Vec<u8>, V>,
    _key: PhantomData<fn() -> K>,
}

impl<K, V> Default for Sequential<K, V> {
    fn default() -> Self {
        Self {
            inner: HashMap::default(),
            _key: PhantomData,
        }
    }
}

trait Key: Arbitrary + 'static {
    fn as_bytes(&self) -> &[u8];
}

impl Key for u8 {
    fn as_bytes(&self) -> &[u8] {
        std::array::from_ref(self)
    }
}

trait Value: Arbitrary + Clone {}

impl Value for u64 {}

impl<K: Key, V: Value> SequentialSpec for Sequential<K, V> {
    type Op = Op<K, V>;
    type Ret = Ret<V>;

    fn exec(&mut self, op: Self::Op) -> Self::Ret {
        match op {
            Op::Insert { key, value } => {
                let old = self.inner.insert(key.as_bytes().to_owned(), value);
                Ret::Insert(old)
            }
            Op::Get { key } => Ret::Get(self.inner.get(key.as_bytes()).cloned()),
        }
    }
}

struct Concurrent<K, V> {
    inner: art::Art,
    _key: PhantomData<fn() -> K>,
    _value: PhantomData<fn() -> V>,
}

impl<K, V> Default for Concurrent<K, V> {
    fn default() -> Self {
        Self {
            inner: art::Art::default(),
            _key: PhantomData,
            _value: PhantomData,
        }
    }
}

impl<K: Key> ConcurrentSpec for Concurrent<K, u64> {
    type Seq = Sequential<K, u64>;
    fn exec(&self, op: lincheck::ConcOp<Self>) -> lincheck::ConcRet<Self> {
        match op {
            Op::Insert { key, value } => Ret::Insert(self.inner.insert(key.as_bytes(), value)),
            Op::Get { key } => Ret::Get(self.inner.get(key.as_bytes())),
        }
    }
}

#[test]
fn two_threads() {
    Lincheck {
        num_ops: 2,
        num_threads: 2,
    }
    .verify_or_panic::<Concurrent<u8, u64>>();
}
