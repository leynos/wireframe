//! Frame encoding utilities and length-prefix helpers built around
//! `tokio_util::codec::LengthDelimitedCodec` (4-byte big-endian by default).

pub mod conversion;
pub mod format;
pub mod metadata;

pub use conversion::{bytes_to_u64, u64_to_bytes};
pub use format::{Endianness, LengthFormat};
pub use metadata::FrameMetadata;

#[cfg(test)]
mod tests;
