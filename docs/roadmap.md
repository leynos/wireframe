# Wireframe combined development roadmap

This document outlines the development roadmap for the Wireframe library,
merging previous roadmap documents into a single source of truth. It details
the planned features, enhancements, and the overall trajectory towards a stable
and production-ready 1.0 release.

## Guiding principles

- Ergonomics: The library should be intuitive and easy to use.
- Performance: Maintain high performance and low overhead.
- Extensibility: Provide clear extension points, especially through middleware.
- Robustness: Ensure the library is resilient and handles errors gracefully.

## 1. Core functionality and API (complete)

This phase established the foundational components of the Wireframe server and
the request and response lifecycle.

### 1.1. Protocol definition

- [x] 1.1.1. Define the basic frame structure for network communication
  (`src/frame/mod.rs`).
- [x] 1.1.2. Implement preamble validation for versioning and compatibility
  (`src/preamble.rs`, `tests/preamble.rs`).

### 1.2. Core server implementation

- [x] 1.2.1. Implement the `Server` struct with `bind` and `run` methods
  (`src/server.rs`).
- [x] 1.2.2. Handle incoming TCP connections and spawn connection-handling
  tasks (`src/connection.rs`).
- [x] 1.2.3. Define `Request`, `Response`, and `Message` structs
  (`src/message.rs`, `src/response.rs`).

### 1.3. Routing and handlers

- [x] 1.3.1. Implement a basic routing mechanism to map requests to handler
  functions (`src/app/`).
- [x] 1.3.2. Support handler functions with flexible, type-safe extractors
  (`src/extractor.rs`).

### 1.4. Error handling

- [x] 1.4.1. Establish a comprehensive set of error types.
- [x] 1.4.2. Implement `From` conversions for ergonomic error handling.
- [x] 1.4.3. Ensure `Display` is implemented for all public error types
  (`tests/error_display.rs`).

### 1.5. Basic testing

- [x] 1.5.1. Develop integration tests for core request and response
  functionality (`tests/server.rs`, `tests/routes.rs`).

## 2. Middleware and extensibility (complete)

This phase focused on building the middleware system, a key feature for
extensibility.

### 2.1. Middleware trait

- [x] 2.1.1. Design and implement the `Middleware` trait
  (`src/middleware.rs`).
- [x] 2.1.2. Define `Next` to allow middleware to pass control to the next in
  the chain.

### 2.2. Middleware integration

- [x] 2.2.1. Integrate the middleware processing loop into the `App` and
  `Connection` logic.
- [x] 2.2.2. Ensure middleware can modify requests and responses.

### 2.3. Testing

- [x] 2.3.1. Write tests to verify middleware functionality, including correct
  execution order (`tests/middleware.rs`, `tests/middleware_order.rs`).

## 3. Push messaging and async operations (complete)

This phase introduced capabilities for asynchronous, server-initiated
communication and streaming.

### 3.1. Push messaging

- [x] 3.1.1. Implement the `Push` mechanism for sending messages from server to
  client without a direct request (`src/push.rs`).
- [x] 3.1.2. Develop `PushPolicies` for broadcasting messages to all or a
  subset of clients.
- [x] 3.1.3. Create tests for various push scenarios (`tests/push.rs`,
  `tests/push_policies.rs`).

### 3.2. Async stream responses

- [x] 3.2.1. Enable handlers to return `impl Stream` of messages
  (`src/response.rs`).
- [x] 3.2.2. Implement the client and server-side logic to handle streaming
  responses (`examples/async_stream.rs`, `tests/async_stream.rs`).

## 4. Advanced connection handling and state (complete)

This phase added sophisticated state management and improved connection
lifecycle control.

### 4.1. Session management

- [x] 4.1.1. Implement a `Session` struct to hold connection-specific state
  (`src/session.rs`).
- [x] 4.1.2. Create a `SessionRegistry` for managing all active sessions
  (`tests/session_registry.rs`).
- [x] 4.1.3. Provide `State` and `Data` extractors for accessing shared and
  session-specific data.

### 4.2. Lifecycle hooks

- [x] 4.2.1. Implement `on_connect` and `on_disconnect` hooks for session
  initialization and cleanup (`src/hooks.rs`).
- [x] 4.2.2. Write tests to verify lifecycle hook behaviour
  (`tests/lifecycle.rs`).

### 4.3. Graceful shutdown

- [x] 4.3.1. Implement a graceful shutdown mechanism for the server, allowing
  active connections to complete their work.

## 5. Production hardening and observability (in progress)

This phase focuses on making the library robust, debuggable, and ready for
production environments.

### 5.1. Logging

- [x] 5.1.1. Integrate `tracing` throughout the library for structured,
  level-based logging.
- [x] 5.1.2. Create a helper crate for test logging setup
  (`wireframe_testing/src/logging.rs`).

### 5.2. Metrics and observability

- [x] 5.2.1. Expose key operational metrics (e.g., active connections, messages
  per second, error rates).
- [x] 5.2.2. Provide an integration guide for popular monitoring systems
  (e.g., Prometheus).

### 5.3. Packet decomposition

- [x] 5.3.1. Introduce `PacketParts` to replace tuple-based packet handling.
- [x] 5.3.2. Treat `correlation_id` as `Option<u64>` so `None` denotes an
  unsolicited event or server-initiated push.

### 5.4. Advanced error handling

- [x] 5.4.1. Implement panic handlers in connection tasks to prevent a single
  connection from crashing the server.

### 5.5. Testing

- [x] 5.5.1. Implement fuzz testing for the protocol parser
  (`tests/advanced/interaction_fuzz.rs`).
- [x] 5.5.2. Use `loom` for concurrency testing of shared state
  (`crates/wireframe-loom`).

## 6. Multi-packet streaming responses (priority focus)

This is the next major feature set. It enables a handler to return multiple,
distinct messages over time in response to a single request, forming a logical
stream.

### 6.1. Protocol enhancement

- [x] 6.1.1. Add a `correlation_id` field to the `Frame` header. For a request,
  this is the unique request ID. For each message in a multi-packet response,
  this ID must match the original request's ID.
- [x] 6.1.2. Define a mechanism to signal the end of a multi-packet stream,
  such as a frame with a specific flag and no payload.

### 6.2. Core library implementation

- [x] 6.2.1. Introduce a `Response::MultiPacket` variant that contains a
  channel `Receiver<Message>`.
- [x] 6.2.2. Modify the `Connection` actor: upon receiving
  `Response::MultiPacket`, it should consume messages from the receiver and
  send each one as a `Frame`.
  - [x] Extend the outbound `select!` loop to own the receiver so multi-packet
    responses share the same back-pressure and shutdown handling as other frame
    sources.
  - [x] Convert each received `Message` into a `Frame` via the existing
    serialization helpers rather than bypassing protocol hooks or metrics.
  - [x] Emit tracing and metrics for each forwarded frame so streaming traffic
    remains visible to observability pipelines.
- [x] 6.2.3. Ensure each sent frame carries the correct `correlation_id` from
  the initial request.
  - [x] Capture the originating request's `correlation_id` before handing
    control to the multi-packet dispatcher.
  - [x] Stamp the stored `correlation_id` onto every frame emitted from the
    channel before it is queued for transmission.
  - [x] Guard against accidental omission by asserting in debug builds and
    covering the behaviour with targeted tests.
