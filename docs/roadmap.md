# Wireframe Combined Development Roadmap

This document outlines the development roadmap for the Wireframe library,
merging previous roadmap documents into a single source of truth. It details
the planned features, enhancements, and the overall trajectory towards a stable
and production-ready 1.0 release.

## Guiding Principles

- **Ergonomics:** The library should be intuitive and easy to use.

- **Performance:** Maintain high performance and low overhead.

- **Extensibility:** Provide clear extension points, especially through
  middleware.

- **Robustness:** Ensure the library is resilient and handles errors gracefully.

## Phase 1: Core Functionality & API (Complete)

This phase established the foundational components of the Wireframe server and
the request/response lifecycle.

- [x] **Protocol Definition:**

  - [x] Define the basic frame structure for network communication
    (`src/frame/mod.rs`).

  - [x] Implement preamble validation for versioning and compatibility
    (`src/preamble.rs`, `tests/preamble.rs`).

- [x] **Core Server Implementation:**

  - [x] Implement the `Server` struct with `bind` and `run` methods
    (`src/server.rs`).

  - [x] Handle incoming TCP connections and spawn connection-handling tasks
    (`src/connection.rs`).

  - [x] Define `Request`, `Response`, and `Message` structs (`src/message.rs`,
    `src/response.rs`).

- [x] **Routing & Handlers:**

  - [x] Implement a basic routing mechanism to map requests to handler
    functions (`src/app.rs`).

  - [x] Support handler functions with flexible, type-safe extractors
    (`src/extractor.rs`).

- [x] **Error Handling:**

  - [x] Establish a comprehensive set of error types.

  - [x] Implement `From` conversions for ergonomic error handling.

  - [x] Ensure `Display` is implemented for all public error types
    (`tests/error_display.rs`).

- [x] **Basic Testing:**

  - [x] Develop a suite of integration tests for core request/response
    functionality (`tests/server.rs`, `tests/routes.rs`).

## Phase 2: Middleware & Extensibility (Complete)

This phase focused on building the middleware system, a key feature for
extensibility.

- [x] **Middleware Trait:**

  - [x] Design and implement the `Middleware` trait (`src/middleware.rs`).

  - [x] Define `Next` to allow middleware to pass control to the next in the
    chain.

- [x] **Middleware Integration:**

  - [x] Integrate the middleware processing loop into the `App` and
    `Connection` logic.

  - [x] Ensure middleware can modify requests and responses.

- [x] **Testing:**

  - [x] Write tests to verify middleware functionality, including correct
    execution order (`tests/middleware.rs`, `tests/middleware_order.rs`).

## Phase 3: Push Messaging & Async Operations (Complete)

This phase introduced capabilities for asynchronous, server-initiated
communication and streaming.

- [x] **Push Messaging:**

  - [x] Implement the `Push` mechanism for sending messages from server to
    client without a direct request (`src/push.rs`).

  - [x] Develop `PushPolicies` for broadcasting messages to all or a subset of
    clients.

  - [x] Create tests for various push scenarios (`tests/push.rs`,
    `tests/push_policies.rs`).

- [x] **Async Stream Responses:**

  - [x] Enable handlers to return `impl Stream` of messages (`src/response.rs`).

  - [x] Implement the client and server-side logic to handle streaming
    responses (`examples/async_stream.rs`, `tests/async_stream.rs`).

## Phase 4: Advanced Connection Handling & State (Complete)

This phase added sophisticated state management and improved connection
lifecycle control.

- [x] **Session Management:**

  - [x] Implement a `Session` struct to hold connection-specific state
    (`src/session.rs`).

  - [x] Create a `SessionRegistry` for managing all active sessions
    (`tests/session_registry.rs`).

  - [x] Provide `State` and `Data` extractors for accessing shared and
    session-specific data.

- [x] **Lifecycle Hooks:**

  - [x] Implement `on_connect` and `on_disconnect` hooks for session
    initialization and cleanup (`src/hooks.rs`).

  - [x] Write tests to verify lifecycle hook behaviour (`tests/lifecycle.rs`).

- [x] **Graceful Shutdown:**

  - [x] Implement a graceful shutdown mechanism for the server, allowing active
    connections to complete their work.

## Phase 5: Production Hardening & Observability (In Progress)

This phase focuses on making the library robust, debuggable, and ready for
production environments.

- [x] **Logging:**

  - [x] Integrate `tracing` throughout the library for structured, level-based
    logging.

  - [x] Create a helper crate for test logging setup
    (`wireframe_testing/src/logging.rs`).

