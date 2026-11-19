//! Shared integration-test support modules.
//!
//! [`prepared_app`] supplies reusable fixtures for the prepared-application
//! integration tests, and [`tagging`] provides the middleware double that tags
//! the frames passing through it and counts the transforms performed during
//! preparation.

pub mod prepared_app;
pub mod tagging;
