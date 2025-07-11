# Wireframe Combined Development Roadmap

This document outlines the development roadmap for the Wireframe library, merging previous roadmap documents into a single source of truth. It details the planned features, enhancements, and the overall trajectory towards a stable and production-ready 1.0 release.

## Guiding Principles

- **Ergonomics:** The library should be intuitive and easy to use.

- **Performance:** Maintain high performance and low overhead.

- **Extensibility:** Provide clear extension points, especially through middleware.

- **Robustness:** Ensure the library is resilient and handles errors gracefully.

## Phase 1: Core Functionality & API (Complete)

This phase established the foundational components of the Wireframe server and the request/response lifecycle.

- [x] **Protocol Definition:**

  - [x] Define the basic frame structure for network communication (`src/frame.rs`).

  - [x] Implement preamble validation for versioning and compatibility (`src/preamble.rs`, `tests/preamble.rs`).

- [x] **Core Server Implementation:**

  - [x] Implement the `Server` struct with `bind` and `run` methods (`src/server.rs`).

  - [x] Handle incoming TCP connections and spawn connection-handling tasks (`src/connection.rs`).

  - [x] Define `Request`, `Response`, and `Message` structs (`src/message.rs`, `src/response.rs`).

- [x] **Routing & Handlers:**

  - [x] Implement a basic routing mechanism to map requests to handler functions (`src/app.rs`).

  - [x] Support handler functions with flexible, type-safe extractors (`src/extractor.rs`).

- [x] **Error Handling:**

  - [x] Establish a comprehensive set of error types.

  - [x] Implement `From` conversions for ergonomic error handling.

  - [x] Ensure `Display` is implemented for all public error types (`tests/error_display.rs`).

- [x] **Basic Testing:**

  - [x] Develop a suite of integration tests for core request/response functionality (`tests/server.rs`, `tests/routes.rs`).

## Phase 2: Middleware & Extensibility (Complete)

This phase focused on building the middleware system, a key feature for extensibility.

- [x] **Middleware Trait:**

  - [x] Design and implement the `Middleware` trait (`src/middleware.rs`).

  - [x] Define `Next` to allow middleware to pass control to the next in the chain.

- [x] **Middleware Integration:**

  - [x] Integrate the middleware processing loop into the `App` and `Connection` logic.

  - [x] Ensure middleware can modify requests and responses.

- [x] **Testing:**

  - [x] Write tests to verify middleware functionality, including correct execution order (`tests/middleware.rs`, `tests/middleware_order.rs`).

## Phase 3: Push Messaging & Async Operations (Complete)

This phase introduced capabilities for asynchronous, server-initiated communication and streaming.

- [x] **Push Messaging:**

  - [x] Implement the `Push` mechanism for sending messages from server to client without a direct request (`src/push.rs`).

  - [x] Develop `PushPolicies` for broadcasting messages to all or a subset of clients.

  - [x] Create tests for various push scenarios (`tests/push.rs`, `tests/push_policies.rs`).

- [x] **Async Stream Responses:**

  - [x] Enable handlers to return `impl Stream` of messages (`src/response.rs`).

  - [x] Implement the client and server-side logic to handle streaming responses (`examples/async_stream.rs`, `tests/async_stream.rs`).

## Phase 4: Advanced Connection Handling & State (Complete)

This phase added sophisticated state management and improved connection lifecycle control.

- [x] **Session Management:**

  - [x] Implement a `Session` struct to hold connection-specific state (`src/session.rs`).

  - [x] Create a `SessionRegistry` for managing all active sessions (`tests/session_registry.rs`).

  - [x] Provide `State` and `Data` extractors for accessing shared and session-specific data.

- [x] **Lifecycle Hooks:**

  - [x] Implement `on_connect` and `on_disconnect` hooks for session initialisation and cleanup (`src/hooks.rs`).

  - [x] Write tests to verify lifecycle hook behaviour (`tests/lifecycle.rs`).

- [x] **Graceful Shutdown:**

  - [x] Implement a graceful shutdown mechanism for the server, allowing active connections to complete their work.

## Phase 5: Production Hardening & Observability (In Progress)

This phase focuses on making the library robust, debuggable, and ready for production environments.

- [x] **Logging:**

  - [x] Integrate `tracing` throughout the library for structured, level-based logging.

  - [x] Create a helper crate for test logging setup (`wireframe_testing/src/logging.rs`).

- [ ] **Metrics & Observability:**

  - [ ] Expose key operational metrics (e.g., active connections, messages per second, error rates).

  - [ ] Provide an integration guide for popular monitoring systems (e.g., Prometheus).

