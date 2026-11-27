//! Parameter wrappers for the message-assembly step definitions.
//!
//! `rstest-bdd` parses each step-capture into a named type through `FromStr`,
//! so the captured text keeps its meaning instead of decaying into a bare
//! integer at the step boundary.

use std::str::FromStr;

use wireframe::message_assembler::{FrameSequence, MessageKey};

/// Wrapper for message key parameters in BDD steps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MessageKeyParam(pub u64);

impl FromStr for MessageKeyParam {
    type Err = std::num::ParseIntError;

    fn from_str(s: &str) -> Result<Self, Self::Err> { s.parse::<u64>().map(MessageKeyParam) }
}

impl MessageKeyParam {
    pub fn to_key(self) -> MessageKey { MessageKey(self.0) }
}

/// Wrapper for sequence number parameters in BDD steps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SequenceParam(pub u32);

impl FromStr for SequenceParam {
    type Err = std::num::ParseIntError;

    fn from_str(s: &str) -> Result<Self, Self::Err> { s.parse::<u32>().map(SequenceParam) }
}

impl SequenceParam {
    pub fn to_seq(self) -> FrameSequence { FrameSequence(self.0) }
}

/// Wrapper for count/size parameters in BDD steps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CountParam(pub usize);

impl FromStr for CountParam {
    type Err = std::num::ParseIntError;

    fn from_str(s: &str) -> Result<Self, Self::Err> { s.parse::<usize>().map(CountParam) }
}

/// Wrapper for timeout duration parameters in BDD steps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeoutParam(pub u64);

impl FromStr for TimeoutParam {
    type Err = std::num::ParseIntError;

    fn from_str(s: &str) -> Result<Self, Self::Err> { s.parse::<u64>().map(TimeoutParam) }
}
