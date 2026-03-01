# Write tests for budget enforcement, back-pressure, and cleanup (8.3.6)

This ExecPlan (execution plan) is a living document. The sections
`Constraints`, `Tolerances`, `Risks`, `Progress`, `Surprises & Discoveries`,
`Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work
proceeds.

Status: COMPLETE

## Purpose / big picture

Roadmap item `8.3.6` requires comprehensive test coverage for the three-tier
per-connection memory budget protection model implemented in items `8.3.1`
through `8.3.5`. The implementation is complete; this task adds tests that
exercise the gaps not covered by existing suites.

After this change an operator reviewing the test suite can verify that:

- budget headroom is correctly reclaimed after assembly completion and
  timeout purge, allowing subsequent frames to succeed in the freed space;
- partial assemblies are cleaned up via RAII when a connection closes;
- soft-limit pressure (80%) escalates to connection termination when
  per-frame budget rejections accumulate;
- pressure recovers after assembly completion (bytes drop below 80%),
  allowing subsequent frames without delay; and
- the tightest aggregate dimension (`min(bytes_per_connection,
  bytes_in_flight)
  `) controls soft and hard thresholds when the two dimensions differ.

Success is observable when:

- three new `#[tokio::test]` unit tests validate `apply_memory_pressure()`
  for each action variant (`Abort`, `Pause`, `Continue`);
- three new `rstest-bdd` v0.5.0 scenarios in a `budget_cleanup` suite
  validate timeout-purge reclamation, completion reclamation, and RAII cleanup
  on connection close;
- three new `rstest-bdd` v0.5.0 scenarios in a `budget_transitions` suite
  validate soft-to-hard escalation, soft-limit recovery, and
  tightest-dimension-wins enforcement;
- all existing tests continue to pass (`make test`);
- `docs/roadmap.md` marks `8.3.6` as done; and
- design decisions are recorded in ADR 0002.

## Constraints

- Scope is strictly roadmap item `8.3.6`: test coverage only. No changes to
  production code under `src/`.
- Do not regress or re-scope any behaviour from `8.3.1` through `8.3.5`.
- Keep all new files at or below the 400-line repository guidance.
- BDD step text must be globally unique across all step files. Use `cleanup-`
  and `transition-` prefixes for the two new suites.
- BDD world parameter names must match the fixture function name exactly.
- All runtime-backed BDD fixtures must call `tokio::time::pause()` before
  spawning the server task.
- Do not add new external dependencies.
- Validate with `rstest` unit tests and `rstest-bdd` v0.5.0 behavioural
  tests.
- Follow testing guidance from `docs/rust-testing-with-rstest-fixtures.md`,
  `docs/reliable-testing-in-rust-via-dependency-injection.md`, and
  `docs/rstest-bdd-users-guide.md`.
- Use en-GB-oxendict spelling in all comments and documentation.
- Record implementation decisions in ADR 0002.
- Mark roadmap item `8.3.6` done only after all gates pass.

## Tolerances (exception triggers)

- Scope: if implementation requires more than 15 new files or more than
  1200 net lines of code, stop and escalate.
- Interface: if any scenario requires a change to a public API signature or
  production code under `src/`, stop and escalate.
- Dependencies: if any new crate is required, stop and escalate.
- File length: if `src/app/frame_handling/backpressure_tests.rs` would
  exceed 400 lines with the new unit tests, extract the `apply_memory_pressure`
  tests into a separate `backpressure_apply_tests.rs` file using the
  `#[path = ...]` pattern.
- Iterations: if the same failing gate persists after 3 focused attempts,
  stop and escalate.

## Risks

- Risk: the `budget_cleanup` timeout-purge scenario depends on precise
  timing interaction between `tokio::time::pause()` auto-advance and the
  server's assembly timeout. If virtual time does not advance past the timeout
  before the next frame arrives, the purge will not fire. Severity: medium.
  Likelihood: low. Mitigation: use a short timeout (200 ms) in the config and
  advance virtual time by 201 ms explicitly before sending the next frame.

