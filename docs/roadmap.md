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
  (`tests/advanced/concurrency_loom.rs`).

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
  handlers can opt into streaming ergonomically.[^adr-0001]
- [x] 6.3.2. Update the multi-packet design documentation and user guide with
  tuple return examples that explain initial-frame handling, back-pressure, and
  graceful termination.[^adr-0001]
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

This phase implements the decisions from ADR 0002,[^adr-0002] adding
first-class streaming request bodies, a generic message assembly abstraction,
and standardized per-connection memory budgets.

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
- [ ] 8.3.5. Define derived defaults based on `buffer_capacity` when budgets
  are not set explicitly.
- [ ] 8.3.6. Write tests for budget enforcement, back-pressure, and cleanup
  semantics.

### 8.4. Transport helper

- [ ] 8.4.1. Implement `send_streaming(frame_header, body_reader)` helper.
- [ ] 8.4.2. Add chunk size configuration with protocol-provided headers.
- [ ] 8.4.3. Implement timeout handling (return `TimedOut`, stop emitting
  frames).
- [ ] 8.4.4. Integrate with connection actor instrumentation and hooks.
- [ ] 8.4.5. Write tests for partial send failures and timeout behaviour.

### 8.5. Testkit utilities

- [ ] 8.5.1. Add utilities for feeding partial frames or fragments into an
  in-process app.
- [ ] 8.5.2. Add slow reader and writer simulation for back-pressure testing.
- [ ] 8.5.3. Add deterministic assertion helpers for reassembly outcomes.
- [ ] 8.5.4. Export utilities as `wireframe::testkit` behind a dedicated
  feature.

### 8.6. Documentation

- [ ] 8.6.1. Update `generic-message-fragmentation-and-re-assembly-design.md`
  with composition guidance.
- [ ] 8.6.2. Update `multi-packet-and-streaming-responses-design.md` with a
  streaming request body section.
- [ ] 8.6.3. Update
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
    `adr-004-pluggable-protocol-codecs.md`.
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
  fragmentation design.[^fragmentation-design] Fragmentation behaviour must
  explicitly define duplicate handling, out-of-order policies, and ownership of
  purge scheduling.
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
  actor.[^outbound-design]
  - [x] Route app-level request and response handling through the
    `FramePipeline` so fragmentation and metrics apply consistently.
  - [x] Remove duplicate codec construction in `src/app/inbound_handler.rs`; the
    `FramePipeline` owns outbound fragmentation.
  - [x] Add integration tests covering the unified pipeline (round-trip,
    fragmentation, sequential requests, disabled fragmentation).
  - [x] Add BDD behavioural tests exercising the unified codec path.
  - [x] Note: protocol hooks (`before_send`) are deferred to a follow-up
    stage because `F::Frame` and `Envelope` types may
    differ.[^streaming-design]

### 9.4. Property-based codec tests

- [x] 9.4.1. Add property-based round-trip tests for the default
  `LengthDelimitedFrameCodec` and a mock protocol codec.
  - [x] Cover boundary sizes and malformed frames using generated inputs.
  - [x] Verify encoder and decoder stateful behaviour with generated
    sequences.

### 9.5. Serializer boundaries and protocol metadata

- [x] 9.5.1. Decouple message encoding from `bincode`-specific traits to
  support alternative serializers.[^router-design][^adr-005]
  - [x] Introduce a serializer-agnostic message trait or adaptor layer for
    `Message` types.
  - [x] Provide optional wire-rs or Serde bridges to reduce manual boilerplate.
  - [x] Define how frame metadata is exposed to the deserialization context to
    enable version negotiation.[^message-versioning]
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
- [ ] 9.7.2. Add codec fixtures in `wireframe_testing` for generating valid and
  invalid frames, including oversized payloads and correlation metadata.
- [ ] 9.7.3. Introduce a test observability harness in `wireframe_testing` that
  captures logs and metrics per test run for asserting codec failures and
  recovery policies.[^adr-006]
- [ ] 9.7.4. Add regression tests in `wireframe_testing` for the `CodecError`
  taxonomy and recovery policy behaviours defined in 9.1.2. Requires 9.1.2.

## 10. Wireframe client library foundation

This phase delivers a first-class client runtime that mirrors the server's
framing, serialization, and lifecycle layers, so both sides share the same
behavioural guarantees.

### 10.1. Connection runtime

- [x] 10.1.1. Implement `WireframeClient` and its builder so callers can
  configure serializers, codec settings (including `max_frame_length` parity),
  and socket options before connecting.
- [x] 10.1.2. Integrate the existing preamble helpers so clients can emit and
  verify preambles before exchanging frames, with integration tests covering
  success and failure callbacks.
- [x] 10.1.3. Expose connection lifecycle hooks (setup, teardown, and error)
  that mirror the server hooks so middleware and instrumentation receive
  matching events.

### 10.2. Request and response pipeline

