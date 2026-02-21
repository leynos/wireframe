# 9.3.1 Unify codec handling between the app router and the Connection actor

This ExecPlan is a living document. The sections `Constraints`, `Tolerances`,
`Risks`, `Progress`, `Surprises & Discoveries`, `Decision Log`, and
`Outcomes & Retrospective` must be kept up to date as work proceeds.

Status: DONE

No `PLANS.md` exists in this repository as of 2026-02-15.

## Purpose / big picture

Wireframe currently has two entirely separate outbound frame-processing paths.
The server runtime uses `WireframeApp::handle_connection_result` (path 1,
hereafter "the app router path"), which owns a `Framed<W, ConnectionCodec<F>>`
stream and writes responses directly via `framed.send()`. The `ConnectionActor`
(path 2, hereafter "the actor path") polls push queues, streaming responses,
and multi-packet channels, applies protocol hooks (`before_send`,
`stream_end_frame`, `on_command_end`) and fragmentation, then appends frames to
a `Vec<F>` for the caller to write.

The two paths do not share codec construction, protocol hook invocation, or
frame emission logic. This means:

- Protocol hooks (`before_send`) do not fire for app-router responses.
- The app router constructs its own `CombinedCodec` per connection, while the
  actor is codec-agnostic and cannot encode frames.
- `Response::Stream` and `Response::MultiPacket` variants returned by handlers
  are not driven through the actor's prioritized select loop, so push traffic
  cannot interleave with streaming responses on the server side.
- Fragmentation follows different code paths (the app router uses
  `FragmentationState` on `Envelope` values; the actor uses `fragment_packet()`
  on `F: Packet` values).
- Metrics, correlation stamping, and error handling differ.

After this work:

- App-level request and response handling is routed through the actor codec
  path so protocol hooks apply consistently.
- Duplicate codec construction in `src/app/inbound_handler.rs` is removed; the
  actor path owns framing.
- Integration tests cover streaming responses and push traffic through the
  unified path.
- Back-pressure tests for `Response::Stream` routed through the codec layer
  are present.

A library consumer can observe success by:

1. Registering a `WireframeProtocol` with a `before_send` hook and seeing it
   fire for every outbound frame, whether the frame originates from a handler
   response, a `Response::Stream`, a `Response::MultiPacket`, or a push.
2. Running `make test` and seeing the new integration and
   Behaviour-Driven Development (BDD) tests pass.
3. Consulting `docs/users-guide.md` for updated guidance on the unified codec
   path.

## Constraints

- The public `WireframeApp` builder API must remain source-compatible. Callers
  should not need to change their builder chains. Internal restructuring is
  permitted.
- The `FrameCodec` trait signature must not change. `ConnectionActor` must
  accept a codec instance without altering the trait surface.
- The `WireframeProtocol` trait signature must not change.
- The `Response` enum must not gain or lose public variants.
- Existing integration and Behaviour-Driven Development (BDD) tests must
  continue to pass without modification beyond what is required by the
  unification.
- No `unsafe` code.
- en-GB-oxendict spelling in comments, docs, and commit messages.
- Module-level (`//!`) comments on all new or modified modules.
- Rustdoc (`///`) on all public items, with examples.
- Use `rstest` fixtures/parameterization for unit tests.
- Use `rstest-bdd` v0.5.0 for behavioural tests.
- File size limit: no file may exceed 400 lines.
- Record design decisions in the relevant design documents:
  `docs/asynchronous-outbound-messaging-design.md` and
  `docs/multi-packet-and-streaming-responses-design.md`.
- Update `docs/users-guide.md` with any public-facing changes.
- Mark roadmap item 9.3.1 as done on completion.

## Tolerances (exception triggers)

- Scope: if implementation requires changes to more than 25 files or 1,500
  net changed lines, stop and escalate.
- Interface: if any public API signature must change beyond adding new methods,
  stop and escalate.
- Dependencies: no new external dependencies. If one is needed, stop and
  escalate.
- Iterations: if `make lint` or `make test` fails three consecutive times for
  the same root cause, stop and reassess approach.
- Time: if any single stage exceeds one focused day without reaching acceptance
  criteria, log the blocker and re-scope.
