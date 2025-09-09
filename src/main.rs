//! Minimal binary demonstrating `wireframe` usage.
//!
//! Parses CLI arguments and prints a greeting.

mod cli;

use clap::Parser;

fn main() {
    // Enable structured logging for examples and integration tests.
    // Applications embedding the library should install their own subscriber.
    tracing_subscriber::fmt::init();

    let cli = cli::Cli::parse();
    if let Some(name) = cli.name {
        println!("Hello, {name} from Wireframe!");
    } else {
        println!("Hello from Wireframe!");
    }
}