- [x] **Metrics & Observability:**

  - [x] Expose key operational metrics (e.g., active connections, messages per
    second, error rates).

- [x] Provide an integration guide for popular monitoring systems (e.g.,
    Prometheus).

- [x] **Packet decomposition:**

  - [x] Introduce `PacketParts` to replace tuple-based packet handling.
  - [x] Treat `correlation_id` as `Option<u64>` so `None` denotes an
    unsolicited event or server-initiated push.

- [x] **Advanced Error Handling:**

  - [x] Implement panic handlers in connection tasks to prevent a single
    connection from crashing the server.

- [ ] **Testing:**

  - [x] Implement fuzz testing for the protocol parser
    (`tests/advanced/interaction_fuzz.rs`).

  - [x] Use `loom` for concurrency testing of shared state
    (`tests/advanced/concurrency_loom.rs`).

## Phase 6: Multi-Packet Streaming Responses (Priority Focus)

This is the next major feature set. It enables a handler to return multiple,
distinct messages over time in response to a single request, forming a logical
stream.

- [ ] **Protocol Enhancement:**

  - [x] Add a `correlation_id` field to the `Frame` header. For a request, this
    is the unique request ID. For each message in a multi-packet response, this
    ID must match the original request's ID.

  - [x] Define a mechanism to signal the end of a multi-packet stream, such as
    a frame with a specific flag and no payload.

- [x] **Core Library Implementation:**

  - [x] Introduce a `Response::MultiPacket` variant that contains a channel
    `Receiver<Message>`.

  - [x] Modify the `Connection` actor: upon receiving `Response::MultiPacket`,
    it should consume messages from the receiver and send each one as a `Frame`.
    - [x] Extend the outbound `select!` loop to own the receiver so
      multi-packet responses share the same back-pressure and shutdown handling
      as other frame sources.
    - [x] Convert each received `Message` into a `Frame` via the existing
      serialization helpers rather than bypassing protocol hooks or metrics.
    - [x] Emit tracing and metrics for each forwarded frame so streaming
      traffic remains visible to observability pipelines.

  - [x] Each sent frame must carry the correct `correlation_id` from the
    initial request.
    - [x] Capture the originating request's `correlation_id` before handing
      control to the multi-packet dispatcher.
    - [x] Stamp the stored `correlation_id` onto every frame emitted from the
      channel before it is queued for transmission.
    - [x] Guard against accidental omission by asserting in debug builds and
      covering the behaviour with targeted tests.

  - [x] When the channel closes, send the end-of-stream marker frame.
    - [x] Detect channel closure (`None` from `recv`) and log the termination
      reason for operational insight.
    - [x] Send the designated end-of-stream marker frame through the same
      send path, reusing the request's `correlation_id`.
    - [x] Notify protocol lifecycle hooks so higher layers can tidy any
      per-request state when a stream drains naturally.

- [x] **Ergonomics & API:**

  - [x] Provide a helper (for example `Response::with_channel`) that returns a
    bounded channel sender alongside a `Response::MultiPacket` so handlers can
    opt into streaming ergonomically.[^adr-0001]
  - [x] Update the multi-packet design documentation and user guide with tuple
    return examples that explain initial-frame handling, back-pressure, and
    graceful termination.[^adr-0001]
  - [x] Add an example handler (or test fixture) demonstrating spawning a
    background task that pushes frames through the returned sender while the
    connection actor manages delivery.

- [x] **Testing:**

  - [x] Develop integration tests where a client sends one request and receives
    multiple, correlated response messages.

  - [x] Test that the end-of-stream marker is sent correctly and handled by the
    client.

  - [x] Test client-side handling of interleaved multi-packet responses from
    different requests.

## Phase 7: Transport-Level Fragmentation & Reassembly

This phase will handle the transport of a single message that is too large to
fit into a single frame, making the process transparent to the application
logic.

- [x] **Core Fragmentation & Reassembly (F&R) Layer:**

  - [x] Define a generic `Fragment` header or metadata containing `message_id`,
    `fragment_index`, and `is_last_fragment` fields.

  - [x] Implement a `Fragmenter` to split a large `Message` into multiple
    `Frame`s, each with a `Fragment` header.

  - [x] Implement a `Reassembler` on the receiving end to collect fragments and
    reconstruct the original `Message`.

  - [x] Manage a reassembly buffer with timeouts to prevent resource
    exhaustion from incomplete messages.

