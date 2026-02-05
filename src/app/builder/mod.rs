//! Application builder configuring routes and middleware.
//! [`WireframeApp`] is an Actix-inspired builder for managing connection
//! state, routing, and middleware in a `WireframeServer`. It exposes
//! convenience methods to register handlers and lifecycle hooks, and
//! serializes messages using a configurable serializer.

mod codec;
mod config;
mod core;
mod lifecycle;
mod protocol;
mod routing;
mod state;

pub use core::WireframeApp;
