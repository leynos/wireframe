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

pub mod helpers;
pub mod logging;

pub use helpers::{
    TEST_MAX_FRAME,
    TestSerializer,
    decode_frames,
    decode_frames_with_max,
    drive_with_bincode,
    drive_with_frame,
    drive_with_frame_mut,
    drive_with_frame_with_capacity,
    drive_with_frames,
    drive_with_frames_mut,
    drive_with_frames_with_capacity,
    new_test_codec,
    run_app,
    run_with_duplex_server,
};
pub use logging::{LoggerHandle, logger};
