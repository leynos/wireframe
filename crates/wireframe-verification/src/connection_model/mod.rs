//! Placeholder Stateright model for the connection actor roadmap.
//!
//! The first milestone keeps the model intentionally small: it exercises the
//! shared harness and encodes a few semantic invariants that later roadmap
//! items can refine toward the real `ConnectionActor`.

mod action;
mod model;
mod properties;
mod state;

pub use action::ConnectionAction;
pub use model::PlaceholderConnectionModel;
pub use state::{ActiveOutput, ConnectionState};
