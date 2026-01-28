//! Domain-specific newtypes for message assembler test parameters.

use std::{num::ParseIntError, str::FromStr};

/// Message key for frame headers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MessageKey(pub u64);

impl FromStr for MessageKey {
    type Err = ParseIntError;
    fn from_str(s: &str) -> Result<Self, Self::Err> { s.parse().map(Self) }
}

/// Sequence number for continuation frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SequenceNumber(pub u32);

impl FromStr for SequenceNumber {
    type Err = ParseIntError;
    fn from_str(s: &str) -> Result<Self, Self::Err> { s.parse().map(Self) }
}

/// Body length in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BodyLength(pub usize);

impl FromStr for BodyLength {
    type Err = ParseIntError;
    fn from_str(s: &str) -> Result<Self, Self::Err> { s.parse().map(Self) }
}

/// Metadata length in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MetadataLength(pub usize);

impl FromStr for MetadataLength {
    type Err = ParseIntError;
    fn from_str(s: &str) -> Result<Self, Self::Err> { s.parse().map(Self) }
}

/// Header length in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeaderLength(pub usize);

impl FromStr for HeaderLength {
    type Err = ParseIntError;
    fn from_str(s: &str) -> Result<Self, Self::Err> { s.parse().map(Self) }
}
