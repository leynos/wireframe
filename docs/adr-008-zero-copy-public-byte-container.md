# Architectural decision record (ADR) 008: zero-copy public byte container

## Status

Proposed.

## Date

2026-04-12.

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
- The default codec path wants shared, cheap-to-clone bytes to remove the
  final copy identified in epic 284.

The project needs a single public byte-container strategy that preserves
zero-copy behaviour for read-only paths without forcing every caller to manage
buffer taxonomy manually.

## Traceability

This ADR governs the Epic 284 work tracked in:

- [`frame-vec-u8-inventory.md`](frame-vec-u8-inventory.md), especially the
  public payload surfaces and resolved direction in sections "Public
  `payload-bound` surfaces" and "Resolved direction for epic 284".
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

### Option B: use `Bytes` for stable storage and expose explicit edit-on-demand wrappers (preferred)

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

### Option D: make the public API permanently generic or dual-surfaced over `Vec<u8>` and `Bytes`

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

## Decision Outcome / Proposed Direction

Adopt Option B: use `Bytes`-compatible storage as the stable public payload
representation, and expose mutation through an explicit edit-on-demand helper
for middleware and client hooks.

The proposed direction is:

- Packet and routing surfaces (`PacketParts`, `Envelope`, serializer output,
  and equivalent internal hand-offs) standardize on shared bytes.
- Middleware and hook surfaces expose an explicit editing entry point that
  copies only when the caller mutates the payload.
- `Vec<u8>` remains available only through explicit compatibility helpers,
  adapters, or migration constructors defined by the rollout ADR.

This keeps the zero-copy path obvious and makes the mutating path explicit.

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
representation, and add explicit conversions for compatibility callers.

### Phase 2: introduce edit-on-demand wrappers

Replace `Vec<u8>`-backed middleware and hook edit points with explicit editing
helpers that preserve the current workflow while copying only when modified.

### Phase 3: update docs and examples

Update the user guide, middleware examples, and migration guide to teach the
new read-only and mutating workflows distinctly.

## Known Risks and Limitations

- Copy-on-write editing helpers still need a concrete public shape, and poor
  naming could hide when a clone occurs.
- Some downstream users may rely on direct `Vec<u8>` methods such as `push` or
  `extend_from_slice`; the migration guide must map those patterns to the new
  editor API explicitly.
- If the project exposes both raw `Bytes` and a custom editor type too widely,
  the API could still feel like a dual-surface design in practice.

## Outstanding Decisions

- Should the public editing surface expose `BytesMut`, a project-defined editor
  wrapper, or closure-based mutation helpers?
- Should serializer output move to the new byte representation in the same
  release as packet and middleware changes, or in a preparatory release
  beforehand?
- Which compatibility constructors or `into_vec` helpers are required for the
  migration window?

## Architectural Rationale

The wider architecture already treats frame transport and payload ownership as
separate concerns. Standardizing stable payload storage on a `Bytes`-compatible
representation aligns the public API with the default codec path, while
explicit editing helpers preserve the middleware-first ergonomics that made
`Vec<u8>` attractive originally.
