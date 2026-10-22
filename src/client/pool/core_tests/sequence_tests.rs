//! Tests for fixed slot storage and index-based acquisition.

use std::{sync::Arc, time::Duration};

use proptest::{
    prelude::*,
    test_runner::{Config as ProptestConfig, TestCaseResult, TestRunner},
};
use rstest::rstest;
use tokio::sync::{Barrier, OwnedSemaphorePermit};

use super::support::{TestCore, four_slot_core, one_slot_core, test_core, two_slot_core};
use crate::client::pool::lease::PooledClientLease;

#[rstest]
#[tokio::test]
async fn acquired_indices_continue_to_resolve_the_same_by_value_slots(
    four_slot_core: Arc<TestCore>,
) {
    let core = four_slot_core;
    let slot_addresses = core
        .slots
        .iter()
        .map(std::ptr::from_ref)
        .collect::<Vec<_>>();

    let leases = (0..4)
        .map(|_| {
            let (slot_index, permit) = core
                .try_acquire_permit()
                .expect("each fixed slot should grant one permit");
            let lease = PooledClientLease::new(Arc::clone(&core), slot_index, permit, None)
                .expect("selected index should resolve in the shared core");
            (slot_index, lease)
        })
        .collect::<Vec<_>>();

    for (slot_index, lease) in &leases {
        let selected_slot = lease
            .slot()
            .expect("lease index should keep resolving while the core is alive");
        assert_eq!(
            std::ptr::from_ref(selected_slot),
            *slot_addresses
                .get(*slot_index)
                .expect("each acquired lease should have a recorded slot address"),
            "the index must continue to identify the original slot"
        );
    }
}

#[rstest]
#[tokio::test]
async fn async_acquisition_index_resolves_to_the_slot_that_granted_its_permit(
    four_slot_core: Arc<TestCore>,
) {
    let core = four_slot_core;
    let selected_index = 2;
    let selected_slot = core
        .slot(selected_index)
        .expect("the designated slot should exist");
    let selected_address = std::ptr::from_ref(selected_slot);
    let held_permits = core
        .slots
        .iter()
        .enumerate()
        .filter(|(slot_index, _)| *slot_index != selected_index)
        .map(|(_, slot)| {
            slot.try_acquire_permit()
                .expect("every other slot should be held")
        })
        .collect::<Vec<_>>();

    let (slot_index, permit) = core
        .acquire_slot_permit()
        .await
        .expect("the only available slot should grant its permit");
    let lease = PooledClientLease::new(Arc::clone(&core), slot_index, permit, None)
        .expect("the returned index should resolve in the core");

    assert_eq!(slot_index, selected_index);
    assert_eq!(
        std::ptr::from_ref(lease.slot().expect("lease slot should resolve")),
        selected_address,
        "the async permit result must identify the slot that granted it"
    );
    drop(held_permits);
}

/// Take both slots, then free the second so the cursor lands on a held slot.
///
/// `PoolCore::new` starts the cursor at zero and every acquisition advances it
/// once, so the *first* attempt in a fresh two-slot core takes slot 0 and leaves
/// the cursor at 1 — the free slot. Releasing slot 1's permit and returning
/// slot 0's gives the caller a pool where the next attempt starts on a slot that
/// cannot grant capacity, which is the case under test. Taking only one permit
/// first would leave these tests passing without ever exercising it.
fn hold_first_slot_and_free_the_second(core: &TestCore) -> OwnedSemaphorePermit {
    let first = core
        .try_acquire_permit()
        .expect("the first slot should grant a permit");
    let second = core
        .try_acquire_permit()
        .expect("the second slot should grant a permit");
    assert_eq!(
        (first.0, second.0),
        (0, 1),
        "fixture slots are ordered, so two acquisitions return the cursor to 0"
    );

    drop(second.1);
    first.1
}

