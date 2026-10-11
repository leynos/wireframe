//! Workloads and measurement for application connection startup.
//!
//! The prepared-connection path exists so an application's route and middleware
//! chains are evaluated once per server run rather than once per connection.
//! This module measures the two topologies the issue names, and — crucially —
//! reports the **factory invocation and middleware transformation counts** they
//! produce. Those counts are the whole point of the prepared path; a timing
//! difference alone cannot distinguish "prepared once" from "prepared per
//! connection", so the counts are recorded alongside the elapsed time and are
//! asserted exactly by the companion test suite.
//!
//! Route and middleware counts are workload parameters, so a run with a
//! different application shape needs no code change.
//!
//! # Paths
//!
//! [`StartupPath::PerConnectionFactory`] builds and prepares a fresh
//! application for every connection: `connections` factory calls and
//! `connections × routes × middleware_layers` transformations.
//!
//! [`StartupPath::PreparedRoot`] builds and prepares once, then serves every
//! connection from that single prepared root: `1` factory call and
//! `routes × middleware_layers` transformations, however many connections
//! follow.
//!
//! # Transport
//!
//! A connection is a `tokio::io::duplex` pair, not a socket. The issue permits
//! transport I/O only for this scenario, and the duplex pair keeps the measured
//! cost in application preparation and per-connection setup rather than in the
//! kernel loopback path. The client half is dropped before the connection is
//! served, so the server sees a clean end-of-stream and no frame round-trips are
//! measured.
//!
//! # Layout
//!
//! [`workload`] holds the scenario as data: the parameters a baseline must
//! state, and the exact counts each topology must produce. [`measurement`]
//! drives that scenario. Keeping the two apart is what lets the baseline
//! outlive a change of ownership topology — the workload definition is the
//! contract a comparison rests on, and it does not move when the implementation
//! does.

mod measurement;
mod workload;

#[cfg(test)]
mod tests;

pub use measurement::measure_startup;
pub use workload::{
    DEFAULT_CONNECTIONS,
    DEFAULT_MIDDLEWARE_LAYERS,
    DEFAULT_ROUTES,
    StartupMeasurement,
    StartupPath,
    StartupWorkload,
};
