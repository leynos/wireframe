# Wireframe 1.0: Detailed Development Roadmap

<!-- markdownlint-disable MD013 MD056 -->

This document provides a granular, task-oriented development roadmap for the
features and capabilities outlined in "The road to Wireframe 1.0 - Feature-set,
Philosophy and Capability Maturity". It is intended to guide implementation by
breaking the project down into four distinct phases, each with a set of
well-defined tasks. The dependencies between tasks are explicitly noted to
ensure a logical and stable development progression.

As of this roadmap's publication, only the push queue utilities exist in
`src/push.rs`. No connection actor or write loop has been implemented. Phase 1
therefore begins by introducing the connection actor with its biased `select!`
loop and integrating the push queues.

## Phase 1: Foundational Mechanics

*Focus: Implementing the core, non-public machinery for duplex communication and
message processing. This phase establishes the internal architecture upon which
all public-facing features will be built.*

| 1.1 | Core Response & Error Types    | Define the new `Response<F, E>` enum with `Single`, `Vec`, `Stream` and `Empty` variants. Implement the generic `WireframeError<E>` enum to distinguish between I/O and protocol errors.                                 | Small  | -    |
| 1.2 | Priority Push Channels         | Implement the internal dual-channel `mpsc` mechanism within the connection state to handle high-priority and low-priority pushed frames.                                                                                 | Medium | -    |
| 1.3 | Connection Actor Write Loop    | Convert the per-request workers into stateful connection actors. Implement a `select!(biased; ...)` loop that polls for shutdown signals, high/low priority pushes and the handler response stream in that strict order. | Large  | #1.2 |
| 1.4 | Initial FragmentStrategy Trait | Define the initial `FragmentStrategy` trait and the `FragmentMeta` struct. Focus on the core methods: `decode_header` and `encode_header`.                                                                               | Medium | -    |
| 1.5 | Basic FragmentAdapter          | Implement the `FragmentAdapter` as a `FrameProcessor`. Build the inbound reassembly logic for a single, non-multiplexed stream of fragments and the outbound logic for splitting a single large frame.                   | Large  | #1.4 |
| 1.6 | Internal Hook Plumbing         | Add the invocation points for the protocol-specific hooks (`before_send`, `on_command_end`, etc.) within the connection actor, even if the public trait is not yet defined.                                              | Small  | #1.3 |

## Phase 2: Public APIs & Developer Ergonomics

*Focus: Exposing the new functionality to developers through a clean, ergonomic,
and idiomatic API. This phase is about making the powerful new mechanics usable
and intuitive.*

| 2.1 | WireframeProtocol Trait & Builder | Define the cohesive `WireframeProtocol` trait to encapsulate all protocol-specific logic. Refactor the `WireframeApp` builder to use a fluent `.with_protocol(MyProtocol)` method instead of multiple closures. | Medium | #1.6             |
| 2.2 | Public PushHandle API             | Implement the public `PushHandle<F>` struct with its `push`, `try_push` and policy-based `push_with_policy` methods. This handle will interact with the dual-channel system from #1.2.                          | Medium | #1.2             |
| 2.3 | Leak-Proof SessionRegistry        | Implement the `SessionRegistry` for discovering connection handles. This must use `dashmap` with `Weak<T>` pointers to prevent memory leaks from terminated connections.                                        | Medium | #2.2             |
| 2.4 | async-stream Integration & Docs   | Remove the proposed `FrameSink` from the design. Update the `Response::Stream` handling and write documentation recommending `async-stream` as the canonical way to create streams imperatively.                | Small  | #1.1             |
| 2.5 | Initial Test Suite                | Write unit and integration tests for the new public APIs. Verify that `Response::Vec` and `Response::Stream` work, and that `PushHandle` can successfully send frames that are received by a client.            | Large  | #2.1, #2.3, #2.4 |
| 2.6 | Basic Fragmentation Example       | Implement a simple `FragmentStrategy` (e.g. `LenFlag32K`) and an example showing the `FragmentAdapter` in use. This validates the adapter's basic functionality.                                                | Medium | #1.5, #2.5       |

## Phase 3: Production Hardening & Resilience

*Focus: Adding the critical features required for robust, secure, and reliable
operation in a production environment. This phase moves the library from
"functional" to "resilient".*

| 3.1 | Graceful Shutdown              | Implement the server-wide graceful shutdown pattern. Use tokio_util::sync::CancellationToken for signalling and tokio_util::task::TaskTracker to ensure all connection actors terminate cleanly.     | Large  | #1.3 |
| 3.2 | Re-assembly DoS Protection     | Harden the FragmentAdapter by adding a non-optional, configurable timeout for partial message re-assembly and strictly enforcing the max_message_size limit to prevent memory exhaustion.            | Medium | #1.5 |
| 3.3 | Multiplexed Re-assembly        | Enhance the FragmentAdapter's inbound logic to support concurrent re-assembly of multiple messages. Use the msg_id from FragmentMeta as a key into a dashmap::DashMap of partial messages.           | Large  | #3.2 |
| 3.4 | Per-Connection Rate Limiting   | Integrate an asynchronous, token-bucket rate limiter into the PushHandle. The rate limit should be configurable on the WireframeApp builder and enforced on every push.                              | Medium | #2.2 |
| 3.5 | Dead Letter Queue (DLQ)        | Implement the optional Dead Letter Queue mechanism. Allow a user to provide a DLQ channel sender during app setup; failed pushes (due to a full queue) can be routed there instead of being dropped. | Medium | #2.2 |
| 3.6 | Context-Aware FragmentStrategy | Enhance the FragmentStrategy trait. max_fragment_payload and encode_header should receive a reference to the logical Frame being processed, allowing for more dynamic fragmentation rules.           | Small  | #1.4 |

*Focus: Finalizing the library with comprehensive instrumentation, advanced
testing, and high-quality documentation to ensure it is stable, debuggable, and
ready for a 1.0 release.*

| 4.1 | Pervasive tracing instrumentation     | Instrument the entire library with `tracing`. Add `span!` calls for connection and request lifecycles and detailed `event!` calls for key state transitions (e.g., back-pressure applied, frame dropped, connection terminated).                                             | Large  | All        |
| 4.2 | Advanced Testing: Concurrency & Logic | Implement the advanced test suite. Use loom to verify the concurrency correctness of the select! loop and PushHandle. Use proptest for stateful property-based testing of complex protocol interactions (e.g., fragmentation and streaming).                                 | Large  | #3.3, #3.5 |
| 4.3 | Advanced Testing: Performance         | Implement the criterion benchmark suite. Create micro-benchmarks for individual components (e.g., PushHandle contention) and macro-benchmarks for end-to-end throughput and latency.                                                                                         | Medium | All        |
| 4.4 | Comprehensive User Guides             | Write the official documentation for the new features. Create separate guides for "Duplex Messaging & Pushes", "Streaming Responses", and "Message Fragmentation". Each guide must include runnable examples and explain the relevant concepts and APIs.                     | Large  | All        |
| 4.5 | High-Quality Examples                 | Create at least two complete, high-quality examples demonstrating real-world use cases. These should include server-initiated MySQL packets (e.g., LOCAL INFILE and session-state trackers) and a push-driven protocol such as WebSocket heart-beats or MQTT broker fan-out. | Medium | All        |
| 4.6 | Changelog & 1.0 Release               | Finalize the CHANGELOG.md with a comprehensive summary of all new features, enhancements, and breaking changes. Tag and publish the 1.0.0 release.                                                                                                                           | Small  | All        |
