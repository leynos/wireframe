# Architectural decision record (ADR) 005: serializer abstraction beyond bincode

## Status

Accepted.

## Date

2025-12-30 (accepted update: 2026-02-22).

## Context and Problem Statement

Wireframe currently couples `Message` to `bincode` by requiring `Encode` and
`BorrowDecode` implementations. The `Serializer` trait does not remove this
dependency, so users who want wire-rs, Protocol Buffers, or Cap'n Proto must
either wrap their data in bincode-compatible types or fork the library. This
leaks a concrete serialization choice into otherwise protocol-agnostic routing
and framing, and it blocks metadata-aware decoding (for example, deriving a
schema version from frame headers). The router design document explicitly
targets derivable encoders/decoders, and message versioning is expected to be
an evolution path.[^router-design][^message-versioning]

Message encoding/decoding must be decoupled from bincode while preserving
backward compatibility and performance.

## Decision Drivers

- Support alternative serialization formats without forking.
- Preserve existing bincode-based integrations where possible.
- Allow deserialization to access protocol metadata for version negotiation.
- Keep the encoding/decoding surface ergonomic for handler authors.
- Maintain low allocation overhead and avoid unnecessary copies.

## Requirements

### Functional requirements

- Allow callers to select a serializer without changing the message model.
- Provide a migration path for existing `Message` types.
- Support metadata-aware decoding for schema/version negotiation.

### Technical requirements

- Keep error types explicit and mappable to `WireframeError`.
- Maintain `Send + Sync + 'static` safety for async connection handling.
- Avoid global trait objects on hot paths unless necessary.

## Options Considered

### Option A: keep the bincode-only `Message` trait (status quo)

Continue to require `bincode` traits and document adaptor strategies for other
serializers.

### Option B: introduce a serializer-agnostic message boundary (preferred)

Introduce a new `MessageBody` (or similar) trait that is independent of
`bincode`, along with a `SerializerAdapter` that bridges to concrete
serializers. Provide bincode and optional wire-rs/Serde adaptors, with a
migration path for existing `Message` types.

### Option C: adopt wire-rs as the primary serializer

Move `Message` to wire-rs `Encode`/`Decode`, ship bincode as a compatibility
layer, and update examples and docs accordingly.

| Topic                        | Option A: status quo | Option B: adaptor | Option C: wire-rs |
| ---------------------------- | -------------------- | ----------------- | ----------------- |
| Alternative serializers      | Poor                 | Strong            | Strong            |
| Backward compatibility       | Full                 | High              | Medium            |
| Metadata-aware deserializing | Poor                 | Strong            | Strong            |
| Performance risk             | Low                  | Medium            | Medium            |
| API churn                    | Low                  | Medium            | High              |

_Table 1: Trade-offs between serialization abstraction options._

## Decision Outcome / Proposed Direction

Adopt Option B: introduce a serializer-agnostic message boundary with adaptor
traits. Provide a bincode adaptor to preserve existing behaviour, and supply
optional wire-rs or Serde adaptors to reduce boilerplate. This allows frame
metadata to participate in deserialization, supports version negotiation, and
avoids committing Wireframe to a single serializer.

Accepted implementation details:

- `message::EncodeWith<S>` and `message::DecodeWith<S>` define serializer-aware
  adaptor boundaries used by `Serializer`.
- `message::DeserializeContext` carries parsed frame metadata into
  `Serializer::deserialize_with_context`.
- `Serializer` now supports `deserialize_with_context` with a default fallback
  to `deserialize`, preserving existing serializers.
- Existing bincode-centric flows remain source-compatible through the
  `message::Message` compatibility layer.
- Legacy `Message` compatibility is now an explicit serializer opt-in through
  `serializer::MessageCompatibilitySerializer`, avoiding blanket impl lockout
  for serializer-specific adaptors.
- An optional Serde bridge ships behind feature `serializer-serde` via
  `message::serde_bridge::{SerdeMessage, IntoSerdeMessage}` plus
  `serializer::SerdeSerializerBridge`.

## Goals and Non-Goals

### Goals

- Allow multiple serializers without forking Wireframe.
- Keep the handler API stable where feasible.
- Enable metadata-aware decoding for version negotiation.

### Non-Goals

- Replacing all existing message types in a single release.
- Implementing message version negotiation in this ADR (tracked separately).

## Migration Plan

### Phase 1: new abstractions

- Introduce `MessageBody` (or equivalent) and `SerializerAdapter` traits.
- Implement a bincode adaptor that preserves current behaviour.
- Add optional adaptors for wire-rs and Serde-based serializers.

### Phase 2: integration and compatibility

- Update `WireframeApp`/`Serializer` plumbing to accept the new adaptor.
- Provide blanket implementations or shims for existing `Message` types.
- Deprecate direct reliance on bincode traits in public APIs.

### Phase 3: documentation and validation

- Update the user guide with migration examples.
- Add tests covering mixed serializers and metadata-aware decoding.

## Known Risks and Limitations

- Adapter layering may increase type complexity for downstream users.
- Supporting multiple serializers can increase maintenance overhead.
- Metadata-aware decoding requires a clear definition of what metadata is
  exposed and when it is safe to consume it.

## Outstanding Decisions

- Decide whether envelope encoding should remain compatibility-first bincode
  for `Message` implementers across all serializers, or move to stricter
  serializer-specific envelope adaptors in a future major release.
- Evaluate whether a wire-rs bridge should be added alongside the Serde bridge
  in a follow-up roadmap item.

## Architectural Rationale

Decoupling the serialization boundary aligns with Wireframe’s goal of being a
protocol-agnostic router and enables future version negotiation without forcing
a single serializer choice. Adapter-based boundaries preserve backward
compatibility while providing a clear path for modern, derivable encoders and
decoders.

[^router-design]: `docs/rust-binary-router-library-design.md`.

[^message-versioning]: `docs/message-versioning.md`.
