//! Step definitions for panic resilience behavioural tests.
//!
//! Steps are synchronous and use the scenario world's shared Tokio runtime to
//! keep its server task alive across step boundaries.

use rstest_bdd_macros::{given, then, when};

use crate::fixtures::panic::{PanicWorld, TestResult};

#[given("a running wireframe server with a panic in connection setup")]
fn start_server(panic_world: &mut PanicWorld) -> TestResult {
    let runtime = panic_world.handle();
    runtime.block_on(panic_world.start_panic_server())
}

#[when("I connect to the server")]
#[when("I connect to the server again")]
fn connect(panic_world: &mut PanicWorld) -> TestResult {
    let runtime = panic_world.handle();
    runtime.block_on(panic_world.connect_once())
}

#[then("both connections succeed")]
fn verify(panic_world: &mut PanicWorld) -> TestResult {
    let runtime = panic_world.handle();
    runtime.block_on(panic_world.verify_and_shutdown())
}
