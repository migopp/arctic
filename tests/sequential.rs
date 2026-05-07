use core::fmt::Debug;
use core::ops::ControlFlow;
use std::collections::BTreeMap;

use proptest::arbitrary::Arbitrary;
use proptest::prelude::Just;
use proptest::prelude::Strategy as _;
use proptest::prop_oneof;
use proptest::sample::Selector;
use proptest_state_machine::ReferenceStateMachine;
use proptest_state_machine::StateMachineTest;
use proptest_state_machine::prop_state_machine;

trait NoOverlap: Sized {
    fn ensure(self) -> Self {
        self
    }
}

impl NoOverlap for u16 {}
impl NoOverlap for u32 {}
impl NoOverlap for u64 {}
impl NoOverlap for u128 {}

impl NoOverlap for String {
    fn ensure(mut self) -> Self {
        self.push('\n');
        self
    }
}

prop_state_machine! {
    #[test]
    fn u16_u64(
        sequential
        1000
        =>
        Arctic<u16, u64>
    );

    #[test]
    fn u32_u64(
        sequential
        1000
        =>
        Arctic<u16, u64>
    );

    #[test]
    fn u64_u64(
        sequential
        1000
        =>
        Arctic<u64, u64>
    );

    #[test]
    fn u128_u64(
        sequential
        1000
        =>
        Arctic<u16, u64>
    );

    #[test]
    fn string_u64(
        sequential
        1000
        =>
        Arctic<String, u64>
    );
}

#[derive(Clone, Debug)]
pub enum Transition<K, V> {
    Upsert(K, V),
    Remove(K),
    Range { ascend: bool, lower: K, upper: K },
}

#[derive(Debug, Clone)]
struct Map<K, V>(BTreeMap<K, V>);

impl<K, V> ReferenceStateMachine for Map<K, V>
where
    K: Arbitrary + Clone + Debug + Default + Ord + NoOverlap + 'static,
    V: Arbitrary + Clone + Debug + 'static,
{
    type State = Self;
    type Transition = Transition<K, V>;

    fn init_state() -> proptest::prelude::BoxedStrategy<Self::State> {
        Just(Self(BTreeMap::new())).boxed()
    }

    fn transitions(state: &Self::State) -> proptest::prelude::BoxedStrategy<Self::Transition> {
        prop_oneof![
            1 => (K::arbitrary().prop_map(NoOverlap::ensure), V::arbitrary()).prop_map(|(key, value)| Transition::Upsert(key, value)),
            1 => proptest::prelude::any::<Selector>().prop_map({
                let state = state.clone();
                move |selector| {
                    let key = if state.0.is_empty() {
                        K::default()
                    } else {
                        selector.select(state.0.keys()).clone()
                    };
                    Transition::Remove(key)
                }
            }),
            1 => (bool::arbitrary(), K::arbitrary().prop_map(NoOverlap::ensure), proptest::prelude::any::<Selector>()).prop_map({
                let state = state.clone();
                move |(ascend, random, selector)| {
                    if state.0.is_empty() {
                        return Transition::Range { ascend, lower: K::default(), upper: K::default() };
                    }

                    let mut lower = random;
                    let mut upper = selector.select(state.0.keys()).clone();

                    if lower > upper {
                        core::mem::swap(&mut lower, &mut upper);
                    }

                    Transition::Range { ascend, lower, upper }
                }
            })
        ].boxed()
    }

    fn apply(mut state: Self::State, transition: &Self::Transition) -> Self::State {
        match transition {
            Transition::Upsert(key, value) => {
                state.0.insert(key.clone(), value.clone());
            }
            Transition::Remove(key) => {
                state.0.remove(key);
            }
            Transition::Range { .. } => (),
        }
        state
    }
}

struct Arctic<K: arctic::Key, V: arctic::Value>(arctic::concurrent::Map<K, V>);

impl<K, V> StateMachineTest for Arctic<K, V>
where
    K: arctic::Key + Arbitrary + Clone + Debug + Default + Ord + NoOverlap + 'static,
    for<'k> K::Read<'k>: From<&'k K>,
    K::Borrowed: Ord + core::fmt::Debug,
    V: arctic::Value + Arbitrary + Clone + Debug + Send + Sync + 'static,
    V::Target: Debug + PartialEq + PartialEq<V>,
{
    type SystemUnderTest = Self;

    type Reference = Map<K, V>;

    fn init_test(_: &<Self::Reference as ReferenceStateMachine>::State) -> Self::SystemUnderTest {
        Arctic(arctic::concurrent::Map::default())
    }

    fn apply(
        state: Self::SystemUnderTest,
        expected: &<Self::Reference as ReferenceStateMachine>::State,
        transition: <Self::Reference as ReferenceStateMachine>::Transition,
    ) -> Self::SystemUnderTest {
        match transition {
            Transition::Upsert(key, value) => {
                state.0.upsert(K::borrow(&key), value);
            }
            Transition::Remove(key) => {
                state.0.remove(K::borrow(&key));
            }
            Transition::Range {
                ascend,
                lower,
                upper,
            } => {
                if let Some(prefix) = state.0.range(&lower..=&upper) {
                    let expected = expected.0.range::<K, _>(lower.clone()..=upper.clone());
                    let mut expected = if ascend {
                        Box::new(expected) as Box<dyn Iterator<Item = _>>
                    } else {
                        Box::new(expected.rev())
                    };

                    macro_rules! compare {
                        () => {
                            |(key_actual, value_actual)| {
                                let (key_expected, value_expected) = expected.next().unwrap();
                                assert_eq!(
                                    key_actual,
                                    key_expected.borrow(),
                                    "actual key: {key_actual:x?}, expected key: {key_expected:x?}, lower: {lower:x?}, upper: {upper:x?}",
                                );
                                assert_eq!(
                                    value_actual, value_expected,
                                    "actual value: {value_actual:x?}, expected value: {value_expected:x?}",
                                );
                                ControlFlow::Continue(())
                            }
                        };
                    }

                    if ascend {
                        prefix
                            .entries::<arctic::Ascend>()
                            .for_each_internal(compare!())
                    } else {
                        prefix
                            .entries::<arctic::Descend>()
                            .for_each_internal(compare!())
                    }

                    assert!(expected.next().is_none());
                }
            }
        }

        state
    }
}