#[rstest]
#[tokio::test]
async fn immediate_selection_skips_a_held_starting_slot(two_slot_core: Arc<TestCore>) {
    let core = two_slot_core;
    // Two acquisitions leave the cursor back at 0, so the attempt under test
    // begins on the held slot. The walk must step past it to the freed slot
    // rather than reporting exhaustion.
    let held_permit = hold_first_slot_and_free_the_second(&core);

    let (selected_index, selected_permit) = core
        .try_acquire_permit()
        .expect("the free slot must still be reachable past the held one");

    assert_eq!(
        selected_index, 1,
        "selection must skip the held starting slot and grant the free one"
    );
    drop(selected_permit);
    drop(held_permit);
}

#[rstest]
#[tokio::test]
async fn permit_race_selection_skips_a_held_starting_slot(two_slot_core: Arc<TestCore>) {
    let core = two_slot_core;
    let held_permit = hold_first_slot_and_free_the_second(&core);

    // The race has no timeout of its own; bound it so a regression that leaves
    // the runnable slot out of the rotation fails instead of hanging the suite.
    let (selected_index, selected_permit) =
        tokio::time::timeout(Duration::from_secs(1), core.acquire_slot_permit())
            .await
            .expect("the permit race must not wait on capacity the free slot can provide")
            .expect("the free slot must still be reachable through the permit race");

    assert_eq!(
        selected_index, 1,
        "the permit race must report the free slot, not the held starting slot"
    );
    drop(selected_permit);
    drop(held_permit);
}

#[rstest]
#[tokio::test]
async fn lease_construction_rejects_an_out_of_range_slot_index(one_slot_core: Arc<TestCore>) {
    let core = one_slot_core;
    let slot = core.slot(0).expect("the only slot should exist");
    let permit = slot
        .try_acquire_permit()
        .expect("the only slot should grant a permit");

    assert!(
        PooledClientLease::new(core, 1, permit, None).is_none(),
        "a lease must not be constructed with an invalid slot index"
    );
}

#[rstest]
#[tokio::test]
async fn shutdown_before_notification_first_poll_is_not_lost(one_slot_core: Arc<TestCore>) {
    let core = one_slot_core;
    let notification = core.shutdown_notified();

    core.shutdown();

    tokio::time::timeout(Duration::from_secs(1), notification)
        .await
        .expect("shutdown wait should observe shutdown before its first poll");
}

#[rstest]
#[tokio::test]
async fn shutdown_after_flag_check_before_notification_poll_is_not_lost(
    one_slot_core: Arc<TestCore>,
) {
    let core = one_slot_core;
    let checked = Arc::new(Barrier::new(2));
    let resume = Arc::new(Barrier::new(2));
    let waiting_core = Arc::clone(&core);
    let waiter_checked = Arc::clone(&checked);
    let waiter_resume = Arc::clone(&resume);
    let waiter = tokio::spawn(async move {
        waiting_core
            .shutdown_notified_after_check(async move {
                waiter_checked.wait().await;
                waiter_resume.wait().await;
            })
            .await;
    });

    checked.wait().await;
    core.shutdown();
    resume.wait().await;

    tokio::time::timeout(Duration::from_secs(1), waiter)
        .await
        .expect("the shutdown notification should wake the paused waiter")
        .expect("the waiter task should complete");
}

#[tokio::test]
async fn generated_slot_sequences_preserve_identity_and_shutdown() {
    let mut runner = TestRunner::new(ProptestConfig {
        cases: 64,
        ..ProptestConfig::default()
    });
    let generated_inputs = (1_usize..=16).prop_flat_map(|slot_count| {
        (
            Just(slot_count),
            prop::collection::vec((any::<bool>(), 0_usize..slot_count), 64),
            0_usize..=64,
        )
    });

    runner
        .run(&generated_inputs, run_generated_slot_sequence)
        .expect("generated slot sequences should preserve core invariants");
}