- [x] 6.2.4. When the channel closes, send the end-of-stream marker frame.
  - [x] Detect channel closure (`None` from `recv`) and log the termination
    reason for operational insight.
  - [x] Send the designated end-of-stream marker frame through the same send
    path, reusing the request's `correlation_id`.
  - [x] Notify protocol lifecycle hooks so higher layers can tidy any
    per-request state when a stream drains naturally.

### 6.3. Ergonomics and API

- [x] 6.3.1. Provide a helper (for example `Response::with_channel`) that
  returns a bounded channel sender alongside a `Response::MultiPacket` so
  handlers can opt into streaming ergonomically (see
  [ADR 0001](adrs/adr-0001-multi-packet-streaming-response-api.md)).
- [x] 6.3.2. Update the multi-packet design documentation and user guide with
  tuple return examples that explain initial-frame handling, back-pressure, and
  graceful termination (see
  [ADR 0001](adrs/adr-0001-multi-packet-streaming-response-api.md)).
- [x] 6.3.3. Add an example handler (or test fixture) demonstrating spawning a
  background task that pushes frames through the returned sender while the
  connection actor manages delivery.

### 6.4. Testing

- [x] 6.4.1. Develop integration tests where a client sends one request and
  receives multiple, correlated response messages.
- [x] 6.4.2. Test that the end-of-stream marker is sent correctly and handled
  by the client.
- [x] 6.4.3. Test client-side handling of interleaved multi-packet responses
  from different requests.

## 7. Transport-level fragmentation and reassembly (complete)

This phase handles the transport of a single message that is too large to fit
into a single frame, making the process transparent to the application logic.

### 7.1. Core fragmentation and reassembly layer

- [x] 7.1.1. Define a generic `Fragment` header or metadata containing
  `message_id`, `fragment_index`, and `is_last_fragment` fields.
- [x] 7.1.2. Implement a `Fragmenter` to split a large `Message` into multiple
  `Frame`s, each with a `Fragment` header.
- [x] 7.1.3. Implement a `Reassembler` on the receiving end to collect
  fragments and reconstruct the original `Message`.
- [x] 7.1.4. Manage a reassembly buffer with timeouts to prevent resource
  exhaustion from incomplete messages.

### 7.2. Integration with core library

- [x] 7.2.1. Integrate the fragmentation and reassembly layer into the
  `Connection` actor's read and write paths.
- [x] 7.2.2. Ensure the fragmentation and reassembly logic is transparent to
  handler functions; they should continue to send and receive complete
  `Message` objects.

### 7.3. Testing

- [x] 7.3.1. Create unit tests for the `Fragmenter` and `Reassembler`.
- [x] 7.3.2. Develop integration tests sending and receiving large messages
  that require fragmentation.
- [x] 7.3.3. Test edge cases: out-of-order fragments, duplicate fragments, and
  reassembly timeouts.

## 8. Streaming requests and shared message assembly

This phase implements the decisions from
[ADR 0002](adrs/adr-0002-streaming-requests-and-shared-message-assembly.md),
adding first-class streaming request bodies, a generic message assembly
abstraction, and standardized per-connection memory budgets.

### 8.1. Streaming request bodies

- [x] 8.1.1. Implement `RequestParts` struct with `id`, `correlation_id`, and
  `metadata` fields.
- [x] 8.1.2. Implement `RequestBodyStream` type alias as a pinned, boxed
  stream of `Result<Bytes, std::io::Error>`.
- [x] 8.1.3. Add an `AsyncRead` adaptor for `RequestBodyStream` so protocol
  crates can reuse existing parsers.
- [x] 8.1.4. Integrate streaming request extraction with the handler dispatch
  path.
- [x] 8.1.5. Write tests for buffered-to-streaming fallback and back-pressure
  propagation.

### 8.2. MessageAssembler abstraction

- [x] 8.2.1. Define a `MessageAssembler` hook trait for protocol-specific
  multi-frame parsing.
- [x] 8.2.2. Implement per-frame header parsing with "first frame" versus
  "continuation frame" handling.
- [x] 8.2.3. Add message key support for multiplexing interleaved assemblies.
- [x] 8.2.4. Implement continuity validation (ordering, missing frames, and
  duplicate frames).
- [x] 8.2.5. Integrate with the connection actor's inbound path, applying
  after transport fragmentation.
- [x] 8.2.6. Write tests for interleaved assembly, ordering violations, and
  timeout behaviour.

### 8.3. Per-connection memory budgets

- [x] 8.3.1. Add `WireframeApp::memory_budgets(...)` builder method.
- [x] 8.3.2. Implement budget enforcement covering bytes per message, bytes
  per connection, and bytes across in-flight assemblies.
- [x] 8.3.3. Implement soft limit (back-pressure by pausing reads) behaviour.
- [x] 8.3.4. Implement hard cap (abort early, release partial state, surface
  `InvalidData`) behaviour.
- [x] 8.3.5. Define derived defaults based on `buffer_capacity` when budgets
  are not set explicitly.
- [x] 8.3.6. Write tests for budget enforcement, back-pressure, and cleanup
  semantics.

### 8.4. Transport helper

- [x] 8.4.1. Implement `send_streaming(frame_header, body_reader)` helper.
- [x] 8.4.2. Add chunk size configuration with protocol-provided headers.
- [x] 8.4.3. Implement timeout handling (return `TimedOut`, stop emitting
  frames).
- [x] 8.4.4. Integrate with connection actor instrumentation and hooks.
- [x] 8.4.5. Write tests for partial send failures and timeout behaviour.

### 8.5. Testkit utilities

- [x] 8.5.1. Add utilities for feeding partial frames or fragments into an
  in-process app.
- [x] 8.5.2. Add slow reader and writer simulation for back-pressure testing.
- [x] 8.5.3. Add deterministic assertion helpers for reassembly outcomes.
- [x] 8.5.4. Export utilities as `wireframe::testkit` behind a dedicated
  feature.

### 8.6. Documentation

- [x] 8.6.1. Update `generic-message-fragmentation-and-re-assembly-design.md`
  with composition guidance.
- [x] 8.6.2. Update `multi-packet-and-streaming-responses-design.md` with a
  streaming request body section.
- [x] 8.6.3. Update
  `the-road-to-wireframe-1-0-feature-set-philosophy-and-capability-maturity.md`
  with MessageAssembler and budget details.

## 9. Pluggable protocol codec hardening

This phase addresses follow-up work discovered during the pluggable codec
implementation, focusing on stateful framing, error taxonomy, and reliability.
The work here will feed into a subsequent round of design document updates that
clarify codec recovery policies, fragmentation behaviour, and serializer
integration boundaries.

### 9.1. Codec enhancements

- [x] 9.1.1. Make `FrameCodec::wrap_payload` instance-aware for stateful
  codecs.
  - [x] Update the trait to accept `&self` and a `Bytes` payload to reduce
    copies, then document the change in
    `adrs/adr-0004-pluggable-protocol-codecs.md`.
  - [x] Update `LengthDelimitedFrameCodec` and any adaptors to use the new
    payload type.
  - [x] Reuse a per-connection encoder, so sequence counters can advance
    deterministically.