- [x] **Integration with Core Library:**

  - [x] Integrate the F&R layer into the `Connection` actor's read/write paths.

  - [x] Ensure the F&R logic is transparent to handler functions; they should
    continue to send and receive complete `Message` objects.

- [x] **Testing:**

  - [x] Create unit tests for the `Fragmenter` and `Reassembler`.

  - [x] Develop integration tests sending and receiving large messages that
    require fragmentation.

  - [x] Test edge cases: out-of-order fragments, duplicate fragments, and
    reassembly timeouts.

## Phase 7.5: Streaming requests & shared message assembly

This phase implements the decisions from [ADR 0002][adr-0002], adding
first-class streaming request bodies, a generic message assembly abstraction,
and standardized per-connection memory budgets.

- [ ] **Streaming request bodies:**

  - [x] Implement `RequestParts` struct with `id`, `correlation_id`, and
    `metadata` fields.

  - [ ] Implement `RequestBodyStream` type alias as a pinned, boxed stream of
    `Result<Bytes, std::io::Error>`.

  - [ ] Add `AsyncRead` adaptor for `RequestBodyStream` so protocol crates can
    reuse existing parsers.

  - [ ] Integrate streaming request extraction with the handler dispatch path.

  - [ ] Write tests for buffered-to-streaming fallback and back-pressure
    propagation.

- [ ] **MessageAssembler abstraction:**

  - [ ] Define `MessageAssembler` hook trait for protocol-specific multi-frame
    parsing.

  - [ ] Implement per-frame header parsing with "first frame" versus
    "continuation frame" handling.

  - [ ] Add message key support for multiplexing interleaved assemblies.

  - [ ] Implement continuity validation (ordering, missing frames, duplicate
    frames).

  - [ ] Integrate with connection actor's inbound path, applying after transport
    fragmentation.

  - [ ] Write tests for interleaved assembly, ordering violations, and timeout
    behaviour.

- [ ] **Per-connection memory budgets:**

  - [ ] Add `WireframeApp::memory_budgets(...)` builder method.

  - [ ] Implement budget enforcement covering bytes per message, bytes per
    connection, and bytes across in-flight assemblies.

  - [ ] Implement soft limit (back-pressure by pausing reads) behaviour.

  - [ ] Implement hard cap (abort early, release partial state, surface
    `InvalidData`) behaviour.

  - [ ] Define derived defaults based on `buffer_capacity` when budgets are not
    set explicitly.

  - [ ] Write tests for budget enforcement, back-pressure, and cleanup
    semantics.

- [ ] **Transport helper:**

  - [ ] Implement `send_streaming(frame_header, body_reader)` helper.

  - [ ] Add chunk size configuration with protocol-provided headers.

  - [ ] Implement timeout handling (return `TimedOut`, stop emitting frames).

  - [ ] Integrate with connection actor instrumentation and hooks.

  - [ ] Write tests for partial send failures and timeout behaviour.

- [ ] **Testkit utilities:**

  - [ ] Add utilities for feeding partial frames/fragments into an in-process
    app.

  - [ ] Add slow reader/writer simulation for back-pressure testing.

  - [ ] Add deterministic assertion helpers for reassembly outcomes.

  - [ ] Export utilities as `wireframe::testkit` behind a dedicated feature.

- [ ] **Documentation:**

  - [ ] Update `generic-message-fragmentation-and-re-assembly-design.md` with
    composition guidance.

  - [ ] Update `multi-packet-and-streaming-responses-design.md` with streaming
    request body section.

  - [ ] Update
        `the-road-to-wireframe-1-0-feature-set-philosophy-and-capability-maturity.md`
    with MessageAssembler and budget details.

## Phase 7.6: Pluggable protocol codec hardening

This phase addresses follow-up work discovered during the pluggable codec
implementation, focusing on stateful framing, error taxonomy, and reliability.
The work here will feed into a subsequent round of design document updates that
clarify codec recovery policies, fragmentation behaviour, and serializer
integration boundaries.

### 7.6.1. Codec enhancements

- [ ] 7.6.1.1. Make `FrameCodec::wrap_payload` instance-aware for stateful
  codecs.
  - [ ] Update the trait to accept `&self` and a `Bytes` payload to reduce
    copies, then document the change in
    `docs/adr-004-pluggable-protocol-codecs.md`.
  - [ ] Update `LengthDelimitedFrameCodec` and any adaptors to use the new
    payload type.
  - [ ] Reuse a per-connection encoder, so sequence counters can advance
    deterministically.
