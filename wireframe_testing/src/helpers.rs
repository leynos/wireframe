//! Helper utilities for driving `WireframeApp` instances in tests.
//!
//! These functions spin up an application on an in-memory duplex stream and
//! collect the bytes written back by the app for assertions.

use wireframe::{
    app::Envelope,
    frame::FrameMetadata,
    serializer::{MessageCompatibilitySerializer, Serializer},
};

mod codec;
mod codec_drive;
mod codec_ext;
mod drive;
mod payloads;
mod runtime;

#[cfg(test)]
mod tests;

/// Serializer bounds expected by the in-memory test harness.
pub trait TestSerializer:
    Serializer
    + MessageCompatibilitySerializer
    + FrameMetadata<Frame = Envelope>
    + Send
    + Sync
    + 'static
{
}

impl<T> TestSerializer for T where
    T: Serializer
        + MessageCompatibilitySerializer
        + FrameMetadata<Frame = Envelope>
        + Send
        + Sync
        + 'static
{
}

pub(crate) const DEFAULT_CAPACITY: usize = 4096;
pub(crate) const MAX_CAPACITY: usize = 1024 * 1024 * 10; // 10MB limit
pub(crate) const EMPTY_SERVER_CAPACITY: usize = 64;
/// Shared frame cap used by helpers and tests to avoid drift.
pub const TEST_MAX_FRAME: usize = DEFAULT_CAPACITY;

pub use codec::{decode_frames, decode_frames_with_max, encode_frame, new_test_codec};
pub use codec_drive::{
    drive_with_codec_frames,
    drive_with_codec_frames_with_capacity,
    drive_with_codec_payloads,
    drive_with_codec_payloads_mut,
    drive_with_codec_payloads_with_capacity,
    drive_with_codec_payloads_with_capacity_mut,
};
pub use codec_ext::{decode_frames_with_codec, encode_payloads_with_codec, extract_payloads};
pub use drive::{
    drive_with_frame,
    drive_with_frame_mut,
    drive_with_frame_with_capacity,
    drive_with_frame_with_capacity_mut,
    drive_with_frames,
    drive_with_frames_mut,
    drive_with_frames_with_capacity,
    drive_with_frames_with_capacity_mut,
};
pub use payloads::{drive_with_bincode, drive_with_payloads, drive_with_payloads_mut};
pub use runtime::{run_app, run_with_duplex_server};
