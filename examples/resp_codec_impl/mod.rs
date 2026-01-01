//! RESP protocol codec implementation.
//!
//! This module provides a `FrameCodec` implementation for the Redis
//! Serialization Protocol (RESP).

pub mod codec;
pub mod encode;
pub mod frame;
pub mod parse;