- Risk: the `budget_transitions` "recovery from soft limit" scenario may
  fail if the soft-limit sleep causes frame processing to stall in a way that
  prevents completion. Severity: medium. Likelihood: low. Mitigation: use
  `tokio::time::pause()` so the 5 ms sleep auto-advances, and use sufficient
  spin attempts (64) to drain payloads.

- Risk: file line counts for new fixtures may approach or exceed 400. The
  most complex existing fixture (`derived_memory_budgets.rs`) is 398 lines.
  Severity: medium. Likelihood: medium. Mitigation: share common patterns via
  method extraction; the cleanup fixture can omit the explicit-budgets startup
  path and only needs one `start_app` method variant. Target 340-350 lines per
  fixture.

## Progress

- [x] (2026-03-01) Drafted ExecPlan for roadmap item `8.3.6`.
- [x] (2026-03-01) Stage A: unit tests for `apply_memory_pressure` — 3 new
  `#[tokio::test]` functions, all passing (23 total backpressure tests).
- [x] (2026-03-01) Stage B: BDD suite `budget_cleanup` — 3 scenarios, all
  passing.
- [x] (2026-03-01) Stage C: BDD suite `budget_transitions` — 3 scenarios,
  all passing.
- [x] (2026-03-01) Stage D: documentation and roadmap updates.
- [x] (2026-03-01) Stage E: all quality gates passed (`fmt`, `check-fmt`,
  `lint`, `test`, `markdownlint`, `nixie`).

## Surprises & discoveries

- Observation: the `budget_cleanup` fixture initially included a
  `spin_runtime()` helper (copied from the `MemoryBudgetHardCapWorld` pattern)
  that was not needed by any of the three cleanup scenarios, triggering a
  `dead_code` compiler warning. Evidence: `cargo test` emitted
  `warning: method spin_runtime is never used`. Impact: removed the unused
  method. The cleanup scenarios do not need explicit runtime spinning because
  virtual-time auto-advance handles the soft-limit sleep and the fixture's
  `assert_payload_received` loop provides sufficient yielding.

- Observation: short fixture functions (where the total line width is under
  100 characters) are collapsed to a single line by `cargo fmt`. Clippy then
  rejects the single-line form with `unused_braces`. The established fix is
  `#[rustfmt::skip]` above `#[fixture]`, matching the pattern already used by
  `client_streaming.rs`, `client_runtime.rs`, `codec_test_harness.rs`, and
  others. Using `#[expect(unused_braces)]` does not work because the rstest
  proc-macro rewrites the function body, leaving the expectation unfulfilled.

## Decision log

- Decision: add two new BDD suites (`budget_cleanup` and
  `budget_transitions`) rather than extending existing suites. Rationale: keeps
  files under the 400-line cap; avoids step-text collisions with existing step
  definitions; groups scenarios by theme (cleanup semantics vs pressure
  transitions). Date/Author: 2026-03-01 / plan phase.

- Decision: use `ROUTE_ID = 92` and `CORRELATION_ID = Some(21)` for
  `budget_cleanup`, `ROUTE_ID = 93` and `CORRELATION_ID = Some(22)` for
  `budget_transitions`. Rationale: avoids collision with existing route IDs (7,
  42, 77, 88, 89, 91) and correlation IDs (5, 9, 10, 20). Date/Author:
  2026-03-01 / plan phase.

- Decision: use `#[tokio::test]` rather than `#[rstest]` for the
  `apply_memory_pressure` unit tests. Rationale: these tests are async and do
  not use fixtures; `#[tokio::test]` is the simplest appropriate harness.
  Date/Author: 2026-03-01 / plan phase.

- Decision: set the duplex buffer to 64 KB (64 * 1024) in both new BDD
  fixtures. Rationale: matches the `derived_memory_budgets.rs` fixture (line
  231) and avoids writer-side back-pressure that can stall tests when the
  buffer is too small. Date/Author: 2026-03-01 / plan phase.

## Outcomes & retrospective

