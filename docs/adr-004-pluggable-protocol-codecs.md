# Architectural decision record (ADR) 004: pluggable protocol codecs

## Status

Accepted - 2025-12-30: introduce a `FrameCodec` trait with a default
length-delimited implementation.

## Date

2025-12-30.

## Context and Problem Statement

Wireframe currently hardcodes `tokio_util::codec::LengthDelimitedCodec` with a
4-byte big-endian length prefix in the connection pipeline (notably in
`src/app/inbound_handler.rs` and `src/app/frame_handling.rs`). This makes
framing inflexible and prevents protocols with alternative framing rules (such
as Hotline, MySQL, and Redis Serialization Protocol (RESP)) from reusing
Wireframe's routing, middleware, and serialization infrastructure.

A pluggable framing layer is required that can:

- Decode frames with protocol-specific headers.
- Preserve protocol metadata (transaction IDs, sequence numbers, etc.).
- Provide correlation identifiers when available.
- Remain backward compatible for existing users.

## Decision Drivers

- Support non-length-delimited protocols (Hotline, MySQL, Redis Serialization
  Protocol (RESP)).
- Preserve protocol-specific metadata by using protocol-native frame types.
- Maintain backward compatibility for current Wireframe users.
- Keep framing logic encapsulated and testable.
- Provide guardrails for maximum frame size to reduce denial-of-service risk.
- Avoid runtime indirection where zero-cost abstractions are feasible.

## Requirements

### Functional requirements

- Allow multiple wire protocols to plug in their own framing rules.
- Preserve protocol metadata in the frame type.
- Support protocols with and without request/response correlation.
- Continue to support the current length-delimited framing by default.

### Technical requirements

- Use Tokio codecs for integration with `Framed` streams.
- Keep framing configuration owned by the codec, not `WireframeApp`.
- Avoid allocations or copies beyond what framing requires.
- Remain compatible with the current minimum supported Rust version.

## Options Considered

### Option A: retain fixed `LengthDelimitedCodec`

Keep the current hardcoded 4-byte big-endian length-delimited framing.

### Option B: introduce a `FrameCodec` trait with a default length-delimited

implementation (preferred)

Define a new trait that supplies a decoder, encoder, payload extraction, frame
wrapping, correlation lookup, and maximum frame length. Provide a default
`LengthDelimitedFrameCodec` for backward compatibility.

### Option C: make `WireframeApp` generic over raw `Decoder`/`Encoder` types or

use trait objects

Expose `tokio_util::codec::Decoder` and `Encoder` directly as type parameters
or boxed trait objects and have `WireframeApp` manage them.

| Topic                  | Option A: fixed codec | Option B: FrameCodec trait | Option C: raw decoder/encoder |
| ---------------------- | --------------------- | -------------------------- | ----------------------------- |
| Protocol support       | None                  | Broad                      | Broad                         |
| Type safety            | Strong                | Strong                     | Varies (trait objects)        |
| Ergonomics             | Simple                | Simple defaults            | Complex API surface           |
| Backward compatibility | Full                  | Full with defaults         | Medium (API changes)          |
| Performance            | Stable                | Stable (associated types)  | Depends on implementation     |

_Table 1: Comparison of framing options._

## Decision Outcome / Proposed Direction

Adopt Option B: introduce a `FrameCodec` trait with a default length-delimited
implementation, and make `WireframeApp` generic over a codec type parameter.

Key elements:

- `FrameCodec` owns the decoder/encoder configuration, payload extraction, and
  maximum frame length.
- Frame types are protocol-specific to preserve metadata.
- The default `LengthDelimitedFrameCodec` uses `Bytes` frames to align with
  `tokio_util::codec::LengthDelimitedCodec` and avoid extra copies.
- A default `LengthDelimitedFrameCodec` provides compatibility for existing
  users without code changes.
- `WireframeApp` becomes generic over `F: FrameCodec` with a default type
  parameter, and the builder gains `.with_codec()` to swap codecs.

For orientation, the trait shape is expected to look like this:

