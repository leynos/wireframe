//! Step definitions for client request hook behavioural tests.

use rstest_bdd_macros::{given, then, when};

use crate::fixtures::client_request_hooks::{ClientRequestHooksWorld, TestResult};

/// Run an async block on a fresh single-threaded Tokio runtime.
fn run_block_on<F, T>(future: F) -> Result<T, wireframe_testing::TestError>
where
    F: std::future::Future<Output = Result<T, wireframe_testing::TestError>>,
{
    tokio::runtime::Runtime::new()?.block_on(future)
}

/// Verify that a counter matches the expected value.
fn check_counter(counter_name: &str, actual: usize, expected: usize) -> TestResult {
    if actual != expected {
        return Err(
            format!("expected {counter_name} counter to be {expected}, got {actual}").into(),
        );
    }
    Ok(())
}

/// Macro to generate a counter-checking step function.
macro_rules! define_counter_step {
    ($step_text:literal, $fn_name:ident, $counter_name:literal, $accessor:ident) => {
        #[then($step_text)]
        fn $fn_name(
            client_request_hooks_world: &mut ClientRequestHooksWorld,
            expected: usize,
        ) -> TestResult {
            check_counter(
                $counter_name,
                client_request_hooks_world.$accessor(),
                expected,
            )
        }
    };
}

#[given("an envelope echo server for hook testing")]
fn given_echo_server(client_request_hooks_world: &mut ClientRequestHooksWorld) -> TestResult {
    run_block_on(client_request_hooks_world.start_echo_server())
}

#[given("a client with a before_send counter hook")]
fn given_before_send_counter(
    client_request_hooks_world: &mut ClientRequestHooksWorld,
) -> TestResult {
    run_block_on(client_request_hooks_world.connect_with_before_send_counter())
}

#[given("a client with an after_receive counter hook")]
fn given_after_receive_counter(
    client_request_hooks_world: &mut ClientRequestHooksWorld,
) -> TestResult {
    run_block_on(client_request_hooks_world.connect_with_after_receive_counter())
}

#[given("a client with two before_send hooks that append markers")]
fn given_marker_hooks(client_request_hooks_world: &mut ClientRequestHooksWorld) -> TestResult {
    run_block_on(client_request_hooks_world.connect_with_marker_hooks())
}

#[given("a client with both counter hooks")]
fn given_both_counters(client_request_hooks_world: &mut ClientRequestHooksWorld) -> TestResult {
    run_block_on(client_request_hooks_world.connect_with_both_counters())
}

#[given("a client with a before_send hook that appends a marker byte")]
fn given_before_send_marker(
    client_request_hooks_world: &mut ClientRequestHooksWorld,
) -> TestResult {
    run_block_on(async {
        client_request_hooks_world.start_capturing_server().await?;
        client_request_hooks_world
            .connect_with_before_send_marker()
            .await
    })
}

#[given("a client with an after_receive hook that replaces the frame")]
fn given_after_receive_replacement(
    client_request_hooks_world: &mut ClientRequestHooksWorld,
) -> TestResult {
    run_block_on(client_request_hooks_world.connect_with_after_receive_replacement())
}

#[when("the client sends an envelope via the hooked client")]
fn when_send_envelope(client_request_hooks_world: &mut ClientRequestHooksWorld) -> TestResult {
    run_block_on(client_request_hooks_world.send_envelope())
}

#[when("the client sends and receives an envelope via the hooked client")]
fn when_send_and_receive(client_request_hooks_world: &mut ClientRequestHooksWorld) -> TestResult {
    run_block_on(client_request_hooks_world.send_and_receive_envelope())
}

#[when("the client performs a correlated call via the hooked client")]
fn when_correlated_call(client_request_hooks_world: &mut ClientRequestHooksWorld) -> TestResult {
    run_block_on(client_request_hooks_world.perform_correlated_call())
}

define_counter_step!(
    "the before_send counter is {expected:usize}",
    then_before_send_counter,
    "before_send",
    before_send_count
);

define_counter_step!(
    "the after_receive counter is {expected:usize}",
    then_after_receive_counter,
    "after_receive",
    after_receive_count
);

#[then("the markers appear in registration order")]
fn then_markers_in_order(client_request_hooks_world: &mut ClientRequestHooksWorld) -> TestResult {
    let log = client_request_hooks_world.marker_log()?;
    if log != [b'A', b'B'] {
        return Err(format!("expected markers [A, B] in registration order, got {log:?}").into());
    }
    Ok(())
}

#[then("the captured frame ends with the marker byte")]
fn then_captured_frame_has_marker(
    client_request_hooks_world: &mut ClientRequestHooksWorld,
) -> TestResult {
    let frames = run_block_on(async {
        client_request_hooks_world
            .collect_captured_frames()
            .await
            .map_err(Into::into)
    })?;
    let frame = frames.first().ok_or("no frames captured")?;
    let marker = ClientRequestHooksWorld::marker_byte();
    if frame.last().copied() != Some(marker) {
        return Err(format!(
            "expected frame to end with marker byte {marker:#04x}, got {:?}",
            frame.last()
        )
        .into());
    }
    Ok(())
}

#[then("the received payload reflects the hook mutation")]
fn then_received_payload_reflects_mutation(
    client_request_hooks_world: &mut ClientRequestHooksWorld,
) -> TestResult {
    let payload = client_request_hooks_world
        .received_payload()
        .ok_or("no received payload stored")?;
    if payload != [99, 98, 97] {
        return Err(format!("expected mutated payload [99, 98, 97], got {payload:?}").into());
    }
    Ok(())
}
