//! Scenario tests for pooled slot-rotation behaviours.

use rstest_bdd_macros::scenario;

use crate::fixtures::client_pool_rotation::*;

#[scenario(
    path = "tests/features/client_pool_rotation.feature",
    name = "Client pool rotates slots and resumes a parked acquisition"
)]
fn client_pool_rotates_slots_and_resumes_parked_acquisition(
    client_pool_rotation_world: ClientPoolRotationWorld,
) {
    let _ = client_pool_rotation_world;
}

#[scenario(
    path = "tests/features/client_pool_rotation.feature",
    name = "Client pool skips a leased starting slot when another slot is free"
)]
fn client_pool_skips_leased_starting_slot(client_pool_rotation_world: ClientPoolRotationWorld) {
    let _ = client_pool_rotation_world;
}
