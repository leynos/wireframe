//! Step definitions for client messaging behavioural tests with correlation IDs.

use rstest_bdd_macros::{given, then, when};

use crate::fixtures::client_messaging::{ClientMessagingWorld, TestResult};

#[given("an envelope echo server")]
fn given_echo_server(client_messaging_world: &mut ClientMessagingWorld) -> TestResult {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        client_messaging_world.start_echo_server().await?;
        client_messaging_world.connect_client().await
    })
}

#[given("an envelope without a correlation ID")]
fn given_envelope_without_correlation(client_messaging_world: &mut ClientMessagingWorld) {
    client_messaging_world.set_envelope_without_correlation();
}

#[given("an envelope with correlation ID {correlation_id:u64}")]
fn given_envelope_with_correlation(
    client_messaging_world: &mut ClientMessagingWorld,
    correlation_id: u64,
) {
    client_messaging_world.set_envelope_with_correlation(correlation_id);
}

#[given("an envelope with message ID {message_id:u32} and payload {payload:string}")]
fn given_envelope_with_payload(
    client_messaging_world: &mut ClientMessagingWorld,
    message_id: u32,
    payload: String,
) {
    client_messaging_world.set_envelope_with_payload(message_id, &payload);
}

#[given("a server that returns mismatched correlation IDs")]
fn given_mismatch_server(client_messaging_world: &mut ClientMessagingWorld) -> TestResult {
    client_messaging_world.abort_server();
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        client_messaging_world.start_mismatch_server().await?;
        client_messaging_world.connect_client().await
    })
}

#[when("the client sends the envelope")]
fn when_client_sends_envelope(client_messaging_world: &mut ClientMessagingWorld) -> TestResult {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(client_messaging_world.send_envelope())
}

#[when("the client calls the server with call_correlated")]
fn when_client_calls_correlated(client_messaging_world: &mut ClientMessagingWorld) -> TestResult {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(client_messaging_world.call_correlated())
}

#[when("the client sends {count:usize} sequential envelopes")]
fn when_client_sends_multiple(
    client_messaging_world: &mut ClientMessagingWorld,
    count: usize,
) -> TestResult {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(client_messaging_world.send_multiple_envelopes(count))
}

#[then("the envelope is stamped with an auto-generated correlation ID")]
fn then_auto_generated_correlation(
    client_messaging_world: &mut ClientMessagingWorld,
) -> TestResult {
    client_messaging_world.verify_auto_generated_correlation()
}

#[then("the returned correlation ID is {expected:u64}")]
fn then_correlation_id_is(
    client_messaging_world: &mut ClientMessagingWorld,
    expected: u64,
) -> TestResult {
    client_messaging_world.verify_correlation_id(expected)
}

#[then("the response has a matching correlation ID")]
fn then_response_has_matching_correlation(
    client_messaging_world: &mut ClientMessagingWorld,
) -> TestResult {
    client_messaging_world.verify_response_correlation_matches()
}

#[then("no correlation mismatch error occurs")]
fn then_no_mismatch_error(client_messaging_world: &mut ClientMessagingWorld) -> TestResult {
    client_messaging_world.verify_no_mismatch_error()
}

#[then("a CorrelationMismatch error is returned")]
fn then_mismatch_error(client_messaging_world: &mut ClientMessagingWorld) -> TestResult {
    client_messaging_world.verify_mismatch_error()
}

#[then("each envelope has a unique correlation ID")]
fn then_unique_correlation_ids(client_messaging_world: &mut ClientMessagingWorld) -> TestResult {
    client_messaging_world.verify_unique_correlation_ids()
}

#[then("the response contains the same message ID and payload")]
fn then_response_matches(client_messaging_world: &mut ClientMessagingWorld) -> TestResult {
    client_messaging_world.verify_response_matches_expected()
}
