//! A v0.6 scenario accepts an owned fallible fixture and an explicit unit result.

use rstest::fixture;
use rstest_bdd_macros::{given, scenario, then};

struct World {
    value: u32,
}

/// Provide an owned result whose inner world is injected into the steps.
#[fixture]
fn world() -> Result<World, String> {
    Ok(World { value: 1 })
}

/// Mutate the unwrapped fixture rather than a borrowed result wrapper.
#[given("a world from an owned fallible fixture")]
fn given_world(world: &mut World) {
    world.value += 1;
}

/// Check that subsequent steps receive the same mutable world.
#[then("the unwrapped world contains two")]
fn then_world(world: &World) {
    assert_eq!(world.value, 2);
}

/// Accept the explicit fallible parameter and unit-result scenario contract.
/// Retain the wrapper because trybuild builds a binary rather than a test harness.
#[scenario(
    path = "rstest_bdd_v06/owned_fallible.feature",
    name = "Owned fallible fixture is accepted"
)]
#[test_attr(inline)]
fn owned_fallible_fixture(world: Result<World, String>) -> Result<(), String> {
    Ok(())
}

/// Execute the generated wrapper and propagate fixture or step failures.
fn main() -> Result<(), String> {
    owned_fallible_fixture()
}
