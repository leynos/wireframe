# Documentation contents

This page summarizes each file in the `docs/` directory. Use it as a quick
reference when navigating the project's design and architecture material.

## Design documents

- [Wire protocol gap analysis survey](wire-protocol-gap-analysis-survey.md)
  Audit of external protocol support and proposed generic extension RFCs.

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

- [RFC 0001: protocol-agnostic test harness lifecycle](rfcs/rfc-0001-protocol-agnostic-test-harness-lifecycle.md)
  Proposed server lifecycle and custom-client connector layers for
  `wireframe_testing`.
- [RFC 0002: model-checking the write loop][rfc-0002] Proposed verification of
  the connection actor's scheduling decision, beyond what Loom can schedule.

[rfc-0002]: rfcs/rfc-0002-model-checking-the-connection-actors-write-loop.md

## Architectural decision records

- [ADR 001: multi-packet streaming response API](adrs/adr-0001-multi-packet-streaming-response-api.md)
  Accepted response API for multi-packet and streaming replies.
- [ADR 002: streaming requests and shared message assembly](adrs/adr-0002-streaming-requests-and-shared-message-assembly.md)
  Decision for request streaming and shared assembly behaviour.
- [ADR 003: replace Cucumber with rstest-bdd](adrs/adr-0003-replace-cucumber-with-rstest-bdd-for-behavioural-tests.md)
  Decision to use Rust-native behaviour-driven development (BDD) tests.
- [ADR 004: pluggable protocol codecs](adrs/adr-0004-pluggable-protocol-codecs.md)
  Decision for protocol codec extension points.
- [ADR 005: serializer abstraction](adrs/adr-0005-serializer-abstraction-beyond-bincode.md)
  Decision for serializer boundaries and responsibilities.
- [ADR 006: test observability](adrs/adr-0006-test-observability-harness.md)
  Decision for test metrics, logs, and assertion support.
- [ADR 007: mutation testing with cargo-mutants](adrs/adr-0007-mutation-testing-with-cargo-mutants.md)
  Decision for mutation testing expectations.
- [ADR 008: zero-copy public byte container](adrs/adr-0008-zero-copy-public-byte-container.md)
  Accepted public byte-container model for the zero-copy payload migration.
- [ADR 009: owned-byte migration rollout](adrs/adr-0009-rollout-strategy-for-the-vec-u8-to-zero-copy-migration.md)
  Rollout policy for owned-byte compatibility during migration.
- [ADR 010: transport frame boundary for zero-copy](adrs/adr-0010-transport-frame-boundary-for-the-zero-copy-migration.md)
  Accepted actor and codec-driver boundary for zero-copy transport framing.
- [ADR 011: runtime ownership and task-lifetime boundaries](adrs/adr-0011-runtime-ownership-and-task-lifetime-boundaries.md)
  Proposed policy for aligning `Arc`, spawning, and borrowing with real runtime
  lifetimes.
- [ADR 012: prepared application templates and connection-local runtimes](adrs/adr-0012-prepared-application-templates-and-connection-local-runtimes.md)
  Proposed separation of the application builder, prepared template, and
  connection runtime.
- [ADR 013: single-owner client-pool scheduler and slot graph](adrs/adr-0013-single-owner-client-pool-scheduler-and-slot-graph.md)
  Proposed persistent scheduler actor and index-based slot ownership for the
  client pool.
- [ADR 014: server shutdown ownership and terminal observation](adrs/adr-0014-server-shutdown-ownership-and-terminal-observation.md)
  Proposed awaitable server shutdown and supervisor-failure observation.
- [ADR 015: declared total body length is authoritative][adr-015] Accepted
  contract for enforcing `total_body_len` during message assembly.
- [ADR 016: forbid unsafe code workspace-wide](adr-016-unsafe-code-boundary.md)
  Proposed workspace lint table, per-root attribute, and enforcement checks.

[adr-015]: adrs/adr-0015-declared-total-body-length-is-authoritative.md

## Execution plans

- [Migrate serializer output to shared payload bytes][serializer-output-plan]
  Draft design for #538, including finite conversions and acceptance evidence.

- [Approve stable public byte container](execplans/10-1-1-approve-stable-public-byte-container.md)
  Completed execution plan for roadmap item `10.1.1` and ADR 008.
- [Approve actor and codec-driver boundary](execplans/10-1-3-approve-actor-and-codec-driver-boundary.md)
  Completed execution plan for roadmap item `10.1.3` and ADR 010.
- [Plan the Loom lane repair](execplans/plan-the-loom-lane-repair.md) Draft
  execution plan for making the scheduled Loom lane compile, run its models,
  and assert something a wrong implementation would fail.
- [Execution plan directory](execplans/) Working plans for roadmap items,
  migrations, verification work, and documentation follow-up tasks.
- [Migrate rstest-bdd to v0.6.0](execplans/adopt-rstest-bdd-v0-6-0.md)
  Implementation and validation plan for the published framework migration.

## Roadmaps

- [Outbound messaging roadmap](asynchronous-outbound-messaging-roadmap.md) Task
  list for implementing asynchronous outbound messaging.