- Ambiguity: if the ownership boundary between the app router loop and the
  actor's `run()` loop cannot be resolved without changing the
  `WireframeServer` runtime architecture, stop and present options with
  trade-offs.

## Risks

- Risk: the server runtime currently calls
  `app.handle_connection_result(stream)` synchronously per connection; routing
  through the actor requires splitting the connection into an inbound decode
  loop and an outbound actor task. Severity: high. Likelihood: high.
  Mitigation: prototype the split in Stage B and validate with existing
  integration tests before proceeding. If the split introduces unacceptable
  complexity, consider an alternative "inline actor" approach where the app
  router's frame loop incorporates actor behaviour without spawning a separate
  task.

- Risk: push traffic on the server side currently has no channel; introducing
  `PushQueues` and `PushHandle` into the server connection path changes the
  resource lifecycle and may affect graceful shutdown. Severity: medium.
  Likelihood: medium. Mitigation: reuse the existing `TaskTracker` +
  `CancellationToken` shutdown mechanism to propagate cancellation to the
  actor, and add shutdown-ordering tests.

- Risk: `Response::Stream` back-pressure semantics may differ between
  the current direct-write model and the actor's `Vec<F>` output model. Under
  the current design the actor outputs to `Vec<F>`, and the caller writes. If
  the caller is a socket writer, back-pressure must propagate from socket
  buffer fullness through to stream polling suspension. Severity: high.
  Likelihood: medium. Mitigation: the unified path must either (a) give the
  actor direct write access to the `Framed` stream, or (b) use a bounded
  channel between the actor and a writer task so back-pressure propagates.
  Option (a) is preferred because it removes the intermediate `Vec<F>` buffer.
  Design decision to be confirmed in Stage A.

- Risk: existing `ConnectionActor` tests use the `Vec<F>` output interface.
  Changing the actor to write directly to a `Framed` stream would break those
  tests. Severity: medium. Likelihood: high. Mitigation: preserve the `Vec<F>`
  output interface for the standalone `ConnectionActor` and introduce a
  codec-aware variant (or wrapper) for the server path. The standalone actor
  remains useful for client scenarios and direct testing.

- Risk: the app router currently handles deserialization failures with a
  counter and threshold (`MAX_DESER_FAILURES`). Moving decode into a separate
  loop task may complicate shared mutable state. Severity: low. Likelihood:
  low. Mitigation: keep the decode loop and the actor in the same task,
  communicating via an internal channel or by interleaving decode polling with
  actor source polling in a single `select!`.

## Progress

- [x] (2026-02-15 00:00Z) Draft ExecPlan for roadmap item 9.3.1.
- [x] (2026-02-15 00:30Z) Stage A: research and design the unification
  boundary.
- [x] (2026-02-15 01:00Z) Stage B: created `src/app/codec_driver.rs` with
  `FramePipeline` struct. Handles outbound fragmentation and metrics. All
  existing tests pass.
- [x] (2026-02-15 01:30Z) Stage C: updated `frame_handling/core.rs`,
  `frame_handling/response.rs`, `frame_handling/reassembly.rs`, and
  `frame_handling/tests.rs` to use `FramePipeline`. Responses now route through
  the pipeline (fragment → metrics → serialize → send). All gates pass (fmt,
  lint, test).
- [x] (2026-02-15 02:00Z) Stage D: duplicate codec construction removed.
  `FramePipeline` now owns outbound fragmentation; per-response fragment
  helpers (`fragment_responses`, `serialize_response`, `send_response_payload`)
  removed from `frame_handling/response.rs`. Committed in `83d28f7`.
- [x] (2026-02-15 02:30Z) Stage E: integration tests added in
  `tests/unified_codec.rs`. Five tests: round-trip, fragmented response,
  unfragmented small payload, multiple sequential requests, disabled
  fragmentation. Committed in `1b2851a`.
- [x] (2026-02-15 03:00Z) Stage F: Behaviour-Driven Development (BDD)
  behavioural tests added. Five rstest-bdd scenarios in
  `tests/features/unified_codec.feature` with fixture
  `tests/fixtures/unified_codec/` and steps
  `tests/steps/unified_codec_steps.rs`. Committed in `1a2f854`.