All acceptance and quality criteria met on 2026-03-01.

**Deliverables**:

- 3 unit tests for `apply_memory_pressure` (backpressure_tests.rs: 339 → 384
  lines, within 400-line cap).
- 6 BDD scenarios across 2 new suites (`budget_cleanup` × 3,
  `budget_transitions` × 3), all passing.
- 8 new files created, 6 existing files modified, 15 artefacts total
  (including this execplan). Well within the 15-file tolerance.
- Roadmap `8.3.6` marked done; ADR 0002 updated with implementation decisions.

**Lessons**:

- Short `#[fixture]` functions are collapsed to single-line by `cargo fmt`,
  triggering clippy `unused_braces`. The fix is `#[rustfmt::skip]` above
  `#[fixture]`, matching the established pattern in the codebase.
  `#[expect(unused_braces)]` does not work because the rstest proc-macro
  rewrites the function body, leaving the expectation unfulfilled.
- The `spin_runtime()` helper pattern from `MemoryBudgetHardCapWorld` is not
  always needed. Cleanup scenarios work without it because virtual-time
  auto-advance handles soft-limit sleeps and `assert_payload_received` provides
  sufficient yielding.
- `if cond { panic!(...) }` triggers clippy `manual_assert`. Use `assert!` /
  `assert_eq!` instead.

## Context and orientation

### Three-tier per-connection memory protection

Wireframe provides three layers of inbound memory protection for connections
configured with `WireframeApp::memory_budgets(...)`:

1. **Per-frame enforcement** (`8.3.2`): implemented in
   `src/message_assembler/budget.rs`. The function `check_aggregate_budgets()`
   rejects individual frames whose addition would push `total_buffered_bytes()`
   over the per-connection or in-flight limit. The offending partial assembly
   is freed; other assemblies survive. Failures increment the
   `DeserFailureTracker` counter.

2. **Soft-limit read pacing** (`8.3.3`): implemented in
   `src/app/frame_handling/backpressure.rs`. The function
   `should_pause_inbound_reads()` returns `true` when buffered bytes reach 80%
   of `active_aggregate_limit_bytes()` (the smaller of `bytes_per_connection`
   and `bytes_in_flight`). The inbound read loop sleeps 5 ms and purges expired
   assemblies.

3. **Hard-cap connection abort** (`8.3.4`): also in `backpressure.rs`. The
   function `has_hard_cap_been_breached()` returns `true` when buffered bytes
   strictly exceed (`>`) the aggregate limit. The connection is terminated
   immediately with `InvalidData`, bypassing the `DeserFailureTracker`.

4. **Derived defaults** (`8.3.5`): in
   `src/app/frame_handling/backpressure.rs`. The function
   `resolve_effective_budgets()` derives budgets from `buffer_capacity` when
   none are explicitly configured, using 16x/64x/64x multipliers from
   `src/app/builder_defaults.rs`.

The combined `evaluate_memory_pressure()` function checks the hard cap first,
then the soft limit, returning a `MemoryPressureAction` enum (`Continue`,
`Pause(Duration)`, or `Abort`). The caller `apply_memory_pressure()` acts on
the result: returning `Err(InvalidData)` for `Abort`, sleeping and purging for
`Pause`, or doing nothing for `Continue`.

### Cleanup semantics

When a connection closes (normal EOF or error), `process_stream` in
`src/app/inbound_handler.rs` returns. Its local variable
`message_assembly: Option<MessageAssemblyState>` drops, which drops the
internal `HashMap<MessageKey, PartialAssembly>`, freeing all partial body and
metadata buffers. There is no custom `Drop` implementation; cleanup relies
entirely on Rust's RAII semantics.

Budget headroom is reclaimed at two points during normal operation:

- **Assembly completion**: when `accept_continuation_frame` returns
  `MessageSeriesStatus::Complete`, the entry is removed from the `HashMap` via
  `entry.remove()` (state.rs line 352). The next call to
  `total_buffered_bytes()` reflects the freed space.
