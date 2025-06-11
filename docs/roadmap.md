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
  - [ ] Implement `WireframeApp` builder to register routes, services,
    middleware, and shared state.
  - [ ] Implement `WireframeServer` to spawn per-worker apps, bind to an
    address, and run.
  - [ ] Create supporting types like the `FrameProcessor` trait, state
    extractors, and middleware trait.
  - [ ] Provide example code mirroring the design document usage snippet.

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
