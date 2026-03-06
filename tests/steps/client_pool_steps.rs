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

#[given("client pool idle recycling is configured")]
fn given_client_pool_idle_recycle(client_pool_world: &mut ClientPoolWorld) {
    let _ = client_pool_world;
}

#[when("client pool warm reuse runs against one socket")]
fn when_client_pool_warm_reuse_runs(client_pool_world: &mut ClientPoolWorld) -> TestResult {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(client_pool_world.run_warm_reuse_scenario())
}

#[when("client pool acquires more leases than one socket allows")]
fn when_client_pool_exhausts_in_flight_budget(
    client_pool_world: &mut ClientPoolWorld,
) -> TestResult {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(client_pool_world.run_in_flight_limit_scenario())
}

#[when("client pool idles past the recycle timeout")]
fn when_client_pool_idles_past_timeout(client_pool_world: &mut ClientPoolWorld) -> TestResult {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(client_pool_world.run_idle_recycle_scenario())
}

#[then("client pool warm reuse preserves one preamble handshake")]
fn then_client_pool_warm_reuse_preserves_preamble(
    client_pool_world: &mut ClientPoolWorld,
) -> TestResult {
    if client_pool_world.warm_reuse_preserved() {
        Ok(())
    } else {
        Err("expected warm reuse to preserve a single preamble handshake".into())
    }
}

#[then("client pool blocks the extra acquire until capacity returns")]
fn then_client_pool_blocks_extra_acquire(client_pool_world: &mut ClientPoolWorld) -> TestResult {
    if client_pool_world.blocked_third_acquire() && client_pool_world.recovered_after_release() {
        Ok(())
    } else {
        Err("expected third acquire to block and later recover".into())
    }
}

#[then("client pool reconnects and replays the preamble")]
fn then_client_pool_reconnects_after_idle(client_pool_world: &mut ClientPoolWorld) -> TestResult {
    if client_pool_world.reconnected_after_idle() {
        Ok(())
    } else {
        Err("expected idle recycle to reconnect and replay preamble".into())
    }
}
