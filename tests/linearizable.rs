use std::collections::HashMap;

use lincheck::ConcurrentSpec;
use lincheck::Lincheck;
use lincheck::SequentialSpec;
use proptest::prelude::Arbitrary;
use proptest::prelude::BoxedStrategy;
use proptest::prelude::Strategy;
use proptest::prop_oneof;

#[derive(Clone, Debug)]
enum Op {
    Insert { key: u64, value: u64 },
    Get { key: u64 },
}

#[derive(Clone, Debug, PartialEq)]
enum Ret {
    Insert(Option<u64>),
    Get(Option<u64>),
}

impl Arbitrary for Op {
    type Parameters = ();
    type Strategy = BoxedStrategy<Self>;

    fn arbitrary_with((): Self::Parameters) -> Self::Strategy {
        prop_oneof![
            (proptest::num::u64::ANY, proptest::num::u64::ANY)
                .prop_map(|(key, value)| Op::Insert { key, value }),
            proptest::num::u64::ANY.prop_map(|key| Op::Get { key }),
        ]
        .boxed()
    }
}

#[derive(Default)]
struct Sequential(HashMap<u64, u64>);

impl SequentialSpec for Sequential {
    type Op = Op;
    type Ret = Ret;

    fn exec(&mut self, op: Self::Op) -> Self::Ret {
        match op {
            Op::Insert { key, value } => {
                let old = self.0.insert(key, value);
                Ret::Insert(old)
            }
            Op::Get { key } => Ret::Get(self.0.get(&key).copied()),
        }
    }
}

#[derive(Default)]
struct Concurrent(art::Art);

impl ConcurrentSpec for Concurrent {
    type Seq = Sequential;
    fn exec(&self, op: lincheck::ConcOp<Self>) -> lincheck::ConcRet<Self> {
        dbg!(&op);
        match op {
            Op::Insert { key, value } => Ret::Insert(self.0.insert(&key.to_be_bytes(), value)),
            Op::Get { key } => Ret::Get(self.0.get(&key.to_be_bytes())),
        }
    }
}

#[test]
fn two_threads() {
    Lincheck {
        num_ops: 8,
        num_threads: 4,
    }
    .verify_or_panic::<Concurrent>();
}