```rust,no_run
use std::io;
use bytes::Bytes;
use tokio_util::codec::{Decoder, Encoder};

/// Trait for pluggable frame codecs supporting different wire protocols.
pub trait FrameCodec: Send + Sync + Clone + 'static {
    /// Frame type produced by decoding.
    type Frame: Send + Sync + 'static;
    /// Decoder type for this codec.
    type Decoder: Decoder<Item = Self::Frame, Error = io::Error> + Send;
    /// Encoder type for this codec.
    type Encoder: Encoder<Self::Frame, Error = io::Error> + Send;

    /// Create a Tokio Decoder for this codec.
    fn decoder(&self) -> Self::Decoder;

    /// Create a Tokio Encoder for this codec.
    fn encoder(&self) -> Self::Encoder;

    /// Extract payload bytes from a frame.
    fn frame_payload(frame: &Self::Frame) -> &[u8];

    /// Wrap payload bytes into a frame for sending.
    fn wrap_payload(&self, payload: Bytes) -> Self::Frame;

    /// Extract correlation ID for request/response matching.
    fn correlation_id(_frame: &Self::Frame) -> Option<u64> { None }

    /// Maximum frame length this codec will accept.
    fn max_frame_length(&self) -> usize;
}
```

## Goals and Non-Goals

### Goals

- Allow Wireframe to support Hotline, MySQL, and Redis Serialization Protocol
  (RESP) framing.
- Preserve protocol metadata in frame types.
- Maintain the existing API surface for users who rely on length-delimited
  framing.
- Keep framing logic encapsulated and testable.

### Non-Goals

- Protocol negotiation or runtime codec selection.
- Replacing the existing message serialization format.
- Introducing asynchronous codec initialization.

## Migration Plan

### Phase 1: Trait definition (non-breaking)

- Create `src/codec.rs` with the `FrameCodec` trait and
  `LengthDelimitedFrameCodec` default.
- Add unit tests covering the default codec behaviour.

### Phase 2: Parameterize `WireframeApp` (breaking change)

- Introduce `F: FrameCodec = LengthDelimitedFrameCodec` to `WireframeApp`.
- Add a `codec: F` field and update the default implementation.
- Provide `.with_codec()` for builder ergonomics.

### Phase 3: Update connection handling

- Parameterize `FrameHandlingContext` and `ResponseContext` over the codec.
- Replace `LengthDelimitedCodec` usage with `FrameCodec` decoder/encoder calls.
- Use `max_frame_length()` for buffer sizing and explicit fragmentation
  configuration helpers (`enable_fragmentation`).

### Phase 4: Update `WireframeServer`

- Propagate the codec type parameter through server factory types.
- Update any connection handling signatures that assume the fixed codec.

### Phase 5: Protocol examples

- Add an example codec for Hotline.
- Add an example codec for MySQL if time permits.

## Implementation Findings

### Codec ergonomics and stateful metadata

- `FrameCodec::wrap_payload` is instance-aware and accepts `Bytes`, so codecs
  can advance sequence counters or stamp metadata without bypassing the normal
  send path.
- Wireframe clones the codec per connection and uses that instance for
  outbound wrapping, which keeps state deterministic per connection when
  `Clone` produces independent state.

### Error handling and recovery

- The codec surface uses `io::Error`, so protocol-specific framing errors are
  collapsed into generic IO failures. Decode errors currently close the
  connection, leaving no path to recover from malformed frames or to emit a
  protocol-specific error response.

### Connection pipeline alignment

- Codec integration lives in the app connection path, while the `Connection`
  actor retains its own framing logic. This makes protocol hooks and streaming
  behaviour inconsistent across read/write paths.

### Fragmentation alignment

- Fragmentation defaults are tied to `max_frame_length`, but fragmentation is
  still integrated directly into the connection pipeline rather than being a
  pluggable `FragmentAdapter` as described in the fragmentation design.

### Mock protocol handler experience (Hotline, MySQL)

- Both example codecs required manual header parsing and validation, including
  explicit checks for payload length, total frame size, and header layout.
- Implementing MySQL’s 3-byte little-endian length header required custom
  helpers; there is no shared utility for non-standard integer widths.
- Clippy’s indexing and truncation lints forced the examples to use
  `bytes::Buf` and explicit `try_from` conversions, suggesting a need for
  shared parsing helpers to reduce boilerplate.

## Implementation guidance

- Add codec-specific tests that exercise encoder/decoder round-trips, payload
  extraction, correlation identifiers, and maximum frame length enforcement.
- Use shared example codecs in `wireframe::codec::examples` to drive regression
  and property-based tests without duplicating framing logic.
- Prefer `wireframe_testing` helpers that work with custom codecs, so test
  harnesses do not assume length-delimited framing.
