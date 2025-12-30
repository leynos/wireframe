# Architectural decision record (ADR) 004: pluggable protocol codecs

## Status

Accepted - 2025-12-30: introduce a `FrameCodec` trait with a default
length-delimited implementation.

## Date

2025-12-30.

## Context and Problem Statement

Wireframe currently hardcodes `tokio_util::codec::LengthDelimitedCodec` with a
4-byte big-endian length prefix in the connection pipeline (notably in
`src/app/connection.rs` and `src/app/frame_handling.rs`). This makes framing
inflexible and prevents protocols with alternative framing rules (such as
Hotline, MySQL, and Redis RESP) from reusing Wireframe's routing, middleware,
and serialisation infrastructure.

We need a pluggable framing layer that can:

- Decode frames with protocol-specific headers.
- Preserve protocol metadata (transaction IDs, sequence numbers, etc.).
- Provide correlation identifiers when available.
- Remain backward compatible for existing users.

## Decision Drivers

- Support non-length-delimited protocols (Hotline, MySQL, Redis RESP).
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
| Performance            | Stable                | Stable (RPITIT)            | Depends on implementation     |

_Table 1: Comparison of framing options._

## Decision Outcome / Proposed Direction

Adopt Option B: introduce a `FrameCodec` trait with a default length-delimited
implementation and make `WireframeApp` generic over a codec type parameter.

Key elements:

- `FrameCodec` owns the decoder/encoder configuration, payload extraction, and
  maximum frame length.
- Frame types are protocol-specific to preserve metadata.
- The default `LengthDelimitedFrameCodec` uses `Vec<u8>` frames to preserve
  `WireframeProtocol` compatibility and existing framed send helpers.
- A default `LengthDelimitedFrameCodec` provides compatibility for existing
  users without code changes.
- `WireframeApp` becomes generic over `F: FrameCodec` with a default type
  parameter, and the builder gains `.with_codec()` to swap codecs.

For orientation, the trait shape is expected to look like this:

```rust,no_run
use std::io;
use tokio_util::codec::{Decoder, Encoder};

/// Trait for pluggable frame codecs supporting different wire protocols.
pub trait FrameCodec: Send + Sync + 'static {
    /// Frame type produced by decoding.
    type Frame: Send + Sync + 'static;

    /// Create a Tokio Decoder for this codec.
    fn decoder(&self) -> impl Decoder<Item = Self::Frame, Error = io::Error> + Send;

    /// Create a Tokio Encoder for this codec.
    fn encoder(&self) -> impl Encoder<Self::Frame, Error = io::Error> + Send;

    /// Extract payload bytes from a frame.
    fn frame_payload(frame: &Self::Frame) -> &[u8];

    /// Wrap payload bytes into a frame for sending.
    fn wrap_payload(payload: Vec<u8>) -> Self::Frame;

    /// Extract correlation ID for request/response matching.
    fn correlation_id(_frame: &Self::Frame) -> Option<u64> { None }

    /// Maximum frame length this codec will accept.
    fn max_frame_length(&self) -> usize;
}
```

## Goals and Non-Goals

### Goals

- Allow Wireframe to support Hotline, MySQL, and Redis RESP framing.
- Preserve protocol metadata in frame types.
- Maintain the existing API surface for users who rely on length-delimited
  framing.
- Keep framing logic encapsulated and testable.

### Non-Goals

- Protocol negotiation or runtime codec selection.
- Replacing the existing message serialisation format.
- Introducing asynchronous codec initialisation.

## Migration Plan

### Phase 1: Trait definition (non-breaking)

- Create `src/codec.rs` with the `FrameCodec` trait and
  `LengthDelimitedFrameCodec` default.
- Add unit tests covering the default codec behaviour.

### Phase 2: Parameterise `WireframeApp` (breaking change)

- Add `F: FrameCodec = LengthDelimitedFrameCodec` to `WireframeApp`.
- Add a `codec: F` field and update the default implementation.
- Add `.with_codec()` for builder ergonomics.

### Phase 3: Update connection handling

- Parameterise `FrameHandlingContext` and `ResponseContext` over the codec.
- Replace `LengthDelimitedCodec` usage with `FrameCodec` decoder/encoder calls.
- Use `max_frame_length()` for buffer sizing and fragmentation defaults.

### Phase 4: Update `WireframeServer`

- Propagate the codec type parameter through server factory types.
- Update any connection handling signatures that assume the fixed codec.

### Phase 5: Protocol examples

- Add an example codec for Hotline.
- Add an example codec for MySQL if time permits.

## Implementation Findings

### Codec ergonomics and stateful metadata

- `FrameCodec::wrap_payload` is a static constructor that takes `Vec<u8>`. The
  Hotline and MySQL example codecs therefore stamp default metadata values
  (transaction ID and sequence number set to `0`) unless the caller manually
  constructs a frame and bypasses the default send path.
- Payload extraction and wrapping require contiguous buffers, leading to extra
  copying when adapters must convert from `BytesMut` into `Vec<u8>`.

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
- Clippy’s indexing and truncation lints forced the examples to use `bytes::Buf`
  and explicit `try_from` conversions, suggesting a need for shared parsing
  helpers to reduce boilerplate.

## Known Risks and Limitations

- RPITIT (impl Trait in return position) requires Rust 1.75+; confirm the
  project's minimum supported Rust version before adoption.
- The new codec type parameter propagates through public APIs, increasing type
  signatures and potentially affecting downstream type inference.
- `LengthDelimitedFrameCodec` converts decoded `Bytes` into `Vec<u8>` to retain
  `WireframeProtocol` compatibility, which adds an extra copy for each frame.

## Outstanding Decisions

- Decide whether `FrameCodec::wrap_payload` should accept `&self` (or other
  state) to support codecs that maintain sequence counters or compression
  dictionaries.
- Decide whether to introduce a `CodecError` taxonomy and recovery policy
  (drop frame versus close connection) for protocol-specific failures.
- Decide how to realign fragmentation with the `FragmentAdapter` design so
  opt-in behaviour and composition order are explicit.
- Decide whether to unify codec handling between the app router path and the
  `Connection` actor to ensure protocol hooks run consistently.
- Future protocol adoption may still require revisiting how correlation
  identifiers are surfaced for FIFO protocols (for example, RESP).

## Architectural Rationale

A dedicated `FrameCodec` abstraction aligns framing with the protocol boundary
and keeps the routing/middleware pipeline agnostic to wire-level details. It
preserves existing behaviour through a default codec while enabling protocol
specificity and reusability. By tying buffer sizing and maximum frame length to
codec configuration, the design keeps transport-level constraints close to the
framing rules that define them.