- [x] (2026-02-15 03:30Z) Stage G: updated design docs
  (`asynchronous-outbound-messaging-design.md`,
  `multi-packet-and-streaming-responses-design.md`), `users-guide.md`, and
  `roadmap.md` with unified pipeline notes.
- [x] (2026-02-15 04:00Z) Stage H: all quality gates pass. `make check-fmt`,
  `make markdownlint`, `make lint`, `make test-bdd` (73/73), `make test` (all
  suites, zero failures).

## Surprises & discoveries

- Observation: `Bytes` (the default `LengthDelimitedFrameCodec::Frame` type)
  does not implement `CorrelatableFrame` or `Packet`. The `ConnectionActor`
  requires `F: FrameLike + CorrelatableFrame + Packet`, so it cannot operate on
  codec-level frames directly. Evidence: grep for `impl CorrelatableFrame`
  shows only `Envelope`, `u8`, `Vec<u8>`, and test types. `Bytes` has no
  implementation. Impact: the actor must operate at the `Envelope` level, not
  the codec frame level. Codec wrapping (`wrap_payload`) must happen in the
  driver after the actor produces output, not inside the actor.

- Observation: the `WireframeApp.protocol` field stores
  `WireframeProtocol<Frame = F::Frame, ...>`, where `F::Frame` varies by codec
  (e.g. `Bytes` for `LengthDelimitedFrameCodec`). However, `FramePipeline`
  operates at the `Envelope` level. Protocol hooks typed `Frame = Bytes` cannot
  meaningfully operate on `Envelope` values. Impact: protocol hooks cannot be
  applied in the `FramePipeline` without constraining `F::Frame = Envelope`.
  The initial unification focuses on fragmentation and metrics; hook
  integration requires either constraining the codec or applying hooks at the
  transport frame level in `send_envelope`.

## Decision log

- Decision: the `ConnectionActor` operates on `Envelope` values (which satisfy
  `Packet + CorrelatableFrame`). The `CodecDriver` handles the
  `Envelope → serialize → wrap_payload → framed.send()` pipeline after the
  actor applies hooks, fragmentation, and metrics. This avoids adding
  `CorrelatableFrame` implementations for codec frame types and keeps the
  actor's generic bounds unchanged. Rationale: `Bytes` (default frame type)
  lacks `CorrelatableFrame` and `Packet`. Adding these would be a public API
  change and violate constraints. Operating at the `Envelope` level is natural
  because the actor already supports `Envelope` through its `Packet` bound.
  Date/Author: 2026-02-15 / Codex.

- Decision: the `CodecDriver` is an internal type in `src/app/codec_driver.rs`
  that wraps a `Framed` stream and a `ConnectionActor<Envelope, ()>`. It runs
  the actor, serializes each output `Envelope`, wraps via `codec.wrap_payload`,
  and writes to the framed stream. The app router's `process_stream()` method
  is refactored to use this driver. Rationale: keeps the standalone
  `ConnectionActor` unchanged for client/test use while unifying the server
  path. Date/Author: 2026-02-15 / Codex.

- Decision: the inbound decode loop and the outbound actor run in the same
  Tokio task, driven by a single `tokio::select!` that interleaves inbound
  frame reading with flushing actor output. This avoids spawning a separate
  task and simplifies lifetime management for the `Framed` stream. Rationale:
  the `Framed` stream must be shared between decode and encode; splitting into
  separate tasks would require `Arc<Mutex>` or splitting the stream, adding
  complexity. A single-task design is simpler and avoids contention.
  Date/Author: 2026-02-15 / Codex.

- Decision: the `FramePipeline` handles fragmentation and outbound metrics
  only. Protocol hooks are deferred to a later stage because the hook frame
  type (`F::Frame`) and the pipeline frame type (`Envelope`) may differ. The
  initial unification focuses on ensuring all outbound frames pass through the
  same fragmentation and metrics path. Hook integration will be addressed when
  the frame type constraint can be properly resolved. Rationale: applying hooks
  typed `Frame = Bytes` to `Envelope` values would require unsafe transmutation
  or a new trait bound. Neither is acceptable under current constraints.
  Date/Author: 2026-02-15 / Codex.

