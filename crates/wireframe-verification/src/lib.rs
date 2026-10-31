//! Internal formal-verification models and harnesses for Wireframe.
//!
//! This crate stays out of the published API surface while providing a place
//! for Stateright-based models and reusable bounded-checker helpers.

#![forbid(unsafe_code)]

pub mod connection_model;
pub mod harness;
