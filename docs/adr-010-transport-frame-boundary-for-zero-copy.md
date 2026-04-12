# Architectural decision record (ADR) 010: transport-frame boundary for the zero-copy migration

## Status

Proposed.

## Date

2026-04-12.

## Context and Problem Statement

The inventory in [`frame-vec-u8-inventory.md`](frame-vec-u8-inventory.md) shows
that the remaining `Vec<u8>` coupling is not only about public payload APIs.
The transport pipeline also has an architectural boundary problem:

- `ConnectionActor` operates on types satisfying
  `FrameLike + CorrelatableFrame + Packet`.
- The default codec frame type is `Bytes`, which does not satisfy those actor
  traits.
- The current server path therefore routes actor output through `Envelope` and
  lets the codec driver perform the final `serialize -> wrap_payload` step.
- Protocol hook alignment across the actor and codec boundary is still a
  follow-up concern after roadmap item 9.3.1.

The zero-copy migration needs an explicit long-term boundary between actor
logic, protocol hooks, serializer output, and transport frame emission so the
project does not keep `Vec<u8>` bridges alive only because the architecture is
underspecified.

## Decision Drivers

- Preserve zero-copy transport framing for the default codec path.
- Avoid expanding `ConnectionActor` bounds just to make codec frame types look
  like packets.
- Keep fragmentation, correlation handling, and protocol hooks explicit.
- Reuse the existing `FramePipeline` direction from roadmap item 9.3.1 where
  it remains sound.
- Minimize public API churn outside the migration's intended scope.

## Options Considered

### Option A: make `ConnectionActor` operate on codec frame types directly

This would require codec frame types such as `Bytes` to satisfy packet- and
correlation-oriented traits, or it would require a new actor-specific wrapper
for every codec frame. That increases coupling between transport framing and
application packet semantics.

### Option B: keep the actor envelope-oriented and let the codec driver own transport frames (preferred)

Under this option, the actor continues to work with `Envelope` or another
packet-shaped type, while the codec driver owns serialization, payload
wrapping, and final transport frame emission. Protocol hooks must be defined
explicitly at whichever boundary they truly need.

### Option C: add a new bridging abstraction that makes codec frames actor-compatible

This could formalize the boundary, but it also risks creating a second layer of
wrapper types whose only job is to satisfy the actor's existing bounds. That
adds complexity without necessarily removing copies.

| Topic                       | Option A: actor on codec frames | Option B: actor on envelopes | Option C: bridge layer |
| --------------------------- | ------------------------------- | ---------------------------- | ---------------------- |
| Zero-copy default path      | Good                            | Good                         | Medium                 |
| Actor/transport separation  | Weak                            | Strong                       | Medium                 |
| Required trait expansion    | High                            | Low                          | Medium                 |
| Implementation complexity   | High                            | Medium                       | High                   |
| Fit with roadmap item 9.3.1 | Weak                            | Strong                       | Medium                 |

_Table 1: Trade-offs for the actor and transport-frame boundary._

## Decision Outcome / Proposed Direction

Adopt Option B: keep the actor packet-oriented, and make the codec driver the
only component that owns transport frame emission.

The proposed direction is:

- `ConnectionActor` continues to reason about packet-shaped values such as
  `Envelope`.
- The serializer and codec driver own the final
  `packet -> bytes -> transport frame` transition.
- Any protocol hook that requires codec-frame visibility is attached at the
  codec-driver boundary, not by forcing codec frames through actor traits they
  do not naturally satisfy.
- `Vec<u8>` frame bridges are removed from the core runtime once the chosen
  codec-driver path is proven.

## Goals and Non-Goals

### Goals

- Keep transport framing and packet semantics decoupled.
- Remove accidental `Vec<u8>` dependencies at the actor boundary.
- Give the zero-copy migration a stable place to enforce pointer reuse and
  allocation tests.

### Non-Goals

- Redesign the full protocol hook API in this ADR.
- Force every custom codec frame to implement packet semantics.
- Remove all test-only `Vec<u8>` helpers immediately if they still provide
  migration value.

## Migration Plan

### Phase 1: formalize the packet-to-frame boundary

Document which component owns serialization, protocol hook invocation, and
`wrap_payload`, and update the runtime so that ownership is reflected in code.

### Phase 2: remove obsolete core bridges

Delete or move `Vec<u8>`-specific core frame bridges once the actor no longer
needs them for production behaviour.

### Phase 3: validate the boundary

Add regression coverage showing that the default codec path stays zero-copy and
that correlation, fragmentation, and protocol hooks still run at the intended
stage.

## Known Risks and Limitations

- The protocol hook story still needs a precise statement of which hooks run on
  packets and which run on transport frames.
- If the codec driver becomes the only framing boundary, its tests must carry
  more of the performance and correctness burden than they do today.
- Some example or test harness code may continue to use `Vec<u8>` as a
  convenience type even after the production boundary is cleaned up.

## Outstanding Decisions

- Which existing protocol hooks should move to the codec-driver boundary, and
  which should remain packet-oriented?
- Should any remaining `Vec<u8>` bridge live in `test_support`, a feature-gated
  compatibility module, or nowhere at all?
- Does the project need a new internal trait to express "serializable packet"
  separately from transport frame semantics?

## Architectural Rationale

Roadmap item 9.3.1 already established that treating codec frames and actor
packets as the same abstraction leads to awkward constraints, especially when
the default codec frame is `Bytes`. Making the codec driver the explicit
transport-frame boundary preserves that lesson, supports the zero-copy goal,
and avoids rebuilding actor semantics around transport-specific frame types.
