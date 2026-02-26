# Implement hard cap connection abort for memory budgets (8.3.4)

This ExecPlan is a living document. The sections `Constraints`, `Tolerances`,
`Risks`, `Progress`, `Surprises & Discoveries`, `Decision Log`, and
`Outcomes & Retrospective` must be kept up to date as work proceeds.

Status: COMPLETE

## Purpose / big picture

Roadmap item `8.3.4` requires hard-cap connection-abort behaviour for
per-connection memory budgets: when buffered assembly bytes exceed 100% of the
configured aggregate cap, Wireframe must immediately abort the connection with
`std::io::ErrorKind::InvalidData` rather than continuing to process frames.

After this change, operators who configure `WireframeApp::memory_budgets(...)`
will have three complementary protections on inbound assembly paths:

- **Budget enforcement** (8.3.2): per-frame rejection when a single frame
  would exceed per-message, per-connection, or in-flight budgets. The offending
  assembly is freed; other assemblies survive.
- **Soft pressure** (8.3.3): paced reads under high buffered-byte pressure (at
  80% of aggregate cap). A 5 ms pause before polling the next frame propagates
  back-pressure to senders.
- **Hard cap** (8.3.4, this item): immediate connection abort when total
  buffered bytes exceed 100% of the aggregate cap. The connection is terminated
  with `InvalidData`. All partial assemblies are freed by the drop of
  `MessageAssemblyState`.

The distinction between 8.3.2 and 8.3.4 is subtle but important. Per-frame
enforcement (8.3.2) rejects individual frames that would push the total over
the limit, freeing the offending assembly. The hard cap is a defence-in-depth
safety net at the connection level: it checks the actual aggregate total at the
top of the read loop and aborts the entire connection if the total is already
over the limit. Under normal operation, per-frame enforcement prevents this
state from being reached; the hard cap catches the edge case where it is not.

Additionally, current budget violations are funnelled through the
`DeserFailureTracker`, meaning a single violation only increments the failure
counter (the connection closes after 10 failures). The hard cap provides a more
decisive response: one breach means immediate connection termination.

Success is observable when:

- the inbound read loop checks for hard-cap breach before polling the next
  frame;
- when the hard cap is breached, `process_stream` returns
  `Err(io::Error::new(InvalidData, ...))` immediately, terminating the
  connection;
- `rstest` unit tests validate the `has_hard_cap_been_breached()` policy
  helper and the combined `evaluate_memory_pressure()` function;
- `rstest-bdd` (v0.5.0) scenarios validate connection termination under budget
  violation and connection health within budget;
- design decisions are recorded in
  `docs/adr-002-streaming-requests-and-shared-message-assembly.md`;
- `docs/users-guide.md` explains the three-tier protection model; and
- `docs/roadmap.md` marks `8.3.4` as done only after all quality gates pass.

## Constraints

- Scope is strictly roadmap item `8.3.4`: implement hard-cap connection abort
  for inbound assembly pressure.
- Do not regress or re-scope budget enforcement semantics from `8.3.2` or
  soft-limit semantics from `8.3.3`.
- Preserve runtime behaviour when `memory_budgets` is not configured.
- Keep public API changes to zero. If a public API change is required, stop
  and escalate.
- Do not add new external dependencies.
- Keep all modified or new source files at or below the 400-line repository
  guidance.
- Validate with `rstest` unit tests and `rstest-bdd` behavioural tests.
- Follow testing guidance from: `docs/rust-testing-with-rstest-fixtures.md`,
  `docs/reliable-testing-in-rust-via-dependency-injection.md`, and
  `docs/rstest-bdd-users-guide.md`.
- Record implementation decisions in the relevant design document(s), primarily
  ADR 0002.
- Update `docs/users-guide.md` for any consumer-visible behavioural change.
- Mark roadmap item `8.3.4` done only after all gates pass.
- Use en-GB-oxendict spelling in all comments and documentation.