- [ ] 7.6.1.2. Introduce a `CodecError` taxonomy.
  - [ ] Add a `CodecError` enum separating framing, protocol, and IO failures.
  - [ ] Extend `WireframeError` to surface `CodecError` and add structured
    logging fields for codec failures.
  - [ ] Define recovery policy hooks for malformed frames (drop, quarantine, or
    disconnect) and document the default behaviour.
  - [ ] Define how EOF mid-frame is surfaced to handlers or protocol hooks and
    add tests for partial-frame closure handling.
  - [ ] Add tests that validate error propagation, recovery policy, and
    structured logging fields.
- [ ] 7.6.1.3. Enable zero-copy payload extraction for codecs.
  - [ ] Update `FrameCodec::frame_payload` to return a `Bytes`-backed view (or
    equivalent) without forcing a `Vec<u8>` allocation.
  - [ ] Update the default codec adaptor to avoid `Bytes` to `Vec<u8>` copying
    on decode.
  - [ ] Add a regression test or benchmark to confirm payloads reuse the
    receive buffer where possible.

### 7.6.2. Fragment adaptor alignment

- [ ] 7.6.2.1. Introduce a `FragmentAdapter` trait as described in the
  fragmentation design.[^fragmentation-design] Fragmentation behaviour must
  explicitly define duplicate handling, out-of-order policies, and ownership of
  purge scheduling.
  - [ ] Make fragmentation opt-in by requiring explicit configuration on the
    `WireframeApp` builder.
  - [ ] Expose a public purge API, so callers can drive timeout eviction.
  - [ ] Document the composition order for codec, fragmentation, and
    serialization layers.
  - [ ] Define and implement duplicate suppression and out-of-order handling
    for fragment series.
  - [ ] Define and test zero-length fragment behaviour and fragment index
    overflow handling.
  - [ ] Add unit and integration tests for opt-in behaviour, interleaved
    reassembly, and duplicate/out-of-order fragments.

### 7.6.3. Unified codec handling

- [ ] 7.6.3.1. Unify codec handling between the app router and the
  `Connection` actor.[^outbound-design]
  - [ ] Route app-level request/response handling through the actor codec
    path so protocol hooks apply consistently.
  - [ ] Remove duplicate codec construction in `src/app/connection.rs` once
    the actor path owns framing.
  - [ ] Add integration tests covering streaming responses and push traffic
    through the unified path.
  - [ ] Add back-pressure tests for `Response::Stream` routed through the
    codec layer.[^streaming-design]

### 7.6.4. Property-based codec tests

- [ ] 7.6.4.1. Add property-based round-trip tests for the default
  `LengthDelimitedFrameCodec` and a mock protocol codec.
  - [ ] Cover boundary sizes and malformed frames using generated inputs.
  - [ ] Verify encoder/decoder stateful behaviour with generated sequences.

### 7.6.5. Serializer boundaries and protocol metadata

- [ ] 7.6.5.1. Decouple message encoding from `bincode`-specific traits to
  support alternative serializers.[^router-design][^adr-005]
  - [ ] Introduce a serializer-agnostic message trait or adaptor layer for
    `Message` types.
  - [ ] Provide optional wire-rs or Serde bridges to reduce manual boilerplate.
  - [ ] Define how frame metadata is exposed to the deserialization context to
    enable version negotiation.[^message-versioning]
  - [ ] Add migration guidance covering existing `bincode` users.

### 7.6.6. Codec performance benchmarks

- [ ] 7.6.6.1. Add targeted benchmarks for codec throughput and latency.
  - [ ] Benchmark encode/decode for small and large frames across the default
    codec and one custom codec.
  - [ ] Measure fragmentation overhead versus unfragmented paths.
  - [ ] Record memory allocation baselines for payload wrapping and decoding.

## Phase 8: Wireframe client library foundation

This phase delivers a first-class client runtime that mirrors the server's
framing, serialization, and lifecycle layers, so both sides share the same
behavioural guarantees.

- [ ] **Connection runtime:**

  - [x] Implement `WireframeClient` and its builder so callers can configure
    serializers, codec settings (including `max_frame_length` parity), and
    socket options before connecting.

  - [x] Integrate the existing preamble helpers so clients can emit and verify
    preambles before exchanging frames, with integration tests covering success
    and failure callbacks.

  - [ ] Expose connection lifecycle hooks (setup, teardown, error) that mirror
    the server hooks so middleware and instrumentation receive matching events.

