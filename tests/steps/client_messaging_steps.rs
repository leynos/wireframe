//! Steps for client messaging behavioural tests with correlation ID support.
//!
//! Cucumber step functions are required to be async by the framework, even when
//! they don't contain await expressions.

#![expect(
    clippy::unused_async,
    reason = "cucumber requires async step functions"
)]

use cucumber::{given, then, when};
use wireframe::app::Packet;

use crate::world::{ClientMessagingWorld, TestResult};

#[given("an envelope echo server")]
async fn given_echo_server(world: &mut ClientMessagingWorld) -> TestResult {
    world.start_echo_server().await?;
    world.connect_client().await
}

#[given("an envelope without a correlation ID")]
async fn given_envelope_without_correlation(world: &mut ClientMessagingWorld) -> TestResult {
    world.set_envelope_without_correlation();
    Ok(())
}

#[given(expr = "an envelope with correlation ID {int}")]
async fn given_envelope_with_correlation(
    world: &mut ClientMessagingWorld,
    correlation_id: u64,
) -> TestResult {
    world.set_envelope_with_correlation(correlation_id);
    Ok(())
}

#[given(expr = "an envelope with message ID {int} and payload {string}")]
async fn given_envelope_with_payload(
    world: &mut ClientMessagingWorld,
    message_id: u32,
    payload: String,
) -> TestResult {
    world.set_envelope_with_payload(message_id, &payload);
    Ok(())
}

#[given("a server that returns mismatched correlation IDs")]
async fn given_mismatch_server(world: &mut ClientMessagingWorld) -> TestResult {
    // First, abort the existing server.
    world.await_server();
    // Start a mismatch server.
    world.start_mismatch_server().await?;
    world.connect_client().await
}

#[when("the client sends the envelope")]
async fn when_client_sends_envelope(world: &mut ClientMessagingWorld) -> TestResult {
    world.send_envelope().await
}

#[when("the client calls the server with call_correlated")]
async fn when_client_calls_correlated(world: &mut ClientMessagingWorld) -> TestResult {
    world.call_correlated().await
}

#[when(expr = "the client sends {int} sequential envelopes")]
async fn when_client_sends_multiple(world: &mut ClientMessagingWorld, count: usize) -> TestResult {
    world.send_multiple_envelopes(count).await
}

#[then("the envelope is stamped with an auto-generated correlation ID")]
async fn then_auto_generated_correlation(world: &mut ClientMessagingWorld) -> TestResult {
    world.verify_auto_generated_correlation()
}

#[then(expr = "the returned correlation ID is {int}")]
async fn then_correlation_id_is(world: &mut ClientMessagingWorld, expected: u64) -> TestResult {
    world.verify_correlation_id(expected)
}

#[then("the response has a matching correlation ID")]
async fn then_response_has_matching_correlation(world: &mut ClientMessagingWorld) -> TestResult {
    world.verify_response_correlation_matches()
}

#[then("no correlation mismatch error occurs")]
async fn then_no_mismatch_error(world: &mut ClientMessagingWorld) -> TestResult {
    world.verify_no_mismatch_error()
}

#[then("a CorrelationMismatch error is returned")]
async fn then_mismatch_error(world: &mut ClientMessagingWorld) -> TestResult {
    world.verify_mismatch_error()
}

#[then("each envelope has a unique correlation ID")]
async fn then_unique_correlation_ids(world: &mut ClientMessagingWorld) -> TestResult {
    world.verify_unique_correlation_ids()
}

#[then(expr = "the response contains the same message ID and payload")]
async fn then_response_matches(world: &mut ClientMessagingWorld) -> TestResult {
    // Get the values from the envelope that was sent (now stored in response).
    let response = world.response.as_ref().ok_or("no response")?;
    let message_id = response.id();
    let payload = String::from_utf8_lossy(&response.clone().into_parts().payload()).to_string();
    world.verify_response_matches(message_id, &payload)
}
