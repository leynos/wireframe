//! Extractor and request context definitions.
//!
//! [`MessageRequest`] carries connection metadata and shared application
//! state. Implement [`FromMessageRequest`] for custom extractors to parse
//! payload bytes or inspect connection info before your handler runs.

mod connection_info;
mod error;
mod message;
mod request;
mod shared_state;
mod streaming;
mod trait_def;

pub use connection_info::ConnectionInfo;
pub use error::ExtractError;
pub use message::Message;
pub use request::{MessageRequest, Payload};
pub use shared_state::SharedState;
pub use streaming::StreamingBody;
pub use trait_def::FromMessageRequest;
