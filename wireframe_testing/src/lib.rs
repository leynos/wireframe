//! Utilities for driving a [`WireframeApp`](wireframe::app::WireframeApp)
//! with in-memory streams during tests.
//!
//! These helpers spawn the application on a `tokio::io::duplex` stream and
//! return all bytes written by the app for easy assertions. They work with any
//! message implementing [`serde::Serialize`]. The payload is encoded with
//! [`bincode::encode_to_vec`] using [`bincode::config::standard()`], which means
//! little-endian byte order, variable-length integer encoding and no byte limit
//! are applied. The example uses a simple `u8` value so no generics are
//! required.
//!
//! ```rust
//! use wireframe::app::WireframeApp;
//! use wireframe_testing::drive_with_bincode;
//!
//! # async fn example(app: WireframeApp) {
//! let bytes = drive_with_bincode(app, 42u8).await.unwrap();
//! # }
//! ```

pub mod echo_server;
pub mod helpers;
pub mod integration_helpers;
pub mod logging;
pub mod macros;
pub mod multi_packet;

pub use echo_server::{ServerMode, process_frame};
pub use helpers::{
    MaxFrameLength,
    PayloadLength,
    TEST_MAX_FRAME,
    TestSerializer,
    TransactionId,
    correlated_hotline_wire,
    decode_frames,
    decode_frames_with_codec,
    decode_frames_with_max,
    drive_with_bincode,
    drive_with_codec_frames,
    drive_with_codec_frames_with_capacity,
    drive_with_codec_payloads,
    drive_with_codec_payloads_mut,
    drive_with_codec_payloads_with_capacity,
    drive_with_codec_payloads_with_capacity_mut,
    drive_with_frame,
    drive_with_frame_mut,
    drive_with_frame_with_capacity,
    drive_with_frames,
    drive_with_frames_mut,
    drive_with_frames_with_capacity,
    drive_with_payloads,
    drive_with_payloads_mut,
    encode_frame,
    encode_payloads_with_codec,
    extract_payloads,
    mismatched_total_size_wire,
    new_test_codec,
    oversized_hotline_wire,
    run_app,
    run_with_duplex_server,
    sequential_hotline_wire,
    truncated_hotline_header,
    truncated_hotline_payload,
    valid_hotline_frame,
    valid_hotline_wire,
};
pub use integration_helpers::{
    CommonTestEnvelope,
    TestApp,
    TestError,
    TestResult,
    factory,
    unused_listener,
};
pub use logging::{LoggerHandle, logger};
#[doc(inline)]
pub use multi_packet::collect_multi_packet;