- [x] 9.1.2. Introduce a `CodecError` taxonomy.
  - [x] Add a `CodecError` enum separating framing, protocol, and IO failures.
  - [x] Extend `WireframeError` to surface `CodecError` and add structured
    logging fields for codec failures.
  - [x] Define recovery policy hooks for malformed frames (drop, quarantine, or
    disconnect) and document the default behaviour.
  - [x] Define how EOF mid-frame is surfaced to handlers or protocol hooks, and
    add tests for partial-frame closure handling.
  - [x] Add tests that validate error propagation, recovery policy, and
    structured logging fields.
- [x] 9.1.3. Enable zero-copy payload extraction for codecs.
  - [x] Add `FrameCodec::frame_payload_bytes` method returning `Bytes` directly
    (with a default implementation that copies from `frame_payload()` for
    backward compatibility).
  - [x] Update the default codec adaptor to avoid `Bytes` to `Vec<u8>` copying
    on decode.
  - [x] Add regression tests confirming payloads reuse the receive buffer via
    pointer equality checks.

### 9.2. Fragment adaptor alignment

- [x] 9.2.1. Introduce a `FragmentAdapter` trait as described in the
  [fragmentation](generic-message-fragmentation-and-re-assembly-design.md)
  design. Fragmentation behaviour must explicitly define duplicate handling,
  out-of-order policies, and ownership of purge scheduling.
  - [x] Make fragmentation opt-in by requiring explicit configuration on the
    `WireframeApp` builder.
  - [x] Expose a public purge API, so callers can drive timeout eviction.
  - [x] Document the composition order for codec, fragmentation, and
    serialization layers.
  - [x] Define and implement duplicate suppression and out-of-order handling
    for fragment series.
  - [x] Define and test zero-length fragment behaviour and fragment index
    overflow handling.
  - [x] Add unit and integration tests for opt-in behaviour, interleaved
    reassembly, and duplicate and out-of-order fragments.

### 9.3. Unified codec handling

- [x] 9.3.1. Unify codec handling between the app router and the `Connection`
  actor (see
  [outbound messaging design](asynchronous-outbound-messaging-design.md)).
  - [x] Route app-level request and response handling through the
    `FramePipeline` so fragmentation and metrics apply consistently.
  - [x] Remove duplicate codec construction in
        `src/app/inbound_handler/mod.rs`; the
    `FramePipeline` owns outbound fragmentation.
  - [x] Add integration tests covering the unified pipeline (round-trip,
    fragmentation, sequential requests, disabled fragmentation).
  - [x] Add BDD behavioural tests exercising the unified codec path.
  - [x] Run protocol hooks (`before_send`) for application responses:
    `FramePipeline` owns the per-connection hooks and applies them to the
    codec-wrapped frame immediately before the transport write. Actor-driven
    push and streaming frames keep their own hook pipeline.

### 9.4. Property-based codec tests

- [x] 9.4.1. Add property-based round-trip tests for the default
  `LengthDelimitedFrameCodec` and a mock protocol codec.
  - [x] Cover boundary sizes and malformed frames using generated inputs.
  - [x] Verify encoder and decoder stateful behaviour with generated
    sequences.

### 9.5. Serializer boundaries and protocol metadata

- [x] 9.5.1. Decouple message encoding from `bincode`-specific traits to
  support alternative serializers (see the
  [binary router library design](rust-binary-router-library-design.md) and
  [ADR 0005](adrs/adr-0005-serializer-abstraction-beyond-bincode.md)).
  - [x] Introduce a serializer-agnostic message trait or adaptor layer for
    `Message` types.
  - [x] Provide optional wire-rs or Serde bridges to reduce manual boilerplate.
  - [x] Define how frame metadata is exposed to the deserialization context to
    enable version negotiation (see the
    [message versioning design](message-versioning.md)).
  - [x] Add migration guidance covering existing `bincode` users.

### 9.6. Codec performance benchmarks

- [x] 9.6.1. Add targeted benchmarks for codec throughput and latency.
  - [x] Benchmark encode and decode for small and large frames across the
    default codec and one custom codec.
  - [x] Measure fragmentation overhead versus unfragmented paths.
  - [x] Record memory allocation baselines for payload wrapping and decoding.

### 9.7. Codec test harness and observability

- [x] 9.7.1. Extend `wireframe_testing` with codec-aware drivers that can run
  `WireframeApp` instances configured with custom `FrameCodec` values.
- [x] 9.7.2. Add codec fixtures in `wireframe_testing` for generating valid and
  invalid frames, including oversized payloads and correlation metadata.
- [x] 9.7.3. Introduce a test observability harness in `wireframe_testing` that
  captures logs and metrics per test run for asserting codec failures and
  recovery policies (see
  [ADR 0006](adrs/adr-0006-test-observability-harness.md)).
- [x] 9.7.4. Add regression tests backed by `wireframe_testing` for the
      `CodecError`
  taxonomy and recovery policy behaviours defined in 9.1.2. Requires 9.1.2.

## 10. Decision closure and baseline

This phase turns the `Frame = Vec<u8>` inventory into approved design choices,
benchmark thresholds, and a concrete migration baseline before public API work
starts. See `docs/frame-vec-u8-inventory.md` and ADRs 008 through 010.

### 10.1. Design decisions

- [x] 10.1.1. Approve the stable public byte-container and edit-on-demand model
  for `PacketParts`, `Envelope`, middleware, client hooks, and serializer
  output. See `docs/adrs/adr-0008-zero-copy-public-byte-container.md`.
- [x] 10.1.2. Approve the compatibility and rollout policy for downstream
  users, including which `Vec<u8>` helpers survive the breaking release. See
  `docs/adrs/adr-0009-rollout-strategy-for-the-vec-u8-to-zero-copy-migration.md`.
- [x] 10.1.3. Approve the actor and codec-driver boundary so `Vec<u8>` bridges
  leave the core runtime deliberately rather than incidentally. See
  `docs/adrs/adr-0010-transport-frame-boundary-for-the-zero-copy-migration.md`.

### 10.2. Baselines and migration planning

- [x] 10.2.1. Capture allocation, copied-byte, throughput, and latency
  baselines for inbound decode, middleware pass-through, request hooks, and
  outbound encode on the default codec path. Requires 10.1.1. The matrix is
  defined in the [zero-copy benchmark plan](zero-copy-benchmark-plan.md) and
  all sixteen rows are captured in the
  [zero-copy benchmark baselines](zero-copy-benchmark-baselines.md), together
  with the per-seam copied-byte counts asserted by the test suite.
- [ ] 10.2.2. Record benchmark acceptance thresholds that require removal of
  the final default-path `Vec<u8>` copy between serialization and
  `FrameCodec::wrap_payload`. Requires 10.2.1. Thresholds are proposed in the
  [zero-copy benchmark baselines](zero-copy-benchmark-baselines.md), including
  the pointer-identity and allocation gate that states the removal requirement;
  this item stays open until a performance reviewer agrees to them.
- [x] 10.2.3. Publish a migration-guide outline listing the exact public
  signatures and workflows that will change for packets, middleware, client
  hooks, and serializers. Requires 10.1.2. See the
  [zero-copy migration guide outline](zero-copy-migration-guide-outline.md).

