# Architectural decision record (ADR) 008: zero-copy public byte container

## Status

Accepted

Accepted on 2026-05-26. Wireframe will use `bytes::Bytes`, or a transparent
project wrapper over `bytes::Bytes`, as the stable public byte representation
for packet, envelope, serializer, middleware, and hook payload hand-offs.
Mutation will be exposed through an explicit edit-on-demand workflow that keeps
read-only paths zero-copy and copies only when an edit requires unique mutable
storage.

## Date

2026-05-26

## Context and Problem Statement

`FrameCodec` already supports `Bytes`-backed frame types, and the default
length-delimited codec uses `Bytes` today. The inventory in
[`frame-vec-u8-inventory.md`](frame-vec-u8-inventory.md), however, shows that
the public packet, middleware, serializer, and client hook surfaces still
assume owned `Vec<u8>` payloads.

Those APIs are not equivalent in how they use bytes:

- `PacketParts`, `Envelope`, and serializer output are primarily transport and
  routing hand-off surfaces.
- `ServiceRequest`, `ServiceResponse`, and `BeforeSendHook` promise editable
  bytes and therefore embed a mutation model into the public API.
- The default codec path requires shared, cheap-to-clone byte buffers to
  eliminate the final copy identified in epic 284.

The project needs a single public byte-container strategy that preserves
zero-copy behaviour for read-only paths without forcing every caller to manage
buffer taxonomy manually.

## Traceability

This ADR governs the Epic 284 work tracked in:

- [`frame-vec-u8-inventory.md`](frame-vec-u8-inventory.md), especially the
  public payload surfaces and resolved direction in sections "Public
  `payload-bound` surfaces" and "Resolved direction for epic 284".
- [`zero-copy-frame-and-payload-migration-roadmap.md`](zero-copy-frame-and-payload-migration-roadmap.md),
  which tracks the dedicated zero-copy migration phases that apply this public
  byte-container decision across the internal migration, public API flip, and
  release workstream.
- [`roadmap.md`](roadmap.md), specifically:
  - roadmap item `10.1.1`, which approves the stable public byte-container and
    edit-on-demand model;
  - roadmap item `11.1.1`, which converts internal packet payload storage and
    serializer output to the approved byte representation;
  - roadmap items `12.1.1` and `12.1.2`, which migrate packet, envelope, and
    middleware surfaces to the approved byte and editing model;
  - roadmap items `12.2.1` and `12.2.3`, which migrate client hooks and
    serializers and publish downstream migration examples;
  - roadmap item `13.1.2`, which writes the migration guide section covering
    the zero-copy API flip.

## Decision Drivers

- Remove the final owned-byte copy from the default outbound path.
- Keep read-only packet and routing paths zero-copy by default.
- Preserve a clear and ergonomic mutation story for middleware and client
  hooks.
- Avoid exposing two equally primary byte-container APIs forever.
- Keep the design compatible with the existing `FrameCodec` and `Bytes`
  default.

## Requirements

### Functional requirements

- Public packet and routing surfaces must support cheap cloning and transport
  without copying when the underlying bytes are already shared.
- Middleware and client hooks must still support intentional mutation of
  serialized bytes.
- The selected API must support migration examples that downstream users can
  apply without learning multiple internal buffer types.

### Technical requirements

- The default codec path must be able to move from serialization to
  `wrap_payload` without materializing a fresh `Vec<u8>`.
- The design must not require `F::Frame = Vec<u8>` or any equivalent
  frame-level coupling.
- Mutation must be explicit, and it must not trigger copies on read-only
  paths.

## Options Considered

### Option A: switch every public byte surface directly to `Bytes`

This would maximize zero-copy reuse for read-only paths, but it would also
replace the current `frame_mut()` and `Fn(&mut Vec<u8>)` contracts with a more
awkward mutation story. Middleware authors would need to choose when to clone,
freeze, or reallocate, and the API would no longer describe the intended
editing workflow clearly.

### Option B: use `Bytes` plus explicit editing (accepted)

Under this option, public packet and routing surfaces store `Bytes` (or a
single project-defined wrapper over `Bytes`) as the stable representation.
Editable surfaces expose mutation through an explicit helper or editor that
performs copy-on-write only when a caller actually mutates the bytes.

This preserves zero-copy behaviour for pass-through traffic while keeping the
current "inspect, optionally edit, then forward" workflow legible.

### Option C: keep `Vec<u8>` as the stable public representation

This preserves the existing mutation model, but it also preserves the final
copy that epic 284 is trying to remove. Any internal `Bytes` use would still
collapse back to owned vectors at the public boundary.

### Option D: make the public API permanently generic or dual-surfaced

This avoids choosing one primary abstraction, but it increases API surface
area, complicates documentation, and pushes conversion logic onto downstream
users. The inventory explicitly calls out the risk of forcing consumers to
write repetitive adapters between byte containers.

