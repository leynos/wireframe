#![doc(html_root_url = "https://docs.rs/wireframe/latest")]
//! Public API for the `wireframe` library.
//!
//! This crate provides building blocks for asynchronous binary protocol
//! servers, including routing, middleware, and connection utilities.

extern crate self as wireframe;

pub mod app;
pub mod app_data_store;
pub mod byte_order;
pub mod codec;
/// Result type alias re-exported for convenience when working with the
/// application builder.
pub use app::error::Result;
pub use app_data_store::AppDataStore;
#[cfg(not(loom))]
pub mod client;
pub mod serializer;
pub use codec::{
    CodecError,
    CodecErrorContext,
    DefaultRecoveryPolicy,
    EofError,
    FrameCodec,
    FramingError,
    LengthDelimitedFrameCodec,
    MAX_FRAME_LENGTH,
    MIN_FRAME_LENGTH,
    ProtocolError,
    RecoveryConfig,
    RecoveryPolicy,
    RecoveryPolicyHook,
};
pub use serializer::{BincodeSerializer, Serializer};
pub mod connection;
pub mod correlation;
#[cfg(not(loom))]
pub mod extractor;
mod fairness;
pub mod fragment;
pub mod frame;
pub mod hooks;
pub mod message;
pub mod message_assembler;
pub mod metrics;
pub mod middleware;
pub mod panic;
pub mod preamble;
pub mod push;
#[cfg(not(loom))]
pub mod request;
pub mod response;
pub mod rewind_stream;
#[cfg(not(loom))]
pub mod server;
pub mod session;
#[cfg(any(test, feature = "test-helpers"))]
pub mod test_helpers;

#[cfg(not(loom))]
pub use client::{
    ClientCodecConfig,
    ClientError,
    ClientProtocolError,
    ClientWireframeError,
    SocketOptions,
    WireframeClient,
};
pub use connection::ConnectionActor;
pub use correlation::CorrelatableFrame;
pub use fragment::{
    DefaultFragmentAdapter,
    FRAGMENT_MAGIC,
    FragmentAdapter,
    FragmentAdapterError,
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
pub use message_assembler::{
    AssembledMessage,
    ContinuationFrameHeader,
    FirstFrameHeader,
    FirstFrameInput,
    FirstFrameInputError,
    FrameHeader,
    FrameSequence,
    MessageAssembler,
    MessageAssemblyError,
    MessageAssemblyState,
    MessageKey,
    MessageSeries,
    MessageSeriesError,
    MessageSeriesStatus,
    ParsedFrameHeader,
};
pub use metrics::{CODEC_ERRORS, CONNECTIONS_ACTIVE, Direction, ERRORS_TOTAL, FRAMES_PROCESSED};
#[cfg(not(loom))]
pub use request::{
    DEFAULT_BODY_CHANNEL_CAPACITY,
    RequestBodyReader,
    RequestBodyStream,
    RequestParts,
    body_channel,
};
pub use response::{FrameStream, Response, WireframeError};
pub use session::{ConnectionId, SessionRegistry};
