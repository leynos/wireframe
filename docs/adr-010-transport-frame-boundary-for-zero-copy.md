# Architectural decision record (ADR) 010: transport-frame boundary for the zero-copy migration

## Status

Accepted

Accepted on 2026-06-22. Wireframe keeps the connection actor
packet-oriented and makes the codec driver the sole owner of transport-frame
emission. Protocol hooks remain packet-oriented until the app-router response
gap is closed under roadmap item `11.2.1`; no public actor-boundary
`Vec<u8>` compatibility shim is introduced, with the public
`CorrelatableFrame for Vec<u8>` bridge removed under roadmap item `11.2.2` and
reviewed under `14.2.1`; and no new "serializable packet" trait is added
because `Packet` plus `EncodeWith<Serializer>` already expresses that
requirement. The remaining final outbound copy is removed under roadmap item
`11.1.2`, and the sole-owner boundary is guarded under roadmap item `11.2.3`.

## Date

Proposed 2026-04-12. Accepted 2026-06-22.

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

## Traceability

This ADR governs the Epic 284 runtime-boundary work tracked in:

- [`frame-vec-u8-inventory.md`](frame-vec-u8-inventory.md), especially the
  "`internal-only` runtime surfaces", "Adjacent constraints that matter but do
  not name `Vec<u8>`", and "Resolved direction for epic 284" sections.
- [`zero-copy-frame-and-payload-migration-roadmap.md`](zero-copy-frame-and-payload-migration-roadmap.md),
  which tracks the dedicated rollout workstream that applies this boundary
  decision and reviews the migration until the codec driver becomes the sole
  owner of transport frame emission. During implementation review, monitor all
  non-driver `FrameCodec::wrap_payload` call sites against this ADR until they
  are removed or relocated into the codec-driver boundary.
- [`roadmap.md`](roadmap.md), specifically:
  - roadmap item `10.1.3`, which approves the actor and codec-driver boundary;
  - roadmap item `11.1.2`, which removes the final default-path `Vec<u8>` copy
    between serialization and `FrameCodec::wrap_payload`;
  - roadmap items `11.2.1` and `11.2.2`, which implement the boundary and
    move `Vec<u8>`-specific runtime traits or bridges out of the core runtime;
  - roadmap item `11.2.3`, which adds allocation and pointer-reuse regressions
    for the internal actor and codec path;
  - roadmap item `14.2.1`, which reviews whether any runtime-only compatibility
    bridges still need to survive after the breaking release.

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

## Decision Outcome

Adopt Option B: keep the actor packet-oriented, and make the codec driver the
only component that owns transport frame emission.

The accepted direction is:

- `ConnectionActor` continues to reason about packet-shaped values such as
  `Envelope`. In production, the actor remains generic over packet semantics
  rather than over transport frame buffers.
- The serializer and codec driver own the final
  `packet -> bytes -> transport frame` transition. Today the only production
  `FrameCodec::wrap_payload` caller is the codec-driver path in
  `src/app/outbound_encoding.rs`.
- Protocol hooks remain packet-oriented. The codec driver hosts serialization
  and transport framing, not a second transport-frame hook surface.
- `Vec<u8>` frame bridges are removed from the core runtime once the approved
  codec-driver path is proven. `Packet for Vec<u8>` stays test-only, while the
  public `CorrelatableFrame for Vec<u8>` bridge leaves the core runtime in the
  breaking release governed by roadmap items `11.2.2` and `14.2.1`.

This closes roadmap item `10.1.3` and the corresponding zero-copy migration
roadmap item `1.1.3`. Runtime implementation is intentionally deferred to
roadmap items `11.1.2`, `11.2.1`, `11.2.2`, and `11.2.3`.

## Goals and Non-Goals

### Goals

- Keep transport framing and packet semantics decoupled. This ADR now records
  that boundary; roadmap item `11.2.1` implements any remaining runtime
  alignment.
- Remove accidental `Vec<u8>` dependencies at the actor boundary. The
  production boundary decision is accepted here; bridge removal is sequenced
  under roadmap item `11.2.2`.
- Give the zero-copy migration a stable place to enforce pointer reuse and
  allocation tests. Roadmap item `11.2.3` adds the regression coverage and the
  guard against new non-driver production `wrap_payload` callers.

