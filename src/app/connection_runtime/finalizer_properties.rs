//! Property coverage for at-most-once finalization.

use std::sync::{Arc, Mutex};

use futures::executor::block_on;
use proptest::prelude::*;

use super::finalizer::ConnectionFinalizer;
use crate::app::lifecycle::ConnectionTeardown;

/// One operation applied to a finalizer in a generated sequence.
#[derive(Clone, Copy, Debug)]
enum FinalizerOp {
    /// Arm the finalizer with a setup value.
    Arm(u32),
    /// Offer the armed state to teardown.
    Finalize,
}

/// Mirror of the finalizer's observable state used as a model.
#[derive(Clone, Copy, Debug)]
enum ModelState {
    /// No setup value is owed.
    NotSetUp,
    /// One setup value is owed.
    Armed(u32),
    /// The value has already been taken.
    Finalized,
}

impl ModelState {
    /// Apply one operation, returning the state a teardown would receive.
    fn apply(&mut self, op: FinalizerOp) -> Option<u32> {
        match op {
            FinalizerOp::Arm(value) => {
                if matches!(self, Self::NotSetUp) {
                    *self = Self::Armed(value);
                }
                None
            }
            FinalizerOp::Finalize => {
                let taken = match self {
                    Self::Armed(value) => Some(*value),
                    Self::NotSetUp | Self::Finalized => None,
                };
                *self = Self::Finalized;
                taken
            }
        }
    }
}

/// Generate arm/finalize operations in arbitrary order.
fn finalizer_op() -> impl Strategy<Value = FinalizerOp> {
    prop_oneof![
        (0u32..16).prop_map(FinalizerOp::Arm),
        Just(FinalizerOp::Finalize),
    ]
}

/// Build a teardown callback that records every state it receives.
fn recording_teardown(states: &Arc<Mutex<Vec<u32>>>) -> Arc<ConnectionTeardown<u32>> {
    let state_log = Arc::clone(states);
    Arc::new(move |state| {
        let received_log = Arc::clone(&state_log);
        Box::pin(async move {
            received_log.lock().expect("state log mutex").push(state);
        })
    })
}

// Teardown must never run before setup state is armed, and must run at most
// once for any ordering of arm and finalize operations.
proptest! {
    #![proptest_config(ProptestConfig {
        cases: 64,
        .. ProptestConfig::default()
    })]

    #[test]
    fn finalizer_runs_teardown_at_most_once(
        ops in prop::collection::vec(finalizer_op(), 0..24),
    ) {
        let observed = Arc::new(Mutex::new(Vec::new()));
        let teardown = recording_teardown(&observed);
        let mut finalizer = ConnectionFinalizer::not_set_up();
        let mut model = ModelState::NotSetUp;
        let mut expected = Vec::new();

        for op in ops {
            match op {
                FinalizerOp::Arm(value) => finalizer.arm(value),
                FinalizerOp::Finalize => block_on(finalizer.finalize(Some(&teardown))),
            }
            if let Some(taken) = model.apply(op) {
                expected.push(taken);
            }
        }

        prop_assert_eq!(&*observed.lock().expect("state log mutex"), &expected);
        prop_assert!(expected.len() <= 1, "teardown ran more than once");
    }
}