fn run_generated_slot_sequence(
    (slot_count, operations, shutdown_step): (usize, Vec<(bool, usize)>, usize),
) -> TestCaseResult {
    let mut sequence = GeneratedSlotSequence::new(slot_count);

    for step in 0..=operations.len() {
        let operation = operations.get(step).copied();
        apply_generated_slot_step(&mut sequence, step, shutdown_step, operation)?;
    }
    Ok(())
}

fn apply_generated_slot_step(
    sequence: &mut GeneratedSlotSequence,
    step: usize,
    shutdown_step: usize,
    operation: Option<(bool, usize)>,
) -> TestCaseResult {
    if step == shutdown_step {
        sequence.shutdown();
    }
    if let Some((should_acquire, slot_index)) = operation {
        sequence.apply(should_acquire, slot_index)?;
    }
    sequence.assert_valid()
}

type TestLease = PooledClientLease<crate::serializer::BincodeSerializer, (), ()>;

struct GeneratedSlotSequence {
    core: Arc<TestCore>,
    slot_addresses: Vec<*const ()>,
    leases: Vec<Option<TestLease>>,
    is_shutdown: bool,
}

impl GeneratedSlotSequence {
    fn new(slot_count: usize) -> Self {
        let core = test_core(slot_count);
        let slot_addresses = core
            .slots
            .iter()
            .map(|slot| std::ptr::from_ref(slot).cast::<()>())
            .collect();
        let leases = (0..slot_count).map(|_| None).collect();

        Self {
            core,
            slot_addresses,
            leases,
            is_shutdown: false,
        }
    }

    fn shutdown(&mut self) {
        self.core.shutdown();
        self.is_shutdown = true;
    }

    fn apply(&mut self, should_acquire: bool, slot_index: usize) -> TestCaseResult {
        if should_acquire {
            self.acquire()
        } else {
            self.release(slot_index);
            Ok(())
        }
    }

    fn acquire(&mut self) -> TestCaseResult {
        let selected = self.core.try_acquire_permit();
        if self.is_shutdown {
            prop_assert!(selected.is_none(), "shutdown must reject new permits");
            return Ok(());
        }
        if self.leases.iter().all(Option::is_some) {
            prop_assert!(selected.is_none(), "held slots must not overgrant permits");
            return Ok(());
        }

        let (slot_index, permit) = selected.expect("an available slot should grant its permit");
        prop_assert!(
            self.leases
                .get(slot_index)
                .expect("the returned slot index should be in range")
                .is_none()
        );
        let lease = PooledClientLease::new(Arc::clone(&self.core), slot_index, permit, None)
            .expect("every returned index should resolve");
        self.assert_lease_identity(slot_index, &lease)?;
        *self
            .leases
            .get_mut(slot_index)
            .expect("the returned slot index should be in range") = Some(lease);
        Ok(())
    }

    fn release(&mut self, slot_index: usize) {
        let released_lease = self
            .leases
            .get_mut(slot_index)
            .expect("generated release indices should be in range")
            .take();
        drop(released_lease);
    }

    fn assert_valid(&self) -> TestCaseResult {
        for (slot_index, lease) in self.leases.iter().enumerate() {
            if let Some(lease) = lease {
                self.assert_lease_identity(slot_index, lease)?;
            }
        }
        if self.is_shutdown {
            prop_assert!(
                self.core.try_acquire_permit().is_none(),
                "shutdown must continue rejecting new permits"
            );
        }
        Ok(())
    }

    fn assert_lease_identity(&self, slot_index: usize, lease: &TestLease) -> TestCaseResult {
        let actual_address =
            std::ptr::from_ref(lease.slot().expect("held lease slot should resolve")).cast::<()>();
        let expected_address = *self
            .slot_addresses
            .get(slot_index)
            .expect("each lease index should have a recorded slot");
        prop_assert_eq!(actual_address, expected_address);
        Ok(())
    }
}

#[path = "core_tests/permit_race.rs"]
mod permit_race;
