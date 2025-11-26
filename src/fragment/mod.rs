//! Fragment metadata primitives for transparent message splitting.
//!
//! This module collects the domain types used by the fragmentation and
//! re-assembly layer. Each sub-module focuses on a single concept to keep the
//! code small and easy to audit while still providing a cohesive API at the
//! crate root.

pub mod error;
pub mod fragmenter;
pub mod header;
pub mod id;
pub mod index;
pub mod reassembler;
pub mod series;

pub use error::{FragmentError, FragmentStatus, FragmentationError, ReassemblyError};
pub use fragmenter::{FragmentBatch, FragmentFrame, Fragmenter};
pub use header::FragmentHeader;
pub use id::MessageId;
pub use index::FragmentIndex;
pub use reassembler::{ReassembledMessage, Reassembler};
pub use series::FragmentSeries;

#[cfg(test)]
mod tests;
