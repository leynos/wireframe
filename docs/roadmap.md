# Roadmap Summary

This document distills the key development goals from
[rust-binary-router-library-design.md](rust-binary-router-library-design.md)
after formatting. Line numbers below refer to that file.

## 1. Core Library Foundations

- [ ] Lines 316-345 outline the layered architecture comprising the Transport
  Layer Adapter, Framing Layer, Serialization engine, Routing engine, Handler
  invocation, and Middleware chain.

- [x] Implement derive macros or wrappers for message serialization (lines
  329-333).

- [ ] Build the Actix-inspired API around `WireframeApp` and `WireframeServer`
  as described in lines 586-676.

  - [x] Implement `WireframeApp` builder. Clarify method signatures (`new`,
    `route`, `service`, `wrap`), expose a consistent `Result<Self>` error
    strategy, and allow registration calls in any order for ergonomic chaining.

  - [x] Implement `WireframeServer`. Worker tasks are spawned using Tokio. Each
    thread receives its own `WireframeApp` instance from a factory closure. A
    Ctrl+C signal triggers graceful shutdown, notifying all workers to stop
    accepting new connections.

  - [x] Standardize supporting trait definitions. Provide naming conventions and
    generic bounds for the `FrameProcessor` trait, state extractors and
    middleware via `async_trait` and associated types.

  - [x] Provide a minimal, runnable example. Include imports and an async `main`
    so the snippet compiles out of the box.

    ```rust
    // No extra imports required
    use wireframe::{
        app::{Service, WireframeApp},
        server::WireframeServer,
    };

    async fn handler() {}

    #[tokio::main]
    async fn main() -> std::io::Result<()> {
        let factory = || {
            WireframeApp::new()
                .unwrap()
                .route(1, Box::new(|| Box::pin(handler())))
                .unwrap()
        };

        WireframeServer::new(factory)
            .bind("127.0.0.1:7878".parse().unwrap())?
            .run()
            .await
    }
    ```

- [x] Add connection preamble support. Provide generic parsing of connection
  preambles with a Hotline handshake example in the tests. Invoke
  user-configured callbacks on decode success or failure. See
  [preamble-validator](preamble-validator.md).

- [x] Add response serialization and transmission. Encode handler responses
  using the selected serialization format and write them back through the
  framing layer.

- [ ] Add connection lifecycle hooks. Integrate setup and teardown stages, so
  sessions can hold state (such as a logged-in user ID) across messages.

## 2. Middleware and Extractors

- [ ] Develop a minimal middleware system and extractor traits for payloads,
  connection metadata, and shared state.
  - [ ] Define `FromMessageRequest` for extractor types (lines 760-782). See
    [`FromMessageRequest`][from-message-request] in
    [`src/extractor.rs`](../src/extractor.rs).
  - [ ] Provide built-in extractors `Message<T>`, `ConnectionInfo`, and
    `SharedState<T>` (lines 792-840). `SharedState<T>` is defined in
    [`src/extractor.rs`](../src/extractor.rs#L54-L87).
  - [ ] Support custom extractors implementing `FromMessageRequest` (lines
    842-858). Refer again to [`src/extractor.rs`](../src/extractor.rs#L39-L52).
  - [ ] Implement middleware using `Transform`/`Service` traits and a simple
    `from_fn` style variant (lines 866-899). Trait definitions live in
    [`src/middleware.rs`](../src/middleware.rs#L71-L84).
  - [ ] Register middleware with `WireframeApp::wrap` and execute it in order
    (lines 900-919). See the [`wrap` method](../src/app.rs#L73-L84).
  - [ ] Document common middleware use cases like logging and authentication
    (lines 920-935).

## 3. Initial Examples and Documentation

- [ ] Provide examples demonstrating routing, serialization, and middleware.
  Document configuration and usage reflecting the API design section.

## 4. Extended Features

- [ ] Add UDP and other transport implementations (lines 1366-1379).
- [ ] Develop built-in `FrameProcessor` variants (lines 1381-1389).
- [ ] Address schema evolution and versioning strategies (lines 1394-1409).
- [ ] Investigate multiplexing and flow control mechanisms (lines 1411-1422).

## 5. Developer Tooling

- [ ] Create a CLI for protocol scaffolding and testing (lines 1424-1429).
- [ ] Improve debugging support and expand documentation (lines 1430-1435).
- [ ] Provide testing utilities for handlers. Offer simple ways to drive
  handlers with raw frames for unit tests. Early examples live in
  [`tests/server.rs`](../tests/server.rs); future helpers may reside in a
  `wireframe-testing` crate.

## 6. Community Engagement and Integration

- [ ] Collaborate with `wire-rs` for trait derivation and future enhancements
  (lines 1437-1442).

[from-message-request]: ../src/extractor.rs#L39-L52
