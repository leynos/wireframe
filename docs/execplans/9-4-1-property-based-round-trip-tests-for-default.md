# 9.4.1 Property-based round-trip tests for default and mock codecs

This ExecPlan is a living document. The sections `Constraints`, `Tolerances`,
`Risks`, `Progress`, `Surprises & Discoveries`, `Decision Log`, and
`Outcomes & Retrospective` must be kept up to date as work proceeds.

Status: DRAFT

No `PLANS.md` exists in this repository as of 2026-02-19.

## Purpose / big picture

Roadmap item `9.4.1` hardens codec reliability by adding generated round-trip
tests for both `LengthDelimitedFrameCodec` and a mock protocol codec. The tests
must cover boundary payload sizes, malformed frame inputs, and stateful
encoder/decoder behaviour under generated sequences.

After this work, maintainers can run a deterministic property suite and observe:

- Default codec frame encoding/decoding is robust across generated boundary
  payloads and malformed wire data.
- A mock protocol codec demonstrates that stateful sequence semantics remain
  correct under generated operation sequences.
- Unit coverage uses `rstest`, behavioural coverage uses `rstest-bdd` v0.5.0,
  and the roadmap entry `9.4.1` is marked done.

## Constraints

- Keep public API source-compatible unless a change is unavoidable. If a
  public signature changes, document it in `docs/users-guide.md`.
- Use existing test stack: `rstest` for unit tests, `proptest` for generated
  input strategies, and `rstest-bdd` v0.5.0 for behavioural tests.
- Do not add new external dependencies for this item; required crates are
  already present in `Cargo.toml`.
- Keep test code deterministic enough for CI by pinning proptest case counts
  and avoiding non-deterministic global state.
- Record design decisions in a relevant design document. For this item, update
  `docs/adr-004-pluggable-protocol-codecs.md`.
- Update `docs/users-guide.md` if, and only if, the public interface for
  library consumers changes.
- Mark roadmap item `9.4.1` and its two child bullets as done in
  `docs/roadmap.md` once validation passes.
- Respect repository quality gates and run commands through `tee` with
  `set -o pipefail`.

## Tolerances (exception triggers)

- Scope: if completion requires changes to more than 14 files or 900 net
  changed lines, stop and re-scope.
- Interface: if `FrameCodec` or public `WireframeApp` signatures must change,
  stop and escalate before implementation.
- Dependencies: if any new crate is required, stop and escalate.
- Iterations: if the same failing root cause persists after 3 fix attempts,
  stop and record options in `Decision Log`.
- Time: if any single stage exceeds one focused day without reaching its stage
  acceptance criteria, stop and re-plan.
- Ambiguity: if "mock protocol codec" semantics are unclear enough to permit
  materially different tests, stop and present alternatives.

## Risks

- Risk: property tests can become flaky or slow in CI.
  Severity: medium. Likelihood: medium. Mitigation: bound case counts, use
  deterministic seeded runners where practical, and keep payload sizes
  constrained to codec limits.

- Risk: malformed-frame strategies may overfit one decoder path and miss real
  failure modes. Severity: medium. Likelihood: medium. Mitigation: generate
  multiple malformed classes (truncated header, truncated payload, oversized
  header length, random trailing bytes) and assert expected error categories.

- Risk: behavioural tests may duplicate unit coverage without proving
  observable behaviour. Severity: low. Likelihood: medium. Mitigation: keep BDD
  scenarios focused on externally observable outcomes (sequence ordering,
  connection isolation, malformed input handling paths).

- Risk: mock codec implementation used in tests may not model stateful
  semantics clearly. Severity: medium. Likelihood: medium. Mitigation:
  implement a dedicated test-only mock codec with explicit state transitions
  and assertions on sequence progression/reset.

## Progress

- [x] (2026-02-19 00:00Z) Draft ExecPlan for roadmap item `9.4.1`.
- [ ] Establish baseline and identify exact insertion points for new tests.
- [ ] Add generated unit tests for `LengthDelimitedFrameCodec` boundaries and
      malformed frames.
- [ ] Add generated unit tests for a mock stateful codec round-tripping
      sequence-aware frames.
- [ ] Add rstest-bdd behavioural scenarios and fixture plumbing for generated
      codec sequences.
- [ ] Update design/user docs and mark roadmap `9.4.1` done.
- [ ] Run full quality gates and capture logs.

## Surprises & Discoveries

- Observation: `proptest = "1.7.0"` is already present in
  `[dev-dependencies]`, so no dependency change is needed. Evidence:
  `Cargo.toml` current dev dependency list. Impact: implementation can focus on
  test content and runner determinism.

