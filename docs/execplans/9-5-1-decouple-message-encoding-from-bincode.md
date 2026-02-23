# 9.5.1 Decouple message encoding from bincode-specific traits

This Execution Plan (ExecPlan) is a living document. The sections
`Constraints`, `Tolerances`, `Risks`, `Progress`, `Surprises & Discoveries`,
`Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work
proceeds.

Status: COMPLETED (2026-02-22)

Maintenance cadence: update `Progress` immediately after each stage completes,
record architecture and scope choices in `Decision Log` when they are made, and
write `Outcomes & Retrospective` at Stage E completion.

No `PLANS.md` exists in this repository as of 2026-02-21.

## Purpose / big picture

Roadmap item `9.5.1` hardens the pluggable codec effort by removing
`bincode`-specific trait coupling from message encoding boundaries. The
implementation must preserve existing bincode flows, add a serialiser-agnostic
message adapter surface, introduce an optional bridge that reduces manual
boilerplate, and define how frame metadata reaches deserialisation for version
negotiation.

After this work, maintainers and library consumers can observe:

- Client and server messaging APIs encode and decode message types through a
  serialiser-agnostic adapter contract instead of direct `bincode` trait bounds.
- Existing bincode message types continue to work with minimal or zero source
  changes through compatibility shims.
- A feature-gated Serde (serialisation/deserialisation) bridge is available as
  an optional path to reduce manual adapter implementations.
- Deserializers can inspect frame metadata through an explicit context object,
  enabling protocol version negotiation patterns.
- Unit tests use `rstest`, behavioural tests use `rstest-bdd` v0.5.0, design
  decisions are recorded in `docs/adr-005-serializer-abstraction.md`, public
  interface changes are documented in `docs/users-guide.md`, and roadmap item
  `9.5.1` is marked done only after all quality gates pass.

Authority boundaries for documentation:

- This ExecPlan is the implementation staging guide and task tracker.
- `docs/adr-005-serializer-abstraction.md` is the source of truth for
  serializer-boundary architecture and trade-offs.
- `docs/users-guide.md` is the source of truth for public API contracts and
  migration guidance for library consumers.

## Constraints

- Preserve runtime behaviour for existing bincode users unless explicitly
  documented as changed in migration guidance.
- Keep `WireframeApp` and `WireframeClient` defaults on bincode-compatible
  behaviour, so existing applications do not need immediate serialiser rewrites.
- Use `rstest` for unit tests and `rstest-bdd` v0.5.0 for behavioural tests.
- Do not introduce `wire-rs` as a mandatory dependency. This item will satisfy
  the roadmap bridge requirement via an optional Serde bridge.
- Keep framing, fragmentation, correlation, and routing semantics unchanged
  except for introducing metadata visibility to deserialization context.
- Record design decisions in `docs/adr-005-serializer-abstraction.md`. If
  metadata context semantics change materially, update
  `docs/message-versioning.md` as well.
- Update `docs/users-guide.md` with public API and migration guidance for
  existing bincode users.
- Mark `9.5.1` and its child bullets done in `docs/roadmap.md` only after
  implementation, documentation updates, and all validations succeed.
- Follow repository quality gates and run commands with `set -o pipefail` and
  `tee` logging.

## Tolerances (exception triggers)

- Scope: if implementation requires touching more than 26 files or more than
  1,800 net changed lines, stop and re-scope before proceeding.
- Interface: if preserving source compatibility for bincode users requires
  impossible trait coherence trade-offs, stop and present options before making
  a breaking change.
- Dependencies: if the chosen optional bridge needs a new external crate beyond
  existing workspace dependencies, stop and escalate.
- Iterations: if the same failing root cause persists after 3 fix attempts,
  stop and document alternatives in `Decision Log`.
- Time: if a single stage takes more than one focused day without meeting its
  validation target, stop and re-plan.
- Ambiguity: if metadata context shape has multiple viable designs with
  materially different API ergonomics, stop and present options with trade-offs.

## Risks

- Risk: adapter trait design can create confusing generic bounds and regress
  ergonomics. Severity: high. Likelihood: medium. Mitigation: keep trait
  surface minimal, provide helper aliases, and document migration with
  before/after snippets.

- Risk: backward compatibility for bincode users can silently break if helper
  methods or trait imports move. Severity: high. Likelihood: medium.
  Mitigation: preserve compatibility shims and add explicit regression tests
  for legacy derive-and-send flows.

- Risk: metadata context plumbing can add allocations or copy-heavy paths in
  hot decode loops. Severity: medium. Likelihood: medium. Mitigation: expose
  borrowed metadata slices where possible and verify no extra heap allocation
  is introduced in default bincode path.

- Risk: behavioural tests may assert implementation details rather than user
  observable outcomes. Severity: medium. Likelihood: medium. Mitigation: keep
  behaviour-driven development (BDD) scenarios focused on round-trip
  compatibility, migration-visible behaviour, and version negotiation outcomes.

- Risk: optional bridge feature matrix can drift from defaults and rot.
  Severity: medium. Likelihood: medium. Mitigation: include targeted unit
  coverage for feature-gated code and ensure `make test` with `--all-features`
  exercises the bridge path.

## Progress

- [x] (2026-02-21 00:00Z) Drafted ExecPlan for roadmap item `9.5.1` with
      staged implementation and acceptance criteria.
- [x] Stage A complete: baseline analysis, adapter API decision, and context
      shape finalized.
- [x] Stage B complete: serializer-agnostic message adapter layer and bincode
      compatibility shim implemented.
- [x] Stage C complete: metadata context propagation wired through decode path
      and version-negotiation integration tests added.
- [x] Stage D complete: optional Serde bridge implemented and validated.
- [x] Stage E complete: documentation updates, roadmap completion, and full
      quality gate validation.

## Surprises & Discoveries

- Observation: there is no `PLANS.md`, so this ExecPlan is the sole execution
  guide for the item. Evidence: repository root listing on 2026-02-21. Impact:
  all execution details must remain self-contained in this file.

- Observation: `src/message.rs` currently defines `Message` as
  `bincode::Encode + bincode::BorrowDecode`, and serializer methods require
  that trait. Evidence: `src/message.rs`, `src/serializer.rs`. Impact: the
  coupling is direct and affects both client and server API bounds.

- Observation: `FrameMetadata::parse` already runs before fallback full
  deserialization in server connection handling. Evidence:
  `src/app/connection.rs::parse_envelope`. Impact: metadata context can be
  introduced without changing framing order.

- Observation: behavioural test harness is already on `rstest-bdd` v0.5.0 with
  strict compile-time validation. Evidence: `Cargo.toml` dev-dependencies and
  `Makefile` target `test-bdd`. Impact: new BDD coverage can follow existing
  `tests/features|fixtures|steps|scenarios` structure without runner migration
  work.

- Observation: `serde` is already a normal dependency.
  Evidence: `Cargo.toml`. Impact: the optional bridge requirement can be
  satisfied without adding a new crate dependency.

## Decision Log

- Decision: implement Architecture Decision Record (ADR-005) Option B
  (serializer-agnostic boundary plus adapters) using compatibility-first
  layering. Rationale: satisfies roadmap goals while minimizing disruption for
  current users. Date/Author: 2026-02-21 / Codex.

- Decision: satisfy the roadmap bridge requirement with an optional Serde
  bridge in this item; defer wire-rs bridge to a later roadmap task unless
  explicitly requested. Rationale: Serde is already available in the dependency
  graph and avoids introducing new external dependencies. Date/Author:
  2026-02-21 / Codex.

- Decision: add an explicit deserialization context object that carries frame
  metadata rather than relying on hidden globals or implicit thread-local
  state. Rationale: explicit context keeps API behaviour testable and supports
  version negotiation deterministically. Date/Author: 2026-02-21 / Codex.

## Outcomes & Retrospective

- Implemented serializer-agnostic adapter boundaries through
  `EncodeWith`/`DecodeWith` and `DeserializeContext`.
- Introduced metadata-context propagation in the inbound parse/deserialize
  path.
- Provided optional Serde bridge support behind feature `serializer-serde`.
- Validated serializer boundaries and metadata context handling with behavioural
  and unit tests.
- Updated `docs/adr-005-serializer-abstraction.md`,
  `docs/users-guide.md`, and `docs/roadmap.md` to reflect shipped behaviour.

## Context and orientation

Current coupling points and likely edit targets:

- `src/message.rs`:
  bincode-bound `Message` trait with `to_bytes` and `from_bytes` helpers.
- `src/serializer.rs`:
  `Serializer` trait requires `M: Message`; `BincodeSerializer` delegates to
  `Message` helpers.
- `src/app/connection.rs`:
  metadata parsing and fallback deserialization entry point for inbound frames.
- `src/client/runtime.rs` and `src/client/messaging.rs`:
  public client send/receive/call APIs currently bound to `Message`.
- `src/fragment/fragmenter.rs` and `src/fragment/reassembler.rs`:
  helper APIs that serialize/deserialize typed messages via `Message`.
- `tests/metadata.rs`:
  existing metadata-before-deserialize coverage to extend for context exposure.
- `tests/features/*` plus
  `tests/fixtures/*`, `tests/steps/*`, `tests/scenarios/*`: behavioural test
  harness based on `rstest-bdd`.

Reference documentation to keep aligned:

- `docs/roadmap.md` (`9.5.1` scope and done criteria).
- `docs/adr-005-serializer-abstraction.md` (architecture decision record).
- `docs/message-versioning.md` (metadata and version negotiation behaviour).
- `docs/users-guide.md` (consumer-facing API and migration guidance).
- `docs/generic-message-fragmentation-and-re-assembly-design.md`.
- `docs/multi-packet-and-streaming-responses-design.md`.
- `docs/the-road-to-wireframe-1-0-feature-set-philosophy-and-capability-maturity.md`.
- `docs/hardening-wireframe-a-guide-to-production-resilience.md`.
- `docs/rust-testing-with-rstest-fixtures.md`.
- `docs/reliable-testing-in-rust-via-dependency-injection.md`.
- `docs/rstest-bdd-users-guide.md`.
- `docs/rust-doctest-dry-guide.md`.

## Plan of work

### Stage A: baseline and boundary design (no behaviour changes)

Map every public and internal API currently bounded by `message::Message`, then
define the serializer-agnostic adapter surface and deserialization context
shape. Keep this stage source-compatible and compile-only where possible.

Planned edits:

- `src/message.rs` and new `src/message/*` support modules for adapter traits
  and context types.
- `src/serializer.rs` trait signature updates with compatibility defaults.
- `docs/adr-005-serializer-abstraction.md` design section update describing the
  chosen trait shape before implementation details are finalized.

Stage A validation:

- `cargo check --all-targets --all-features` passes.
- No runtime behaviour changes yet.

Go/no-go:

- Proceed only after adapter trait signatures and context representation are
  settled and documented in `Decision Log`.

### Stage B: implement serializer-agnostic adapters plus bincode compatibility

Introduce serializer-aware encode/decode adapter traits and keep bincode users
working via blanket compatibility shims.

Planned edits:

- `src/message.rs`:
  define adapter traits (encode/decode), context model, and compatibility
  helpers for existing bincode message types.
- `src/serializer.rs`:
  switch serializer methods to adapter trait bounds and add context-aware
  deserialization entry point.
- `src/prelude.rs` and any public re-exports in `src/lib.rs` that must expose
  new adapter types.
- Call-site updates in:
  `src/client/runtime.rs`, `src/client/messaging.rs`,
  `src/client/response_stream.rs`, `src/app/connection.rs`,
  `src/fragment/fragmenter.rs`, `src/fragment/reassembler.rs`, and related
  tests.

Stage B validation:

- Legacy bincode derive types still compile with unchanged call sites in core
  examples/tests.
- New unit tests (rstest) prove compatibility and adapter dispatch.

Go/no-go:

- Proceed only if bincode compatibility can be preserved without breaking
  default APIs.

### Stage C: expose frame metadata to deserialization context

Wire parsed metadata into a context object passed to deserializers, enabling
version-aware decode behaviour without changing framing order.

Planned edits:

- `src/app/connection.rs`:
  build context from `FrameMetadata::parse` output and pass it into
  deserializer entry points.
- `src/frame/metadata.rs` or `src/serializer.rs`:
  define stable metadata view semantics (what fields are guaranteed and when).
- `tests/metadata.rs`:
  extend coverage to assert context receives metadata and fallback behaviour
  remains correct.
- Add or extend unit tests around version negotiation plumbing, likely in
  `src/app/*` or `tests/*` integration modules.

Stage C validation:

- Metadata-aware decode tests pass for parse-success and parse-fallback paths.
- Existing routes and envelope decode behaviour remain unchanged for
  non-metadata serializers.

Go/no-go:

- Proceed only if context API remains explicit, deterministic, and free of
  hidden global state.

### Stage D: optional bridge and behavioural coverage

Implement a feature-gated Serde bridge and add BDD scenarios proving observable
compatibility and negotiation behaviour.

Planned edits:

- `Cargo.toml` feature table:
  add optional bridge feature flag(s) if needed (without new external crates).
- New bridge module(s) under `src/message/` or `src/serializer/` for Serde
  helper wrappers/adapters.
- New behavioural suite:
  `tests/features/serializer_boundaries.feature`,
  `tests/fixtures/serializer_boundaries.rs`,
  `tests/steps/serializer_boundaries_steps.rs`,
  `tests/scenarios/serializer_boundaries_scenarios.rs`, plus
  `tests/fixtures/mod.rs`, `tests/steps/mod.rs`, and `tests/scenarios/mod.rs`
  wiring.

Behavioural scenarios should cover:

- Existing bincode message workflow still succeeds.
- Optional bridge workflow reduces manual boilerplate and round-trips messages.
- Metadata-driven version selection path is observable via scenario outcomes.

Stage D validation:

- `make test-bdd` passes and includes new serializer-boundary scenario names.
- Bridge feature path is exercised under `--all-features`.

Go/no-go:

- Proceed only if BDD assertions remain user-observable and not coupled to
  private implementation internals.

### Stage E: docs, migration guidance, roadmap completion, and hardening

Finalize consumer and design documentation, then run full quality gates.

Planned edits:

- `docs/adr-005-serializer-abstraction.md`:
  record final API decisions, trade-offs, and status update.
- `docs/users-guide.md`:
  add migration guidance for bincode users and document new serializer adapter
  surfaces plus metadata context semantics.
- `docs/message-versioning.md`:
  update deserialization-context expectations if metadata contract changes.
- `docs/roadmap.md`:
  mark `9.5.1` and all child bullets done once validations pass.

Stage E validation:

- Documentation matches implemented API.
- Roadmap status matches shipped behaviour.
- All quality gates pass.

## Concrete steps

All commands run from repository root: `/home/user/project`.

1. Baseline before editing:

    ```shell
    set -o pipefail && cargo check --all-targets --all-features 2>&1 | tee /tmp/9-5-1-check-baseline.log
    set -o pipefail && make test-bdd 2>&1 | tee /tmp/9-5-1-test-bdd-baseline.log
    ```

2. After Stage B and Stage C edits, run focused verification:

    ```shell
    set -o pipefail && cargo test message --all-features 2>&1 | tee /tmp/9-5-1-message-tests.log
    set -o pipefail && cargo test metadata --all-features 2>&1 | tee /tmp/9-5-1-metadata-tests.log
    set -o pipefail && make test-bdd 2>&1 | tee /tmp/9-5-1-test-bdd.log
    ```

3. Final quality gates:

    ```shell
    set -o pipefail && make fmt 2>&1 | tee /tmp/9-5-1-fmt.log
    set -o pipefail && make check-fmt 2>&1 | tee /tmp/9-5-1-check-fmt.log
    set -o pipefail && make markdownlint 2>&1 | tee /tmp/9-5-1-markdownlint.log
    set -o pipefail && make nixie 2>&1 | tee /tmp/9-5-1-nixie.log
    set -o pipefail && make lint 2>&1 | tee /tmp/9-5-1-lint.log
    set -o pipefail && make test-doc 2>&1 | tee /tmp/9-5-1-test-doc.log
    set -o pipefail && make test 2>&1 | tee /tmp/9-5-1-test.log
    ```

Expected success indicators:

- Focused tests and `make test-bdd` include new serializer boundary coverage.
- `make lint` passes with zero warnings (`-D warnings`).
- `make test-doc` and `make test` pass across all enabled features.
- Markdown and formatting gates pass for documentation updates.

## Validation and acceptance

Acceptance is complete only when all the following are true:

- Public APIs no longer require direct `bincode` traits at serializer
  boundaries; adapter traits are used instead.
- Existing bincode users retain a clear migration path and compatibility
  behaviour.
- Optional bridge support is available (Serde path for this item) and covered
  by tests.
- Frame metadata is exposed to deserialization through a documented, tested
  context contract that supports version negotiation.
- Unit tests use `rstest`; behavioural tests use `rstest-bdd` v0.5.0.
- `docs/adr-005-serializer-abstraction.md` and `docs/users-guide.md` are
  updated, and `docs/roadmap.md` marks `9.5.1` as done.
- All commands listed in `Concrete steps` pass.

## Idempotence and recovery

- All validation commands are safe to re-run.
- If trait-bound changes cause widespread compile errors, first add temporary
  compatibility shims, then migrate call sites incrementally.
- If metadata context propagation breaks existing serializers, keep a default
  empty-context deserialization path while migrating serializers one by one.
- If behavioural tests become flaky, capture deterministic fixtures and inputs
  in world state and convert them into stable regression scenarios.
- If docs drift from implementation, stop roadmap completion until docs and
  code are aligned.

## Artifacts and notes

Keep the following artifacts during implementation:

- `/tmp/9-5-1-*.log` command logs.
- Final adapter trait signatures and context type definitions.
- List of migrated call sites that previously required `message::Message`.
- Names of new rstest unit tests and rstest-bdd scenarios.
- Migration examples added to `docs/users-guide.md`.

## Interfaces and dependencies

Target interface shape (names may vary slightly, intent must remain):

`DeserializeContext` fields must be projections of the canonical metadata model
returned by `FrameMetadata::parse` (`FrameMetadata::Frame`). If the metadata
model changes, update this context shape in lockstep (or expose that model
directly) to avoid drift.

```rust
pub struct DeserializeContext<'a> {
    // Keep these convenience fields aligned with identifiers extracted from
    // FrameMetadata::Frame in the active serializer implementation.
    pub frame_metadata: Option<&'a [u8]>,
    pub message_id: Option<u32>,
    pub correlation_id: Option<u64>,
    pub metadata_bytes_consumed: Option<usize>,
}

pub trait EncodeWith<S: Serializer> {
    fn encode_with(
        &self,
        serializer: &S,
    ) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>>;
}

pub trait DecodeWith<S: Serializer>: Sized {
    fn decode_with(
        serializer: &S,
        bytes: &[u8],
        context: &DeserializeContext<'_>,
    ) -> Result<(Self, usize), Box<dyn std::error::Error + Send + Sync>>;
}

pub trait Serializer {
    fn serialize<M>(&self, value: &M)
        -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>>
    where
        M: EncodeWith<Self>,
        Self: Sized;

    fn deserialize_with_context<M>(
        &self,
        bytes: &[u8],
        context: &DeserializeContext<'_>,
    ) -> Result<(M, usize), Box<dyn std::error::Error + Send + Sync>>
    where
        M: DecodeWith<Self>,
        Self: Sized;
}
```

Dependency expectations:

- No new mandatory dependencies.
- Continue using existing `rstest`, `rstest-bdd`, and `rstest-bdd-macros`
  versions already pinned in `Cargo.toml`.
- Optional bridge implementation should prefer existing `serde` dependency and
  feature-gating in crate features.

## Revision note

2026-02-21: Initial draft created for roadmap item `9.5.1`, defining staged
implementation, serializer-adapter direction, metadata-context requirements,
test strategy, migration documentation scope, and roadmap completion criteria.
