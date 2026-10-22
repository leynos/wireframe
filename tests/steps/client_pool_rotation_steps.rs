//! Step definitions for pooled slot-rotation behavioural tests.

use rstest_bdd_macros::{given, then, when};

use crate::fixtures::client_pool_rotation::{ClientPoolRotationWorld, TestResult};

#[given("client pool rotation is configured with two single-permit slots")]
fn given_client_pool_rotation_is_configured(
    client_pool_rotation_world: &mut ClientPoolRotationWorld,
) {
    let _ = client_pool_rotation_world;
}

#[when(
    "client pool acquires two leases and parks a third acquisition",
    result
)]
fn when_client_pool_acquires_and_parks(
    client_pool_rotation_world: &mut ClientPoolRotationWorld,
) -> TestResult {
    block_on(client_pool_rotation_world.run_multi_slot_rotation_scenario())
}

#[when("client pool keeps one lease and releases the other", result)]
fn when_client_pool_releases_one_lease(
    client_pool_rotation_world: &mut ClientPoolRotationWorld,
) -> TestResult {
    block_on(client_pool_rotation_world.run_saturated_start_skip_scenario())
}

#[then("client pool leases rotate across both slots", result)]
fn then_client_pool_leases_rotate_across_slots(
    client_pool_rotation_world: &mut ClientPoolRotationWorld,
) -> TestResult {
    if client_pool_rotation_world.multi_slot_rotation_preserved() {
        Ok(())
    } else {
        Err("expected successive leases to rotate across distinct slots".into())
    }
}

#[then(
    "client pool resumes the parked acquisition on the released slot",
    result
)]
fn then_client_pool_resumes_parked_acquisition(
    client_pool_rotation_world: &mut ClientPoolRotationWorld,
) -> TestResult {
    if client_pool_rotation_world.saturated_slot_recovered() {
        Ok(())
    } else {
        Err("expected the parked acquisition to resume on the released slot".into())
    }
}

#[then("client pool observes one physical connection per slot", result)]
fn then_client_pool_observes_one_connection_per_slot(
    client_pool_rotation_world: &mut ClientPoolRotationWorld,
) -> TestResult {
    if client_pool_rotation_world.slot_count() == 2
        && client_pool_rotation_world.distinct_sockets_observed()
    {
        Ok(())
    } else {
        Err("expected one physical connection and preamble per configured slot".into())
    }
}

#[then("client pool reacquires from the released slot", result)]
fn then_client_pool_reacquires_from_released_slot(
    client_pool_rotation_world: &mut ClientPoolRotationWorld,
) -> TestResult {
    if client_pool_rotation_world.saturated_start_skipped() {
        Ok(())
    } else {
        Err("expected the reacquisition to skip the still-leased starting slot".into())
    }
}

#[then("client pool leaves the kept lease usable", result)]
fn then_client_pool_keeps_lease_usable(
    client_pool_rotation_world: &mut ClientPoolRotationWorld,
) -> TestResult {
    if client_pool_rotation_world.kept_lease_usable() {
        Ok(())
    } else {
        Err("expected the lease held across the reacquisition to stay usable".into())
    }
}

/// Run an async world method on a fresh multi-threaded runtime.
fn block_on(scenario: impl std::future::Future<Output = TestResult>) -> TestResult {
    tokio::runtime::Runtime::new()?.block_on(scenario)
}