## Tolerances (exception triggers)

- Scope: if implementation requires changes to more than 16 files or more than
  800 net lines of code (LOC), stop and escalate.
- Interface: if implementing `8.3.4` requires a new public builder method,
  changed public function signatures, or new public types, stop and escalate.
- Dependencies: if any new crate is required, stop and escalate.
- Semantics ambiguity: if the project requires a specific hard-cap threshold
  other than 100% of `active_aggregate_limit_bytes`, stop and present options
  with trade-offs before finalizing behaviour.
- Iterations: if the same failing gate persists after 3 focused attempts, stop
  and escalate.
- Time: if any single stage exceeds 4 hours elapsed effort, stop and escalate.
- File length: if `src/app/inbound_handler.rs` would exceed 400 lines after
  changes, extract additional logic into `src/app/frame_handling/` helpers
  before continuing.

## Risks

- Risk: `src/app/inbound_handler.rs` is currently 392 lines (8 lines from the
  400-line cap). Adding the hard-cap check directly in the read loop may exceed
  the limit. Severity: high. Likelihood: high. Mitigation: introduce a combined
  `evaluate_memory_pressure()` function in `backpressure.rs` that returns a
  `MemoryPressureAction` enum. This replaces two separate `if` blocks with a
  single `match`, keeping the net line increase to approximately +6 lines (~398
  total).

- Risk: `src/message_assembler/state.rs` is 401 lines. Any modification would
  exceed the cap. Severity: medium. Likelihood: low. Mitigation: this plan does
  not modify `state.rs`. All new logic is in `backpressure.rs` and
  `inbound_handler.rs`, consuming the existing `total_buffered_bytes()` query
  from the outside.

