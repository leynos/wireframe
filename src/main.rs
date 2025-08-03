//! Minimal binary demonstrating `wireframe` usage.
//!
//! Currently prints a greeting and exits.

fn main() {
    // Enable structured logging for examples and integration tests.
    // Applications embedding the library should install their own subscriber.
    tracing_subscriber::fmt::init();
    println!("Hello from Wireframe!");
}
