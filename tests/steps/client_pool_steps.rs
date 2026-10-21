//! Step definitions for pooled-client behavioural tests.

use rstest_bdd_macros::{given, then, when};

use crate::fixtures::client_pool::{ClientPoolWorld, TestResult};

#[given("client pool warm reuse is configured")]
fn given_client_pool_warm_reuse(client_pool_world: &mut ClientPoolWorld) {
    let _ = client_pool_world;
}

#[given("client pool in-flight limiting is configured")]
fn given_client_pool_in_flight_limit(client_pool_world: &mut ClientPoolWorld) {
    let _ = client_pool_world;
}

#[given("client pool waiter admission is bounded")]
fn given_client_pool_waiter_admission_is_bounded(client_pool_world: &mut ClientPoolWorld) {
    let _ = client_pool_world;
}

#[given("client pool idle recycling is configured")]
fn given_client_pool_idle_recycle(client_pool_world: &mut ClientPoolWorld) {
    let _ = client_pool_world;
}

#[given("client pool cancellation safety is configured")]
fn given_client_pool_cancellation_safety(client_pool_world: &mut ClientPoolWorld) {
    let _ = client_pool_world;
}

#[when("client pool warm reuse runs against one socket", result)]
fn when_client_pool_warm_reuse_runs(client_pool_world: &mut ClientPoolWorld) -> TestResult {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(client_pool_world.run_warm_reuse_scenario())
}

#[when("client pool acquires more leases than one socket allows", result)]
fn when_client_pool_exhausts_in_flight_budget(
    client_pool_world: &mut ClientPoolWorld,
) -> TestResult {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(client_pool_world.run_in_flight_limit_scenario())
}

#[when("client pool holds every permit and exceeds its waiter limit", result)]
fn when_client_pool_exceeds_waiter_limit(client_pool_world: &mut ClientPoolWorld) -> TestResult {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(client_pool_world.run_waiter_admission_scenario())
}

#[when("client pool idles past the recycle timeout", result)]
fn when_client_pool_idles_past_timeout(client_pool_world: &mut ClientPoolWorld) -> TestResult {
    // This scenario uses paused time plus `advance()`, which is only
    // deterministic on a current-thread runtime.
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(client_pool_world.run_idle_recycle_scenario())
}

#[when("client pool cancels a delayed in-flight call", result)]
fn when_client_pool_cancels_in_flight_call(client_pool_world: &mut ClientPoolWorld) -> TestResult {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(client_pool_world.run_cancelled_call_scenario())
}

#[then("client pool warm reuse preserves one preamble handshake", result)]
fn then_client_pool_warm_reuse_preserves_preamble(
    client_pool_world: &mut ClientPoolWorld,
) -> TestResult {
    if client_pool_world.warm_reuse_preserved() {
        Ok(())
    } else {
        Err("expected warm reuse to preserve a single preamble handshake".into())
    }
}

#[then("client pool blocks the extra acquire until capacity returns", result)]
fn then_client_pool_blocks_extra_acquire(client_pool_world: &mut ClientPoolWorld) -> TestResult {
    if client_pool_world.blocked_third_acquire() && client_pool_world.recovered_after_release() {
        Ok(())
    } else {
        Err("expected third acquire to block and later recover".into())
    }
}

#[then(
    "client pool blocks an allowed waiter and rejects the excess acquire",
    result
)]
fn then_client_pool_rejects_excess_waiter(client_pool_world: &mut ClientPoolWorld) -> TestResult {
    if client_pool_world.bounded_waiter_admission_preserved() {
        Ok(())
    } else {
        Err("expected an in-limit waiter to block and excess admission to fail fast".into())
    }
}

#[then("client pool reconnects and replays the preamble", result)]
fn then_client_pool_reconnects_after_idle(client_pool_world: &mut ClientPoolWorld) -> TestResult {
    if client_pool_world.reconnected_after_idle() {
        Ok(())
    } else {
        Err("expected idle recycle to reconnect and replay preamble".into())
    }
}

#[then(
    "client pool reconnects after cancellation and releases capacity",
    result
)]
fn then_client_pool_reconnects_after_cancellation(
    client_pool_world: &mut ClientPoolWorld,
) -> TestResult {
    if client_pool_world.reconnected_after_cancelled_call() {
        Ok(())
    } else {
        Err("expected cancellation to release capacity and reconnect".into())
    }
}
