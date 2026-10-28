//! Rotation, cancellation, and resource-drop coverage for permit selection.

use std::{sync::Arc, time::Duration};

use rstest::rstest;
use tokio::sync::OwnedSemaphorePermit;

use super::support::{TestCore, four_slot_core, test_core};

pub(super) fn hold_all_permits(core: &TestCore) -> Vec<OwnedSemaphorePermit> {
    core.slots
        .iter()
        .map(|slot| {
            slot.try_acquire_permit()
                .expect("each slot should have one available permit")
        })
        .collect()
}

fn hold_every_permit_except(core: &TestCore, free_index: usize) -> Vec<OwnedSemaphorePermit> {
    core.slots
        .iter()
        .enumerate()
        .filter(|(slot_index, _)| *slot_index != free_index)
        .map(|(_, slot)| {
            slot.try_acquire_permit()
                .expect("every slot other than the free slot should be held")
        })
        .collect()
}

pub(super) fn assert_all_slots_are_available(core: &TestCore) {
    let permits = hold_all_permits(core);
    drop(permits);
}

#[rstest]
#[case::one_slot(1, &[0, 0, 0])]
#[case::two_slots(2, &[0, 1, 0])]
#[case::four_slots(4, &[0, 1, 2, 3, 0])]
#[tokio::test]
async fn immediate_probe_rotates_across_slots(
    #[case] slot_count: usize,
    #[case] expected: &[usize],
) {
    let core = test_core(slot_count);
    let selected = expected
        .iter()
        .map(|_| {
            let (slot_index, permit) = core
                .try_acquire_permit()
                .expect("a free slot should grant an immediate permit");
            drop(permit);
            slot_index
        })
        .collect::<Vec<_>>();

    assert_eq!(selected.as_slice(), expected);
}

#[rstest]
#[tokio::test(flavor = "current_thread")]
async fn failed_and_cancelled_acquisitions_each_advance_rotation_once() {
    let core = test_core(2);
    let held = hold_all_permits(&core);

    assert!(core.try_acquire_permit().is_none());
    drop(held);

    let (after_failed_try, permit) = core
        .try_acquire_permit()
        .expect("the next immediate probe should find free capacity");
    assert_eq!(after_failed_try, 1);
    drop(permit);

    let held = hold_all_permits(&core);
    {
        let mut acquisition = std::pin::pin!(core.acquire_slot_permit());
        assert!(futures::poll!(acquisition.as_mut()).is_pending());
    }
    drop(held);

    let (after_cancelled_race, permit) = core
        .try_acquire_permit()
        .expect("the next immediate probe should find free capacity");
    assert_eq!(after_cancelled_race, 1);
    drop(permit);
}

#[rstest]
#[tokio::test]
async fn empty_core_rejects_both_immediate_and_async_acquisition() {
    let core = test_core(0);

    assert!(core.try_acquire_permit().is_none());
    assert!(core.acquire_slot_permit().await.is_err());
}

#[rstest]
#[tokio::test(flavor = "current_thread")]
async fn shutdown_rejects_acquisition_before_waiting_for_slot_capacity() {
    let core = test_core(2);
    let held = hold_all_permits(&core);
    core.shutdown();

    let result = tokio::time::timeout(Duration::from_millis(100), core.acquire_slot_permit()).await;

    assert!(matches!(result, Ok(Err(_))));
    drop(held);
    assert_all_slots_are_available(&core);
}

#[rstest]
#[tokio::test]
async fn async_probe_rotates_across_four_slots(four_slot_core: Arc<TestCore>) {
    let mut selected = Vec::new();
    for _ in 0..5 {
        let (slot_index, permit) = four_slot_core
            .acquire_slot_permit()
            .await
            .expect("a free slot should grant an immediate async permit");
        drop(permit);
        selected.push(slot_index);
    }

    assert_eq!(selected, [0, 1, 2, 3, 0]);
}

#[rstest]
#[tokio::test]
async fn sync_and_async_probes_share_one_rotation_cursor(four_slot_core: Arc<TestCore>) {
    let (first, permit) = four_slot_core
        .try_acquire_permit()
        .expect("the first immediate probe should succeed");
    drop(permit);
    let (second, permit) = four_slot_core
        .acquire_slot_permit()
        .await
        .expect("the second immediate probe should succeed");
    drop(permit);
    let (third, permit) = four_slot_core
        .try_acquire_permit()
        .expect("the third immediate probe should succeed");
    drop(permit);
    let (fourth, permit) = four_slot_core
        .acquire_slot_permit()
        .await
        .expect("the fourth immediate probe should succeed");

    assert_eq!([first, second, third, fourth], [0, 1, 2, 3]);
    drop(permit);
}