- **Timeout purge**: `purge_expired_at()` removes assemblies that have
  exceeded their timeout, freeing all buffered bytes.

### Existing test coverage

**Unit tests** (`src/app/frame_handling/backpressure_tests.rs`, 339 lines):

- Soft-limit threshold at 79 / 80 / 95%.
- Hard-cap threshold at 99 / 100 / 101%.
- Combined `evaluate_memory_pressure` for all three action variants.
- Pause duration validation (5 ms).
- `resolve_effective_budgets` for explicit vs derived budgets.

**BDD tests** (rstest-bdd v0.5.0):

- `budget_enforcement` (6 scenarios): accept/reject frames, reclaim after
  completion, reclaim after timeout purge, single-frame bypass.
- `memory_budget_backpressure` (2 scenarios): soft pressure delays payload;
  low pressure continues.
- `memory_budget_hard_cap` (2 scenarios): connection terminates; connection
  survives.
- `derived_memory_budgets` (3 scenarios): derived budgets enforce, allow,
  explicit overrides.
- `memory_budgets` (4 scenarios): builder configuration, codec composition,
  zero-budget rejection.

### Gap analysis

The following areas are not exercised by existing tests:

1. **Cleanup semantics end-to-end**: no runtime-backed test sends frames
   that fill budget, purges/completes to free space, then verifies new frames
   succeed in the freed headroom. The existing `budget_enforcement` suite
   operates on `MessageAssemblyState` directly (no server, no connection).

2. **RAII cleanup on connection close**: no test verifies that partial
   assemblies are cleaned up when a client disconnects (normal EOF).

3. **Pressure transitions**: no test drives a connection from soft-limit
   into per-frame rejection, no test recovers from soft-limit by completing an
   assembly.

4. **Dimension interactions**: no end-to-end test demonstrates that the
   tighter aggregate dimension controls enforcement.

5. **`apply_memory_pressure` async function**: the function that maps
   `MemoryPressureAction` to I/O effects (error, sleep+purge, no-op) has no
   direct unit tests.

### Key files

| File                                           | Lines | Role                                  |
| ---------------------------------------------- | ----- | ------------------------------------- |
| `src/app/frame_handling/backpressure.rs`       | 148   | Production: pressure evaluation       |
| `src/app/frame_handling/backpressure_tests.rs` | 339   | Unit tests to extend                  |
| `src/app/frame_handling/mod.rs`                | 35    | Re-exports, test module declarations  |
| `src/message_assembler/budget.rs`              | 106   | Per-frame budget enforcement          |
| `src/message_assembler/state.rs`               | 401   | Assembly state and cleanup            |
| `src/app/inbound_handler.rs`                   | 400   | Inbound read loop (integration point) |
| `tests/fixtures/derived_memory_budgets.rs`     | 398   | BDD fixture pattern to follow         |
| `tests/fixtures/memory_budget_hard_cap.rs`     | 394   | BDD fixture pattern (abort)           |

## Plan of work

### Stage A: unit tests for `apply_memory_pressure`

Append three `#[tokio::test]` functions to
`src/app/frame_handling/backpressure_tests.rs` (currently 339 lines; target
approximately 384 lines after additions).

The tests import `apply_memory_pressure` from `super::backpressure` and
exercise each variant of `MemoryPressureAction`:

- `apply_memory_pressure_abort_returns_invalid_data`: calls
  `apply_memory_pressure(Abort, || {})` and verifies it returns `Err` with kind
  `InvalidData`.
- `apply_memory_pressure_pause_invokes_purge_closure`: calls
  `apply_memory_pressure(Pause(1ms), || purge_called = true)` inside a
  `tokio::time::pause()` context and verifies the closure was called.
- `apply_memory_pressure_continue_is_noop`: calls
  `apply_memory_pressure(Continue, || unreachable!())` and verifies `Ok(())`.

Go/no-go: if the file exceeds 400 lines, extract the new tests into a separate
`backpressure_apply_tests.rs` using the `#[path = ...]` pattern.

### Stage B: BDD suite `budget_cleanup`

