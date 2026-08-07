//! Tests for the public default fragment adapter wrapper.

use std::{num::NonZeroUsize, time::Duration};

use crate::fragment::{
    DefaultFragmentAdapter,
    FragmentHeader,
    FragmentIndex,
    FragmentParts,
    Fragmentable,
    FragmentationConfig,
    MessageId,
    encode_fragment_payload,
};

#[derive(Clone, Debug, PartialEq, Eq)]
struct TestPacket {
    id: u32,
    correlation_id: Option<u64>,
    payload: Vec<u8>,
}

impl Fragmentable for TestPacket {
    fn into_fragment_parts(self) -> FragmentParts {
        FragmentParts::new(self.id, self.correlation_id, self.payload)
    }

    fn from_fragment_parts(parts: FragmentParts) -> Self {
        Self {
            id: parts.id(),
            correlation_id: parts.correlation_id(),
            payload: parts.into_payload(),
        }
    }
}

/// Result alias for fallible fragment-adapter test helpers.
type TestResult<T = ()> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

fn adapter_config() -> TestResult<FragmentationConfig> {
    Ok(FragmentationConfig {
        fragment_payload_cap: NonZeroUsize::new(4)
            .ok_or("fragment payload cap must be non-zero")?,
        max_message_size: NonZeroUsize::new(64).ok_or("max message size must be non-zero")?,
        reassembly_timeout: Duration::from_secs(30),
    })
}

fn build_test_packet() -> TestPacket {
    TestPacket {
        id: 42,
        correlation_id: Some(777),
        payload: (0_u8..10).collect(),
    }
}

fn assert_fragment_metadata(fragments: &[TestPacket], packet: &TestPacket) {
    assert!(
        fragments.len() > 1,
        "payload beyond fragment cap should produce multiple fragments"
    );
    for fragment in fragments {
        assert_eq!(fragment.id, packet.id);
        assert_eq!(fragment.correlation_id, packet.correlation_id);
    }
}

fn reassemble_fragment_sequence(
    adapter: &mut DefaultFragmentAdapter,
    fragments: &[TestPacket],
) -> TestResult<Vec<TestPacket>> {
    let first = fragments
        .first()
        .cloned()
        .ok_or("fragment list must contain at least one fragment")?;
    if adapter.reassemble(first.clone())?.is_some() {
        return Err("first fragment should not complete a message".into());
    }
    if adapter.reassemble(first)?.is_some() {
        return Err("duplicate fragment should be suppressed".into());
    }

    let mut reassembled = Vec::new();
    for fragment in fragments.iter().skip(1).cloned() {
        if let Some(packet) = adapter.reassemble(fragment)? {
            reassembled.push(packet);
        }
    }
    Ok(reassembled)
}

#[test]
fn default_fragment_adapter_fragments_and_reassembles_test_packets() {
    let mut adapter = DefaultFragmentAdapter::new(adapter_config().expect("build adapter config"));
    let packet = build_test_packet();

    let fragments = adapter
        .fragment(packet.clone())
        .expect("fragmenting packet should succeed");
    assert_fragment_metadata(&fragments, &packet);

    let reconstructed = reassemble_fragment_sequence(&mut adapter, &fragments)
        .expect("reassemble fragment sequence");
    assert_eq!(reconstructed.len(), 1);
    assert_eq!(
        reconstructed.first(),
        Some(&packet),
        "expected exactly one reconstructed packet matching input"
    );
}

#[test]
fn default_fragment_adapter_passes_through_non_fragment_payloads() {
    let mut adapter = DefaultFragmentAdapter::new(adapter_config().expect("build adapter config"));
    let packet = TestPacket {
        id: 12,
        correlation_id: Some(9),
        payload: b"not encoded as fragment payload".to_vec(),
    };

    let result = adapter
        .reassemble(packet.clone())
        .expect("non-fragment payload should pass through");

    assert_eq!(result, Some(packet));
}

#[test]
fn default_fragment_adapter_exposes_purge_api() {
    let mut config = adapter_config().expect("build adapter config");
    config.reassembly_timeout = Duration::ZERO;
    let mut adapter = DefaultFragmentAdapter::new(config);
    let header = FragmentHeader::new(MessageId::new(81), FragmentIndex::zero(), false);
    let encoded_payload = encode_fragment_payload(header, &[1_u8, 2]).expect("encode fragment");
    let packet = TestPacket {
        id: 42,
        correlation_id: Some(7),
        payload: encoded_payload,
    };

    assert!(
        adapter
            .reassemble(packet)
            .expect("adapter should accept first fragment")
            .is_none()
    );
    assert_eq!(adapter.purge_expired(), vec![MessageId::new(81)]);
}
