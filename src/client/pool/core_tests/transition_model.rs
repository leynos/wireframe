//! Bounded release, cancellation, and shutdown schedules for blocked races.

use std::{future::Future, io::ErrorKind, pin::Pin, task::Poll, time::Duration};

use rstest::rstest;

use super::{
    permit_race::hold_all_permits,
    support::{TestCore, test_core},
};
use crate::{
    WireframeError,
    client::{ClientError, pool::core::SlotPermit},
};

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

/// Enumerate every non-empty ready set and each release order for that set.
fn readiness_orders(slot_count: usize) -> Vec<(usize, Vec<usize>)> {
    let mut schedules = Vec::new();
    for ready in 1..(1_usize << slot_count) {
        let slots = (0..slot_count)
            .filter(|index| ready & (1 << *index) != 0)
            .collect::<Vec<_>>();
        let mut orders = Vec::new();
        release_orders(&slots, &mut Vec::new(), &mut orders);
        schedules.extend(orders.into_iter().map(|order| (ready, order)));
    }
    schedules
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

impl Terminal {
    /// Report whether the scenario should return a winning permit.
    const fn is_grant(self) -> bool { matches!(self, Self::Grant) }

    /// Report whether cancellation should leave the core open.
    const fn is_cancel(self) -> bool { matches!(self, Self::Cancel) }

    /// Report whether shutdown is published before the race is dropped.
    const fn is_shutdown(self) -> bool { matches!(self, Self::Shutdown | Self::ShutdownCancel) }

    /// Report whether the shutdown race is polled after capacity is released.
    const fn is_inflight_shutdown(self) -> bool { matches!(self, Self::Shutdown) }
}

/// One bounded readiness and terminal-action scenario.
struct Schedule {
    /// Number of stable slots in the core.
    slot_count: usize,
    /// Bit set of slots released after registration.
    ready: usize,
    /// Order in which the selected slot permits are released.
    order: Vec<usize>,
    /// Action taken after the selected slots are ready.
    terminal: Terminal,
}

/// Recognize the typed disconnected error used by pool shutdown.
fn check_disconnected(error: ClientError) -> Result<(), String> {
    match error {
        ClientError::Wireframe(WireframeError::Io(source))
            if source.kind() == ErrorKind::UnexpectedEof =>
        {
            Ok(())
        }
        other => Err(format!("expected disconnected error, got {other}")),
    }
}

/// Check every raw slot can grant a permit at the same time.
fn check_all_slots_available(core: &TestCore) -> Result<(), String> {
    let permits = core
        .slots
        .iter()
        .map(|slot| {
            slot.try_acquire_permit()
                .ok_or_else(|| "a slot lost capacity".to_owned())
        })
        .collect::<Result<Vec<_>, _>>()?;
    drop(permits);
    Ok(())
}

/// Verify the winner is ready and no losing future retains released capacity.
async fn check_grant<F>(
    core: &TestCore,
    schedule: &Schedule,
    acquisition: Pin<&mut F>,
) -> Result<(), String>
where
    F: Future<Output = Result<SlotPermit, ClientError>>,
{
    let (winner, permit) = tokio::time::timeout(Duration::from_secs(1), acquisition)
        .await
        .map_err(|error| format!("released capacity did not wake the race: {error}"))?
        .map_err(|error| format!("a ready slot did not grant a permit: {error}"))?;
    if schedule.ready & (1 << winner) == 0 {
        return Err(format!(
            "winner {winner} was not ready for {:?}",
            schedule.order
        ));
    }
    for index in 0..schedule.slot_count {
        let available = core
            .slot(index)
            .ok_or("slot index must exist")?
            .try_acquire_permit();
        let expected = index != winner && schedule.ready & (1 << index) != 0;
        if available.is_some() != expected {
            return Err(format!(
                "unexpected capacity for {:?}, order {:?}",
                schedule.terminal, schedule.order
            ));
        }
    }
    drop(permit);
    Ok(())
}

/// Verify a ready in-flight race rejects its permit after shutdown.
async fn check_inflight_shutdown<F>(acquisition: Pin<&mut F>) -> Result<(), String>
where
    F: Future<Output = Result<SlotPermit, ClientError>>,
{
    let result = tokio::time::timeout(Duration::from_secs(1), acquisition)
        .await
        .map_err(|error| format!("ready capacity did not wake the shutdown race: {error}"))?;
    match result {
        Err(error) => check_disconnected(error),
        Ok(_) => Err("shutdown granted a permit".to_owned()),
    }
}

/// Apply the terminal action while the pinned acquisition remains in scope.
async fn apply_terminal<F>(
    core: &TestCore,
    schedule: &Schedule,
    acquisition: Pin<&mut F>,
) -> Result<(), String>
where
    F: Future<Output = Result<SlotPermit, ClientError>>,
{
    if schedule.terminal.is_shutdown() {
        core.shutdown();
    }
    if schedule.terminal.is_grant() {
        return check_grant(core, schedule, acquisition).await;
    }
    if schedule.terminal.is_inflight_shutdown() {
        return check_inflight_shutdown(acquisition).await;
    }
    Ok(())
}

/// Check a later acquisition after cancellation or shutdown.
async fn check_later_acquisition(core: &TestCore, terminal: Terminal) -> Result<(), String> {
    if terminal.is_grant() {
        return Ok(());
    }
    let result = tokio::time::timeout(Duration::from_secs(1), core.acquire_slot_permit())
        .await
        .map_err(|error| format!("a later race remained queued: {error}"))?;
    if terminal.is_cancel() {
        let (_, permit) =
            result.map_err(|error| format!("a later race did not acquire capacity: {error}"))?;
        drop(permit);
        return check_all_slots_available(core);
    }
    match result {
        Err(error) => check_disconnected(error),
        Ok(_) => Err("a shut-down core granted a later permit".to_owned()),
    }
}

/// Register one race, release its selected slots, and check the terminal state.
async fn run_schedule(schedule: &Schedule) -> Result<(), String> {
    let core = test_core(schedule.slot_count);
    let mut held = hold_all_permits(&core)
        .into_iter()
        .map(Some)
        .collect::<Vec<_>>();
    {
        let mut acquisition = std::pin::pin!(core.acquire_slot_permit());
        if !matches!(futures::poll!(acquisition.as_mut()), Poll::Pending) {
            return Err("a saturated race must register and remain pending".to_owned());
        }
        for index in &schedule.order {
            drop(held.get_mut(*index).ok_or("slot index must exist")?.take());
        }
        apply_terminal(&core, schedule, acquisition.as_mut()).await?;
    }
    drop(held);
    check_all_slots_available(&core)?;
    check_later_acquisition(&core, schedule.terminal).await
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
    for (ready, order) in readiness_orders(slot_count) {
        let schedule = Schedule {
            slot_count,
            ready,
            order,
            terminal,
        };
        run_schedule(&schedule)
            .await
            .expect("the bounded race schedule must preserve capacity");
    }
}