- [ ] **Request/response pipeline:**

  - [ ] Provide async `send`, `receive`, and `call` APIs that encode `Message`
    implementers, forward correlation identifiers, and deserialize typed
    responses using the configured serializer.

  - [ ] Map decode and transport failures into `WireframeError` variants and
    add integration tests that round-trip multiple message types through a
    sample server.

- [ ] **Streaming and multi-packet parity:**

  - [ ] Support `Response::Stream` and `Response::MultiPacket` on the client by
    propagating back-pressure, validating terminator frames, and draining
    push traffic without starving request-driven responses.

  - [ ] Exercise interleaved high- and low-priority push queues to prove
    fairness and rate limits remain symmetrical.

- [ ] **Documentation and examples:**

  - [ ] Publish a runnable example where a client connects to the `echo`
    server, issues a login request, and decodes the acknowledgement.

  - [ ] Extend `docs/users-guide.md` and `docs/wireframe-client-design.md`
    with configuration tables, lifecycle diagrams, and troubleshooting
    guidance for the new APIs.

## Phase 9: Client ergonomics & extensions

This phase layers on the ergonomic features outlined in the client design
document so larger deployments can adopt the library confidently.

- [ ] **Middleware and observability:**

  - [ ] Add middleware hooks for outgoing requests and incoming frames so
    metrics, retries, and authentication tokens can be injected symmetrically
    with server middleware.

  - [ ] Provide structured logging and tracing spans around connect, send,
    receive, and stream lifecycle events, plus configuration for per-command
    timing.

- [ ] **Connection pooling and concurrency:**

  - [ ] Implement a configurable connection pool that preserves preamble state,
    enforces in-flight request limits per socket, and recycles idle
    connections.

  - [ ] Expose a `PoolHandle` API with fairness policies so callers can
    multiplex many logical sessions without violating back-pressure.

- [ ] **Streaming helpers and test utilities:**

  - [ ] Ship helper traits or macros for consuming streaming responses (for
    example typed iterators over `Response::Stream`) so multiplexed protocols
    remain ergonomic.

  - [ ] Publish reusable test harnesses that spin up an in-process server and
    client pair, allowing downstream crates to verify compatibility.

- [ ] **Docs and adoption:**

  - [ ] Update the user guide with migration advice for the pooled client and
    document known limitations or out-of-scope behaviours.

  - [ ] Add a troubleshooting section that enumerates the most common client
    misconfigurations (codec length mismatch, preamble errors, TLS issues) and
    how to detect them.

## Phase 10: Advanced Features & Ecosystem (Future)

This phase includes features that will broaden the library's applicability and
ecosystem.

- [ ] **Alternative Transports:**

  - [ ] Abstract the transport layer to support protocols other than raw TCP
    (e.g., WebSockets, QUIC).

- [ ] **Message Versioning:**

  - [ ] Implement a formal message versioning system to allow for protocol
    evolution.

  - [ ] Ensure version negotiation can consume codec metadata without leaking
    framing details into handlers.[^message-versioning]

- [ ] **Security:**

  - [ ] Provide built-in middleware or guides for implementing TLS.

## Phase 11: Documentation & Community (Ongoing)

Continuous improvement of documentation and examples is essential for adoption
and usability.

- [x] **Initial Documentation:**

  - [x] Write comprehensive doc comments for all public APIs.

  - [x] Create a high-level `README.md` and a `docs/contents.md`.

- [x] **Examples:**

  - [x] Create a variety of examples demonstrating core features (`ping_pong`,
    `echo`, `metadata_routing`, `async_stream`).

- [ ] **Website & User Guide:**

  - [ ] Develop a dedicated website with a detailed user guide.

  - [ ] Write tutorials for common use cases.

- [ ] **API Documentation:**

  - [ ] Ensure all public items have clear, useful documentation examples.

  - [ ] Publish documentation to `docs.rs`.

[^adr-0001]:
    Refer to [ADR 0001](./adr/0001-multi-packet-streaming-response-api.md).

[adr-0002]: adr/0002-streaming-requests-and-shared-message-assembly.md

[^fragmentation-design]:
    `docs/generic-message-fragmentation-and-re-assembly-design.md`.

[^outbound-design]: `docs/asynchronous-outbound-messaging-design.md`.

[^streaming-design]: `docs/multi-packet-and-streaming-responses-design.md`.

[^router-design]: `docs/rust-binary-router-library-design.md`.

[^message-versioning]: `docs/message-versioning.md`.

[^adr-005]: `docs/adr-005-serializer-abstraction.md`.