- Risk: behaviour-driven development (BDD) step-text collisions with existing
  step definitions from `memory_budget_backpressure_steps.rs` or
  `memory_budgets_steps.rs`. Severity: medium. Likelihood: medium. Mitigation:
  all new step phrases will use a `hard-cap` prefix (e.g., "a hard-cap inbound
  app configured as …", "the hard-cap connection terminates with an error").

- Risk: The BDD test must prove the connection is terminated, which is harder
  to observe than a delayed payload. Severity: medium. Likelihood: medium.
  Mitigation: the BDD world will await the server `JoinHandle<io::Result<()>>`
  and check that it resolves with `Err`. The fixture exposes an
  `assert_connection_aborted()` method for this purpose.

## Progress

- [x] (2026-02-26) Drafted ExecPlan for roadmap item `8.3.4`.
- [x] (2026-02-26) Stage A: added `has_hard_cap_been_breached()`,
  `MemoryPressureAction`, and `evaluate_memory_pressure()` with unit coverage.
- [x] (2026-02-26) Stage B: integrated `evaluate_memory_pressure` into inbound
  read loop.
- [x] (2026-02-26) Stage C: added behavioural coverage (`rstest-bdd` v0.5.0).
- [x] (2026-02-26) Stage D: updated ADR, design docs, user guide, and roadmap.
- [x] (2026-02-26) Stage E: all quality gates passed.

## Surprises & Discoveries

- **Line-count management in `inbound_handler.rs`**: the initial `match` block
  pushed the file to 407 lines, well over the 400-line cap. Resolved by: (1)
  importing `MemoryPressureAction` at the top to shorten match arm paths (→402
  lines), and (2) compressing the frame-length warning `warn!()` from 4 lines
  to 2 (→400 lines). The combined `evaluate_memory_pressure()` approach was
  essential; two separate `if` blocks would have been unworkable.

- **Clippy `redundant_async_block`**: the BDD fixture initially used
  `async { server.await }` to wrap a spawned server handle, triggering a clippy
  warning. Resolved by calling `self.block_on(server)?` directly.

- **`pub(super)` visibility for test imports**: unit tests in
  `backpressure_tests.rs` import via `super::backpressure::`, requiring the
  individual helper functions (`has_hard_cap_been_breached`,
  `should_pause_inbound_reads`) to be `pub(super)` even though they are not
  re-exported from `mod.rs`. Only the combined `evaluate_memory_pressure` and
  `MemoryPressureAction` are `pub(crate)`.

## Decision Log

- Decision: place the hard-cap check at the TOP of the read loop in
  `process_stream`, before the soft-limit check. Rationale: the hard cap is a
  more severe condition than soft pressure. Checking it first ensures the
  connection is aborted before any further processing (including the sleep from
  soft pressure). Date/Author: 2026-02-26 / plan phase.

- Decision: use `>` (strictly exceeds) for the hard-cap comparison, not `>=`.
  Rationale: this matches the assembler's `check_aggregate_budgets` in
  `budget.rs` which uses `new_total > limit.get()`. The limit value itself is
  permitted; only exceeding it triggers the hard cap. Date/Author: 2026-02-26 /
  plan phase.

- Decision: do not explicitly purge partial assemblies before returning the
  error from `process_stream`. Rationale: when `process_stream` returns, its
  local `message_assembly: Option<MessageAssemblyState>` is dropped, which
  drops the internal `HashMap<MessageKey, PartialAssembly>`, freeing all
  partial buffers. An explicit purge would add complexity without benefit.
  Date/Author: 2026-02-26 / plan phase.

- Decision: the hard-cap check returns an `io::Error` directly from
  `process_stream`, bypassing `DeserFailureTracker`. Rationale: the hard cap is
  a connection-level safety net, not a per-frame failure. It should terminate
  the connection immediately, not increment a counter that requires 10 failures
  to close. Date/Author: 2026-02-26 / plan phase.

- Decision: reuse the existing `active_aggregate_limit_bytes()` helper (private
  in `backpressure.rs`) for the hard-cap threshold. Rationale: both soft limit
  and hard cap use the same aggregate dimension
  (`min(bytes_per_connection, bytes_in_flight)`). The soft limit triggers at
  80%; the hard cap triggers at 100%. Reusing the same helper ensures
  consistency. Date/Author: 2026-02-26 / plan phase.

- Decision: introduce a combined `evaluate_memory_pressure()` function
  returning `MemoryPressureAction` (`Continue | Pause(Duration) | Abort`).
  Rationale: keeps `inbound_handler.rs` under the 400-line cap by replacing two
  separate `if` blocks with a single `match`. Co-locates all memory-budget
  policy logic in `backpressure.rs`. Date/Author: 2026-02-26 / plan phase.

## Outcomes & Retrospective

All acceptance and quality criteria met. Gate results:

- `make fmt` — pass (reformatted imports in `backpressure_tests.rs`).
- `make check-fmt` — pass.
- `make markdownlint MDLINT=/root/.bun/bin/markdownlint-cli2` — pass.
- `make lint` — pass (after fixing `redundant_async_block`).
- `make test` — pass (363+ lib tests, 120 BDD tests including 2 new hard-cap
  scenarios, 0 failures).
- `make nixie` — pass (all mermaid diagrams validated).

Artefact count: 4 new files, 11 modified files — within the 16-file tolerance.
Net LOC well within the 800-line tolerance.

The combined `evaluate_memory_pressure()` approach proved effective at keeping
`inbound_handler.rs` at exactly 400 lines while co-locating all pressure policy
in `backpressure.rs`. The three-tier protection model (per-frame enforcement,
soft-limit pacing, hard-cap abort) is now fully implemented and documented.

## Context and orientation

### Current budget plumbing

- `src/app/memory_budgets.rs` defines `MemoryBudgets` (three `BudgetBytes`
  fields: `bytes_per_message`, `bytes_per_connection`, `bytes_in_flight`) and
  `BudgetBytes` (a `NonZeroUsize` wrapper). `MemoryBudgets` is `Copy`.
- `src/app/builder/config.rs` exposes `WireframeApp::memory_budgets(...)`
  which stores it as `Option<MemoryBudgets>` on the app.
- `src/app/frame_handling/assembly.rs::new_message_assembly_state` threads
  budgets into `MessageAssemblyState::with_budgets(...)`.
- `src/message_assembler/state.rs` enforces per-frame budget checks and
  exposes `total_buffered_bytes()` and `buffered_count()`.
- `src/message_assembler/budget.rs` contains `check_aggregate_budgets()` and
  `check_size_limit()`, invoked by `state.rs` during frame acceptance.

### Current inbound read path

- `WireframeApp::process_stream` in `src/app/inbound_handler.rs` (392 lines)
  drives the inbound loop:
  1. **Soft-limit check** (lines 281--288): calls
     `frame_handling::should_pause_inbound_reads(...)`. If `true`, pauses 5 ms
     and purges expired state.
  2. **Frame read** (lines 290--311):
     `timeout(timeout_dur, framed.next()).await`.
  3. **Frame processing** (delegated to `handle_frame`): decode, reassemble,
     assemble, route.
- `handle_frame` (lines 317--388) delegates to `decode_envelope`,
  `reassemble_if_needed`, `assemble_if_needed` (where per-frame budget
  enforcement happens via `DeserFailureTracker`), and `forward_response`.

### Soft-limit (8.3.3)

- `src/app/frame_handling/backpressure.rs` (48 lines):
  `should_pause_inbound_reads()` checks if `buffered_bytes >= 80%` of
  `active_aggregate_limit_bytes`. The private helper
  `active_aggregate_limit_bytes(budgets)` computes
  `min(bytes_per_connection, bytes_in_flight)` as `usize`.
  `soft_limit_pause_duration()` returns 5 ms.
- `src/app/frame_handling/backpressure_tests.rs` (119 lines): unit tests
  using `rstest`.
- `src/app/frame_handling/mod.rs` (30 lines): re-exports
  `should_pause_inbound_reads` and `soft_limit_pause_duration`.

### Behaviour-driven development (BDD) test pattern

The project uses a 4-file pattern for BDD tests with `rstest-bdd` v0.5.0:

1. **Feature file** (`tests/features/<name>.feature`): Gherkin scenarios.
2. **Fixture** (`tests/fixtures/<name>.rs`): a world struct with helper
   methods. Typically creates a `tokio::runtime::Runtime`, spawns server via
   `handle_connection_result`, uses `tokio::io::duplex` for client/server
   streams, and exposes assertion methods.
3. **Steps** (`tests/steps/<name>_steps.rs`): `#[given]`/`#[when]`/`#[then]`
   functions from `rstest_bdd_macros` that call methods on the world.
4. **Scenarios** (`tests/scenarios/<name>_scenarios.rs`): `#[scenario]`
   functions that bind feature file scenarios to the fixture.

Each new module must be registered in `tests/fixtures/mod.rs`,
`tests/steps/mod.rs`, and `tests/scenarios/mod.rs`. Step text must be globally
unique across all step files.

### Relevant docs to keep aligned

- `docs/roadmap.md` (`8.3.4` target row, currently unchecked).
- `docs/adr-002-streaming-requests-and-shared-message-assembly.md`.
- `docs/generic-message-fragmentation-and-re-assembly-design.md`.
- `docs/users-guide.md`.

## Plan of work

### Stage A: add policy helpers with unit tests

Create `has_hard_cap_been_breached()`, `MemoryPressureAction`, and
`evaluate_memory_pressure()` in `src/app/frame_handling/backpressure.rs`. Add
unit tests in `src/app/frame_handling/backpressure_tests.rs`. Update re-exports
in `src/app/frame_handling/mod.rs`.

The `has_hard_cap_been_breached()` function has the same signature as
`should_pause_inbound_reads()` and returns `true` when
`total_buffered_bytes() > active_aggregate_limit_bytes(budgets)`.

The `evaluate_memory_pressure()` function checks the hard cap first, then the
soft limit, returning `MemoryPressureAction::Abort`, `::Pause(Duration)`, or
`::Continue` respectively.

Unit tests cover:

- `has_hard_cap_been_breached`: no budgets, no state, below/at/above limit,
  smallest aggregate dimension.
- `evaluate_memory_pressure`: no budgets, below soft limit, at soft limit,
  above hard cap.

Go/no-go: if the policy helper requires access to non-public state, stop and
escalate.

### Stage B: integrate into inbound read loop

Replace the existing soft-limit block in `WireframeApp::process_stream` (lines
281--288 of `src/app/inbound_handler.rs`) with a single `match` on
`evaluate_memory_pressure(...)`. On `Abort`, return
`Err(io::Error::new( InvalidData, ...))`. On `Pause(d)`, sleep `d` and purge
expired state. On `Continue`, proceed normally.

Go/no-go: if the file exceeds 400 lines, extract further logic into
`src/app/frame_handling/` helpers.

### Stage C: add behavioural tests (`rstest-bdd` v0.5.0)

Add a dedicated BDD suite for hard-cap budget behaviour:

- `tests/features/memory_budget_hard_cap.feature`
- `tests/fixtures/memory_budget_hard_cap.rs`
- `tests/steps/memory_budget_hard_cap_steps.rs`
- `tests/scenarios/memory_budget_hard_cap_scenarios.rs`

Register modules in `tests/fixtures/mod.rs`, `tests/steps/mod.rs`,
`tests/scenarios/mod.rs`.

Scenarios:

1. **Connection terminates after budget violations**: send frames that trigger
   repeated per-frame budget rejections (10 failures close the connection via
   `DeserFailureTracker`). Assert the server `JoinHandle` resolves with error.
2. **Connection survives within budget**: send frames within budget, assert
   payload delivery and no connection error.

### Stage D: documentation and roadmap updates

- `docs/adr-002-streaming-requests-and-shared-message-assembly.md`: add 8.3.4
  implementation decisions.
- `docs/users-guide.md`: expand "Per-connection memory budgets" section.
- `docs/generic-message-fragmentation-and-re-assembly-design.md`: align
  wording if needed.
- `docs/roadmap.md`: mark `8.3.4` as done.

### Stage E: quality gates and final verification

Run all required gates with `set -o pipefail` and `tee` to log files. No
roadmap checkbox update until every gate is green.

## Concrete steps

Run from the repository root (`/home/user/project`).

1. Implement Stage A helper + unit tests.
2. Run focused unit tests:

```shell
set -o pipefail
cargo test --lib frame_handling 2>&1 | tee /tmp/wireframe-8-3-4-unit-a.log
```

1. Implement Stage B inbound loop refactor.
2. Run focused tests and lint:

```shell
set -o pipefail
cargo test --lib frame_handling 2>&1 | tee /tmp/wireframe-8-3-4-unit-b.log
make lint 2>&1 | tee /tmp/wireframe-8-3-4-lint-b.log
```

1. Implement Stage C BDD tests.
2. Run targeted BDD scenarios:

```shell
set -o pipefail
cargo test --test bdd --all-features memory_budget_hard_cap \
  2>&1 | tee /tmp/wireframe-8-3-4-bdd.log
```

1. Implement Stage D documentation updates.
2. Run full quality gates:

```shell
set -o pipefail
make fmt 2>&1 | tee /tmp/wireframe-8-3-4-fmt.log
make markdownlint MDLINT=/root/.bun/bin/markdownlint-cli2 \
  2>&1 | tee /tmp/wireframe-8-3-4-markdownlint.log
make check-fmt 2>&1 | tee /tmp/wireframe-8-3-4-check-fmt.log
make lint 2>&1 | tee /tmp/wireframe-8-3-4-lint.log
make test 2>&1 | tee /tmp/wireframe-8-3-4-test.log
make nixie 2>&1 | tee /tmp/wireframe-8-3-4-nixie.log
```

## Validation and acceptance

Acceptance criteria:

- Hard-cap behaviour: the inbound read loop checks for aggregate budget breach
  before polling the next frame. If breached, the connection is terminated with
  `InvalidData`.
- Soft-limit behaviour from `8.3.3` remains intact (no regressions).
- Budget enforcement from `8.3.2` remains intact (no regressions).
- Unit tests (`rstest`) validate both `has_hard_cap_been_breached()` and
  `evaluate_memory_pressure()` policy logic.
- Behavioural tests (`rstest-bdd` v0.5.0) validate connection health under
  budget and connection termination when budget is violated.
- Design documentation records hard-cap decisions in ADR 0002.
- User guide reflects the three-tier protection model.
- `docs/roadmap.md` marks `8.3.4` done.

Quality criteria:

- tests: `make test` passes.
- lint: `make lint` passes with no warnings.
- formatting: `make fmt` and `make check-fmt` pass.
- markdown: `make markdownlint` passes.
- mermaid validation: `make nixie` passes.

## Idempotence and recovery

All planned edits are additive and safe to rerun. If a step fails:

- preserve local changes;
- inspect the relevant `/tmp/wireframe-8-3-4-*.log` file;
- apply the minimal fix;
- rerun only the failed command first, then downstream gates.

Avoid destructive git commands. If rollback is required, revert only files
changed for `8.3.4`.

## Artefacts and notes

Expected artefacts after completion:

- Modified: `src/app/frame_handling/backpressure.rs`.
- Modified: `src/app/frame_handling/backpressure_tests.rs`.
- Modified: `src/app/frame_handling/mod.rs`.
- Modified: `src/app/inbound_handler.rs`.
- New: `tests/features/memory_budget_hard_cap.feature`.
- New: `tests/fixtures/memory_budget_hard_cap.rs`.
- New: `tests/steps/memory_budget_hard_cap_steps.rs`.
- New: `tests/scenarios/memory_budget_hard_cap_scenarios.rs`.
- Modified: `tests/fixtures/mod.rs`.
- Modified: `tests/steps/mod.rs`.
- Modified: `tests/scenarios/mod.rs`.
- Modified: `docs/adr-002-streaming-requests-and-shared-message-assembly.md`.
- Modified: `docs/users-guide.md`.
- Modified: `docs/roadmap.md`.
- New: `docs/execplans/8-3-4-hard-cap-behaviour.md`.
- Gate logs: `/tmp/wireframe-8-3-4-*.log`.

## Interfaces and dependencies

No new external dependencies are required.

Internal interfaces expected at the end of this milestone:

In `src/app/frame_handling/backpressure.rs`:

```rust
/// Action to take based on current memory budget pressure.
pub(crate) enum MemoryPressureAction {
    /// No pressure; proceed normally.
    Continue,
    /// Soft pressure; pause reads briefly before continuing.
    Pause(Duration),
    /// Hard cap breached; abort the connection immediately.
    Abort,
}

/// Evaluate memory budget pressure and return the appropriate action.
#[must_use]
pub(crate) fn evaluate_memory_pressure(
    state: Option<&MessageAssemblyState>,
    budgets: Option<MemoryBudgets>,
) -> MemoryPressureAction
```

In `src/app/inbound_handler.rs`:

- `process_stream` consults `evaluate_memory_pressure(...)` before polling
  `framed.next()`. On `Abort`, returns `Err(io::Error::new(InvalidData, ...))`.
  On `Pause(d)`, sleeps `d` and purges expired state. On `Continue`, proceeds
  normally.

In behavioural tests:

- A new `rstest-bdd` world (`MemoryBudgetHardCapWorld`) validates connection
  health and termination semantics with deterministic virtual-time control.
