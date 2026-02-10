use core::fmt::Debug;
use std::collections::BTreeMap;

use proptest::arbitrary::Arbitrary;
use proptest::prelude::Just;
use proptest::prelude::Strategy as _;
use proptest::prop_oneof;
use proptest_state_machine::prop_state_machine;
use proptest_state_machine::ReferenceStateMachine;
use proptest_state_machine::StateMachineTest;

prop_state_machine! {
    #[test]
    fn u64_u64(
        sequential
        1000
        =>
        Arctic<u64, u64>
    );
}

#[derive(Clone, Debug)]
pub enum Transition<K, V> {
    Insert(K, V),
}

#[derive(Debug, Clone)]
struct Map<K, V>(BTreeMap<K, V>);

impl<K, V> ReferenceStateMachine for Map<K, V>
where
    K: Arbitrary + Clone + Debug + Ord + 'static,
    V: Arbitrary + Clone + Debug + 'static,
{
    type State = Self;
    type Transition = Transition<K, V>;

    fn init_state() -> proptest::prelude::BoxedStrategy<Self::State> {
        Just(Self(BTreeMap::default())).boxed()
    }

    fn transitions(_: &Self::State) -> proptest::prelude::BoxedStrategy<Self::Transition> {
        prop_oneof![
            1 => (K::arbitrary(), V::arbitrary()).prop_map(|(key, value)| Transition::Insert(key, value)),
        ]
        .boxed()
    }

    fn apply(mut state: Self::State, transition: &Self::Transition) -> Self::State {
        match transition {
            Transition::Insert(key, value) => {
                state.0.insert(key.clone(), value.clone());
            }
        }
        state
    }
}

struct Arctic<K: arctic::Key, V: arctic::Value>(arctic::concurrent::Map<K, V>);

impl<K, V> StateMachineTest for Arctic<K, V>
where
    K: arctic::Key + Arbitrary + Clone + Debug + Ord + 'static,
    V: arctic::Value + Arbitrary + Clone + Debug + Send + Sync + 'static,
    for<'a, 'b> V::Borrow<'a>: Debug + PartialEq<V::Borrow<'b>>,
{
    type SystemUnderTest = Self;

    type Reference = Map<K, V>;

    fn init_test(_: &<Self::Reference as ReferenceStateMachine>::State) -> Self::SystemUnderTest {
        Arctic(arctic::concurrent::Map::default())
    }

    fn apply(
        mut state: Self::SystemUnderTest,
        expected: &<Self::Reference as ReferenceStateMachine>::State,
        transition: <Self::Reference as ReferenceStateMachine>::Transition,
    ) -> Self::SystemUnderTest {
        match transition {
            Transition::Insert(key, value) => {
                let mut pin = state.0.pin();
                pin.upsert(K::borrow(&key), value);
            }
        }

        state
            .0
            .as_sequential()
            .iter::<false>()
            .zip(expected.0.iter())
            .for_each(
                |((actual_key, actual_value), (expected_key, expected_value))| {
                    assert_eq!(actual_key, *expected_key);
                    assert_eq!(actual_value, V::borrow(expected_value));
                },
            );

        state
    }
}
