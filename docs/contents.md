# Documentation contents

This page summarizes each file in the `docs/` directory. Use it as a quick
reference when navigating the project's design and architecture material.

## Design documents

- [Outbound messaging design](asynchronous-outbound-messaging-design.md)
  Comprehensive design for server-initiated pushes.
- [Message fragments](generic-message-fragmentation-and-re-assembly-design.md)
  Design for fragmenting and reassembling messages.
- [Streaming response design](multi-packet-and-streaming-responses-design.md)
  Design for handling multi-packet and streaming responses.
- [Router library design](rust-binary-router-library-design.md) In-depth design
  of the binary router library.
- [Client design](wireframe-client-design.md) Proposal for adding client-side
  support.
- [Frame metadata](frame-metadata.md) How frame metadata assists with routing
  decisions.
- [Message versioning](message-versioning.md) Approaches to message versioning
  and compatibility.
- [Preamble validator](preamble-validator.md) Validating client connection
  preambles.

## Requests for comments

- [RFC 0001: protocol-agnostic test harness lifecycle][rfc-0001]
  Proposed server lifecycle and custom-client connector layers for
  `wireframe_testing`.

[rfc-0001]: rfcs/0001-protocol-agnostic-test-harness-lifecycle.md

## Architectural decision records

- [ADR 001: multi-packet streaming response API](adr-001-multi-packet-streaming-response-api.md)
  Accepted response API for multi-packet and streaming replies.
- [ADR 002: streaming requests and shared message assembly](adr-002-streaming-requests-and-shared-message-assembly.md)
  Decision for request streaming and shared assembly behaviour.
- [ADR 003: replace Cucumber with rstest-bdd](adr-003-replace-cucumber-with-rstest-bdd.md)
  Decision to use Rust-native behaviour-driven development (BDD) tests.
- [ADR 004: pluggable protocol codecs](adr-004-pluggable-protocol-codecs.md)
  Decision for protocol codec extension points.
- [ADR 005: serializer abstraction](adr-005-serializer-abstraction.md)
  Decision for serializer boundaries and responsibilities.
- [ADR 006: test observability](adr-006-test-observability.md)
  Decision for test metrics, logs, and assertion support.
- [ADR 007: mutation testing with cargo-mutants](adr-007-mutation-testing-with-cargo-mutants.md)
  Decision for mutation testing expectations.
- [ADR 008: zero-copy public byte container](adr-008-zero-copy-public-byte-container.md)
  Accepted public byte-container model for the zero-copy payload migration.
- [ADR 009: owned-byte migration rollout](adr-009-vec-u8-migration-rollout.md)
  Rollout policy for owned-byte compatibility during migration.
- [ADR 010: transport frame boundary for zero-copy](adr-010-transport-frame-boundary-for-zero-copy.md)
  Decision for the frame boundary during zero-copy migration.

## Execution plans

- [Approve stable public byte container](execplans/10-1-1-approve-stable-public-byte-container.md)
  Completed execution plan for roadmap item `10.1.1` and ADR 008.
- [Execution plan directory](execplans/) Working plans for roadmap items,
  migrations, verification work, and documentation follow-up tasks.

## Roadmaps

- [Outbound messaging roadmap](asynchronous-outbound-messaging-roadmap.md) Task
  list for implementing asynchronous outbound messaging.
- [Wireframe 1.0 roadmap](wireframe-1-0-detailed-development-roadmap.md)
  Detailed tasks leading to Wireframe 1.0.
- [Project roadmap](roadmap.md) High-level development roadmap.
- [Zero-copy frame and payload migration roadmap](zero-copy-frame-and-payload-migration-roadmap.md)
  Phased plan for migrating from `Vec<u8>`-owned frame APIs to a zero-copy
  alternative.
<!-- markdownlint-disable-next-line MD013 -->
- [1.0 philosophy][philosophy] Philosophy and feature set for Wireframe 1.0.

[philosophy]:
the-road-to-wireframe-1-0-feature-set-philosophy-and-capability-maturity.md

## Testing

- [Multi-layered testing strategy](multi-layered-testing-strategy.md) Overview
  of the library's testing approach.
- [RSTest fixtures](rust-testing-with-rstest-fixtures.md) Using `rstest`
  fixtures for clean tests.
- [Mocking network outages](mocking-network-outages-in-rust.md) Tutorial for
  simulating network outages in tests.
- [Testing helpers](wireframe-testing-crate.md) Design and public API for the
  `wireframe_testing` companion crate.

## Operations and resilience

- [Resilience guide](hardening-wireframe-a-guide-to-production-resilience.md)
  Guidance on achieving production resilience.
- [Observability and operability](observability-operability-and-maturity.md)
  Guide to observability and operational maturity.

## Reference guides

- [Developers' guide](developers-guide.md) Canonical architectural vocabulary
  and naming invariants.
- [Frame = `Vec<u8>` inventory](frame-vec-u8-inventory.md) Inventory of frame-
  and payload-shaped APIs that still rely on owned byte vectors.
- [Formal verification methods](formal-verification-methods-in-wireframe.md)
  Recommended use of Kani, Stateright, and Verus in Wireframe.
- [Refactoring guide](complexity-antipatterns-and-refactoring-strategies.md)
  Strategies for taming code complexity and refactoring.
- [Builder pattern conventions](builder-pattern-conventions.md) Guidance for
  type-transitioning builders and reconstruction patterns.
- [Documentation style guide](documentation-style-guide.md) Conventions for
  writing project documentation.
- [Server configuration](server/configuration.md) Tuning accept loop backoff
  behaviour and builder options.