## 11. Internal zero-copy foundations

This phase tests whether shared byte hand-offs can remove mandatory vector
intermediates. Serializer work in 11.1.2 is an explicit breaking contract
slice; packet storage and the remaining public API migration stay separate.

### 11.1. Internal byte surfaces

Establish shared ownership at serializer and packet boundaries, then use the
ownership and wire evidence to guide channel and replay-buffer migration.

- [ ] 11.1.1. Convert internal packet payload storage to the approved
  zero-copy-capable byte representation. Requires 10.1.1. Serializer output is
  tracked by 11.1.2.
- [ ] 11.1.2. Deliver shared serializer output through server and no-hook
  client transport paths. Requires 10.1.1 and 10.1.2.
  - Design complete for issue
    [#538](https://github.com/leynos/wireframe/issues/538);
    implementation remains open. See the
    [plan](execplans/issue-538-plan-zero-copy-serializer-output-migration.md).
  - [ ] 11.1.2.1. Accept the serializer-specific ADR 009 amendment, then migrate
    `Serializer`, `EncodeWith`, and the Serde bridge atomically with all
    implementations and callers (EP-M1). Include direct Bincode encoding into
    `BytesMut`, finite vector conversions, and migration examples.
    Success: pointer-sharing, wire-equivalence, conversion, and hook-order
    regressions pass; raw length-delimited output is not double-framed.
  - [ ] 11.1.2.2. Deliver the default-feature and Serde-feature end-to-end
    matrix (EP-M2), after 11.1.2.1. Cover default and protocol-native codecs,
    custom serializers, hook mutation/order, and serialize/send failures.
    Success: record each supported combination and passing test evidence.
  - Packet storage is not a prerequisite for this serializer-only slice.
    Keep 11.1.1 and 12.2.1 open for their remaining packet and hook work.
- [ ] 11.1.3. Update zero-copy-capable internal channels and replay buffers
  identified in the inventory so they no longer force `Vec<u8>` hand-offs.
  Requires 11.1.1.

### 11.2. Actor and codec-driver boundary

- [ ] 11.2.1. Implement the approved actor and codec-driver boundary in the
  runtime so zero-copy transport framing does not depend on `Vec<u8>` bridges.
  Requires 10.1.3.
- [ ] 11.2.2. Move `CorrelatableFrame for Vec<u8>` and similar runtime-only
  bridges out of production paths once the replacement boundary is covered.
  Requires 11.2.1.
- [ ] 11.2.3. Add allocation and pointer-reuse regressions for the internal
  byte hand-off path on the default codec and at least one protocol-native
  codec. Requires 11.1.2 and 11.2.1.

## 12. Public API migration

This phase flips the public packet, middleware, and client surfaces to the new
byte model while preserving explicit editing ergonomics.

### 12.1. Packet and middleware surfaces

- [ ] 12.1.1. Update `PacketParts`, `Envelope`, `ServiceRequest`, and
  `ServiceResponse` to the approved byte model. Requires 10.1.1 and 11.1.1.
- [ ] 12.1.2. Preserve explicit edit-on-demand ergonomics for middleware, with
  compatibility helpers only where the rollout policy requires them. Requires
  10.1.2 and 12.1.1.
- [ ] 12.1.3. Update server-side examples and behavioural coverage so
  `Vec<u8>` is no longer taught as the default middleware frame shape. Requires
  12.1.2.

### 12.2. Client-facing byte APIs

- [ ] 12.2.1. Update `BeforeSendHook` and `Serializer::serialize` to the
  approved byte surface. Requires 10.1.1 and 11.1.1. Serializer delivery is
  tracked by 11.1.2; hook editing remains open.
- [ ] 12.2.2. Re-evaluate client preamble leftovers and document whether they
  remain `Vec<u8>` for the first breaking release or migrate with the rest of
  the client byte surface. Requires 10.1.2 and 12.2.1.
- [ ] 12.2.3. Add migration examples showing how existing `Vec<u8>` hook and
  serializer call sites move to the new API. Requires 10.2.3 and 12.2.1.

## 13. Validation, ecosystem updates, and documentation

This phase proves the new byte model operationally and updates the companion
docs and test utilities to match.

### 13.1. Compatibility cleanup

- [ ] 13.1.1. Extend `wireframe_testing`, behavioural coverage, and examples so
  `Vec<u8>` is no longer the default taught frame shape for codec and client
  workflows. Requires 12.1.3 and 12.2.3.
- [ ] 13.1.2. Publish the migration guide and breaking-change summary for the
  zero-copy API flip. Requires 10.2.3 and 12.2.3.
- [ ] 13.1.3. Close or supersede ADRs 008 through 010 once the implementation
  and published documentation match the approved design. Requires 13.1.2.

### 13.2. Performance and downstream validation

- [ ] 13.2.1. Run the benchmark suite and compare results against the approved
  thresholds from 10.2.2. Requires 11.1.2 and 12.2.1.
- [ ] 13.2.2. Add regression coverage proving read-only paths stay zero-copy
  while mutation paths copy only on demand. Requires 12.1.2 and 12.2.1.
- [ ] 13.2.3. Run downstream canaries against representative middleware, hook,
  and custom codec examples before release. Requires 13.1.2 and 13.2.1.

## 14. Release rollout

This phase packages the breaking change, publishes the upgrade story, and
captures any follow-up compatibility cleanup.

### 14.1. Breaking release preparation

- [ ] 14.1.1. Finalize the versioning plan for the public API break and record
  whether it ships as the next major or pre-1.0 point release. Requires 10.1.2
  and 13.2.3.
- [ ] 14.1.2. Publish release notes, changelog entries, and upgrade guidance
  covering removed `Vec<u8>` contracts and retained compatibility helpers.
  Requires 13.1.2 and 14.1.1.
- [ ] 14.1.3. Move `CorrelatableFrame for Vec<u8>` and any similar temporary
  bridges fully out of production paths once release validation is complete.
  Requires 11.2.2 and 14.1.2.

### 14.2. Post-release cleanup

- [ ] 14.2.1. Review retained compatibility helpers and either accept them as
  narrow shims or schedule their removal in follow-up work. Requires 14.1.2.
- [ ] 14.2.2. Capture any intentionally deferred `Vec<u8>` surfaces, together
  with explicit rationale and the next review point. Requires 14.1.2.

## 15. Formal verification

This phase establishes the formal verification infrastructure and applies
bounded model checking, state-space exploration, and deductive proofs to
Wireframe's protocol, framing, and message assembly layers.

### 15.1. Verification workspace and tooling

- [x] 15.1.1. Convert the root manifest into a hybrid workspace while keeping
  the root package as the default member. See
  [formal verification guide §Root `Cargo.toml` changes](formal-verification-methods-in-wireframe.md#root-cargotoml-changes).
  Success criteria: `cargo build` and `cargo test --workspace` pass with the
  new layout.
- [x] 15.1.2. Add `crates/wireframe-verification` as an internal crate for
  Stateright models and shared verification harnesses. Requires 15.1.1. See
  [formal verification guide §Why Stateright belongs in a separate verification crate](formal-verification-methods-in-wireframe.md#why-stateright-belongs-in-a-separate-verification-crate)
  and
  [§Suggested Stateright file layout](formal-verification-methods-in-wireframe.md#suggested-stateright-file-layout).
  Success criteria: the crate compiles, is included as a workspace member, and
  contains a placeholder Stateright model that passes `cargo test`.
- [x] 15.1.3. Add pinned Kani and Verus tool metadata plus repo-local Makefile
  install and run entry points. See
  [formal verification guide §Recommended repository layout](formal-verification-methods-in-wireframe.md#recommended-repository-layout)
  and
  [§Verus should *not* live inside the main build](formal-verification-methods-in-wireframe.md#why-verus-should-not-live-inside-the-main-build).
  Success criteria: a contributor can run `make install-kani` and
  `make install-verus` to obtain pinned versions through the pinned
  `rust-prover-tools` entry point.
- [x] 15.1.4. Add `make test-verification`, `make kani`, `make kani-full`,
  `make verus`, `make formal-pr`, and `make formal-nightly` Makefile targets.
  Requires 15.1.2 and 15.1.3. See
  [formal verification guide §Recommended Makefile changes](formal-verification-methods-in-wireframe.md#recommended-makefile-changes).
  Success criteria: each target is accepted by `mbake validate Makefile` and
  returns exit 0 on a clean tree.
- [ ] 15.1.5. Add separate CI jobs for Stateright, Kani smoke, and Verus
  proofs without changing the existing `build-test` coverage flow. See
  [formal verification guide §Recommended CI changes](formal-verification-methods-in-wireframe.md#recommended-ci-changes).
  Success criteria: CI pipelines pass on the default branch with the new jobs
  visible and green.

### 15.2. Protocol contract decisions

Each decision below is analysed in the
[formal verification guide](formal-verification-methods-in-wireframe.md); the
cited section names the specific question it answers.

- [ ] 15.2.1. Support a determined set of length-prefix widths (either `1`,
  `2`, `4`, and `8`, or the full `1..=8` range) and enforce them in
  constructors, conversions, and tests; record the decision in an ADR. Requires
  15.1.1. See §"What widths does Wireframe actually support for length
  prefixes?". Success criteria: an ADR records the decision, constructors
  enforce the chosen set, and existing tests cover rejected widths.
- [x] 15.2.2. Treat `total_body_len` as either authoritative or advisory and
  enforce or rename it consistently across the message assembly path; record
  the decision in an ADR and add tests for both conforming and violating
  inputs. Requires 15.1.1. See §"Is `total_body_len` authoritative or
  advisory?". Success criteria: an ADR records the decision, runtime code
  enforces the chosen semantics, and tests verify both conforming and violating
  inputs. See
  [ADR 015](adrs/adr-0015-declared-total-body-length-is-authoritative.md); issue
  [#544](https://github.com/leynos/wireframe/issues/544) added conformance and
  violation coverage.
- [ ] 15.2.3. Publish named fairness and priority guarantees for
  `ConnectionActor` and encode them as model properties for Stateright checks.
  Requires 15.1.1. See §"What fairness guarantee does `ConnectionActor`
  actually make?". Success criteria: the design document enumerates each
  guarantee as a named property that can be referenced by Stateright model
  checks.

### 15.3. Kani bounded model checks

- [ ] 15.3.1. Add smoke harnesses for supported length-prefix round-trips and
  unsupported-width rejection in `src/frame/*`. See
  [formal verification guide §Phase 1 smoke harnesses](formal-verification-methods-in-wireframe.md#phase-1-smoke-harnesses).
  Requires 15.1.3 and 15.2.1. Success criteria: `make kani` completes with all
  harnesses verified.
- [ ] 15.3.2. Add harnesses for `FragmentSeries`, `Reassembler`, and
  `MessageSeries` covering duplicates, gaps, completion, and oversize cleanup.
  See
  [formal verification guide §Phase 1 smoke harnesses](formal-verification-methods-in-wireframe.md#phase-1-smoke-harnesses)
  and
  [§Phase 2 full harnesses](formal-verification-methods-in-wireframe.md#phase-2-full-harnesses).
  Requires 15.1.3 and 15.2.2. Success criteria: `make kani-full` completes
  with all fragment and assembly harnesses verified.
- [ ] 15.3.3. Extend existing Proptest coverage for fragment round-trips and
  mixed actor action traces where Kani bounds would be too small. See
  [formal verification guide §Second priority: `src/fragment/*`](formal-verification-methods-in-wireframe.md#second-priority-srcfragment)
  and
  [§How Proptest and Loom fit after these changes](formal-verification-methods-in-wireframe.md#how-proptest-and-loom-fit-after-these-changes).
  Requires 15.3.1. Success criteria: `make test` includes the new Proptest
  property tests and they pass.

### 15.4. Stateright model checks

- [ ] 15.4.1. Model queue arrivals, active response and multi-packet outputs,
  shutdown races, fairness state, and terminal markers in
  `crates/wireframe-verification`. See
  [formal verification guide §Model scope](formal-verification-methods-in-wireframe.md#model-scope).
  Requires 15.1.2 and 15.2.3. Success criteria: the model compiles and a
  bounded BFS run completes without panics or assertion failures.
- [ ] 15.4.2. Add a shared checker harness that separates safety properties
  from reachability properties and reports both deterministically. See
  [formal verification guide §Properties to encode](formal-verification-methods-in-wireframe.md#properties-to-encode)
  and
  [§Shared checker harness](formal-verification-methods-in-wireframe.md#shared-checker-harness).
  Requires 15.4.1. Success criteria: `make test-verification` exercises the
  checker and reports property results.
- [ ] 15.4.3. Gate a bounded breadth-first search (BFS) model run in pull
  request CI and a deeper run in scheduled or manual workflows. See
  [formal verification guide §Shared checker harness](formal-verification-methods-in-wireframe.md#shared-checker-harness)
  and
  [§Recommended CI changes](formal-verification-methods-in-wireframe.md#recommended-ci-changes).
  Requires 15.1.5 and 15.4.2. Success criteria: the PR CI job completes within
  5 minutes; the nightly job explores a deeper state space.

### 15.5. Verus proofs for message assembly

- [x] 15.5.1. Enforce the chosen `total_body_len` contract in runtime code
  before relying on proofs. See
  [formal verification guide §"Is `total_body_len` authoritative or advisory?"](formal-verification-methods-in-wireframe.md#2-is-total_body_len-authoritative-or-advisory)
  and
  [§"What Verus should prove in Wireframe"](formal-verification-methods-in-wireframe.md#what-verus-should-prove-in-wireframe).
  Requires 15.2.2. Success criteria: runtime assertions or checks enforce the
  contract, and existing tests confirm the enforcement. See
  [ADR 015](adrs/adr-0015-declared-total-body-length-is-authoritative.md).
- [ ] 15.5.2. Add proof-only modules under `verus/` for declared-total and
  buffered-byte accounting invariants. See
  [formal verification guide §Proof style recommendation](formal-verification-methods-in-wireframe.md#proof-style-recommendation)
  and
  [§Representative proof tree](formal-verification-methods-in-wireframe.md#representative-proof-tree).
  Requires 15.1.3 and 15.5.1. Success criteria: `make verus` verifies all
  proof modules without errors.
- [ ] 15.5.3. Document proof trigger discipline and contributor expectations
  for running `make verus`. See
  [formal verification guide §Trigger discipline](formal-verification-methods-in-wireframe.md#trigger-discipline)
  and
  [§Recommended Makefile changes](formal-verification-methods-in-wireframe.md#recommended-makefile-changes).
  Requires 15.5.2. Success criteria: a contributor guide section explains
  trigger patterns, and `CONTRIBUTING.md` or the user guide references it.

## 16. Wireframe client library foundation

This phase delivers a first-class client runtime that mirrors the server's
framing, serialization, and lifecycle layers, so both sides share the same
behavioural guarantees.

### 16.1. Connection runtime

- [x] 16.1.1. Implement `WireframeClient` and its builder so callers can
  configure serializers, codec settings (including `max_frame_length` parity),
  and socket options before connecting.
- [x] 16.1.2. Integrate the existing preamble helpers so clients can emit and
  verify preambles before exchanging frames, with integration tests covering
  success and failure callbacks.
- [x] 16.1.3. Expose connection lifecycle hooks (setup, teardown, and error)
  that mirror the server hooks so middleware and instrumentation receive
  matching events.

### 16.2. Request and response pipeline

- [x] 16.2.1. Provide async `send`, `receive`, and `call` APIs that encode
  `Message` implementers, forward correlation identifiers, and deserialize
  typed responses using the configured serializer.
- [x] 16.2.2. Map decode and transport failures into `WireframeError` variants
  and add integration tests that round-trip multiple message types through a
  sample server.

### 16.3. Streaming and multi-packet parity

- [x] 16.3.1. Support `Response::Stream` and `Response::MultiPacket` on the
  client by propagating back-pressure, validating terminator frames, and
  draining push traffic without starving request-driven responses.
- [x] 16.3.2. Exercise interleaved high- and low-priority push queues to prove
  fairness and rate limits remain symmetrical.

### 16.4. Documentation and examples

- [x] 16.4.1. Publish a runnable example where a client connects to the `echo`
  server, issues a login request, and decodes the acknowledgement.
- [x] 16.4.2. Extend `docs/users-guide.md` and `docs/wireframe-client-design.md`
  with configuration tables, lifecycle diagrams, and troubleshooting guidance
  for the new APIs.

## 17. Client ergonomics and extensions

This phase layers on the ergonomic features outlined in the client design
document so larger deployments can adopt the library confidently.

### 17.1. Middleware and observability

- [x] 17.1.1. Add middleware hooks for outgoing requests and incoming frames so
  metrics, retries, and authentication tokens can be injected symmetrically
  with server middleware.
- [x] 17.1.2. Provide structured logging and tracing spans around connect,
  send, receive, call, stream, and close lifecycle events, plus configuration
  for per-command timing.

### 17.2. Connection pooling and concurrency

- [x] 17.2.1. Implement a configurable connection pool that preserves preamble
  state, enforces in-flight request limits per socket, and recycles idle
  connections.
- [x] 17.2.2. Expose a `PoolHandle` API with fairness policies, so callers can
  multiplex many logical sessions without violating back-pressure.

### 17.3. Streaming helpers and test utilities

- [x] 17.3.1. Ship helper traits or macros for consuming streaming responses
  (for example typed iterators over `Response::Stream`) so multiplexed
  protocols remain ergonomic.
- [x] 17.3.2. Publish reusable test harnesses that spin up an in-process server
  and client pair, allowing downstream crates to verify compatibility.
- [ ] 17.3.3. Extract a protocol-agnostic server lifecycle harness that
  accepts a fully configured, unbound `WireframeServer`, binds a reserved
  loopback listener, waits for readiness, and returns an idempotently stoppable
  `RunningWireframeServer`. Preserve the existing pair helpers as additive,
  source-compatible wrappers. Cover the shutdown-idempotence, readiness, and
  concurrent-handle invariants with `proptest` alongside the example-based
  `rstest` cases. See RFC 0001 for the lifecycle harness and
  `RunningWireframeServer` requirements
  ([§5.2](rfcs/rfc-0001-protocol-agnostic-test-harness-lifecycle.md#52-running-server-handle)
  and
  [§5.3](rfcs/rfc-0001-protocol-agnostic-test-harness-lifecycle.md#53-lifecycle-guarantees)
  and
  [§5.5](rfcs/rfc-0001-protocol-agnostic-test-harness-lifecycle.md#55-existing-pair-compatibility))
  and for the `rstest` and `proptest` requirements
  ([§7.1](rfcs/rfc-0001-protocol-agnostic-test-harness-lifecycle.md#71-lifecycle-integration-tests)
  and
  [§7.5](rfcs/rfc-0001-protocol-agnostic-test-harness-lifecycle.md#75-property-based-invariant-coverage)).
  The source-compatibility wrapper clause applies to the existing harness
  audience; it does not constrain the separate pre-1.0 native-protocol API
  redesign in phase 21.
- [ ] 17.3.4. Add a caller-supplied asynchronous connector that composes the
  running server with any protocol-specific client while cleaning up the server
  if connection fails. Prove the public API with a non-default codec, a typed
  preamble, type-changing builder configuration, downstream-style behavioural
  tests, and a `trybuild` compile-time test that pins the retained pair
  helpers' inferred types and rejects misconfigured lifecycle calls. Requires
  17.3.3. See RFC 0001 for the caller-supplied connector and connection-failure
  cleanup
  ([§5.4](rfcs/rfc-0001-protocol-agnostic-test-harness-lifecycle.md#54-caller-supplied-client-connector))
  and for protocol-generic and `trybuild` verification
  ([§7.2](rfcs/rfc-0001-protocol-agnostic-test-harness-lifecycle.md#72-protocol-generic-proof)
  and
  [§7.3](rfcs/rfc-0001-protocol-agnostic-test-harness-lifecycle.md#73-compatibility-and-compile-time-coverage)).
- [ ] 17.3.5. Make `wireframe_testing` an explicit quality-gate target. Run
  `cargo test -p wireframe_testing --all-targets --all-features` and
  `cargo test -p wireframe_testing --doc --all-features` in the Makefile and
  Continuous Integration workflow, repair the examples tracked by issue #578,
  and prevent the companion crate from drifting behind the root package. This
  item must complete before publishing the lifecycle extension. See RFC 0001
  [§7.4](rfcs/rfc-0001-protocol-agnostic-test-harness-lifecycle.md#74-companion-crate-quality-gates).

### 17.4. Docs and adoption

- [x] 17.4.1. Update the user guide with migration advice for the pooled client
  and document known limitations or out-of-scope behaviours.
- [x] 17.4.2. Add a troubleshooting section that enumerates the most common
  client misconfigurations (codec length mismatch, preamble errors, TLS issues)
  and how to detect them.

## 18. Advanced features and ecosystem (future)

This phase includes features that will broaden the library's applicability and
ecosystem.

### 18.1. Alternative transports

- [ ] 18.1.1. Abstract the transport layer to support protocols other than raw
  TCP (e.g., WebSockets, QUIC).

### 18.2. Message versioning

- [ ] 18.2.1. Implement a formal message versioning system to allow for
  protocol evolution.
- [ ] 18.2.2. Ensure version negotiation can consume codec metadata without
  leaking framing details into handlers (see the
  [message versioning design](message-versioning.md)).

### 18.3. Security

- [ ] 18.3.1. Provide built-in middleware or guides for implementing TLS.

## 19. Documentation and community (ongoing)

Continuous improvement of documentation and examples is essential for adoption
and usability.

### 19.1. Initial documentation

- [x] 19.1.1. Write comprehensive doc comments for all public APIs.
- [x] 19.1.2. Create a high-level `README.md` and a `docs/contents.md`.

### 19.2. Examples

- [x] 19.2.1. Create a variety of examples demonstrating core features
  (`ping_pong`, `echo`, `metadata_routing`, and `async_stream`).

### 19.3. Website and user guide

- [ ] 19.3.1. Develop a dedicated website with a detailed user guide.
- [ ] 19.3.2. Write tutorials for common use cases.

### 19.4. API documentation

- [ ] 19.4.1. Ensure all public items have clear, useful documentation
  examples.
- [ ] 19.4.2. Publish documentation to `docs.rs`.

## 20. Prepared application and runtime ownership (in progress)

This phase makes the builder-to-runtime ownership boundary explicit while
sequencing the remaining server and connection-runtime work separately.

### 20.1. Prepared application transition

The connection-local runtime in 20.1.3 is shared foundation for phase 21; phase
21 validates and extends it through native toy servers rather than creating a
parallel runtime.

- [x] 20.1.1. Add the consuming `WireframeApp::prepare().await` transition and
  immutable `PreparedApp`, building route middleware chains once and providing
  prepared connection drivers. See issue
  [#641](https://github.com/leynos/wireframe/issues/641) and
  [ADR 012](adrs/adr-0012-prepared-application-templates-and-connection-local-runtimes.md).
- [x] 20.1.2. Prepare the application factory once before server readiness and
  share the immutable prepared root with accept loops and connection tasks. See
  issue [#642](https://github.com/leynos/wireframe/issues/642).
- [ ] 20.1.3. Extract connection-local runtime ownership and lifecycle
  finalization. See issue
  [#643](https://github.com/leynos/wireframe/issues/643).

## 21. Protocol-neutral adapters for the first consumers

Idea: if the first vertical slice lets `mxd` serve Hotline through generic
public Wireframe primitives while the same APIs serve Modbus TCP, then
Wireframe has a useful native-protocol framework without making Hotline its
sole purpose. Each slice delivers a named toy server with independent wire
fixtures. Existing protocol libraries may be test-only oracles when licensed;
production code uses Wireframe primitives. Pre-1.0 APIs may be replaced. See the
[gap survey](wire-protocol-gap-analysis-survey.md) and
[RFC 0003](rfcs/rfc-0003-native-message-adapter-boundary.md)–[RFC 0009](rfcs/rfc-0009-datagram-session-transport.md).

### 21.1. Prove the native adapter with mxd and Modbus

This step answers whether the public adapter boundary can replace mxd's
Envelope translation and provide a small unrelated protocol vertical slice. The
toy servers are the first evidence for the later runtime work. Phase 21 uses
the shared connection-local lifecycle from 20.1.3 and test harness work from
17.3.3–17.3.5; it does not create a second runtime or test harness.

- [ ] 21.1.1. Implement native frame metadata and reply context for a Hotline
  toy server and a Modbus TCP toy server. See
  [RFC 0003](rfcs/rfc-0003-native-message-adapter-boundary.md).
  - Preserve Hotline's native header fields without a compulsory Envelope.
  - Preserve Modbus transaction and unit identifiers.
  - Success: both server adapters use the same public API and pass independent
    fixtures. No server is called delivered until it is runnable.
- [ ] 21.1.2. Implement configurable length framing required by both toy
  servers. See [RFC 0006](rfcs/rfc-0006-configurable-framing-primitives.md).
  - Cover partial reads, coalesced frames, checked lengths, EOF, and bounded
    allocation.
  - Requires 21.1.1.
  - Success: fixture contracts are executable before server implementation;
    production dependencies contain no protocol parser.
- [ ] 21.1.3. Extend the test harness for raw peers and deterministic split
  reads, writes, and time. See
  [RFC 0008](rfcs/rfc-0008-external-adapter-conformance-harness.md).
  - Requires 21.1.1.
  - Success: raw peer fixtures can exercise the production public entry point.

### 21.2. Integrate budgets, duplex sessions, and mxd

This step answers whether the runtime can sustain independent input and output
while bounding native message assembly. It consolidates work already named by
20.1.3; that task owns the shared connection runtime, and this step validates
it with consumers.

- [ ] 21.2.1. Implement channel-aware budgets and bounded assembly for Hotline
  and Modbus. See [RFC 0007](rfcs/rfc-0007-budgets-scheduling-and-assembly.md).
  - Requires 21.1.2 and 21.1.3.
  - Success: oversized and incomplete messages stay within configured memory
    and time limits; one channel cannot starve another.
- [ ] 21.2.2. Implement the connection-local duplex driver for Hotline and
  Modbus. See [RFC 0004](rfcs/rfc-0004-duplex-session-driver.md).
  - Requires 21.2.1 and 20.1.3.
  - Success: a stalled handler cannot block control reads or an unrelated
    connection; cancellation and close reclaim tasks and queued output.
- [ ] 21.2.3. Validate mxd's migration against its pinned Hotline fixtures and
  deliver standalone Hotline and Modbus toy servers. See
  [RFC 0003](rfcs/rfc-0003-native-message-adapter-boundary.md), [RFC 0004](rfcs/rfc-0004-duplex-session-driver.md),
  and [RFC 0007](rfcs/rfc-0007-budgets-scheduling-and-assembly.md).
  - Requires 21.2.1 and 21.2.2.
  - Success: mxd-specific codec and assembly are not production dependencies
    of Wireframe; its existing push and broadcast behaviour remains validated.

### 21.3. Prove generic negotiation and database sessions

This step answers whether the same session API supports phase changes and
ordered multi-message replies for established database protocols.

- [ ] 21.3.1. Implement negotiated byte-stream setup, upgrade barriers, and
  buffered-byte hand-off as a generic transport entry point. See
  [RFC 0005](rfcs/rfc-0005-negotiated-transport-entry-points.md).
  - Requires 21.2.2.
  - Success: server-first and client-first toy handshakes preserve coalesced
    application bytes and have bounded deadlines.
- [ ] 21.3.2. Implement a Kafka toy broker for a pinned API-version subset.
  Cover version-aware replies, ordered pipelining, and no-response produce. See
  [RFC 0003](rfcs/rfc-0003-native-message-adapter-boundary.md), [RFC 0004](rfcs/rfc-0004-duplex-session-driver.md),
  and [RFC 0005](rfcs/rfc-0005-negotiated-transport-entry-points.md).
  - Requires 21.2.2 and 21.3.1.
  - Success: independent clients validate wire versions, order, and no reply.
- [ ] 21.3.3. Implement a PostgreSQL protocol 3.x toy server with startup,
  Sync recovery, COPY, and cancellation. See
  [RFC 0005](rfcs/rfc-0005-negotiated-transport-entry-points.md) and
  [RFC 0007](rfcs/rfc-0007-budgets-scheduling-and-assembly.md).
  - Requires 21.2.2 and 21.3.1.
  - Success: pinned clients verify phase changes, cross-connection cancel, and
    bounded result streaming.
- [ ] 21.3.4. Implement separate classic MySQL and MariaDB toy servers.
  Cover greeting, TLS transition, sequence progression, continuation packets,
  no-response commands, and result streams. See
  [RFC 0005](rfcs/rfc-0005-negotiated-transport-entry-points.md) and
  [RFC 0007](rfcs/rfc-0007-budgets-scheduling-and-assembly.md).
  - Requires 21.2.2 and 21.3.1.
  - Success: each pinned client profile validates independently; protocol
    libraries are test-only and licence-reviewed.

- [ ] 21.3.5. Implement an OSCAR FLAP/SNAC toy server with login negotiation,
  sequence validation, and server-originated messages. See
  [RFC 0003](rfcs/rfc-0003-native-message-adapter-boundary.md) and
  [RFC 0004](rfcs/rfc-0004-duplex-session-driver.md).
  - Requires 21.2.2 and 21.3.1.
  - Success: an independent client validates login, native routes, and push.
- [ ] 21.3.6. Implement an AMQP 0-9-1 toy server with channel multiplexing,
  heartbeat, content assembly, and consumer prefetch. See
  [RFC 0005](rfcs/rfc-0005-negotiated-transport-entry-points.md) and
  [RFC 0007](rfcs/rfc-0007-budgets-scheduling-and-assembly.md).
  - Requires 21.2.2 and 21.3.1.
  - Success: a licensed independent peer validates flow control and control
    progress. Keep the AMQP 1.0 server in 21.3.7 as a separate profile.
- [ ] 21.3.7. Implement an AMQP 1.0 toy server with session/link flow control
  and settlement. See
  [RFC 0005](rfcs/rfc-0005-negotiated-transport-entry-points.md) and
  [RFC 0007](rfcs/rfc-0007-budgets-scheduling-and-assembly.md).
  - Requires 21.2.2 and 21.3.1.
  - Success: an independent peer validates transfer and credit boundaries.

### 21.4. Prove timed control, streaming, and cross-connection fan-out

This step answers whether timers, native segmentation, and bounded fan-out fit
the shared session API.

- [ ] 21.4.1. Implement an IEC 60870-5-104 toy server with timed control,
  acknowledgements, and spontaneous updates. See
  [RFC 0004](rfcs/rfc-0004-duplex-session-driver.md) and
  [RFC 0007](rfcs/rfc-0007-budgets-scheduling-and-assembly.md).
  - Requires 21.2.1 and 21.2.2.
  - Success: independent fixtures verify idle deadlines, sequence windows,
    and bounded unsolicited output.
- [ ] 21.4.2. Implement a DNP3 TCP toy server pinned to IEEE Std 1815-2010,
  with link checksums, segmentation, and unsolicited responses. See
  [RFC 0006](rfcs/rfc-0006-configurable-framing-primitives.md) and
  [RFC 0007](rfcs/rfc-0007-budgets-scheduling-and-assembly.md).
  - Requires 21.2.1 and 21.2.2.
  - Success: licensed independent fixtures validate checksum and resource
    failure cases.
- [ ] 21.4.3. Implement RTSP 1.0 and 2.0 toy servers for TCP interleaving,
  streamed media records, and control progress. See
  [RFC 0004](rfcs/rfc-0004-duplex-session-driver.md) and
  [RFC 0005](rfcs/rfc-0005-negotiated-transport-entry-points.md).
  - Requires 21.2.1 and 21.2.2.
  - Success: both profiles pass independent text/binary interleaving fixtures.
- [ ] 21.4.4. Implement an OpenRA toy server for the pinned network profile,
  covering server-first handshake, framed orders, pings, and bounded fan-out.
  See [RFC 0004](rfcs/rfc-0004-duplex-session-driver.md) and
  [RFC 0007](rfcs/rfc-0007-budgets-scheduling-and-assembly.md).
  - Requires 21.2.1 and 21.2.2.
  - Success: an independent client validates relay ordering and slow-peer
    isolation.

### 21.5. Prove generic datagrams and native WebSocket

This step implements the generic datagram entry point, then validates it with
Quake 3; it also verifies native WebSocket control semantics.

- [ ] 21.5.1. Implement a native WebSocket RFC 6455 toy server with HTTP
  upgrade, masking, continuation, control progress, close, and limits. See
  [RFC 0005](rfcs/rfc-0005-negotiated-transport-entry-points.md) and
  [RFC 0006](rfcs/rfc-0006-configurable-framing-primitives.md).
  - Requires 21.2.1 and 21.2.2.
  - Success: raw-wire fixtures cover protocol behaviour; parser-library
    bridging is a lower-priority comparison.
- [ ] 21.5.2. Implement source-address metadata, datagram boundaries, bounded
  queues, expiry, and send errors as a generic datagram transport. See
  [RFC 0009](rfcs/rfc-0009-datagram-session-transport.md).
  - Requires 21.2.2.
  - Success: a connectionless toy peer uses the API without Quake-specific
    state.
- [ ] 21.5.3. Implement a Quake 3 UDP toy server for a pinned connectionless
  and connected subset, including qport, fragmentation, loss, reordering, and
  peer expiry. See [RFC 0009](rfcs/rfc-0009-datagram-session-transport.md).
  - Requires 21.5.2.
  - Success: fixtures preserve datagram boundaries and test bounded peer state.

### 21.6. Integrate parsed-frame libraries after native delivery

This step delivers the lower-priority parsed-frame bridge after native codecs
and sessions. OpenRA fan-out is validated in 21.4.4.

- [ ] 21.6.1. Implement a generic adapter for pre-parsed frame stream/sink
  libraries, using `tokio-tungstenite` only as a test peer. See
  [RFC 0005](rfcs/rfc-0005-negotiated-transport-entry-points.md).
  - Requires 21.5.1.
  - Success: frame kinds and close events remain intact, and production code
    depends on no protocol parser.

### 21.7. Audit toy coverage against the surveyed scope

- [ ] 21.7.1. Publish an evidence matrix for every scoped protocol and profile,
  linking runnable toy, independent oracle, licence, malformed corpus, bounds,
  and verified behaviour. See
  [RFC 0008](rfcs/rfc-0008-external-adapter-conformance-harness.md).
  - Requires steps 21.1-21.6.
  - Include Hotline, Modbus, OSCAR, WebSocket, RTSP 1.0/2.0, AMQP 0-9-1/1.0,
    Kafka, MySQL, MariaDB, PostgreSQL, IEC 104, DNP3 IEEE Std 1815-2010,
    OpenRA, and Quake 3.
  - Generate lifecycle, ordered-completion, setup-transition, reservation,
    scheduling, and datagram traces with `proptest`. Assert each operation
    context is consumed on completion or released on termination exactly once,
    and each reservation is released exactly once. Check ordering, byte
    conservation, finite budgets, and datagram boundaries under faults.
  - Success: every profile has a runnable server and independent peer, golden
    vectors, split/coalescing and malformed/truncation tests, budget and
    lifecycle checks, and offline execution. No protocol implementation appears
    in Wireframe production dependencies.
