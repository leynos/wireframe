//! Property tests for the decoder-failure fallbacks.
//!
//! The fixed cases in [`super`] document the boundary examples; these
//! properties hold for any order of decoded frames and decoder failures, and
//! for any drop budget. A failing case prints the generated sequence, so the
//! shrinker can report the shortest ordering that breaks a fallback.

use std::io;

use proptest::prelude::*;

use super::super::{test_support::ScriptedHook, *};
use crate::codec::RecoveryPolicy;

/// One step of a generated recovery sequence.
#[derive(Clone, Copy, Debug)]
enum Step {
    /// A frame decoded, which restarts the drop streak.
    Frame,
    /// A decoder failure the hook classified as `policy`.
    Failure {
        /// Whether the failure came from `decode_eof`.
        eof: bool,
        /// Bytes the decoder consumed before failing.
        consumed: usize,
        /// Policy the hook selected.
        policy: RecoveryPolicy,
    },
}

impl Step {
    /// The policy this step's hook answers with, when it fails.
    const fn policy(self) -> Option<RecoveryPolicy> {
        match self {
            Self::Frame => None,
            Self::Failure { policy, .. } => Some(policy),
        }
    }

    /// Whether the connection could drop this failure if the streak allows it.
    const fn is_droppable(self) -> bool {
        matches!(
            self,
            Self::Failure {
                eof: false,
                consumed,
                policy: RecoveryPolicy::Drop,
            } if consumed > 0
        )
    }
}

/// Any of the three policies a hook may select.
fn policy_strategy() -> impl Strategy<Value = RecoveryPolicy> {
    prop_oneof![
        Just(RecoveryPolicy::Drop),
        Just(RecoveryPolicy::Quarantine),
        Just(RecoveryPolicy::Disconnect),
    ]
}

/// A decoded frame, or a failure with any origin, consumption and policy.
fn step_strategy() -> impl Strategy<Value = Step> {
    prop_oneof![
        Just(Step::Frame),
        (any::<bool>(), 0usize..3, policy_strategy()).prop_map(|(eof, consumed, policy)| {
            Step::Failure {
                eof,
                consumed,
                policy,
            }
        }),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 32, ..Default::default() })]

    /// The documented fallbacks hold for any order of frames and failures.
    #[test]
    fn fallbacks_hold_over_generated_event_sequences(
        max_drops in 1_u32..5,
        steps in prop::collection::vec(step_strategy(), 1..24),
    ) {
        let policies = steps.iter().filter_map(|step| step.policy()).collect();
        let hook = ScriptedHook::new(policies);
        let config = RecoveryConfig::default().max_consecutive_drops(max_drops);
        let mut coordinator = RecoveryCoordinator::new(Some(&hook), config);
        // Applied drops since the last decoded frame, as the sequence saw them.
        let mut drops = 0_u32;
        let mut just_reset = false;

        for step in steps {
            let Step::Failure { eof, consumed, .. } = step else {
                coordinator.record_decoded_frame();
                drops = 0;
                just_reset = true;
                continue;
            };
            let origin = if eof {
                DecodeOrigin::DecodeEof
            } else {
                DecodeOrigin::Decode
            };
            let failure = DecodeFailure::new(
                io::Error::new(io::ErrorKind::InvalidData, "malformed"),
                origin,
                consumed,
            );
            let action = coordinator.record_failure(failure).action;

            if eof {
                prop_assert_eq!(action, RecoveryAction::Disconnect, "an EOF failure disconnects");
            }
            if consumed == 0 {
                prop_assert!(
                    !matches!(action, RecoveryAction::Drop | RecoveryAction::Quarantine(_)),
                    "a failure that consumed nothing cannot be discarded"
                );
            }
            if step.is_droppable() {
                if just_reset {
                    prop_assert_eq!(action, RecoveryAction::Drop, "a frame resets the streak");
                }
                if drops >= max_drops {
                    prop_assert_eq!(
                        action,
                        RecoveryAction::Disconnect,
                        "the failure after the budget must disconnect"
                    );
                } else {
                    prop_assert_eq!(action, RecoveryAction::Drop, "a budgeted drop is applied");
                    drops += 1;
                }
            }
            just_reset = false;
        }
    }
}
