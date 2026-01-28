//! Scenario tests for wireframe client preamble behaviours.

use rstest_bdd_macros::scenario;

use crate::fixtures::client_preamble::*;

#[scenario(
    path = "tests/features/client_preamble.feature",
    name = "Client sends preamble and server acknowledges"
)]
fn client_preamble_send_and_ack(client_preamble_world: ClientPreambleWorld) {
    let _ = client_preamble_world;
}

#[scenario(
    path = "tests/features/client_preamble.feature",
    name = "Client receives server acknowledgement in success callback"
)]
fn client_preamble_receives_ack(client_preamble_world: ClientPreambleWorld) {
    let _ = client_preamble_world;
}

#[scenario(
    path = "tests/features/client_preamble.feature",
    name = "Client preamble timeout triggers failure callback"
)]
fn client_preamble_timeout_failure(client_preamble_world: ClientPreambleWorld) {
    let _ = client_preamble_world;
}

#[scenario(
    path = "tests/features/client_preamble.feature",
    name = "Client without preamble connects normally"
)]
fn client_preamble_no_preamble(client_preamble_world: ClientPreambleWorld) {
    let _ = client_preamble_world;
}