- Validate observability signals (logs, metrics, and protocol hooks) for codec
  failures and recovery policies using the test observability harness once
  available.[^adr-006]

## Known Risks and Limitations

- Associated types avoid RPITIT for `FrameCodec`, but the overall design still
  depends on Rust 1.75+ for return-position `impl Trait` elsewhere in the
  connection stack.
- The new codec type parameter propagates through public APIs, increasing type
  signatures and potentially affecting downstream type inference.
- `LengthDelimitedFrameCodec` now returns `Bytes`, so any code requiring owned
  `Vec<u8>` payloads must convert explicitly, reintroducing copies.

## Outstanding Decisions

- Decide how to realign fragmentation with the `FragmentAdapter` design so
  opt-in behaviour and composition order are explicit.
- Decide whether to unify codec handling between the app router path and the
  `Connection` actor to ensure protocol hooks run consistently.
- Future protocol adoption may still require revisiting how correlation
  identifiers are surfaced for FIFO protocols (for example, RESP).

## Resolved Decisions

### CodecError taxonomy and recovery policies (resolved 2026-01-06)

A structured `CodecError` taxonomy has been implemented with the following
design:

- **Three-tier error classification plus EOF**: `FramingError` for wire-level
  issues, `ProtocolError` for semantic violations, `io::Error` for transport
  failures (wrapped), and `EofError` with clean/mid-frame/mid-header variants.
- **Recovery policies**: Each error type has a default policy (`Drop`,
  `Quarantine`, or `Disconnect`). Custom policies can be installed via the
  `RecoveryPolicyHook` trait.
- **Backward compatibility**: `From<CodecError> for io::Error` preserves
  compatibility with the `tokio_util::codec` trait bounds.
- **Protocol hooks**: `WireframeProtocol::on_eof` provides a callback for EOF
  conditions during frame decoding.
- **Observability**: The `wireframe_codec_errors_total` metric tracks codec
  errors by type and recovery policy when the `metrics` feature is enabled.

See `src/codec/error.rs` and `src/codec/recovery.rs` for implementation details.

### Zero-copy payload extraction (resolved 2026-01-19)

A `frame_payload_bytes` method was added to `FrameCodec` to enable zero-copy
payload extraction:

- **New method**: `fn frame_payload_bytes(frame: &Self::Frame) -> Bytes`
- **Default behaviour**: Copies from `frame_payload()` for backward
  compatibility
- **Optimized implementations**: Return `frame.payload.clone()` for
  `Bytes`-backed frames (cheap atomic reference count increment)

Guidelines for custom codecs:

1. Use `Bytes` instead of `Vec<u8>` for payload storage in frame types
2. Use `BytesMut::freeze()` in decoders instead of `.to_vec()`
3. Override `frame_payload_bytes` to return `frame.payload.clone()`
4. In `wrap_payload`, store the `Bytes` directly without conversion

Verification via pointer equality:

```rust
let extracted = MyCodec::frame_payload_bytes(&frame);
assert_eq!(frame.payload.as_ptr(), extracted.as_ptr());
```

See `src/codec/tests.rs` for zero-copy regression tests covering
`LengthDelimitedFrameCodec`, `HotlineFrameCodec`, and `MysqlFrameCodec`.

### Property-based codec round-trip hardening (resolved 2026-02-19)

Roadmap item 9.4.1 is now covered by deterministically generated tests for both
the default codec and a mock protocol codec:

- `LengthDelimitedFrameCodec` now has generated round-trip checks over boundary
  payload sizes and generated malformed-frame checks (partial headers,
  truncated payloads, and oversized declared lengths).
- A mock stateful protocol codec now has generated sequence checks that verify
  per-connection reset semantics and stateful encoder/decoder ordering rules.
- Behavioural coverage mirrors these guarantees through
  `tests/features/codec_property_roundtrip.feature` and associated rstest-bdd
  fixtures and steps.

This hardening keeps the public API unchanged while increasing confidence in
codec recovery behaviour and state-machine consistency.

## Architectural Rationale

A dedicated `FrameCodec` abstraction aligns framing with the protocol boundary
and keeps the routing/middleware pipeline agnostic to wire-level details. It
preserves existing behaviour through a default codec while enabling protocol
specificity and reusability. By tying buffer sizing and maximum frame length to
codec configuration, the design keeps transport-level constraints close to the
framing rules that define them.

[^adr-006]: See [adr-006-test-observability.md](adr-006-test-observability.md).