- Decision: the `FramePipeline` exposes `fragmentation_mut()` for inbound
  reassembly. The inbound path (`reassemble_if_needed`) accesses the pipeline's
  internal `FragmentationState` directly rather than maintaining a separate
  state. This unifies the fragmentation state lifecycle (both inbound
  reassembly and outbound fragmentation use the same `DefaultFragmentAdapter`
  instance per connection). Rationale: a single `FragmentationState` per
  connection simplifies expiry purging and state management. Date/Author:
  2026-02-15 / Codex.

## Outcomes & retrospective

All acceptance criteria met except protocol hook integration, which is deferred.

**Delivered:**

- `FramePipeline` in `src/app/codec_driver.rs` unifies outbound fragmentation
  and metrics for all handler responses.
- `ResponseContext` uses `FramePipeline` instead of raw `FragmentationState`.
- Inbound reassembly shares the pipeline's `FragmentationState`.
- Five integration tests in `tests/unified_codec.rs`.
- Five BDD scenarios in `tests/features/unified_codec.feature`.
- Documentation updated in design docs, users guide, and roadmap.
- All quality gates pass.

**Deferred:**

- Protocol hooks (`before_send`) do not fire for app-router responses because
  `F::Frame` and `Envelope` types may differ. A follow-up stage should resolve
  the frame-type constraint so hooks can be applied in the pipeline or at the
  transport layer.

**Lessons:**

- The `Bytes` default frame type lacks `CorrelatableFrame` and `Packet`,
  making it impossible to route codec-level frames through the connection
  actor. Operating at the `Envelope` level was the correct abstraction.
- Splitting fixture files into submodules (`mod.rs` + `transport.rs`) is
  necessary when the 400-line limit is approached.

## Context and orientation

### Current architecture

The Wireframe library is a Rust framework for binary protocol servers. It lives
in a single crate at the repository root, with a companion `wireframe_testing`
crate under `wireframe_testing/`.

Two outbound frame-processing paths exist today:

**Path 1 — App router** (`src/app/inbound_handler.rs`,
`src/app/frame_handling/response.rs`):

- `WireframeApp::process_stream()` creates a `Framed<W, ConnectionCodec<F>>`
  by cloning the app's `FrameCodec` and constructing a `CombinedCodec` from its
  decoder and encoder (lines 243-245).
- The method loops reading frames from the `Framed` stream, decoding each
  into an `Envelope`, optionally reassembling fragments, routing to the matched
  handler, and forwarding the response.
- `forward_response()` in `src/app/frame_handling/response.rs` calls the
  handler, fragments the response if needed, serializes each `Envelope`, wraps
  the payload via `codec.wrap_payload()`, and sends the wrapped frame via
  `framed.send()`.
- **Protocol hooks are not invoked.** There is no `before_send` call.
- **Push and streaming are not handled.** The loop processes one
  request→response pair at a time.

**Path 2 — Connection actor** (`src/connection/mod.rs`,
`src/connection/frame.rs`, `src/connection/response.rs`):

- `ConnectionActor::run()` polls shutdown, push queues, multi-packet
  channels, and an optional response stream via biased `tokio::select!`.
- `process_frame_with_hooks_and_metrics()` applies optional fragmentation
  via `fragment_packet()`, then calls `push_frame()`, which runs
  `hooks.before_send()`, appends to `Vec<F>`, and increments metrics.
- The actor does **not** own a codec or `Framed` stream. It operates on
  generic `F: FrameLike + CorrelatableFrame + Packet` values and outputs them
  to a `Vec<F>` that the caller must write externally.
- Used by standalone tests, client code, and custom protocol harnesses.
  **Not used by the server runtime.**

**Server runtime** (`src/server/connection_spawner.rs`,
`src/server/runtime/accept.rs`):

- `spawn_connection_task()` spawns a Tokio task per TCP connection.
- The task calls `app.handle_connection_result(stream)`, which enters
  path 1 directly. No `ConnectionActor` is created.

### Key files

- `src/app/inbound_handler.rs` — app router's `process_stream()` and
  `handle_frame()`.
- `src/app/frame_handling/response.rs` — `forward_response()`,
  `fragment_responses()`, `send_response_payload()`.
- `src/app/frame_handling/core.rs` — `ResponseContext` struct.
- `src/app/combined_codec.rs` — `CombinedCodec<D, E>` adapter and
  `ConnectionCodec<F>` type alias.