| Topic                     | Option A: `Bytes` only | Option B: `Bytes` + explicit editor | Option C: keep `Vec<u8>` | Option D: dual support |
| ------------------------- | ---------------------- | ----------------------------------- | ------------------------ | ---------------------- |
| Zero-copy read path       | Excellent              | Excellent                           | Poor                     | Good                   |
| Editing ergonomics        | Weak                   | Strong                              | Strong                   | Medium                 |
| Public API complexity     | Medium                 | Medium                              | Low                      | High                   |
| Long-term maintainability | Good                   | Good                                | Poor                     | Poor                   |
| Migration burden          | Medium                 | Medium                              | Low short-term           | High                   |

_Table 1: Trade-offs for the public byte-container choice._

## Decision Outcome

Adopt Option B: use `Bytes`-compatible storage as the stable public payload
representation, and expose mutation through an explicit edit-on-demand helper
for middleware and client hooks.

The accepted direction is:

- Packet and routing surfaces (`PacketParts`, `Envelope`, serializer output,
  and equivalent internal hand-offs) standardize on shared bytes.
- Middleware and hook surfaces expose an explicit editing entry point that
  copies only when the caller mutates the payload.
- `Vec<u8>` remains available only through explicit compatibility helpers,
  adapters, or migration constructors defined by the rollout ADR.

This keeps the zero-copy path obvious and makes the mutating path explicit.

The affected public surfaces are:

- `PacketParts` and `Envelope`, which will store and hand off the stable shared
  byte representation.
- `ServiceRequest` and `ServiceResponse`, which will expose read-only access to
  stable shared bytes and explicit edit-on-demand mutation.
- `BeforeSendHook`, which will move from `Fn(&mut Vec<u8>)` to the same
  explicit editing workflow used by middleware.
- `Serializer::serialize`, which will return the stable shared byte
  representation, so the default outbound path can reach
  `FrameCodec::wrap_payload` without materializing a fresh `Vec<u8>`.

The editing workflow must be project-defined rather than raw caller-managed
buffer conversion. It may use `bytes::BytesMut` internally, but callers should
not need to manually shuttle between `Vec<u8>`, `Bytes`, and `BytesMut` for
ordinary middleware or client-hook edits. The workflow must document when an
edit can reuse unique storage and when it may copy shared storage.

## Goals and Non-Goals

### Goals

- Remove the final default-path copy without sacrificing middleware
  ergonomics.
- Make the read-only versus mutating cost model visible in the API.
- Give downstream users a single primary byte model to target.

### Non-Goals

- Guarantee zero allocation for every mutation path.
- Eliminate every internal `Vec<u8>` immediately, including bounded one-shot
  replay buffers that remain intentionally separate.
- Rework unrelated message-body APIs that are outside the frame and payload
  migration scope.

## Migration Plan

### Phase 1: establish the stable byte representation

Convert packet payload storage and serializer output to the shared byte
representation. Exact constructor and conversion names are implementation
details for roadmap items `11.1.1`, `12.1.1`, and `12.2.1`; compatibility
helper lifetime and feature-gating belong to ADR 009 and roadmap item `10.1.2`.

### Phase 2: introduce edit-on-demand wrappers

Replace `Vec<u8>`-backed middleware and hook edit points with explicit editing
helpers that preserve the current inspect, optionally edit, then forward
workflow while copying only when modified.

### Phase 3: update docs and examples

Update the user guide, middleware examples, and migration guide to teach the
new read-only and mutating workflows distinctly.

## Known Risks and Limitations

- Copy-on-write editing helper names still need to be finalized during API
  implementation, and poor naming could hide when a clone occurs.
- Some downstream users may rely on direct `Vec<u8>` methods such as `push` or
  `extend_from_slice`; the migration guide must map those patterns to the new
  editor API explicitly.
- If the project exposes both raw `Bytes` and a custom editor type too widely,
  the API could still feel like a dual-surface design in practice.

## Follow-up Decisions

- Roadmap item `10.1.2` and ADR 009 decide which `Vec<u8>` compatibility
  helpers survive the breaking release, whether any helpers are feature-gated,
  and when retained helpers are reviewed again.
- Roadmap item `11.1.1` introduces the shared byte representation for internal
  packet payload storage and serializer output.
- Roadmap items `12.1.1` and `12.1.2` define the exact packet, envelope,
  middleware, and edit-on-demand API names.
- Roadmap item `12.2.1` defines the exact client hook and serializer
  signatures that apply this decision.
- Roadmap item `13.1.2` publishes migration-guide examples that map common
  `Vec<u8>` mutation patterns onto the new editing workflow.

## Architectural Rationale

The wider architecture already treats frame transport and payload ownership as
separate concerns. Standardizing stable payload storage on a `Bytes`-compatible
representation aligns the public API with the default codec path, while
explicit editing helpers preserve the middleware-first ergonomics that made
`Vec<u8>` attractive originally.
