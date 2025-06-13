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
  - [ ] Implement `WireframeApp` builder.
        Clarify method signatures (`new`, `route`, `service`, `wrap`),
        expose a consistent `Result<Self>` error strategy, and allow
        registration calls in any order for ergonomic chaining.
  - [x] Implement `WireframeServer`.
        Worker tasks are spawned using Tokio. Each thread receives its own
        `WireframeApp` instance from a factory closure. A Ctrl+C signal triggers
        graceful shutdown, notifying all workers to stop accepting new
        connections.
  - [x] Standardize supporting trait definitions.
        Provide naming conventions and generic bounds for the
        `FrameProcessor` trait, state extractors and middleware via
        `async_trait` and associated types.
  - [x] Provide a minimal, runnable example.
        Include imports and an async `main` so the snippet compiles out of
        the box.

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

## 2. Middleware and Extractors

- [ ] Develop a minimal middleware system and extractor traits for payloads,
  connection metadata, and shared state.

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

## 6. Community Engagement and Integration

- [ ] Collaborate with `wire-rs` for trait derivation and future enhancements
  (lines 1437-1442).
