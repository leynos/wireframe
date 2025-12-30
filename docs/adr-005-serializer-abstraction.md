# Architectural decision record (ADR) 005: serialiser abstraction beyond bincode

## Status

Proposed.

## Date

2025-12-30.

## Context and Problem Statement

Wireframe currently couples `Message` to `bincode` by requiring `Encode` and
`BorrowDecode` implementations. The `Serializer` trait does not remove this
dependency, so users who want wire-rs, Protocol Buffers, or Cap'n Proto must
either wrap their data in bincode-compatible types or fork the library. This
leaks a concrete serialisation choice into otherwise protocol-agnostic routing
and framing, and it blocks metadata-aware decoding (for example, deriving a
schema version from frame headers). The router design document explicitly
targets derivable encoders/decoders, and message versioning is expected to be
an evolution path.[^router-design][^message-versioning]

We need to decouple message encoding/decoding from bincode while preserving
backward compatibility and performance.

## Decision Drivers

- Support alternative serialisation formats without forking.
- Preserve existing bincode-based integrations where possible.
- Allow deserialisation to access protocol metadata for version negotiation.
- Keep the encoding/decoding surface ergonomic for handler authors.
- Maintain low allocation overhead and avoid unnecessary copies.

## Requirements

### Functional requirements

- Allow callers to select a serialiser without changing the message model.
- Provide a migration path for existing `Message` types.
- Support metadata-aware decoding for schema/version negotiation.

### Technical requirements

- Keep error types explicit and mappable to `WireframeError`.
- Maintain `Send + Sync + 'static` safety for async connection handling.
- Avoid global trait objects on hot paths unless necessary.

## Options Considered

### Option A: keep the bincode-only `Message` trait (status quo)

Continue to require `bincode` traits and document adapter strategies for other
serialisers.

### Option B: introduce a serialiser-agnostic message boundary (preferred)

Introduce a new `MessageBody` (or similar) trait that is independent of
`bincode`, along with a `SerializerAdapter` that bridges to concrete
serialisers. Provide bincode and optional wire-rs/Serde adapters, with a
migration path for existing `Message` types.

### Option C: adopt wire-rs as the primary serialiser

Move `Message` to wire-rs `Encode`/`Decode`, ship bincode as a compatibility
layer, and update examples and docs accordingly.

| Topic                        | Option A: status quo | Option B: adapter | Option C: wire-rs |
| ---------------------------- | -------------------- | ----------------- | ----------------- |
| Alternative serialisers      | Poor                 | Strong            | Strong            |
| Backward compatibility       | Full                 | High              | Medium            |
| Metadata-aware deserialising | Poor                 | Strong            | Strong            |
| Performance risk             | Low                  | Medium            | Medium            |
| API churn                    | Low                  | Medium            | High              |

_Table 1: Trade-offs between serialisation abstraction options._

## Decision Outcome / Proposed Direction

Adopt Option B: introduce a serialiser-agnostic message boundary with adapter
traits. Provide a bincode adapter to preserve existing behaviour, and supply
optional wire-rs or Serde adapters to reduce boilerplate. This allows frame
metadata to participate in deserialisation, supports version negotiation, and
avoids committing Wireframe to a single serialiser.

## Goals and Non-Goals

### Goals

- Allow multiple serialisers without forking Wireframe.
- Keep the handler API stable where feasible.
- Enable metadata-aware decoding for version negotiation.

### Non-Goals

- Replacing all existing message types in a single release.
- Implementing message version negotiation in this ADR (tracked separately).

## Migration Plan

### Phase 1: new abstractions

- Introduce `MessageBody` (or equivalent) and `SerializerAdapter` traits.
- Implement a bincode adapter that preserves current behaviour.
- Add optional adapters for wire-rs and Serde-based serialisers.

### Phase 2: integration and compatibility

- Update `WireframeApp`/`Serializer` plumbing to accept the new adapter.
- Provide blanket implementations or shims for existing `Message` types.
- Deprecate direct reliance on bincode traits in public APIs.

### Phase 3: documentation and validation

- Update the user guide with migration examples.
- Add tests covering mixed serialisers and metadata-aware decoding.

## Known Risks and Limitations

- Adapter layering may increase type complexity for downstream users.
- Supporting multiple serialisers can increase maintenance overhead.
- Metadata-aware decoding requires a clear definition of what metadata is
  exposed and when it is safe to consume it.

## Outstanding Decisions

- Define the precise `MessageBody` surface (borrowed vs owned payloads).
- Decide how codec metadata is passed into the deserialiser context.
- Choose which optional adapters ship by default and which are feature-gated.

## Architectural Rationale

Decoupling the serialisation boundary aligns with Wireframe’s goal of being a
protocol-agnostic router and enables future version negotiation without forcing
a single serialiser choice. Adapter-based boundaries preserve backward
compatibility while providing a clear path for modern, derivable encoders and
decoders.

[^router-design]: `docs/rust-binary-router-library-design.md`.

[^message-versioning]: `docs/message-versioning.md`.