- [Wireframe 1.0 roadmap](wireframe-1-0-detailed-development-roadmap.md)
  Detailed tasks leading to Wireframe 1.0.
- [Project roadmap](roadmap.md) High-level development roadmap.
<!-- markdownlint-disable-next-line MD013 -->
- [1.0 philosophy][philosophy] Philosophy and feature set for Wireframe 1.0.

[philosophy]:
the-road-to-wireframe-1-0-feature-set-philosophy-and-capability-maturity.md

## Zero-copy migration

- [Zero-copy frame and payload migration roadmap](zero-copy-frame-and-payload-migration-roadmap.md)
  Phased plan for migrating from `Vec<u8>`-owned frame APIs to a zero-copy
  alternative.
- [Zero-copy benchmark plan](zero-copy-benchmark-plan.md) Measurement matrix
  covering inbound decode, middleware pass-through, request hooks, and outbound
  encode.
- [Zero-copy benchmark baselines](zero-copy-benchmark-baselines.md) Measured
  baselines for every benchmark-matrix row, per-seam copied-byte counts, and
  proposed acceptance thresholds.
- [Zero-copy migration guide outline](zero-copy-migration-guide-outline.md)
  Affected public signatures and before-and-after examples for the migration
  guide.
- [Zero-copy release plan](zero-copy-release-plan.md) Release sequencing,
  canaries, validation gates, rollback criteria, and versioning determination.

## Testing

- [rstest-bdd user's guide](rstest-bdd-users-guide.md) Upstream framework guide
  imported from the published v0.6.0 source commit.
- [Wireframe rstest-bdd guide](wireframe-rstest-bdd-users-guide.md)
  Wireframe-specific notes preserved from the pre-migration guide.
- [rstest-bdd v0.6.0 migration guide](rstest-bdd-v0-6-0-migration-guide.md)
  Upstream migration reference imported from the published v0.6.0 source.
- [Multi-layered testing strategy](multi-layered-testing-strategy.md) Overview
  of the library's testing approach.
- [RSTest fixtures](rust-testing-with-rstest-fixtures.md) Using `rstest`
  fixtures for clean tests.
- [Mocking network outages](mocking-network-outages-in-rust.md) Tutorial for
  simulating network outages in tests.
- [Testing helpers](wireframe-testing-crate.md) In-process server and client
  pair helpers provided by the `wireframe_testing` companion crate.

## Migration guides

- [v0.3.0 to v0.4.0 migration guide](v0-3-0-to-v0-4-0-migration-guide.md)
  Moving from builder connection handling to reusable prepared applications.

## Operations and resilience

- [Resilience guide](hardening-wireframe-a-guide-to-production-resilience.md)
  Guidance on achieving production resilience.
- [Observability and operability](observability-operability-and-maturity.md)
  Guide to observability and operational maturity.

## Reference guides

- [Repository layout](repository-layout.md) Map of the repository's main
  source, tooling, and documentation areas.
- [Developers' guide](developers-guide.md) Canonical architectural vocabulary
  and naming invariants.
- [Frame = `Vec<u8>` inventory](frame-vec-u8-inventory.md) Inventory of frame-
  and payload-shaped APIs that still rely on owned byte vectors.
- [Formal verification methods](formal-verification-methods-in-wireframe.md)
  Recommended use of Kani, Stateright, and Verus in Wireframe.
- [Unsafe-code boundary verification](rust-boundary-verification.md) What the
  workspace-wide prohibition checks establish and what they cannot.
- [Refactoring guide](complexity-antipatterns-and-refactoring-strategies.md)
  Strategies for taming code complexity and refactoring.
- [Builder pattern conventions](builder-pattern-conventions.md) Guidance for
  type-transitioning builders and reconstruction patterns.
- [Documentation style guide](documentation-style-guide.md) Conventions for
  writing project documentation.
- [Runtime-ownership baselines](runtime-ownership-baselines.md) Recorded
  baselines for the four hot paths Epic #635 proposes to re-topologize.
- [Server configuration](server/configuration.md) Tuning accept loop backoff
  behaviour and builder options.

[serializer-output-plan]:
execplans/issue-538-plan-zero-copy-serializer-output-migration.md

## Protocol extension RFCs

- [RFC 0003: native message adapter boundary](rfcs/rfc-0003-native-message-adapter-boundary.md)
- [RFC 0004: duplex session driver](rfcs/rfc-0004-duplex-session-driver.md)
- [RFC 0005: negotiated transport entry points](rfcs/rfc-0005-negotiated-transport-entry-points.md)
- [RFC 0006: configurable framing primitives](rfcs/rfc-0006-configurable-framing-primitives.md)
- [RFC 0007: budgets, scheduling, and assembly](rfcs/rfc-0007-budgets-scheduling-and-assembly.md)
- [RFC 0008: external adapter conformance harness](rfcs/rfc-0008-external-adapter-conformance-harness.md)
- [RFC 0009: datagram session transport](rfcs/rfc-0009-datagram-session-transport.md)
