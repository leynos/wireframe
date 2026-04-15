# Zero-copy frame and payload migration roadmap

This roadmap turns [`frame-vec-u8-inventory.md`](frame-vec-u8-inventory.md)
into an implementation plan for epic 284. It keeps transport-frame
substitution, payload-facing API migration, performance validation, and release
rollout as separate workstreams under one breaking-change programme so the work
can be reviewed and delivered in observable increments.

The roadmap assumes the current direction captured by the inventory:

- The default codec path should become `Bytes`-compatible end to end.
- Transport-frame migration and public payload API migration should not be
  treated as one undifferentiated refactor.
- Downstream middleware and client hooks need an editing model that does not
  force avoidable `Vec<u8>` conversions.

## Scope and success measures

In scope:

- Replacing the remaining `Vec<u8>`-centric frame and payload surfaces that
  block zero-copy decode or force the final outbound copy.
- Defining the public API shape for packet, middleware, serializer, and client
  byte-editing surfaces.
- Benchmarking the default codec path and documenting rollout for a breaking
  release.

Out of scope:

- Reworking unrelated body-buffering paths where `Vec<u8>` is not part of the
  frame or payload hand-off contract.
- Runtime protocol negotiation.
- A permanent dual-surface API that keeps `Vec<u8>` and the zero-copy
  alternative equally primary forever.

Success is measured by the following outcomes:

- The default length-delimited path no longer materializes a final
  `Vec<u8>` copy between serialization and `FrameCodec::wrap_payload`.
- Public packet, middleware, serializer, and client byte APIs use the
  selected zero-copy representation or an explicit edit-on-demand wrapper.
- Benchmark results are recorded with acceptance thresholds before the public
  API flip lands.
- Release documentation includes a migration guide, a breaking-change summary,
  and compatibility notes for downstream middleware, hooks, and codecs.

## 1. Decision closure and baseline

### 1.1. Resolve the remaining design choices

- [ ] 1.1.1. Approve the public byte-container and editing model. See
  [ADR 008](adr-008-zero-copy-public-byte-container.md).
  - [ ] Confirm the stable read-only representation for packet, middleware,
    serializer, and hook surfaces.
  - [ ] Confirm how mutation becomes explicit without forcing every caller to
    clone into `Vec<u8>`.
  - [ ] Success criteria: a single approved API direction exists for
    `PacketParts`, `Envelope`, `ServiceRequest`, `ServiceResponse`,
    `BeforeSendHook`, and `Serializer::serialize`.
- [ ] 1.1.2. Approve the compatibility and rollout policy. See
  [ADR 009](adr-009-vec-u8-migration-rollout.md).
  - [ ] Decide whether the major release carries finite compatibility shims,
    and document their lifetime.
  - [ ] Decide what compile-time adapters or helper constructors remain for
    `Vec<u8>` callers during the migration window.
  - [ ] Success criteria: the release branch has an agreed downstream migration
    story before code changes start landing.
- [ ] 1.1.3. Approve the transport-frame boundary for the zero-copy path. See
  [ADR 010](adr-010-transport-frame-boundary-for-zero-copy.md).
  - [ ] Confirm whether `ConnectionActor` remains envelope-oriented or takes on
    codec-frame responsibilities.
  - [ ] Confirm where protocol hooks execute when `F::Frame` differs from the
    actor payload type.
  - [ ] Success criteria: the actor, codec driver, and protocol hook boundary
    are documented and no longer depend on ad hoc `Vec<u8>` bridges.

### 1.2. Establish performance and migration baselines

- [ ] 1.2.1. Define the benchmark matrix for the migration. Requires 1.1.1.
  - [ ] Cover inbound decode, middleware pass-through, request-hook mutation,
    and outbound encode for the default codec path.
  - [ ] Include at least one protocol-native example codec to confirm the plan
    is not length-delimited specific.
- [ ] 1.2.2. Record baseline measurements and acceptance thresholds. Requires
  1.2.1.
  - [ ] Capture allocations, copied bytes, throughput, and latency for the
    current `Vec<u8>`-bound surfaces.
  - [ ] Set acceptance thresholds that require removal of the final outbound
    copy on the default path and forbid more than a small agreed regression in
    throughput or latency.
  - [ ] Success criteria: the repository contains a benchmark note or report
    that later phases can compare against directly.
- [ ] 1.2.3. Inventory downstream breakpoints and draft the migration guide
  outline. Requires 1.1.2.
  - [ ] List the exact public signatures that will change.
  - [ ] Capture before-and-after examples for middleware, hooks, serializers,
    and custom codecs.

## 2. Internal zero-copy foundations

### 2.1. Remove the remaining internal `Vec<u8>` bottlenecks

- [ ] 2.1.1. Introduce the chosen internal byte abstraction for packet payloads
  and outbound serialization. Requires 1.1.1.
  - [ ] Update the serializer boundary so the default path can hand encoded
    payload bytes to the codec without an intermediate `Vec<u8>` round-trip.
  - [ ] Success criteria: the default outbound path is capable of reusing
    shared byte storage end to end.
- [ ] 2.1.2. Convert `PacketParts` and `Envelope` to the new payload
  representation. Requires 2.1.1.
  - [ ] Preserve explicit adapter constructors or conversions defined by the
    rollout policy from 1.1.2.
  - [ ] Update packet reconstruction points in app routing and response
    handling.