- [x] 10.2.1. Provide async `send`, `receive`, and `call` APIs that encode
  `Message` implementers, forward correlation identifiers, and deserialize
  typed responses using the configured serializer.
- [x] 10.2.2. Map decode and transport failures into `WireframeError` variants
  and add integration tests that round-trip multiple message types through a
  sample server.

### 10.3. Streaming and multi-packet parity

- [x] 10.3.1. Support `Response::Stream` and `Response::MultiPacket` on the
  client by propagating back-pressure, validating terminator frames, and
  draining push traffic without starving request-driven responses.
- [x] 10.3.2. Exercise interleaved high- and low-priority push queues to prove
  fairness and rate limits remain symmetrical.

### 10.4. Documentation and examples

- [x] 10.4.1. Publish a runnable example where a client connects to the `echo`
  server, issues a login request, and decodes the acknowledgement.
- [x] 10.4.2. Extend `docs/users-guide.md` and `docs/wireframe-client-design.md`
  with configuration tables, lifecycle diagrams, and troubleshooting guidance
  for the new APIs.

## 11. Client ergonomics and extensions

This phase layers on the ergonomic features outlined in the client design
document so larger deployments can adopt the library confidently.

### 11.1. Middleware and observability

- [ ] 11.1.1. Add middleware hooks for outgoing requests and incoming frames so
  metrics, retries, and authentication tokens can be injected symmetrically
  with server middleware.
- [ ] 11.1.2. Provide structured logging and tracing spans around connect,
  send, receive, and stream lifecycle events, plus configuration for
  per-command timing.

### 11.2. Connection pooling and concurrency

- [ ] 11.2.1. Implement a configurable connection pool that preserves preamble
  state, enforces in-flight request limits per socket, and recycles idle
  connections.
- [ ] 11.2.2. Expose a `PoolHandle` API with fairness policies so callers can
  multiplex many logical sessions without violating back-pressure.

### 11.3. Streaming helpers and test utilities

- [ ] 11.3.1. Ship helper traits or macros for consuming streaming responses
  (for example typed iterators over `Response::Stream`) so multiplexed
  protocols remain ergonomic.
- [ ] 11.3.2. Publish reusable test harnesses that spin up an in-process server
  and client pair, allowing downstream crates to verify compatibility.

### 11.4. Docs and adoption

- [ ] 11.4.1. Update the user guide with migration advice for the pooled client
  and document known limitations or out-of-scope behaviours.
- [ ] 11.4.2. Add a troubleshooting section that enumerates the most common
  client misconfigurations (codec length mismatch, preamble errors, TLS issues)
  and how to detect them.

## 12. Advanced features and ecosystem (future)

This phase includes features that will broaden the library's applicability and
ecosystem.

### 12.1. Alternative transports

- [ ] 12.1.1. Abstract the transport layer to support protocols other than raw
  TCP (e.g., WebSockets, QUIC).

### 12.2. Message versioning

- [ ] 12.2.1. Implement a formal message versioning system to allow for
  protocol evolution.
- [ ] 12.2.2. Ensure version negotiation can consume codec metadata without
  leaking framing details into handlers.[^message-versioning]

### 12.3. Security

- [ ] 12.3.1. Provide built-in middleware or guides for implementing TLS.

## 13. Documentation and community (ongoing)

Continuous improvement of documentation and examples is essential for adoption
and usability.

### 13.1. Initial documentation

- [x] 13.1.1. Write comprehensive doc comments for all public APIs.
- [x] 13.1.2. Create a high-level `README.md` and a `docs/contents.md`.

### 13.2. Examples

- [x] 13.2.1. Create a variety of examples demonstrating core features
  (`ping_pong`, `echo`, `metadata_routing`, and `async_stream`).

### 13.3. Website and user guide

- [ ] 13.3.1. Develop a dedicated website with a detailed user guide.
- [ ] 13.3.2. Write tutorials for common use cases.

### 13.4. API documentation

- [ ] 13.4.1. Ensure all public items have clear, useful documentation
  examples.
- [ ] 13.4.2. Publish documentation to `docs.rs`.

[^adr-0001]: Refer to
[ADR 0001](adr-001-multi-packet-streaming-response-api.md).
[^adr-0002]: Refer to
[ADR 0002](adr-002-streaming-requests-and-shared-message-assembly.md).
[^fragmentation-design]: See
  [fragmentation doc](generic-message-fragmentation-and-re-assembly-design.md).
[^outbound-design]: See
  [outbound messaging design](asynchronous-outbound-messaging-design.md).
[^streaming-design]: See
  [streaming responses design](multi-packet-and-streaming-responses-design.md).
[^router-design]: See
[rust-binary-router-library-design.md](rust-binary-router-library-design.md).
[^message-versioning]: See
[message-versioning.md](message-versioning.md).
[^adr-005]: See
[adr-005-serializer-abstraction.md](adr-005-serializer-abstraction.md).
[^adr-006]: See
[adr-006-test-observability.md](adr-006-test-observability.md).