### Non-Goals

- Redesign the full protocol hook API in this ADR.
- Force every custom codec frame to implement packet semantics.
- Remove all test-only `Vec<u8>` helpers immediately if they still provide
  migration value.

## Migration Plan

### Phase 1: formalize the packet-to-frame boundary

Document which component owns serialization, protocol hook invocation, and
`wrap_payload`, and update the runtime so that ownership is reflected in code
where any gap remains. The accepted owner is the codec driver. The known
app-router `before_send` asymmetry is closed under roadmap item `11.2.1`;
guard coverage for new non-driver production `wrap_payload` callers is added
under roadmap item `11.2.3`.

### Phase 2: remove obsolete core bridges

Delete or move `Vec<u8>`-specific core frame bridges once the actor no longer
needs them for production behaviour. `Packet for Vec<u8>` remains in test
support. `CorrelatableFrame for Vec<u8>` is a public production impl, so its
removal is a breaking-release change tracked under roadmap items `11.2.2` and
`14.2.1`.

### Phase 3: validate the boundary

Add regression coverage showing that the default codec path stays zero-copy and
that correlation, fragmentation, and protocol hooks still run at the intended
stage.

## Known Risks and Limitations

- Protocol hooks stay packet-oriented. The current limitation is narrower:
  `before_send` fires for actor-driven push and multi-packet frames, but not yet
  for app-router responses routed through `FramePipeline`. Roadmap item
  `11.2.1` owns that closure.
- If the codec driver becomes the only framing boundary, its tests must carry
  more of the performance and correctness burden than they do today.
- Some example or test harness code may continue to use `Vec<u8>` as a
  convenience type even after the production boundary is cleaned up.
- The final serializer-to-codec bridge still materializes `Vec<u8>` before
  converting to `Bytes`. Roadmap item `11.1.2` and issue
  <https://github.com/leynos/wireframe/issues/538> own the
  `Bytes`-native serializer contract that removes this copy.
- The project needs guard coverage against future non-driver production
  `FrameCodec::wrap_payload` callers. Roadmap item `11.2.3` owns that guard.

## Tracked transport-frame call sites

At acceptance time, the sole production `FrameCodec::wrap_payload` caller is
`src/app/outbound_encoding.rs`, which sits inside the codec-driver boundary and
performs the current `Serializer::serialize -> Bytes::from -> wrap_payload`
transition. Other `wrap_payload` call sites are tests, examples, test helpers,
or testkit support.

Roadmap item `11.2.3` must add a guard that fails if a new non-driver
production caller appears. Until that guard exists, reviews should verify this
inventory manually.

## Resolved Decisions

- Which existing protocol hooks should move to the codec-driver boundary, and
  which should remain packet-oriented? Existing hooks remain packet-oriented and
  continue to run against `Envelope` before serialization. The codec driver
  does not gain a new transport-frame-level hook surface. The app-router
  response `before_send` gap is documented as a limitation and tracked under
  roadmap item `11.2.1`.
- Should any remaining `Vec<u8>` bridge live in `test_support`, a feature-gated
  compatibility module, or nowhere at all? No public feature-gated actor-boundary
  compatibility shim is introduced. `Packet for Vec<u8>` stays in test support.
  The public `CorrelatableFrame for Vec<u8>` impl leaves the core runtime under
  the breaking-release work in roadmap item `11.2.2`, with retained bridges
  reviewed under roadmap item `14.2.1`.
- Does the project need a new internal trait to express "serializable packet"
  separately from transport frame semantics? No. The existing `Packet` trait
  composed with `EncodeWith<Serializer>` already expresses "a packet that can be
  serialized"; `Envelope` satisfies both. Adding a bridging trait would recreate
  rejected Option C without removing copies.

## Architectural Rationale

Roadmap item 9.3.1 already established that treating codec frames and actor
packets as the same abstraction leads to awkward constraints, especially when
the default codec frame is `Bytes`. Making the codec driver the explicit
transport-frame boundary preserves that lesson, supports the zero-copy goal,
and avoids rebuilding actor semantics around transport-specific frame types.