- [ ] 2.1.3. Update internal channels and helper structs that still pin
  `Vec<u8>` without needing to. Requires 2.1.2.
  - [ ] Cover dead-letter queue hand-offs, response forwarding, and any
    zero-copy-capable internal replay buffers.

### 2.2. Stabilize the actor and codec-driver boundary

- [ ] 2.2.1. Implement the boundary chosen in ADR 010. Requires 1.1.3.
  - [ ] Ensure the default codec path does not regain a hidden copy while
    crossing between actor output and transport frame emission.
  - [ ] Keep correlation, fragmentation, and protocol hook behaviour explicit.
- [ ] 2.2.2. Remove core `Vec<u8>` frame bridges that exist only because the
  actor and codec layers were previously misaligned. Requires 2.2.1.
  - [ ] Move `CorrelatableFrame for Vec<u8>` out of the core runtime once the
    replacement path is proven, keeping it only where the rollout policy says
    it still belongs.
- [ ] 2.2.3. Add allocation- and pointer-reuse regressions for the internal
  boundary. Requires 2.2.1.
  - [ ] Cover the default codec path.
  - [ ] Cover at least one codec whose frame stores payload bytes directly.

## 3. Public API migration

### 3.1. Move packet and middleware APIs off owned `Vec<u8>`

- [ ] 3.1.1. Update `PacketParts`, `Envelope`, and `Packet` consumers to the
  selected byte surface. Requires 2.1.2.
  - [ ] Preserve developer-facing ergonomics for payload inspection and
    reconstruction.
- [ ] 3.1.2. Replace the middleware request and response wrappers with the new
  editing model. Requires 1.1.1 and 2.1.2.
  - [ ] Port the current `frame()`, `frame_mut()`, and `into_inner()` workflows
    to their zero-copy equivalents.
  - [ ] Update middleware tests to prove read-only paths stay zero-copy and
    mutation paths copy only on demand.
- [ ] 3.1.3. Update server-side examples and behavioural tests that currently
  teach `Vec<u8>` mutation. Requires 3.1.2.
  - [ ] Cover packet rewriting, response rewriting, and correlation-preserving
    middleware flows.

### 3.2. Move client-facing byte APIs off owned `Vec<u8>`

- [ ] 3.2.1. Replace the `before_send` hook contract with the approved
  edit-on-demand surface. Requires 1.1.1.
  - [ ] Keep hook behaviour observable in tracing and tests.
- [ ] 3.2.2. Update `Serializer::serialize` and its callers to the selected
  byte representation. Requires 2.1.1.
  - [ ] Confirm the new contract across runtime, examples, and docs.
- [ ] 3.2.3. Re-evaluate client preamble leftovers against the chosen public
  byte model. Requires 1.1.1.
  - [ ] Keep the existing `Vec<u8>` replay path only if the approved API
    direction still makes that the least surprising compatibility boundary.
  - [ ] Success criteria: the preamble policy is either explicitly retained and
    documented, or migrated consistently with the rest of the client surface.

## 4. Validation, ecosystem updates, and documentation

### 4.1. Remove stale teaching and compatibility drift

- [ ] 4.1.1. Update `wireframe_testing`, examples, and design documents to stop
  treating `Vec<u8>` as the default frame shape. Requires 3.1.3 and 3.2.2.
  - [ ] Leave only the compatibility examples that the rollout policy still
    promises.
- [ ] 4.1.2. Publish the migration guide and breaking-change summary. Requires
  1.2.3 and 3.2.3.
  - [ ] Include before-and-after code for middleware, hooks, serializers, and
    custom codecs.
  - [ ] Include a short decision summary for why the public surface changed.
- [ ] 4.1.3. Close or supersede ADRs 008 through 010 once implementation and
  documentation match the approved design. Requires 4.1.2.

### 4.2. Validate the final performance and correctness story

- [ ] 4.2.1. Run the benchmark suite and compare against the thresholds from
  1.2.2. Requires 3.2.2.
  - [ ] Record the results in a committed benchmark note or release document.
- [ ] 4.2.2. Run targeted downstream canaries. Requires 4.1.2.
  - [ ] Compile and test representative middleware, hook, and custom codec
    examples against the release candidate.
  - [ ] Success criteria: downstream breakage is either fixed or called out in
    the migration guide with a documented workaround.

## 5. Release rollout

### 5.1. Prepare the breaking release

- [ ] 5.1.1. Finalize the versioning plan for the public API break. Requires
  1.1.2 and 4.2.2.
  - [ ] Decide whether the change ships as the next major release or a pre-1.0
    point release under the project's semver policy.
- [ ] 5.1.2. Publish release notes, changelog entries, and upgrade guidance.
  Requires 4.1.2.
  - [ ] Call out the removed `Vec<u8>` contracts, the new zero-copy defaults,
    and any finite compatibility helpers.
- [ ] 5.1.3. Capture post-release follow-up work. Requires 5.1.2.
  - [ ] List any compatibility helpers scheduled for later removal.
  - [ ] List any deferred areas that intentionally remained on `Vec<u8>`.
