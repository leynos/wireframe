//! Cucumber step implementations for panic resilience testing.
//!
//! Defines Given-When-Then steps that verify server stability
//! when connection tasks panic during setup.

use cucumber::{given, then, when};

use crate::world::PanicWorld;

#[given("a running wireframe server with a panic in connection setup")]
async fn start_server(world: &mut PanicWorld) {
    world.start_panic_server();
    std::future::ready(()).await;
}

#[when("I connect to the server")]
#[when("I connect to the server again")]
async fn connect(world: &mut PanicWorld) { world.connect_once().await; }

#[then("both connections succeed")]
async fn verify(world: &mut PanicWorld) { world.verify_and_shutdown().await; }
