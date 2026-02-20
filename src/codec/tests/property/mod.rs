//! Property-oriented tests for codec round-tripping and malformed inputs.

pub(super) use crate::codec::FrameCodec as FrameCodecForTests;

mod default_codec;
mod mock_codec;
mod mock_stateful_codec;
mod shared;
