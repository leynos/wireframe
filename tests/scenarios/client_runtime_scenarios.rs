//! Scenario tests for wireframe client runtime behaviours.

use rstest_bdd_macros::scenario;

use crate::fixtures::client_runtime::*;

#[scenario(
    path = "tests/features/client_runtime.feature",
    name = "Client sends and receives with configured frame length"
)]
fn client_runtime_send_receive(client_runtime_world: ClientRuntimeWorld) {
    let _ = client_runtime_world;
}

#[scenario(
    path = "tests/features/client_runtime.feature",
    name = "Client reports errors when server frame limit is exceeded"
)]
fn client_runtime_oversize_error(client_runtime_world: ClientRuntimeWorld) {
    let _ = client_runtime_world;
}

#[scenario(
    path = "tests/features/client_runtime.feature",
    name = "Client maps malformed responses to decode protocol errors"
)]
fn client_runtime_decode_error_mapping(client_runtime_world: ClientRuntimeWorld) {
    let _ = client_runtime_world;
}
