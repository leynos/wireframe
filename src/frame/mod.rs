//! Frame encoding utilities and length-prefixed processors.

pub mod conversion;
pub mod format;
pub mod processor;

pub use conversion::{bytes_to_u64, u64_to_bytes};
pub use format::{Endianness, LengthFormat};
pub use processor::{FrameMetadata, FrameProcessor, LengthPrefixedProcessor};

#[cfg(test)]
mod tests;