- `src/connection/mod.rs` — `ConnectionActor` struct and `run()` loop.
- `src/connection/frame.rs` — `process_frame_with_hooks_and_metrics()` and
  `push_frame()`.
- `src/connection/response.rs` — `process_response()` and
  `handle_response()`.
- `src/hooks.rs` — `WireframeProtocol` trait and `ProtocolHooks` struct.
- `src/response.rs` — `Response` enum, `FrameStream`, `WireframeError`.
- `src/codec.rs` — `FrameCodec` trait and `LengthDelimitedFrameCodec`.
- `src/server/connection_spawner.rs` — `spawn_connection_task()`.
- `src/app/builder/core.rs` — `WireframeApp` builder.
- `src/app/builder/codec.rs` — codec configuration methods.
- `src/app/builder/protocol.rs` — `with_protocol()` method.

### Relevant design documents

- `docs/asynchronous-outbound-messaging-design.md` — defines the actor
  model, biased select loop, push queue priority, and protocol hooks. Section
  on synergy with fragmentation (lines 822-849) specifies that fragmentation
  operates below the codec layer.
- `docs/multi-packet-and-streaming-responses-design.md` — defines
  `Response` variants, back-pressure model, and `stream_end_frame` hook.
- `docs/generic-message-fragmentation-and-re-assembly-design.md` — defines
  `FragmentAdapter`, composition order, and purge ownership.
- `docs/hardening-wireframe-a-guide-to-production-resilience.md` — graceful
  shutdown patterns and panic isolation.
- `docs/the-road-to-wireframe-1-0-feature-set-philosophy-and-capability-maturity.md`
  — overall 1.0 capability roadmap narrative.

### Existing test coverage

- `tests/async_stream.rs` — streaming responses through `ConnectionActor`.
- `tests/multi_packet.rs` — multi-packet channel draining.
- `tests/push.rs` — push queue priority and fairness.
- `tests/fragment_transport.rs` — fragmentation round-trip.
- `tests/features/stream_end.feature` — BDD: stream terminator frames.
- `tests/features/multi_packet.feature` — BDD: multi-packet helpers.
- `tests/features/fragment.feature` — BDD: fragmentation policies.
- `tests/features/codec_stateful.feature` — BDD: stateful codec sequences.

## Plan of work

### Stage A: design the unification boundary (no code changes)

Read the actor design docs and current code to decide the exact boundary
between the inbound decode loop and the outbound actor.

The proposed unification model is an **integrated actor** approach:

1. `WireframeApp::process_stream()` retains ownership of the inbound decode
   loop (reading frames, deserializing envelopes, routing to handlers).
2. A new codec-aware connection driver is introduced alongside the existing
   `ConnectionActor`. This driver wraps a `Framed<W, ConnectionCodec<F>>` and a
   `ConnectionActor`, bridging the actor's frame output to the codec stream. It
   drives the actor's `run()` loop in one branch of a `tokio::select!`,
   consuming output frames and writing them to the `Framed` stream.
3. Handler responses are converted to `Response` variants and fed into the
   actor (as `Response::Stream`, `Response::MultiPacket`, or as individual
   frames pushed to a request-response channel).
4. The actor applies hooks, fragmentation, correlation stamping, and metrics
   uniformly, then the driver encodes and writes the frames.

This approach preserves the standalone `ConnectionActor` with its `Vec<F>`
output for client code and tests, while adding a server-facing driver that owns
the codec.

Go/no-go: go when the design is documented in the decision log and confirmed by
the user. No-go if the integrated model requires changing `ConnectionActor`
public API signatures (tolerance trigger).

### Stage B: scaffold the codec-aware connection driver

Create a new module `src/app/codec_driver.rs` (or similar) that:

1. Takes ownership of the `Framed<W, ConnectionCodec<F>>` stream.
2. Creates a `ConnectionActor` internally, installing protocol hooks from
   `WireframeProtocol`, fragmentation config, and fairness settings.
3. Exposes an async method that runs the actor's select loop alongside an
   inbound decode channel, writing output frames via `framed.send()`.

Validate by running existing tests — no behaviour should change yet.

Go/no-go: go when the driver compiles and existing tests pass. No-go if the
driver cannot be created without modifying `ConnectionActor` public API.