Create a new BDD suite for cleanup semantics and budget reclamation through a
live connection.

**Feature file** (`tests/features/budget_cleanup.feature`):

Three scenarios:

1. **Timeout purge reclaims budget for subsequent frames**: a first frame
   for key 1 consumes 8 bytes of a 10-byte per-connection budget. Virtual time
   advances 201 ms (past the 200 ms assembly timeout). On the next read loop
   iteration, `purge_expired` fires, freeing key 1's 8 bytes. A subsequent
   first frame for key 2 then fits within the freed headroom, followed by a
   final continuation that delivers the payload.

2. **Completed assembly reclaims budget for subsequent frames**: a first
   frame for key 3 consumes 8 bytes; a final continuation completes it (freeing
   all 8 bytes). A subsequent first frame for key 4 then uses the reclaimed
   headroom, followed by a final continuation that delivers the payload.

3. **Connection close frees all partial assemblies**: two first frames for
   keys 5 and 6 are accepted (generous budget). The client disconnects (drops
   its half of the duplex stream). The server exits cleanly via EOF with no
   connection error.

**Fixture** (`tests/fixtures/budget_cleanup.rs`):

A `BudgetCleanupWorld` struct modelled on `DerivedMemoryBudgetsWorld`. Uses
`ROUTE_ID = 92`, `CORRELATION_ID = Some(21)`, 64 KB duplex buffer,
`tokio::time::pause()`. Config parsed from
`timeout_ms/per_message/per_connection/in_flight` using `CleanupConfig` with
`FromStr`. Methods: `start_app`, `send_first_frame`,
`send_final_continuation_frame`, `advance_millis`, `disconnect_client` (drops
the Framed half), `assert_payload_received`, `assert_no_connection_error`,
`join_server` (with 2 s timeout).

**Steps** (`tests/steps/budget_cleanup_steps.rs`):

Seven step functions using the `cleanup` prefix in step text:

- Given: `"a cleanup app configured as {config}"`
- When: `"a cleanup first frame for key {key} with body {body} arrives"`,
  `"a cleanup final continuation for key {key} sequence {seq} with body {body} arrives"`,
   `"cleanup virtual time advances by {millis} milliseconds"`,
  `"the cleanup client disconnects"`
- Then: `"cleanup payload {expected} is eventually received"`,
  `"no cleanup connection error is recorded"`

**Scenarios** (`tests/scenarios/budget_cleanup_scenarios.rs`):

Three `#[scenario]` functions binding the feature file scenarios to
`budget_cleanup_world`.

**Registration**: add `pub mod budget_cleanup;` to `tests/fixtures/mod.rs`,
`mod budget_cleanup_steps;` to `tests/steps/mod.rs`,
`mod budget_cleanup_scenarios;` to `tests/scenarios/mod.rs`.

### Stage C: BDD suite `budget_transitions`

Create a new BDD suite for pressure-level transitions and dimension
interactions through a live connection.

**Feature file** (`tests/features/budget_transitions.feature`):

Three scenarios:

1. **Soft pressure escalates to connection termination**: with
   per-connection = 10 and in-flight = 10 (aggregate limit 10), first frames
   with body `"aa"` (2 bytes each) are sent for keys 1 to 15. At 5 frames (10
   bytes), the budget is at the limit. Frame 6 triggers per-frame rejection.
   After 10 consecutive failures, `DeserFailureTracker` closes the connection.
   The test asserts connection termination.

2. **Recovery from soft limit after assembly completion**: with
   per-connection = 10 and in-flight = 10, a first frame for key 20 with body
   `"aaaaaaaa"` (8 bytes = 80%) puts the connection in the soft-limit zone. A
   final continuation completes the assembly, freeing all bytes. A subsequent
   first frame and continuation for key 21 succeed without delay.

3. **Tightest aggregate dimension controls enforcement**: with
   per-connection = 20 and in-flight = 10 (aggregate limit 10), first frames
   for keys 30 to 40 each with body `"aa"` are sent. Enforcement triggers at
   the in-flight limit (10), not the connection limit (20). The connection
   terminates.