- Observation: `make test-bdd` runs `cargo test --test bdd --all-features`.
  Evidence: `Makefile` target `test-bdd`. Impact: BDD coverage can be validated
  independently and in full suite runs.

- Observation: repository already has stateful codec behavioural scaffolding in
  `tests/features/codec_stateful.feature` and corresponding world/steps.
  Evidence: `tests/features/codec_stateful.feature`,
  `tests/fixtures/codec_stateful.rs`, `tests/steps/codec_stateful_steps.rs`.
  Impact: new behavioural work can follow existing architecture and style.

## Decision Log

- Decision: implement generated checks as `rstest` test functions that execute
  bounded proptest runners internally. Rationale: satisfies roadmap demand for
  generated inputs while preserving the project convention that unit tests are
  authored with `rstest`. Date/Author: 2026-02-19 / Codex.

- Decision: add a dedicated test-only mock protocol codec for this item rather
  than reusing app-level integration codecs. Rationale: keeps stateful
  semantics explicit and avoids coupling property tests to unrelated
  integration concerns. Date/Author: 2026-02-19 / Codex.

- Decision: add a dedicated rstest-bdd feature file and world for generated
  codec behaviours. Rationale: preserves clear separation between existing
  stateful scenarios and new property-oriented acceptance behaviour.
  Date/Author: 2026-02-19 / Codex.

## Outcomes & Retrospective

To be completed when implementation is done.

## Context and orientation

`FrameCodec` is defined in `src/codec.rs`, with default implementation
`LengthDelimitedFrameCodec`. Existing unit tests for default codec behaviour
are in `src/codec/tests.rs`, and custom codec integration tests live in
`tests/frame_codec.rs`.

Current behavioural coverage uses `rstest-bdd` structure:

- Feature files in `tests/features/`.
- Worlds/fixtures in `tests/fixtures/`.
- Step bindings in `tests/steps/`.
- Scenario entrypoints in `tests/scenarios/`.

The new work should align with these files:

- `src/codec/tests.rs`
- `tests/features/`
- `tests/fixtures/mod.rs`
- `tests/steps/mod.rs`
- `tests/scenarios/mod.rs`
- `docs/adr-004-pluggable-protocol-codecs.md`
- `docs/users-guide.md`
- `docs/roadmap.md`

Reference documents that inform this plan:

- `docs/generic-message-fragmentation-and-re-assembly-design.md`
- `docs/multi-packet-and-streaming-responses-design.md`
- `docs/the-road-to-wireframe-1-0-feature-set-philosophy-and-capability-maturity.md`
- `docs/hardening-wireframe-a-guide-to-production-resilience.md`
- `docs/rust-testing-with-rstest-fixtures.md`
- `docs/reliable-testing-in-rust-via-dependency-injection.md`
- `docs/behavioural-testing-in-rust-with-cucumber.md` (historical context)
- `docs/rstest-bdd-users-guide.md`
- `docs/rust-doctest-dry-guide.md`

## Plan of work

### Stage A: Baseline and strategy scaffolding (no behaviour changes)

Confirm the baseline in `src/codec/tests.rs` and identify where generated
helpers should live. Define reusable strategy helpers for:

- Boundary payload sizes (empty, near-limit, exactly-limit, over-limit).
- Malformed wire input classes (truncated headers, truncated payloads,
  oversized length declarations, junk tails).
- Stateful operation sequences (encode/decode sequences and reset events) for
  a mock protocol codec.

Stage A validation:

- Existing tests still pass before adding new assertions.
- Strategy helpers compile and are isolated to test modules.

Go/no-go: proceed only if helpers can be introduced without touching public
interfaces.

### Stage B: Unit property coverage with rstest and generated inputs

Extend `src/codec/tests.rs` with new `rstest`-driven tests that execute bounded
generated runs:

- `LengthDelimitedFrameCodec` round-trip property for generated payloads with
  boundary emphasis.
- Malformed-frame property checks asserting expected decode/decode_eof error
  categories and non-panicking behaviour.
- Mock protocol codec property checks for stateful encoder/decoder sequences,
  including reset-per-connection semantics.

Prefer small helper functions over very long tests to keep readability high.

Stage B validation:

- New unit tests fail before implementation and pass after.
- Existing codec unit tests remain green.

Go/no-go: proceed only if generated tests are deterministic enough for repeated
local runs.

### Stage C: Behavioural coverage with rstest-bdd v0.5.0

Add behavioural tests that prove observable outcomes from generated sequences:

- New feature file for codec property behaviours (for example,
  `tests/features/codec_property_roundtrip.feature`).
- New fixture/world module encapsulating generated sequence execution and
  assertions.
- New step definitions and scenario entrypoints.
- Module wiring updates in `tests/fixtures/mod.rs`, `tests/steps/mod.rs`, and
  `tests/scenarios/mod.rs`.

