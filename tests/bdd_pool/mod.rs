//! rstest-bdd pooled-client behavioural tests.
//!
//! This separate target keeps the optional `pool` feature isolated from the
//! main BDD harness while still exercising `client_pool.feature`.

#![cfg(all(not(loom), feature = "pool"))]

mod fixtures;
mod scenarios;