**Fixture** (`tests/fixtures/budget_transitions.rs`):

A `BudgetTransitionsWorld` struct modelled on `MemoryBudgetHardCapWorld`. Uses
`ROUTE_ID = 93`, `CORRELATION_ID = Some(22)`, 64 KB duplex buffer,
`tokio::time::pause()`. Config parsed from
`timeout_ms/per_message/per_connection/in_flight` using `TransitionConfig` with
`FromStr`. Methods: `start_app`, `send_first_frames_for_range`,
`send_first_frame`, `send_final_continuation_frame`,
`assert_connection_aborted`, `assert_payload_received`,
`assert_no_connection_error`, `join_server`.

**Steps** (`tests/steps/budget_transitions_steps.rs`):

Seven step functions using the `transition` prefix in step text:

- Given: `"a transition app configured as {config}"`
- When: `"transition first frames for keys {start} to {end} each with body
  {body} arrive"`, `"a transition first frame for key {key} with body {body}
  arrives"`, `"a transition final continuation for key {key} sequence {seq}
  with body {body} arrives"`
- Then: `"the transition connection terminates with an error"`,
  `"transition payload {expected} is eventually received"`,
  `"no transition connection error is recorded"`

**Scenarios** (`tests/scenarios/budget_transitions_scenarios.rs`):

Three `#[scenario]` functions binding the feature file scenarios to
`budget_transitions_world`.

**Registration**: add `pub mod budget_transitions;` to `tests/fixtures/mod.rs`,
`mod budget_transitions_steps;` to `tests/steps/mod.rs`,
`mod budget_transitions_scenarios;` to `tests/scenarios/mod.rs`.

### Stage D: documentation and roadmap updates

1. Mark `docs/roadmap.md` line 302: change `- [ ] 8.3.6.` to
   `- [x] 8.3.6.`.
2. Add an implementation decisions section for `8.3.6` to
   `docs/adr-002-streaming-requests-and-shared-message-assembly.md`, recording
   that cleanup relies on RAII drop of `MessageAssemblyState` and that test
   coverage exercises all three protection tiers end-to-end.
3. Update this ExecPlan with final progress, surprises, and outcomes.

### Stage E: quality gates

Run all required gates with `set -o pipefail` and `tee` to log files:

```shell
set -o pipefail
make fmt 2>&1 | tee /tmp/wireframe-8-3-6-fmt.log
make markdownlint MDLINT=/root/.bun/bin/markdownlint-cli2 \
  2>&1 | tee /tmp/wireframe-8-3-6-markdownlint.log
make check-fmt 2>&1 | tee /tmp/wireframe-8-3-6-check-fmt.log
make lint 2>&1 | tee /tmp/wireframe-8-3-6-lint.log
make test 2>&1 | tee /tmp/wireframe-8-3-6-test.log
make nixie 2>&1 | tee /tmp/wireframe-8-3-6-nixie.log
```

No roadmap checkbox update until every gate is green.

## Concrete steps

Run from the repository root (`/home/user/project`).

1. Write the ExecPlan document (this file).
2. Implement Stage A unit tests.
3. Run focused unit tests:

```shell
set -o pipefail
cargo test --lib frame_handling::backpressure \
  2>&1 | tee /tmp/wireframe-8-3-6-unit-a.log
```

1. Implement Stage B BDD suite (`budget_cleanup`).
2. Run targeted BDD scenarios:

```shell
set -o pipefail
cargo test --test bdd --all-features budget_cleanup \
  2>&1 | tee /tmp/wireframe-8-3-6-bdd-cleanup.log
```

1. Implement Stage C BDD suite (`budget_transitions`).
2. Run targeted BDD scenarios:

```shell
set -o pipefail
cargo test --test bdd --all-features budget_transitions \
  2>&1 | tee /tmp/wireframe-8-3-6-bdd-transitions.log
```

1. Implement Stage D documentation updates.
2. Run full quality gates (Stage E).