Behavioural scenarios should verify:

- Boundary payload sequences round-trip through default codec.
- Malformed generated frames are rejected with expected behaviour.
- Mock codec sequence state advances correctly and resets per connection.

Stage C validation:

- `make test-bdd` passes with new scenarios.
- Scenario names and step text remain readable and business-facing.

Go/no-go: proceed only if BDD coverage adds observable confidence beyond unit
assertions.

### Stage D: Documentation, roadmap completion, and hardening

Update documentation after tests are stable:

- Record design choices in
  `docs/adr-004-pluggable-protocol-codecs.md` (strategy bounds, malformed class
  coverage, stateful sequence guarantees).
- Update `docs/users-guide.md` if public interface changed; if no public API
  changed, add no new API section.
- Mark roadmap entry `9.4.1` and both sub-bullets done in `docs/roadmap.md`.

Finish by running all required quality gates and preserving logs.

Stage D validation:

- Docs reflect implemented behaviour and decisions.
- Roadmap status is synchronized with delivered tests.
- Quality gates pass.

## Concrete steps

All commands run from repository root: `/home/user/project`.

1. Baseline checks before edits:

    set -o pipefail && make test-bdd 2>&1 | tee /tmp/9-4-1-test-bdd-baseline.log
    set -o pipefail && make test 2>&1 | tee /tmp/9-4-1-test-baseline.log

2. Implement Stage B and Stage C file changes, then run focused verification:

    set -o pipefail && cargo test codec --all-features 2>&1 | tee /tmp/9-4-1-codec-tests.log
    set -o pipefail && make test-bdd 2>&1 | tee /tmp/9-4-1-test-bdd.log

3. Run formatting and lint/test gates:

    set -o pipefail && make fmt 2>&1 | tee /tmp/9-4-1-fmt.log
    set -o pipefail && make check-fmt 2>&1 | tee /tmp/9-4-1-check-fmt.log
    set -o pipefail && make markdownlint 2>&1 | tee /tmp/9-4-1-markdownlint.log
    set -o pipefail && make nixie 2>&1 | tee /tmp/9-4-1-nixie.log
    set -o pipefail && make lint 2>&1 | tee /tmp/9-4-1-lint.log
    set -o pipefail && make test 2>&1 | tee /tmp/9-4-1-test.log

Expected success indicators:

- `make test-bdd` exits `0` and includes the new codec property scenario names.
- `make test` exits `0` with all targets passing.
- `make lint` exits `0` with no warnings (warnings denied).
- Markdown and formatting checks exit `0`.

## Validation and acceptance

Acceptance is behavioural and observable:

- Unit (rstest): generated boundary/malformed/stateful codec tests pass and are
  repeatable.
- Behavioural (rstest-bdd): feature scenarios pass and confirm sequence
  behaviour at user-observable level.
- Documentation: ADR and roadmap updates are present; users-guide reflects
  public interface impact when applicable.

Done criteria:

- `9.4.1` and both child bullets are checked in `docs/roadmap.md`.
- New tests fail prior to implementation and pass after implementation.
- All quality gate commands in `Concrete steps` pass.

## Idempotence and recovery

- All test and lint commands are safe to re-run.
- If generated tests are too slow or flaky, first lower case counts while
  preserving boundary/malformed coverage; record the change in `Decision Log`.
- If a behavioural scenario fails intermittently, capture seed/input details in
  the fixture output and keep the failing case as a deterministic regression.
- If documentation updates diverge from implementation, block roadmap completion
  until docs and behaviour match.

## Artifacts and notes

Capture and keep these artifacts during implementation:

- `/tmp/9-4-1-*.log` command logs.
- Names of newly added tests/scenarios.
- Any deterministic failing proptest seeds retained as regression examples.

## Interfaces and dependencies

No new runtime interfaces are expected. This item primarily adds tests and
documentation.

Expected touched interfaces and modules:

- `wireframe::codec::FrameCodec` test usage in `src/codec/tests.rs`.
- `LengthDelimitedFrameCodec::decoder`, `encoder`, `wrap_payload`,
  `frame_payload_bytes` exercised under generated inputs.
- Test-only mock codec implementing `FrameCodec` for stateful sequence checks.
- rstest-bdd scenario plumbing via:
  `tests/features/*`, `tests/fixtures/*`, `tests/steps/*`, `tests/scenarios/*`.

Dependency expectations:

- Continue using existing `rstest`, `proptest`, `rstest-bdd`, and
  `rstest-bdd-macros` versions already pinned in `Cargo.toml`.

## Revision note

2026-02-19: Initial draft created for roadmap item `9.4.1`, defining staged
implementation, quality gates, documentation obligations, and roadmap
completion criteria.