- [ ] **Advanced Error Handling:**

  - [ ] Implement panic handlers in connection tasks to prevent a single connection from crashing the server.

- [ ] **Testing:**

  - [ ] Implement fuzz testing for the protocol parser (`tests/advanced/interaction_fuzz.rs`).

  - [ ] Use `loom` for concurrency testing of shared state (`tests/advanced/concurrency_loom.rs`).

## Phase 6: Application-Level Streaming (Multi-Packet Responses) (Priority Focus)

This is the next major feature set. It enables a handler to return multiple, distinct messages over time in response to a single request, forming a logical stream.

- [ ] **Protocol Enhancement:**

  - [ ] Add a `correlation_id` field to the `Frame` header. For a request, this is the unique request ID. For each message in a multi-packet response, this ID must match the original request's ID.

  - [ ] Define a mechanism to signal the end of a multi-packet stream, such as a frame with a specific flag and no payload.

- [ ] **Core Library Implementation:**

  - [ ] Introduce a `Response::MultiPacket` variant that contains a channel `Receiver<Message>`.

  - [ ] Modify the `Connection` actor: upon receiving `Response::MultiPacket`, it should consume messages from the receiver and send each one as a `Frame`.

  - [ ] Each sent frame must carry the correct `correlation_id` from the initial request.

  - [ ] When the channel closes, send the end-of-stream marker frame.

- [ ] **Ergonomics & API:**

  - [ ] Provide a clean API for handlers to return a multi-packet response, likely by returning a `(Sender<Message>, Response)`.

- [ ] **Testing:**

  - [ ] Develop integration tests where a client sends one request and receives multiple, correlated response messages.

  - [ ] Test that the end-of-stream marker is sent correctly and handled by the client.

  - [ ] Test client-side handling of interleaved multi-packet responses from different requests.

## Phase 7: Transport-Level Fragmentation & Reassembly

This phase will handle the transport of a single message that is too large to fit into a single frame, making the process transparent to the application logic.

- [ ] **Core Fragmentation & Reassembly (F&R) Layer:**

  - [ ] Define a generic `Fragment` header or metadata containing `message_id`, `fragment_index`, and `is_last_fragment` fields.

  - [ ] Implement a `Fragmenter` to split a large `Message` into multiple `Frame`s, each with a `Fragment` header.

  - [ ] Implement a `Reassembler` on the receiving end to collect fragments and reconstruct the original `Message`.

  - [ ] Manage a reassembly buffer with timeouts to prevent resource exhaustion from incomplete messages.

- [ ] **Integration with Core Library:**

  - [ ] Integrate the F&R layer into the `Connection` actor's read/write paths.

  - [ ] Ensure the F&R logic is transparent to handler functions; they should continue to send and receive complete `Message` objects.

- [ ] **Testing:**

  - [ ] Create unit tests for the `Fragmenter` and `Reassembler`.

  - [ ] Develop integration tests sending and receiving large messages that require fragmentation.

  - [ ] Test edge cases: out-of-order fragments, duplicate fragments, and reassembly timeouts.

## Phase 8: Advanced Features & Ecosystem (Future)

This phase includes features that will broaden the library's applicability and ecosystem.

- [ ] **Client Library:**

  - [ ] Develop a dedicated, ergonomic Rust client library for Wireframe.

- [ ] **Alternative Transports:**

  - [ ] Abstract the transport layer to support protocols other than raw TCP (e.g., WebSockets, QUIC).

- [ ] **Message Versioning:**

  - [ ] Implement a formal message versioning system to allow for protocol evolution.

- [ ] **Security:**

  - [ ] Provide built-in middleware or guides for implementing TLS.

## Phase 9: Documentation & Community (Ongoing)

Continuous improvement of documentation and examples is essential for adoption and usability.

- [x] **Initial Documentation:**

  - [x] Write comprehensive doc comments for all public APIs.

  - [x] Create a high-level `README.md` and a `docs/contents.md`.

- [x] **Examples:**

  - [x] Create a variety of examples demonstrating core features (`ping_pong`, `echo`, `metadata_routing`, `async_stream`).

- [ ] **Website & User Guide:**

  - [ ] Develop a dedicated website with a detailed user guide.

  - [ ] Write tutorials for common use cases.

- [ ] **API Documentation:**

  - [ ] Ensure all public items have clear, useful documentation examples.

  - [ ] Publish documentation to `docs.rs`.