## Validation and acceptance

Acceptance criteria:

- Three new `#[tokio::test]` unit tests for `apply_memory_pressure` pass.
- Three new `budget_cleanup` BDD scenarios pass.
- Three new `budget_transitions` BDD scenarios pass.
- All existing tests continue to pass (no regressions).
- `docs/roadmap.md` marks `8.3.6` as done.
- Design decisions recorded in ADR 0002.

Quality criteria:

- Tests: `make test` passes.
- Lint: `make lint` passes with no warnings.
- Formatting: `make fmt` and `make check-fmt` pass.
- Markdown: `make markdownlint` passes.
- Mermaid validation: `make nixie` passes.

## Idempotence and recovery

All planned edits are additive and safe to rerun. If a step fails:

- Preserve local changes.
- Inspect the relevant `/tmp/wireframe-8-3-6-*.log` file.
- Apply the minimal fix.
- Rerun only the failed command first, then downstream gates.

Avoid destructive git commands. If rollback is required, revert only files
changed for `8.3.6`.

## Artefacts and notes

Expected artefacts after completion:

- New: `tests/features/budget_cleanup.feature`
- New: `tests/fixtures/budget_cleanup.rs`
- New: `tests/steps/budget_cleanup_steps.rs`
- New: `tests/scenarios/budget_cleanup_scenarios.rs`
- New: `tests/features/budget_transitions.feature`
- New: `tests/fixtures/budget_transitions.rs`
- New: `tests/steps/budget_transitions_steps.rs`
- New: `tests/scenarios/budget_transitions_scenarios.rs`
- Modified: `src/app/frame_handling/backpressure_tests.rs`
- Modified: `tests/fixtures/mod.rs`
- Modified: `tests/steps/mod.rs`
- Modified: `tests/scenarios/mod.rs`
- Modified: `docs/roadmap.md`
- Modified: `docs/adr-002-streaming-requests-and-shared-message-assembly.md`
- New: `docs/execplans/8-3-6-tests-for-per-connection-memory-budgets.md`
- Gate logs: `/tmp/wireframe-8-3-6-*.log`

## Interfaces and dependencies

No new external dependencies are required.

No production interfaces are created or modified. This task adds test code only.

Internal test interfaces created:

In `tests/fixtures/budget_cleanup.rs`:

```rust
pub struct BudgetCleanupWorld { /* runtime, client, server, observed_rx, ... */ }

#[fixture]
pub fn budget_cleanup_world() -> BudgetCleanupWorld;

impl BudgetCleanupWorld {
    pub fn start_app(&mut self, config: CleanupConfig) -> TestResult;
    pub fn send_first_frame(&mut self, key: u64, body: &str) -> TestResult;
    pub fn send_final_continuation_frame(&mut self, key: u64, sequence: u32, body: &str) -> TestResult;
    pub fn advance_millis(&mut self, millis: u64) -> TestResult;
    pub fn disconnect_client(&mut self);
    pub fn assert_payload_received(&mut self, expected: &str) -> TestResult;
    pub fn assert_no_connection_error(&mut self) -> TestResult;
}
```

In `tests/fixtures/budget_transitions.rs`:

```rust
pub struct BudgetTransitionsWorld { /* runtime, client, server, observed_rx, ... */ }

#[fixture]
pub fn budget_transitions_world() -> BudgetTransitionsWorld;

impl BudgetTransitionsWorld {
    pub fn start_app(&mut self, config: TransitionConfig) -> TestResult;
    pub fn send_first_frames_for_range(&mut self, start: u64, end: u64, body: &str) -> TestResult;
    pub fn send_first_frame(&mut self, key: u64, body: &str) -> TestResult;
    pub fn send_final_continuation_frame(&mut self, key: u64, sequence: u32, body: &str) -> TestResult;
    pub fn assert_connection_aborted(&mut self) -> TestResult;
    pub fn assert_payload_received(&mut self, expected: &str) -> TestResult;
    pub fn assert_no_connection_error(&mut self) -> TestResult;
}
```
