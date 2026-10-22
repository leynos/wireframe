//! Rotation-order coverage for client-pool slot selection.
//!
//! These tests exercise the pure rotation arithmetic and the live cursor
//! directly. Slot construction is lazy (`build_unchecked`), so no test here
//! needs a live socket.

use std::{sync::atomic::Ordering, time::Duration};

use proptest::{
    prelude::*,
    test_runner::{Config as ProptestConfig, TestCaseResult, TestRunner},
};
use rstest::rstest;

use super::{PoolCore, rotation_indices, rotation_start};
use crate::{client::pool::test_support::test_pool_slot, serializer::BincodeSerializer};

type TestCore = PoolCore<BincodeSerializer, (), ()>;

/// Build a core of `slot_count` lazy slots without connecting.
fn test_core(slot_count: usize) -> TestCore {
    let slots = (0..slot_count)
        .map(|_| test_pool_slot(1, Duration::from_secs(30)))
        .collect::<Vec<_>>()
        .into_boxed_slice();
    TestCore::new(slots)
}

/// Rotation must start at the cursor, wrap once, and visit every slot exactly
/// once so no socket is favoured or skipped.
///
/// The overflowing cases matter for non-power-of-two pools: a wrapped `usize`
/// is congruent to its value minus `2^word`, and that difference is a multiple
/// of the slot count only for power-of-two sizes. Reducing the seed after
/// adding the offset would therefore repeat one slot and skip another.
#[rstest]
#[case::from_cursor(3, 1, &[1, 2, 0])]
#[case::overflowing_power_of_two(4, usize::MAX - 1, &[2, 3, 0, 1])]
#[case::overflowing_three_slots(3, usize::MAX - 1, &[2, 0, 1])]
#[case::overflowing_five_slots(5, usize::MAX, &[0, 1, 2, 3, 4])]
fn rotation_visits_each_slot_once_from_a_reduced_seed(
    #[case] slot_count: usize,
    #[case] cursor: usize,
    #[case] expected: &[usize],
) {
    let start = rotation_start(slot_count, cursor);
    let visited = rotation_indices(slot_count, start).collect::<Vec<_>>();

    assert_eq!(visited, expected);
}

/// An empty pool must yield no indices and must not consume a cursor value,
/// matching the previous no-op rotation.
#[tokio::test]
async fn empty_pool_consumes_no_cursor_value() {
    let core = test_core(0);

    assert!(core.try_acquire_permit().is_none());
    assert_eq!(core.next_slot.load(Ordering::Relaxed), 0);
}

/// Each non-empty attempt must advance the shared cursor exactly once, so
/// successive acquisitions start on successive slots.
#[tokio::test]
async fn each_non_empty_attempt_advances_the_cursor_once() {
    let core = test_core(3);
    let visited = (0..3)
        .filter_map(|_| core.try_acquire_permit().map(|(slot_index, _)| slot_index))
        .collect::<Vec<_>>();

    assert_eq!(visited, vec![0, 1, 2]);
    assert_eq!(core.next_slot.load(Ordering::Relaxed), 3);
}

/// For any non-empty slot count and cursor, the rotation must yield every
/// index exactly once in cyclic order, starting at the reduced cursor.
#[tokio::test]
async fn generated_rotations_visit_each_slot_once_in_cyclic_order() {
    let mut runner = TestRunner::new(ProptestConfig {
        cases: 128,
        ..ProptestConfig::default()
    });
    let generated_inputs = (1_usize..=64, any::<usize>());

    runner
        .run(&generated_inputs, check_generated_rotation)
        .expect("generated rotations should visit each slot exactly once");
}

fn check_generated_rotation((slot_count, cursor): (usize, usize)) -> TestCaseResult {
    let start = rotation_start(slot_count, cursor);
    // Pin the seed contract itself, not only the indices derived from it: the
    // assertions below compare `rotation_indices` against `start`, so a seed
    // bug would otherwise be self-consistent and go unnoticed.
    prop_assert_eq!(
        start,
        cursor.wrapping_rem(slot_count),
        "seed must be the cursor reduced below the slot count"
    );
    let visited = rotation_indices(slot_count, start).collect::<Vec<_>>();

    prop_assert_eq!(visited.len(), slot_count);
    let mut sorted = visited.clone();
    sorted.sort_unstable();
    sorted.dedup();
    prop_assert_eq!(sorted.len(), slot_count, "each slot index must be unique");

    for (offset, index) in visited.iter().enumerate() {
        prop_assert!(*index < slot_count);
        let expected = start.wrapping_add(offset).wrapping_rem(slot_count);
        prop_assert_eq!(*index, expected, "rotation must advance one slot at a time");
    }
    prop_assert_eq!(
        visited.first().copied(),
        Some(start),
        "rotation must start at the reduced cursor"
    );
    Ok(())
}