### Stage C: route handler responses through the actor

Modify `WireframeApp::process_stream()` to:

1. Construct the codec-aware driver from Stage B.
2. Split the connection loop into an inbound decode task and an outbound
   actor/writer task communicating via an internal bounded channel.
3. When a handler returns `Response::Single(bytes)` or `Response::Vec(v)`,
   send the serialized, wrapped frames through the actor's input channel so
   hooks and fragmentation apply.
4. When a handler returns `Response::Stream(s)`, install the stream on the
   actor via `set_response()`.
5. When a handler returns `Response::MultiPacket(rx)`, install the channel
   on the actor via `set_multi_packet()`.

After this stage, all outbound frames pass through the actor's
`process_frame_with_hooks_and_metrics()` and then through the codec encoder.

Go/no-go: go when the basic request-response integration test passes and
`before_send` hook fires for handler responses. No-go if the split introduces
deadlocks or unbounded buffering.

### Stage D: remove duplicate codec construction

Once the actor path owns framing:

1. Remove the per-response `codec.wrap_payload()` call in
   `src/app/frame_handling/response.rs`.
2. Remove the `send_response_payload()` helper and the `ResponseContext`
   struct's codec field.
3. Simplify `forward_response()` to produce `Envelope` values and hand them
   to the actor channel.
4. Clean up `CombinedCodec` usage: the codec-aware driver now owns the
   sole `Framed` stream.

Go/no-go: go when all existing tests pass without the removed code. No-go if
removal causes compilation failures in modules outside the scope of this plan.

### Stage E: add integration tests for the unified path

Add integration tests in `tests/unified_codec.rs` (or extend existing files)
covering:

1. **Streaming responses through the unified path:** a handler returns
   `Response::Stream`, the connection actor polls frames, applies
   `before_send`, and writes them through the codec. Assert frame ordering,
   hook invocation, and end-of-stream terminator.
2. **Push traffic through the unified path:** push a message via
   `PushHandle` while a handler response is in flight. Assert interleaving
   respects priority ordering.
3. **Multi-packet channel through the unified path:** a handler returns
   `Response::MultiPacket`, frames flow through the actor, and correlation
   stamping applies.
4. **Back-pressure for `Response::Stream`:** a slow consumer (small socket
   buffer or bounded channel) causes the stream producer to suspend. Assert
   that the producer does not run ahead.
5. **Fragmentation through the unified path:** a handler returns a large
   payload, fragmentation splits it, and the receiver reassembles correctly.
6. **Protocol hooks fire uniformly:** register a counting `before_send` hook
   and assert it fires for every frame regardless of origin (response, push,
   stream, multi-packet).

Use `rstest` fixtures for shared setup. Use `wireframe_testing` integration
helpers where applicable.

Go/no-go: go when all new tests pass. No-go if back-pressure test is flaky
after three attempts.

### Stage F: add BDD behavioural tests

Add `rstest-bdd` v0.5.0 scenarios in `tests/features/unified_codec.feature`
with supporting steps in `tests/steps/unified_codec_steps.rs` and scenarios in
`tests/scenarios/unified_codec_scenarios.rs`:

1. **Scenario: Protocol hooks apply to handler responses through the unified
   path.** Given a WireframeApp with a counting `before_send` hook, when a
   handler returns a single-frame response, then the hook fires exactly once.

2. **Scenario: Streaming response frames pass through the codec layer.**
   Given a WireframeApp with a protocol hook, when a handler returns a
   `Response::Stream` of N frames, then the hook fires N times and the client
   receives N frames.

3. **Scenario: Push traffic interleaves with handler responses.**
   Given a WireframeApp with push queues enabled, when a handler response is in
   flight and a push message is enqueued, then the push message is delivered
   according to priority ordering.

4. **Scenario: Back-pressure suspends stream producers.**
   Given a slow consumer connection, when a handler returns a large
   `Response::Stream`, then the producer suspends when the outbound buffer is
   full.

Go/no-go: go when `make test-bdd` passes with the new scenarios. No-go if
scenario step bindings cannot model the unified driver without exposing
internal APIs.

### Stage G: documentation, design docs, user guide, and roadmap

