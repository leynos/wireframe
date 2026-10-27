//! Bounded release, cancellation, and shutdown schedules for blocked races.

use std::{io::ErrorKind, time::Duration};

use rstest::rstest;

use super::{
    permit_race::{assert_all_slots_are_available, hold_all_permits},
    support::test_core,
};
use crate::{WireframeError, client::ClientError};

/// Enumerate each ordering of a ready subset without relying on poll order.
fn release_orders(remaining: &[usize], prefix: &mut Vec<usize>, result: &mut Vec<Vec<usize>>) {
    if remaining.is_empty() {
        result.push(prefix.clone());
        return;
    }
    for (position, &slot) in remaining.iter().enumerate() {
        let rest = remaining
            .iter()
            .enumerate()
            .filter_map(|(index, &value)| (index != position).then_some(value))
            .collect::<Vec<_>>();
        prefix.push(slot);
        release_orders(&rest, prefix, result);
        let _ = prefix.pop();
    }
}

/// Terminal action after a set of slots has become ready.
#[derive(Clone, Copy, Debug)]
enum Terminal {
    /// Let one ready acquisition complete.
    Grant,
    /// Drop the outer acquisition before it is polled again.
    Cancel,
    /// Publish shutdown before polling the ready acquisitions.
    Shutdown,
    /// Publish shutdown, then drop the still-pending acquisition.
    ShutdownCancel,
}

/// Exercise all non-empty readiness subsets and their release permutations.
#[rstest]
#[case::grant_two(2, Terminal::Grant)]
#[case::grant_three(3, Terminal::Grant)]
#[case::grant_four(4, Terminal::Grant)]
#[case::cancel_two(2, Terminal::Cancel)]
#[case::cancel_three(3, Terminal::Cancel)]
#[case::cancel_four(4, Terminal::Cancel)]
#[case::shutdown_two(2, Terminal::Shutdown)]
#[case::shutdown_three(3, Terminal::Shutdown)]
#[case::shutdown_four(4, Terminal::Shutdown)]
#[case::shutdown_cancel_two(2, Terminal::ShutdownCancel)]
#[case::shutdown_cancel_three(3, Terminal::ShutdownCancel)]
#[case::shutdown_cancel_four(4, Terminal::ShutdownCancel)]
#[tokio::test(flavor = "current_thread")]
async fn blocked_race_transition_schedules_preserve_capacity(
    #[case] slot_count: usize,
    #[case] terminal: Terminal,
) {
    for ready in 1..(1_usize << slot_count) {
        let slots = (0..slot_count)
            .filter(|index| ready & (1 << *index) != 0)
            .collect::<Vec<_>>();
        let mut orders = Vec::new();
        release_orders(&slots, &mut Vec::new(), &mut orders);
        for order in orders {
            let core = test_core(slot_count);
            let mut held = hold_all_permits(&core)
                .into_iter()
                .map(Some)
                .collect::<Vec<_>>();
            {
                let mut acquisition = std::pin::pin!(core.acquire_slot_permit());
                assert!(futures::poll!(acquisition.as_mut()).is_pending());
                for index in &order {
                    drop(held.get_mut(*index).expect("slot index must exist").take());
                }
                match terminal {
                    Terminal::Grant => {
                        let (winner, permit) =
                            tokio::time::timeout(Duration::from_secs(1), acquisition.as_mut())
                                .await
                                .expect("released capacity must wake the race")
                                .expect("a ready slot must grant a permit");
                        assert_ne!(ready & (1 << winner), 0);
                        for index in 0..slot_count {
                            let available = core
                                .slot(index)
                                .expect("slot index must exist")
                                .try_acquire_permit();
                            assert_eq!(
                                available.is_some(),
                                index != winner && ready & (1 << index) != 0,
                                "unexpected capacity for {terminal:?}, order {order:?}"
                            );
                        }
                        drop(permit);
                    }
                    Terminal::Cancel => {}
                    Terminal::ShutdownCancel => core.shutdown(),
                    Terminal::Shutdown => {
                        core.shutdown();
                        let error =
                            tokio::time::timeout(Duration::from_secs(1), acquisition.as_mut())
                                .await
                                .expect("ready capacity must wake the shutdown race")
                                .expect_err("shutdown must reject the permit");
                        assert!(matches!(
                            error,
                            ClientError::Wireframe(WireframeError::Io(ref source))
                                if source.kind() == ErrorKind::UnexpectedEof
                        ));
                    }
                }
            }
            drop(held);
            assert_all_slots_are_available(&core);
            if matches!(terminal, Terminal::Cancel) {
                let (_, permit) =
                    tokio::time::timeout(Duration::from_secs(1), core.acquire_slot_permit())
                        .await
                        .expect("a later race must not remain queued")
                        .expect("a later race must acquire capacity");
                drop(permit);
                assert_all_slots_are_available(&core);
            }
            if matches!(terminal, Terminal::Shutdown | Terminal::ShutdownCancel) {
                let error = core
                    .acquire_slot_permit()
                    .await
                    .expect_err("a shut-down core must reject later acquisitions");
                assert!(matches!(
                    error,
                    ClientError::Wireframe(WireframeError::Io(ref source))
                        if source.kind() == ErrorKind::UnexpectedEof
                ));
            }
        }
    }
}
