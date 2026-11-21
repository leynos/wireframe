//! Checks on the production hand-off from a serializer buffer to `Bytes`.
//!
//! The hand-off is where a byte recorder and the allocator can disagree: the
//! recorder charges payload copies while the allocator charges memory asked
//! for. These checks measure the same operation with both instruments, and the
//! spare-capacity case proves the zero-allocation reading is not vacuous.

use std::hint::black_box;

use wireframe::{
    app::Envelope,
    benchmark_counters::bytes_from_vec,
    serializer::{BincodeSerializer, Serializer},
};
use wireframe_testing::{
    benchmark_alloc_support::AllocationTraffic,
    codec_benchmarks::{LARGE_PAYLOAD_BYTES, PayloadClass, payload_for_class},
};

use crate::support::{WINDOW_SLACK_BYTES, outbound_traffic, traffic};

/// The production outbound operation allocates one payload, and no more.
///
/// This is the independent oracle the byte accounting is checked against: the
/// allocator and the copied-byte recorder must agree that one outbound message
/// materializes exactly once. A recorder that fires without a materialization,
/// or a materialization the recorder never sees, breaks the agreement at one
/// end or the other. The bound is stated in payloads (`P`) rather than in
/// absolute bytes, because the encode result's framing suffix is tiny.
pub(super) fn production_outbound_allocates_one_payload() {
    for payload_class in [PayloadClass::Small, PayloadClass::Large] {
        let payload_bytes = payload_class.len();
        let traffic = outbound_traffic(payload_class);

        assert!(
            traffic.bytes >= payload_bytes,
            "the production outbound path allocated only {} bytes for the {payload_class:?} \
             payload of {payload_bytes} bytes; the serialized buffer was never materialized",
            traffic.bytes
        );
        assert!(
            traffic.bytes < 2 * payload_bytes,
            "the production outbound path allocated {} bytes for a {payload_bytes}-byte payload; \
             two payload-sized allocations mean the hand-off stopped sharing the serializer's \
             buffer",
            traffic.bytes
        );
    }
}

/// Spare capacity added to the in-place control vector.
///
/// Any spare capacity at all stops `bytes` taking the exact-capacity path, so a
/// small margin suffices; it only has to be excluded from the payload bound.
const SPARE_HANDOFF_BYTES: usize = 256;

/// An exact-capacity hand-off takes the payload in place and allocates nothing.
///
/// `bytes` materializes a boxed slice without reallocating when the vector's
/// length equals its capacity, so a `Bytes` built from the serializer's own
/// buffer must cost no allocator traffic at all.
pub(super) fn exact_capacity_handoff_allocates_nothing() {
    let serialized = match BincodeSerializer.serialize(&Envelope::new(
        1,
        Some(7),
        payload_for_class(PayloadClass::Large).to_vec(),
    )) {
        Ok(serialized) => serialized,
        Err(err) => panic!("the default serializer must encode its own envelope: {err}"),
    };
    assert_eq!(
        serialized.len(),
        serialized.capacity(),
        "the default serializer must return an exact-capacity buffer; a spare-capacity buffer \
         would take the control-block path and make this check vacuous"
    );

    let traffic = traffic("exact-capacity hand-off", move || {
        let bytes = bytes_from_vec(serialized);
        let _len = black_box(bytes.len());
        Ok(())
    });

    assert_eq!(
        traffic,
        AllocationTraffic::default(),
        "an exact-capacity vector must hand off with no allocator traffic; the window reported \
         {traffic:?}"
    );
}

/// A spare-capacity hand-off allocates one control block, and keeps the payload.
///
/// This is the control that proves the exact-capacity check is not vacuous: the
/// same seam over the same payload must show allocator traffic when the vector
/// has spare capacity, while the payload bytes themselves stay in place. The
/// traffic is one control block, far below the payload, which is why the
/// hand-off can still be called zero-copy.
pub(super) fn spare_capacity_handoff_allocates_a_control_block() {
    let mut buffer = Vec::with_capacity(LARGE_PAYLOAD_BYTES + SPARE_HANDOFF_BYTES);
    buffer.extend_from_slice(&payload_for_class(PayloadClass::Large));
    let source = buffer.as_ptr() as usize;
    let mut handed_off = 0_usize;
    let traffic = traffic("spare-capacity hand-off", || {
        let bytes = bytes_from_vec(buffer);
        handed_off = bytes.as_ptr() as usize;
        let _len = black_box(bytes.len());
        Ok(())
    });

    assert_eq!(
        handed_off, source,
        "a spare-capacity hand-off must keep the payload bytes in place: {source} -> {handed_off}"
    );
    assert_eq!(
        traffic.operations, 1,
        "a spare-capacity hand-off must allocate exactly one control block; it recorded {} \
         operations",
        traffic.operations
    );
    assert!(
        traffic.bytes > 0 && traffic.bytes <= WINDOW_SLACK_BYTES,
        "a spare-capacity hand-off allocated {} bytes for a {LARGE_PAYLOAD_BYTES}-byte payload; a \
         control block is tens of bytes, so a payload-sized figure means the bytes were copied",
        traffic.bytes
    );
}