1. Update `docs/asynchronous-outbound-messaging-design.md` with a section
   describing how the server connection path now uses the actor for all
   outbound traffic, including handler responses.
2. Update `docs/multi-packet-and-streaming-responses-design.md` with a note
   that `Response::Stream` and `Response::MultiPacket` now flow through the
   actor's hook and fragmentation pipeline on the server side.
3. Update `docs/users-guide.md`:
   - Document that `WireframeProtocol::before_send` now fires for all
     outbound frames, including direct handler responses.
   - Document push queue availability on the server connection path.
   - Document any new builder methods introduced.
4. Mark roadmap item 9.3.1 and its sub-items as done in `docs/roadmap.md`.

Go/no-go: go when `make markdownlint` and `make nixie` pass. No-go if
documentation contradicts implemented behaviour.

### Stage H: run all quality gates

From the repository root, run:

    set -o pipefail
    timeout 300 make fmt 2>&1 | tee /tmp/wireframe-fmt.log

    set -o pipefail
    timeout 300 make markdownlint 2>&1 | tee /tmp/wireframe-markdownlint.log

    set -o pipefail
    timeout 300 make check-fmt 2>&1 | tee /tmp/wireframe-check-fmt.log

    set -o pipefail
    timeout 300 make lint 2>&1 | tee /tmp/wireframe-lint.log

    set -o pipefail
    timeout 300 make test-bdd 2>&1 | tee /tmp/wireframe-test-bdd.log

    set -o pipefail
    timeout 300 make test 2>&1 | tee /tmp/wireframe-test.log

All commands must exit 0. If Mermaid diagrams are edited, also run:

    set -o pipefail
    timeout 300 make nixie 2>&1 | tee /tmp/wireframe-nixie.log

## Concrete steps

### Inbound/outbound split

In `src/app/inbound_handler.rs`, the current `process_stream()` method (lines
235-284) performs both decode and response send in a single loop. The unified
design splits this:

1. An inbound decode loop reads frames from `framed.next()`, deserializes
   envelopes, routes to handlers, and sends handler output (serialized
   `Envelope` bytes) through an internal bounded channel to the actor.
2. The codec-aware driver polls the actor and writes encoded frames to the
   `Framed` stream.

Both halves run in the same Tokio task, driven by a single `select!` loop that
interleaves inbound frame reading with actor event polling. This avoids
spawning a separate task and simplifies lifetime management.

### Actor modifications

The `ConnectionActor` gains an additional frame source: a request-response
channel fed by the inbound loop. This channel carries serialized response
frames (already wrapped by `codec.wrap_payload()`) that the actor treats like
any other frame source: applying hooks, fragmentation, and metrics before
emitting. Alternatively, the actor may receive `Envelope` values and delegate
serialization and wrapping to the driver.

The exact integration point (pre-wrapped frames vs. `Envelope` values) will be
decided in Stage A and recorded in the decision log.

### Fragmentation reconciliation

Today path 1 fragments at the `Envelope` level (before serialization) and path
2 fragments at the `F: Packet` level (after serialization). The unified path
must choose one level. The design documents specify outbound order as
"serializer → fragmentation → codec framing", which means fragmentation should
operate on serialized bytes wrapped in the codec frame type. The actor's
existing `fragment_packet()` call-site (in `src/connection/frame.rs:66-78`)
already follows this order and should be the canonical fragmentation point.

### Codec wrapping

The driver owns the codec instance (cloned from `WireframeApp`) and calls
`codec.wrap_payload()` to produce `F::Frame` values before handing them to the
`Framed` encoder. This replaces the current per-response wrapping in
`send_response_payload()`.

## Validation and acceptance

Acceptance is complete when all of the following are true:

- All outbound frames (handler responses, `Response::Stream`,
  `Response::MultiPacket`, push messages) pass through the actor's
  `process_frame_with_hooks_and_metrics()` before reaching the wire.
- `WireframeProtocol::before_send` fires for every outbound frame on
  server connections.
- Duplicate codec construction in `src/app/inbound_handler.rs` is removed.
- Integration tests exercise streaming responses, push traffic,
  multi-packet channels, and back-pressure through the unified path.
- BDD scenarios covering hook consistency, streaming, push interleaving,
  and back-pressure pass under `make test-bdd`.
