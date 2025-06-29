# Asynchronous Outbound Messaging Roadmap

This document tracks the tasks required to build reliable server-initiated
pushes. Completed work is ticked off. Section references point to the relevant
design documents.

## 1. Foundations

- [x] **Priority push queues** (`PushQueues`, `PushHandle`) with high and low
  channels. See [Design §3.1][design-queues].
- [x] **Unified `Response<F, E>` and `WireframeError<E>` types** to capture
  protocol errors and transport failures ([Roadmap #1.1][roadmap-1-1]).
- [x] **Connection actor** with a biased `select!` loop that polls for shutdown,
  high/low queues and response streams as described in
  [Design §3.2][design-write-loop].
- [x] **Fairness counter** to yield to the low-priority queue after bursts of
  high-priority frames ([Design §3.2.1][design-fairness]).
- [x] **Run state consolidation** using `Option` receivers and a closed source
  counter ([Design §3.4][design-actor-state]).
- [X] **Internal protocol hooks** `before_send` and `on_command_end` invoked
  from the actor ([Design §4.3][design-hooks]).

## 2. Public API and Ergonomics

- [ ] **`WireframeProtocol` trait** and builder integration to consolidate
  callbacks ([Roadmap #2.1][roadmap-2-1], [Design §4.3][design-hooks]).
- [x] **Public `PushHandle` API** with `push` and `try_push` methods
  ([Design §4.1][design-push-handle]).
- [x] **Leak-proof `SessionRegistry`** using `dashmap::DashMap` and `Weak`
  pointers ([Design §4.2][design-registry],
  [Resilience Guide §3.2][resilience-registry]).
- [x] **Document `async-stream`** for creating `Response::Stream` values
  ([Roadmap #2.4][roadmap-2-4]).
- [ ] **Example handler using `async-stream`** demonstrating `Response::Stream`
  generation in the examples directory.
- [ ] **Tests covering streams and push delivery** drawing on
  [Testing Guide §4][testing-guide-advanced].

## 3. Production Hardening

- [ ] **Graceful shutdown** using `CancellationToken` and `TaskTracker`
  ([Resilience Guide §2][resilience-shutdown]).
- [ ] **Typed `WireframeError`** for recoverable protocol errors
  ([Design §5][design-errors]).
- [ ] **Per-connection rate limiting** on pushes via a token bucket
  ([Resilience Guide §4.1][resilience-rate]).
- [ ] **Optional Dead Letter Queue** for full queues
  ([Design §5.2][design-dlq]).

## 4. Observability and Quality Assurance

- [ ] **Tracing instrumentation** for pushes and connections with metrics export
  ([Resilience Guide][resilience-guide]).
- [ ] **Concurrency tests with `loom`** and property tests with `proptest`
  ([Testing Guide §4.2][testing-loom], [Testing Guide §4.3][testing-proptest]).
- [ ] **Benchmarks** for push throughput using `criterion`
  ([Roadmap #4.3][roadmap-4-3]).
- [ ] **User guides and examples** demonstrating server-initiated messaging
  ([Design §7][design-use-cases]).

[design-actor-state]: asynchronous-outbound-messaging-design.md#34-actor-state-management
[design-dlq]: asynchronous-outbound-messaging-design.md#52-optional-dead-letter-queue-dlq-for-critical-messages
[design-errors]: asynchronous-outbound-messaging-design.md#5-error-handling--resilience
[design-fairness]: asynchronous-outbound-messaging-design.md#321-fairness-for-low-priority-frames
[design-hooks]: asynchronous-outbound-messaging-design.md#43-configuration-via-the-wireframeprotocol-trait
[design-push-handle]: asynchronous-outbound-messaging-design.md#41-the-pushhandle
[design-queues]: asynchronous-outbound-messaging-design.md#31-prioritised-message-queues
[design-registry]: asynchronous-outbound-messaging-design.md#42-the-sessionregistry
[design-use-cases]: asynchronous-outbound-messaging-design.md#7-use-cases
[design-write-loop]: asynchronous-outbound-messaging-design.md#32-the-prioritised-write-loop
[resilience-guide]: hardening-wireframe-a-guide-to-production-resilience.md
[resilience-rate]: hardening-wireframe-a-guide-to-production-resilience.md#41-throttling-with-per-connection-rate-limiting
[resilience-registry]: hardening-wireframe-a-guide-to-production-resilience.md#32-leak-proof-registries-with-weakarc
[resilience-shutdown]: hardening-wireframe-a-guide-to-production-resilience.md#2-coordinated-graceful-shutdown
[roadmap-1-1]: wireframe-1-0-detailed-development-roadmap.md
[roadmap-2-1]: wireframe-1-0-detailed-development-roadmap.md
[roadmap-2-4]: wireframe-1-0-detailed-development-roadmap.md
[roadmap-4-3]: wireframe-1-0-detailed-development-roadmap.md
[testing-guide-advanced]: multi-layered-testing-strategy.md#4-advanced-testing
[testing-loom]: multi-layered-testing-strategy.md#42-concurrency-fuzzing-with-loom
[testing-proptest]: multi-layered-testing-strategy.md#43-interaction-fuzzing-with-proptest
