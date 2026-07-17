//! Tests for serializer configuration accessors.
//!
//! The `should_deserialize_after_parse` override on [`BincodeSerializer`] is
//! consumed by the inbound handler but never asserted, so a mutant forcing it
//! to the trait default (`true`) survives without a direct check.
#![cfg(not(loom))]

use wireframe::serializer::{BincodeSerializer, Serializer};

#[test]
fn bincode_serializer_does_not_deserialize_after_parse() {
    assert!(!BincodeSerializer.should_deserialize_after_parse());
}