#[rstest]
#[case::two_first(2, 0)]
#[case::two_second(2, 1)]
#[case::four_first(4, 0)]
#[case::four_second(4, 1)]
#[case::four_third(4, 2)]
#[case::four_fourth(4, 3)]
#[tokio::test(flavor = "current_thread")]
async fn immediate_probe_finds_each_free_slot(
    #[case] slot_count: usize,
    #[case] free_index: usize,
) {
    let core = test_core(slot_count);
    let _held = hold_every_permit_except(&core, free_index);

    let (selected, permit) = core
        .acquire_slot_permit()
        .await
        .expect("the only free slot should grant its permit");

    assert_eq!(selected, free_index);
    drop(permit);
}

#[rstest]
#[case::two_first(2, 0)]
#[case::two_second(2, 1)]
#[case::four_first(4, 0)]
#[case::four_second(4, 1)]
#[case::four_third(4, 2)]
#[case::four_fourth(4, 3)]
#[tokio::test(flavor = "current_thread")]
async fn blocked_race_returns_the_single_slot_released_after_registration(
    #[case] slot_count: usize,
    #[case] free_index: usize,
) {
    let core = test_core(slot_count);
    let mut held = hold_all_permits(&core);
    let mut acquisition = std::pin::pin!(core.acquire_slot_permit());
    assert!(futures::poll!(acquisition.as_mut()).is_pending());

    drop(held.swap_remove(free_index));
    let (selected, permit) = tokio::time::timeout(Duration::from_secs(1), acquisition)
        .await
        .expect("released capacity should wake the blocked race")
        .expect("the released slot should grant exactly one permit");

    assert_eq!(selected, free_index);
    assert!(
        core.slot(selected)
            .expect("the selected slot should exist")
            .try_acquire_permit()
            .is_none(),
        "the winner's permit must remain owned by the acquisition"
    );
    drop(permit);
    drop(held);
    assert_all_slots_are_available(&core);
}

#[rstest]
#[tokio::test(flavor = "current_thread")]
async fn simultaneous_readiness_returns_one_permit_and_drops_the_loser() {
    let core = test_core(2);
    let held = hold_all_permits(&core);
    let mut acquisition = std::pin::pin!(core.acquire_slot_permit());
    assert!(futures::poll!(acquisition.as_mut()).is_pending());

    drop(held);
    let (winner_index, winner_permit) = tokio::time::timeout(Duration::from_secs(1), acquisition)
        .await
        .expect("both released slots should wake the race")
        .expect("one of the ready slots should win");
    let loser_index = 1_usize.saturating_sub(winner_index);

    assert!(
        core.slot(winner_index)
            .expect("the winner should be a valid slot")
            .try_acquire_permit()
            .is_none(),
        "the returned permit must reserve exactly the winning slot"
    );
    let loser_permit = core
        .slot(loser_index)
        .expect("the other slot should exist")
        .try_acquire_permit();
    assert!(
        loser_permit.is_some(),
        "a losing ready future must not retain its acquired permit"
    );

    drop(loser_permit);
    drop(winner_permit);
    assert_all_slots_are_available(&core);
}

#[rstest]
#[tokio::test(flavor = "current_thread")]
async fn cancelling_a_blocked_race_leaves_every_slot_usable() {
    let core = test_core(4);
    let held = hold_all_permits(&core);
    {
        let mut acquisition = std::pin::pin!(core.acquire_slot_permit());
        assert!(futures::poll!(acquisition.as_mut()).is_pending());
    }

    drop(held);
    assert_all_slots_are_available(&core);
    let (slot_index, permit) =
        tokio::time::timeout(Duration::from_secs(1), core.acquire_slot_permit())
            .await
            .expect("a fresh acquisition should complete after cancellation")
            .expect("cancellation must not lose any slot permit");

    assert!(slot_index < core.slots.len());
    drop(permit);
}

/// Exhaust all non-empty readiness sets for small blocked races.
#[rstest]
#[case::two_slots(2)]
#[case::three_slots(3)]
#[case::four_slots(4)]
#[tokio::test(flavor = "current_thread")]
async fn blocked_race_preserves_capacity_for_every_ready_set(#[case] slot_count: usize) {
    for ready in 1..(1_usize << slot_count) {
        let core = test_core(slot_count);
        let mut held = hold_all_permits(&core)
            .into_iter()
            .map(Some)
            .collect::<Vec<_>>();
        let mut acquisition = std::pin::pin!(core.acquire_slot_permit());
        assert!(futures::poll!(acquisition.as_mut()).is_pending());

        for (index, permit) in held.iter_mut().enumerate() {
            if ready & (1 << index) != 0 {
                drop(permit.take());
            }
        }
        let (winner, permit) = tokio::time::timeout(Duration::from_secs(1), acquisition)
            .await
            .expect("a ready slot must wake the race")
            .expect("a ready slot must grant a permit");
        assert_ne!(ready & (1 << winner), 0);

        for index in 0..slot_count {
            let available = core
                .slot(index)
                .expect("the model slot should exist")
                .try_acquire_permit();
            assert_eq!(
                available.is_some(),
                index != winner && ready & (1 << index) != 0,
                "unexpected capacity for slot {index}, ready set {ready:#b}"
            );
        }
        drop(permit);
        drop(held);
        assert_all_slots_are_available(&core);
    }
}
