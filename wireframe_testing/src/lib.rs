//! Utilities for driving a [`WireframeApp`](wireframe::app::WireframeApp)
//! with in-memory streams during tests.
//!
//! These helpers spawn the application on a `tokio::io::duplex` stream and
//! return all bytes written by the app for easy assertions.
//!
//! ```rust
//! use wireframe::app::WireframeApp;
//! use wireframe_testing::drive_with_bincode;
//!
//! # async fn example(app: WireframeApp<_, _, ()>) {
//! let bytes = drive_with_bincode(app, 42u8).await.unwrap();
//! # }
//! ```

pub mod helpers;

pub use helpers::{
    TestSerializer,
    drive_with_bincode,
    drive_with_frame,
    drive_with_frame_mut,
    drive_with_frame_with_capacity,
    drive_with_frames,
    drive_with_frames_mut,
    drive_with_frames_with_capacity,
};