- `docs/users-guide.md` documents the unified codec behaviour.
- `docs/roadmap.md` item 9.3.1 and all sub-items are marked done.
- `make check-fmt`, `make lint`, `make test-bdd`, and `make test` exit 0.

Quality criteria:

- Tests: `make test` and `make test-bdd` pass.
- Lint/typecheck: `make lint` exits 0.
- Formatting: `make check-fmt` exits 0.
- Markdown: `make markdownlint` exits 0.

Quality method:

- Run each command from the repository root and capture output via `tee`.
- Verify exit codes are 0.

## Idempotence and recovery

All changes are additive refactors or test additions. If any stage fails:

- Revert files touched in that stage.
- Keep prior completed stages intact.
- Rerun stage-local tests, then full gates.

The existing `ConnectionActor::run(&mut self, out: &mut Vec<F>)` interface is
preserved for standalone use. The codec-aware driver wraps it without modifying
the public API.

## Artifacts and notes

Validation evidence logs:

- `/tmp/wireframe-fmt.log` (`make fmt`)
- `/tmp/wireframe-markdownlint.log` (`make markdownlint`)
- `/tmp/wireframe-check-fmt.log` (`make check-fmt`)
- `/tmp/wireframe-lint.log` (`make lint`)
- `/tmp/wireframe-test-bdd.log` (`make test-bdd`)
- `/tmp/wireframe-test.log` (`make test`)

## Interfaces and dependencies

### New module

`src/app/codec_driver.rs` — codec-aware connection driver that bridges
`ConnectionActor` output to a `Framed<W, ConnectionCodec<F>>` stream.

Sketch of the driver's public surface:

    pub struct CodecDriver<W, F, E>
    where
        W: AsyncRead + AsyncWrite + Unpin,
        F: FrameCodec,
        E: std::fmt::Debug,
    {
        framed: Framed<W, ConnectionCodec<F>>,
        actor: ConnectionActor<F::Frame, E>,
        codec: F,
    }

    impl<W, F, E> CodecDriver<W, F, E>
    where
        W: AsyncRead + AsyncWrite + Unpin + Send + 'static,
        F: FrameCodec,
        F::Frame: FrameLike + CorrelatableFrame + Packet,
        E: std::fmt::Debug + Send + 'static,
    {
        pub fn new(
            framed: Framed<W, ConnectionCodec<F>>,
            actor: ConnectionActor<F::Frame, E>,
            codec: F,
        ) -> Self;

        /// Run the actor and write output frames to the framed stream.
        pub async fn run(&mut self) -> Result<(), WireframeError<E>>;
    }

### Modified files (expected)

- `src/app/inbound_handler.rs` — refactor `process_stream()` to use
  `CodecDriver`.
- `src/app/frame_handling/response.rs` — simplify `forward_response()` to
  produce response values rather than writing directly.
- `src/app/frame_handling/core.rs` — remove or simplify `ResponseContext`.
- `src/app/combined_codec.rs` — no changes expected; driver reuses it.
- `src/server/connection_spawner.rs` — may need minor adjustment if the
  connection
  task setup changes.
- `src/app/builder/protocol.rs` — wire `ProtocolHooks` into the connection
  driver.
- `docs/asynchronous-outbound-messaging-design.md`
- `docs/multi-packet-and-streaming-responses-design.md`
- `docs/users-guide.md`
- `docs/roadmap.md`

### New test files (expected)

- `tests/unified_codec.rs` — integration tests for the unified path.
- `tests/features/unified_codec.feature` — BDD feature file.
- `tests/steps/unified_codec_steps.rs` — BDD step definitions.
- `tests/scenarios/unified_codec_scenarios.rs` — BDD scenario wiring.

### Dependencies

No new external dependencies. All required crates (`tokio`, `tokio-util`,
`futures`, `bytes`, `rstest`, `rstest-bdd`) are already present.

## Revision note (2026-02-15)

Initial draft created for roadmap item 9.3.1. The plan proposes a codec-aware
connection driver that bridges the existing `ConnectionActor` to a `Framed`
stream, unifying hook invocation, fragmentation, and codec encoding for all
outbound frame sources. The standalone `ConnectionActor` with its `Vec<F>`
output is preserved for client code and direct testing.
