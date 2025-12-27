//! Domain-specific newtype wrappers for Cucumber step parameters.
//!
//! These types eliminate primitive obsession in step definitions by providing
//! semantic meaning to integer parameters parsed from feature files.

use std::str::FromStr;

/// Protocol-specific packet or message identifier for routing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestId(pub u32);

impl FromStr for RequestId {
    type Err = std::num::ParseIntError;

    fn from_str(s: &str) -> Result<Self, Self::Err> { s.parse().map(Self) }
}

/// Correlation identifier tying a request to a logical session or prior exchange.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CorrelationId(pub u64);

impl FromStr for CorrelationId {
    type Err = std::num::ParseIntError;

    fn from_str(s: &str) -> Result<Self, Self::Err> { s.parse().map(Self) }
}

/// Single byte of protocol-defined metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MetadataByte(pub u8);

impl FromStr for MetadataByte {
    type Err = std::num::ParseIntError;

    fn from_str(s: &str) -> Result<Self, Self::Err> { s.parse().map(Self) }
}

/// Expected length of metadata bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MetadataLength(pub usize);

impl FromStr for MetadataLength {
    type Err = std::num::ParseIntError;

    fn from_str(s: &str) -> Result<Self, Self::Err> { s.parse().map(Self) }
}
