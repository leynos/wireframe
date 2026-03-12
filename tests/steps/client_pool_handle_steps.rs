//! Step definitions for `PoolHandle` behavioural tests.

use rstest_bdd_macros::{given, then, when};

use crate::fixtures::client_pool_handle::{ClientPoolHandleWorld, TestResult};

#[given("client pool handle round-robin fairness is configured")]
fn given_client_pool_handle_round_robin(client_pool_handle_world: &mut ClientPoolHandleWorld) {
    let _ = client_pool_handle_world;
}

#[given("client pool handle FIFO fairness is configured")]
fn given_client_pool_handle_fifo(client_pool_handle_world: &mut ClientPoolHandleWorld) {
    let _ = client_pool_handle_world;
}

#[given("client pool handle back-pressure is configured")]
fn given_client_pool_handle_back_pressure(client_pool_handle_world: &mut ClientPoolHandleWorld) {
    let _ = client_pool_handle_world;
}

#[given("client pool handle warm reuse and idle recycle are configured")]
fn given_client_pool_handle_reuse_and_recycle(
    client_pool_handle_world: &mut ClientPoolHandleWorld,
) {
    let _ = client_pool_handle_world;
}

#[when("two logical sessions repeatedly acquire one pooled socket")]
fn when_two_logical_sessions_repeat(
    client_pool_handle_world: &mut ClientPoolHandleWorld,
) -> TestResult {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(client_pool_handle_world.run_round_robin_scenario())
}

#[when("three logical sessions wait in order on one pooled socket")]
fn when_three_logical_sessions_wait_in_order(
    client_pool_handle_world: &mut ClientPoolHandleWorld,
) -> TestResult {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(client_pool_handle_world.run_fifo_scenario())
}

#[when("one logical session holds the only pooled lease")]
fn when_one_logical_session_holds_the_only_lease(
    client_pool_handle_world: &mut ClientPoolHandleWorld,
) -> TestResult {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(client_pool_handle_world.run_back_pressure_scenario())
}

#[when("one logical session reuses a warm socket and then idles past timeout")]
fn when_one_logical_session_reuses_then_idles(
    client_pool_handle_world: &mut ClientPoolHandleWorld,
) -> TestResult {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(client_pool_handle_world.run_warm_reuse_then_idle_recycle_scenario())
}

#[then("the logical sessions alternate fairly")]
fn then_the_logical_sessions_alternate_fairly(
    client_pool_handle_world: &mut ClientPoolHandleWorld,
) -> TestResult {
    if client_pool_handle_world.sessions_alternate_fairly() {
        Ok(())
    } else {
        Err("expected logical sessions to alternate fairly".into())
    }
}

#[then("the logical sessions are served in arrival order")]
fn then_the_logical_sessions_are_served_in_arrival_order(
    client_pool_handle_world: &mut ClientPoolHandleWorld,
) -> TestResult {
    if client_pool_handle_world.fifo_order_preserved() {
        Ok(())
    } else {
        Err("expected FIFO fairness to preserve arrival order".into())
    }
}

#[then("another logical session stays blocked until capacity returns")]
fn then_another_logical_session_stays_blocked(
    client_pool_handle_world: &mut ClientPoolHandleWorld,
) -> TestResult {
    if client_pool_handle_world.back_pressure_preserved() {
        Ok(())
    } else {
        Err("expected waiting logical session to remain blocked".into())
    }
}

#[then("the handle preserves warm reuse and later reconnects after idle")]
fn then_the_handle_preserves_warm_reuse_then_recycles(
    client_pool_handle_world: &mut ClientPoolHandleWorld,
) -> TestResult {
    if client_pool_handle_world.warm_reuse_then_recycle_preserved() {
        Ok(())
    } else {
        Err("expected handle warm reuse to preserve preamble and later recycle".into())
    }
}
