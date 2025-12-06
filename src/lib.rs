#![doc(html_root_url = "https://docs.rs/wireframe/latest")]
//! Public API for the `wireframe` library.
//!
//! This crate provides building blocks for asynchronous binary protocol
//! servers, including routing, middleware, and connection utilities.

pub mod app;
pub mod byte_order;
/// Result type alias re-exported for convenience when working with the
/// application builder.
pub use app::error::Result;
pub mod serializer;
pub use serializer::{BincodeSerializer, Serializer};
pub mod connection;
pub mod correlation;
pub mod extractor;
mod fairness;
pub mod fragment;
pub mod frame;
pub mod hooks;
pub mod message;
pub mod metrics;
pub mod middleware;
pub mod panic;
pub mod preamble;
pub mod push;
pub mod response;
pub mod rewind_stream;
#[cfg(not(loom))]
pub mod server;
pub mod session;

pub use connection::ConnectionActor;
pub use correlation::CorrelatableFrame;
pub use fragment::{
    FRAGMENT_MAGIC,
    FragmentBatch,
    FragmentError,
    FragmentFrame,
    FragmentHeader,
    FragmentIndex,
    FragmentSeries,
    FragmentStatus,
    FragmentationConfig,
    FragmentationError,
    Fragmenter,
    MessageId,
    ReassembledMessage,
    Reassembler,
    ReassemblyError,
    decode_fragment_payload,
    encode_fragment_payload,
    fragment_overhead,
};
pub use hooks::{ConnectionContext, ProtocolHooks, WireframeProtocol};
pub use metrics::{CONNECTIONS_ACTIVE, Direction, ERRORS_TOTAL, FRAMES_PROCESSED};
pub use response::{FrameStream, Response, WireframeError};
pub use session::{ConnectionId, SessionRegistry};
