# Implement budget enforcement for message assembly (8.3.2)

This execution plan (ExecPlan) is a living document. The sections
`Constraints`, `Tolerances`, `Risks`, `Progress`, `Surprises & Discoveries`,
`Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work
proceeds.

Status: COMPLETE

No `PLANS.md` exists in this repository as of 2026-02-21.

## Purpose / big picture

Roadmap item `8.3.2` adds runtime enforcement of the three memory budget
dimensions introduced by `8.3.1` (configuration-only). After this change, when
a library consumer configures `MemoryBudgets` on `WireframeApp`, the message
assembly subsystem will actively enforce:

- **bytes per message** — no single assembled message may exceed the configured
  cap;
- **bytes per connection** — the sum of all bytes buffered across every
  in-flight assembly on one connection may not exceed the configured cap; and
- **bytes in flight** — a second aggregate guard on buffered assembly bytes,
  today equivalent to the per-connection cap but expressed as a separate
  dimension so future work (streaming body buffers, transport-layer buffering)
  can differentiate them.

When a budget is exceeded the framework aborts the offending assembly, frees
its partial buffers, and surfaces `std::io::ErrorKind::InvalidData` through the
existing deserialisation-failure policy. No handler is invoked for the aborted
message. Other in-flight assemblies on the same connection are unaffected.

Success is observable when:

- Unit tests (`rstest`) prove that each budget dimension is enforced, that
  partial buffers are freed on violation, and that completed or purged
  assemblies reclaim budget headroom.
- Behavioural tests (behaviour-driven development, via the `rstest-bdd` v0.5.0
  crate) prove the enforcement behaviour in scenario form.
- `docs/users-guide.md` documents that configured budgets are now enforced.
- `docs/roadmap.md` marks `8.3.2` as done after all quality gates pass.
- Design decisions are recorded in the relevant design document.

## Constraints

- Scope is strictly roadmap item `8.3.2`: enforcement of the three budget
  dimensions during message assembly. Soft-limit back-pressure (`8.3.3`),
  hard-cap abort-early semantics (`8.3.4`), and derived defaults (`8.3.5`) are
  out of scope.
- Preserve current runtime behaviour when `MemoryBudgets` is not configured.
  The existing `max_message_size` guard derived from fragmentation config
  remains the sole per-message limit when budgets are absent.
- When both `max_message_size` (from fragmentation) and `bytes_per_message`
  (from `MemoryBudgets`) are present, use the minimum of the two so neither
  guard is silently overridden (per architecture decision record (ADR) 0002
  guidance on compatible values).
- Single-frame messages (`is_last: true` on first frame) are returned
  immediately and never buffered. They must pass the per-message size check but
  must not count against connection or in-flight budgets because they are never
  "in flight" in the assembly sense.
- Budget violations must abort the assembly for the offending message key,
  free its partial buffers, and surface `InvalidData` through the existing
  `DeserFailureTracker` path, consistent with ADR 0002 section "Failure modes
  and cleanup semantics".
- Do not add new external dependencies.
- Keep source modules under the 400-line project guidance.
- Validate with `rstest` unit tests and `rstest-bdd` behavioural tests.
- Record the enforcement design in the ADR 0002 implementation decisions.

## Tolerances (exception triggers)

- Scope: if this work requires touching more than 15 files or exceeds 600 net
  lines of code, stop and escalate.
- Interface: if implementing enforcement requires changing any existing public
  API signatures (other than adding new constructors or methods), stop and
  escalate.
- Dependencies: if any new crate is required, stop and escalate.
- Ambiguity: if the distinction between `bytes_per_connection` and
  `bytes_in_flight` requires different accounting than the sum of buffered
  assembly bytes, stop and present options with trade-offs.
- Iterations: if the same failing test or lint issue persists after 3 focused
  attempts, stop and escalate.

## Risks

- Risk: `state.rs` (365 lines) and `state_tests.rs` (384 lines) are both
  near the 400-line limit. Adding enforcement logic and tests may exceed it.
  Severity: medium. Likelihood: high. Mitigation: extract budget-checking
  functions into a new `src/message_assembler/budget.rs` module and budget
  tests into `src/message_assembler/budget_tests.rs`.

- Risk: `total_buffered_bytes()` is O(n) over all in-flight assemblies per
  accepted frame. Severity: low. Likelihood: low. Mitigation: acceptable for
  8.3.2 because concurrent assemblies per connection are typically in single
  digits. A cached running total can be added in a future optimisation pass if
  profiling warrants it.

- Risk: the conceptual overlap between `bytes_per_connection` and
  `bytes_in_flight` may confuse consumers. Severity: low. Likelihood: medium.
  Mitigation: keep them as separate checks with separate error variants and
  document the distinction clearly. Today they track the same accounting;
  future items may diverge when streaming body buffers are introduced.

- Risk: BDD step definitions may interact poorly with `Instant` for
  timeout-purge scenarios. Severity: low. Likelihood: low. Mitigation: use the
  existing `_at` method variants that accept an explicit `Instant`, matching
  the pattern in `state_tests.rs`.

## Progress

- [x] (2026-02-21) Drafted ExecPlan for roadmap item `8.3.2`.
- [x] Stage A: add budget enforcement to `MessageAssemblyState`.
- [x] Stage B: wire budgets through the integration layer.
- [x] Stage C: add unit tests (`rstest`) — 20 tests covering all budget
      dimensions.
- [x] Stage D: add behavioural tests (`rstest-bdd`) — 6 scenarios.
- [x] Stage E: documentation, roadmap, and quality gates.

## Surprises & discoveries

- `state.rs` hit 429 lines after adding budget fields and wiring. Extracted
  `check_size_limit` to `budget.rs` as planned by the risk mitigation.
- Borrow checker conflict in `accept_continuation_frame_at`: calling
  `self.total_buffered_bytes()` while holding a mutable `entry` borrow.
  Resolved with a snapshot pattern — read all immutable state into locals
  before the `self.assemblies.entry(key)` call.
- Clippy `too_many_arguments` triggered on `check_aggregate_budgets` (5 args).
  Introduced `AggregateBudgets` struct to bundle the two optional limits.
- BDD step text "the buffered count is" collided with an existing step in
  `message_assembly_steps.rs`. Renamed to "the active assembly count is" to
  avoid ambiguity.

## Decision log

- Decision: add budget tracking directly to `MessageAssemblyState` rather
  than creating a separate `BudgetEnforcer` wrapper. Rationale:
  `MessageAssemblyState` already owns the assembly map and is the sole
  authority over buffer lifecycle. A wrapper would duplicate or proxy all
  state-mutation methods with no clear separation-of-concerns benefit.
  Date/Author: 2026-02-21 / DevBoxer.

- Decision: use the minimum of `max_message_size` and `bytes_per_message`
  when both are present, rather than letting one override the other. Rationale:
  ADR 0002 states "the effective message cap is whichever guard triggers first"
  and recommends operators set compatible values. Using the minimum enforces
  this automatically. Date/Author: 2026-02-21 / DevBoxer.

- Decision: keep `bytes_per_connection` and `bytes_in_flight` as separate
  checks with separate error variants even though they currently track the same
  sum. Rationale: the ADR names them as distinct budget dimensions. The cost of
  two comparisons per frame is negligible, and divergence is expected in future
  items. Date/Author: 2026-02-21 / DevBoxer.

- Decision: extract budget-checking functions into
  `src/message_assembler/budget.rs` to keep `state.rs` under 400 lines and give
  enforcement a clear module boundary for future 8.3.3/8.3.4 work. Date/Author:
  2026-02-21 / DevBoxer.

- Decision: account for both `body_buffer.len()` and `metadata.len()` in
  `buffered_bytes()` because metadata is also heap-allocated and contributes to
  memory pressure. The existing `accumulated_len()` (body only) continues to
  serve the per-message size check. Date/Author: 2026-02-21 / DevBoxer.

## Outcomes & retrospective

Implementation complete. All quality gates pass.

**Files created:**

- `src/message_assembler/budget.rs` — budget enforcement helpers
  (`AggregateBudgets`, `check_aggregate_budgets`, `check_size_limit`)
- `src/message_assembler/budget_tests.rs` — 20 unit tests
- `tests/features/budget_enforcement.feature` — 6 BDD scenarios
- `tests/fixtures/budget_enforcement.rs` — `BudgetEnforcementWorld`
- `tests/steps/budget_enforcement_steps.rs` — step definitions
- `tests/scenarios/budget_enforcement_scenarios.rs` — scenario bindings

**Files modified:**

- `src/message_assembler/mod.rs` — registered `budget` module
- `src/message_assembler/state.rs` — added `AggregateBudgets` field,
  `with_budgets()` constructor, `total_buffered_bytes()`, wired budget checks
  (398 lines, within limit)
- `src/message_assembler/error.rs` — added `ConnectionBudgetExceeded` and
  `InFlightBudgetExceeded` variants
- `src/message_assembler/tests.rs` — registered `budget_tests` module
- `src/app/frame_handling/assembly.rs` — added `Option<MemoryBudgets>` param,
  effective per-message limit computation, budget wiring
- `src/app/connection.rs` — passes `self.memory_budgets` to assembly state
- `src/app/memory_budgets.rs` — updated doc comment
- `docs/roadmap.md` — marked 8.3.2 done
- `docs/users-guide.md` — documented enforcement semantics
- `docs/adr-002-streaming-requests-and-shared-message-assembly.md` — added
  implementation decisions
- `tests/fixtures/mod.rs`, `tests/steps/mod.rs`, `tests/scenarios/mod.rs` —
  registered budget enforcement modules

## Context and orientation

### Existing configuration surface (8.3.1 — complete)

`src/app/memory_budgets.rs` defines `MemoryBudgets` and `BudgetBytes`. The type
is stored as `memory_budgets: Option<MemoryBudgets>` on `WireframeApp` (in
`src/app/builder/core.rs`) and set via the
`WireframeApp::memory_budgets(budgets)` builder method (in
`src/app/builder/config.rs`). The three accessors are:

- `bytes_per_message() -> BudgetBytes`
- `bytes_per_connection() -> BudgetBytes`
- `bytes_in_flight() -> BudgetBytes`

Each wraps a `NonZeroUsize`.

### Message assembly subsystem (8.2 — complete)

`src/message_assembler/state.rs` defines `MessageAssemblyState`, which manages
concurrent in-flight assemblies keyed by `MessageKey`. Key methods:

- `new(max_message_size: NonZeroUsize, timeout: Duration)` — constructor.
- `accept_first_frame(input: FirstFrameInput)` — starts a new assembly;
  checks `max_message_size` against declared total body length.
- `accept_first_frame_at(input, now: Instant)` — explicit-clock variant.
- `accept_continuation_frame(header, body)` — continues an assembly; checks
  accumulated size via `check_size_limit()`.
- `accept_continuation_frame_at(header, body, now)` — explicit-clock variant.
- `purge_expired()` / `purge_expired_at(now)` — evicts stale assemblies.
- `buffered_count()` — number of in-flight assemblies.

Internal `PartialAssembly` tracks `series`, `routing`, `metadata`,
`body_buffer`, and `started_at`. The method `accumulated_len()` returns
`body_buffer.len()` (body bytes only).

`src/message_assembler/error.rs` defines `MessageAssemblyError` with variants
`Series(MessageSeriesError)`, `DuplicateFirstFrame { key }`, and
`MessageTooLarge { key, attempted, limit }`.

### Integration layer (8.2.5 — complete)

`src/app/frame_handling/assembly.rs` provides:

- `new_message_assembly_state(fragmentation, frame_budget)` — builds a
  `MessageAssemblyState` from fragmentation config or frame budget defaults.
- `assemble_if_needed(runtime, deser_failures, env, max_deser_failures)` —
  applies message assembly to a complete post-fragment envelope, routing errors
  through `DeserFailureTracker`.

The call site is in `src/app/connection.rs` line 257:

    frame_handling::new_message_assembly_state(self.fragmentation, requested_frame_length)

### Testing topology

- Unit tests: `rstest` in crate-local modules. Budget-related state tests
  currently live in `src/message_assembler/state_tests.rs` (384 lines).
- Behavioural tests: `rstest-bdd` fixtures and scenarios under
  `tests/features/`, `tests/fixtures/`, `tests/steps/`, and `tests/scenarios/`.
  The 8.3.1 configuration scenarios are in
  `tests/features/memory_budgets.feature`.
- Feature files use Gherkin syntax. Fixtures provide a `World` struct with
  helper methods. Steps map Gherkin phrases to Rust functions via procedural
  macros. Scenarios bind feature + fixture via `#[scenario]`.

### Relevant design documents

- `docs/adr-002-streaming-requests-and-shared-message-assembly.md` — sections
  on budget enforcement and failure modes.
- `docs/generic-message-fragmentation-and-re-assembly-design.md` — section
  9.3 on memory budget integration.
- `docs/hardening-wireframe-a-guide-to-production-resilience.md` — resource
  cap guidance.

## Plan of work

### Stage A: add budget enforcement to `MessageAssemblyState`

Create a new module `src/message_assembler/budget.rs` containing the budget
enforcement helpers. This module houses:

1. A `buffered_bytes()` method on `PartialAssembly` returning
   `body_buffer.len() + metadata.len()`.

2. A `total_buffered_bytes()` free function (or method) that sums
   `buffered_bytes()` across all assemblies.

3. A `check_aggregate_budgets()` function that takes the current total, the
   additional bytes about to be accepted, the message key, and optional
   `connection_budget` and `in_flight_budget` limits. It returns
   `Result<(), MessageAssemblyError>`.

Then modify `src/message_assembler/state.rs`:

1. Add two optional fields to `MessageAssemblyState`:
   `connection_budget: Option<NonZeroUsize>` and
   `in_flight_budget: Option<NonZeroUsize>`.

2. Add a new constructor `with_budgets(max_message_size, timeout,
   connection_budget,
   in_flight_budget)`. Refactor the existing `new()` to delegate to `
   with_budgets` with `None` for both budget fields.

3. Add a public `total_buffered_bytes(&self) -> usize` query method for
   diagnostics and testing.

4. Wire `check_aggregate_budgets()` into `accept_first_frame_at()` — after
   the existing `MessageTooLarge` check, before inserting the partial assembly
   — using `input.body.len() + input.metadata.len()` as the incoming bytes.

5. Wire `check_aggregate_budgets()` into `accept_continuation_frame_at()` —
   after the existing `check_size_limit` call, before `push_body` — using
   `body.len()` as the incoming bytes. On budget violation, call
   `entry.remove()` to free partial buffers before returning the error.

Modify `src/message_assembler/error.rs` to add two new variants to
`MessageAssemblyError`:

- `ConnectionBudgetExceeded { key, attempted, limit }`
- `InFlightBudgetExceeded { key, attempted, limit }`

Register the new module in `src/message_assembler/mod.rs`.

Go/no-go: if `state.rs` exceeds 400 lines after changes, extract more logic to
`budget.rs` before proceeding.

### Stage B: wire budgets through the integration layer

Modify `src/app/frame_handling/assembly.rs`:

1. Add `memory_budgets: Option<MemoryBudgets>` as a third parameter to
   `new_message_assembly_state`.

2. Inside the function, after computing `max_message_size` from fragmentation
   config, compute the effective per-message limit as the minimum of
   `max_message_size` and `budgets.bytes_per_message().get()` when budgets are
   configured.

3. Extract `connection_budget` and `in_flight_budget` from the budgets and
   pass all three to `MessageAssemblyState::with_budgets()`.

4. Add the `MemoryBudgets` import.

Modify `src/app/connection.rs`:

1. Pass `self.memory_budgets` as the third argument at the call site
   (line 257). `MemoryBudgets` is `Copy`, so no ownership issues.

Go/no-go: `make lint` and `make test` must pass before proceeding.

### Stage C: add unit tests (`rstest`)

Create `src/message_assembler/budget_tests.rs` to keep `state_tests.rs` under
400 lines. Register it in `src/message_assembler/tests.rs` with
`#[path = "budget_tests.rs"] mod budget_tests;`.

Tests to add (all using `rstest` fixtures):

**`total_buffered_bytes` accounting:**

- `state_reports_zero_buffered_bytes_when_empty` — fresh state returns 0.
- `state_reports_buffered_bytes_for_single_assembly` — after accepting one
  multi-frame first frame, total equals body + metadata length.
- `state_reports_buffered_bytes_for_multiple_assemblies` — after starting two
  assemblies, total is their sum.
- `state_reduces_buffered_bytes_after_completion` — completing an assembly
  reduces the total.
- `state_reduces_buffered_bytes_after_timeout_purge` — purging an expired
  assembly reduces the total.

**Connection budget enforcement:**

- `state_rejects_first_frame_exceeding_connection_budget` — single message
  whose body + metadata exceeds `connection_budget`.
- `state_rejects_continuation_exceeding_connection_budget` — second assembly
  pushes aggregate over `connection_budget`.
- `state_allows_frame_within_connection_budget` — multiple assemblies fitting
  within budget.
- `state_frees_partial_assembly_on_connection_budget_violation` —
  `buffered_count()` drops after rejection.

**In-flight budget enforcement:**

- `state_rejects_first_frame_exceeding_in_flight_budget` — analogous to
  connection budget tests.
- `state_rejects_continuation_exceeding_in_flight_budget` — analogous.
- `state_frees_partial_assembly_on_in_flight_budget_violation` — analogous.

**Per-message budget override:**

- `state_uses_minimum_of_max_message_size_and_budget` — construct with
  `max_message_size=1024` and `bytes_per_message=512`; verify 512 is the
  effective limit.

**Budget interaction with existing behaviour:**

- `state_budget_not_enforced_when_none` — default constructor allows any
  amount (backward compatibility).
- `state_budget_violation_does_not_affect_other_assemblies` — key 1 exceeds
  budget; key 2 continues unaffected.
- `state_timeout_purge_reclaims_budget_headroom` — after purge,
  previously-blocked frame now fits.

Go/no-go: all new and existing unit tests pass.

### Stage D: add behavioural tests (`rstest-bdd`)

Create `tests/features/budget_enforcement.feature` with scenarios covering the
observable enforcement behaviours:

1. Accept frames within all budget limits.
2. Reject frame exceeding per-message budget.
3. Reject continuation exceeding connection budget; verify partial freed.
4. Reject continuation exceeding in-flight budget; verify partial freed.
5. Reclaim budget headroom after assembly completes.
6. Reclaim budget headroom after timeout purge.

Create supporting files:

- `tests/fixtures/budget_enforcement.rs` — `BudgetEnforcementWorld` fixture.
- `tests/steps/budget_enforcement_steps.rs` — step definitions.
- `tests/scenarios/budget_enforcement_scenarios.rs` — `#[scenario]` bindings.

Register the new files in `tests/fixtures/mod.rs`, `tests/steps/mod.rs`, and
`tests/scenarios/mod.rs`.

Go/no-go: all BDD scenarios pass via `make test`.

### Stage E: documentation, roadmap, and quality gates

1. Update `src/app/memory_budgets.rs` module doc comment.
2. Update `docs/users-guide.md` "Per-connection memory budgets" section.
3. Update `docs/adr-002-streaming-requests-and-shared-message-assembly.md`.
4. Mark `8.3.2` as done in `docs/roadmap.md`.
5. Run full quality gates.

Go/no-go: do not mark roadmap done until all gates pass.

## Concrete steps

Run from repository root (`/home/user/project`).

1. Create `src/message_assembler/budget.rs` with budget enforcement helpers.

2. Modify `src/message_assembler/mod.rs` to register the new module.

3. Add error variants to `src/message_assembler/error.rs`.

4. Modify `src/message_assembler/state.rs`: add fields, `with_budgets()`
   constructor, `total_buffered_bytes()` query, and wire budget checks into
   `accept_first_frame_at()` and `accept_continuation_frame_at()`.

5. Verify state module compiles and existing tests pass:

       set -o pipefail
       cargo test --lib message_assembler 2>&1 | tee /tmp/wireframe-8-3-2-unit-a.log

   Expected: all existing `state_tests` and `series_tests` pass.

6. Modify `src/app/frame_handling/assembly.rs`: update
   `new_message_assembly_state` to accept `Option<MemoryBudgets>` and thread
   budget values into `MessageAssemblyState::with_budgets()`.

7. Modify `src/app/connection.rs` line 257: pass `self.memory_budgets`.

8. Verify integration compiles and existing tests pass:

       set -o pipefail
       make lint 2>&1 | tee /tmp/wireframe-8-3-2-lint-b.log
       make test 2>&1 | tee /tmp/wireframe-8-3-2-test-b.log

   Expected: no regressions.

9. Create `src/message_assembler/budget_tests.rs` with the unit tests
   described in Stage C. Register in `src/message_assembler/tests.rs`.

10. Run unit tests:

        set -o pipefail
        cargo test --lib message_assembler 2>&1 | tee /tmp/wireframe-8-3-2-unit-c.log

    Expected: all new budget tests pass alongside existing tests.

11. Create the four BDD test files described in Stage D. Register in
    `tests/fixtures/mod.rs`, `tests/steps/mod.rs`, `tests/scenarios/mod.rs`.

12. Run BDD tests:

        set -o pipefail
        cargo test --test bdd budget_enforcement 2>&1 | tee /tmp/wireframe-8-3-2-bdd.log

    Expected: all new scenarios pass.

13. Update documentation files as described in Stage E.

14. Run full quality gates:

        set -o pipefail
        make fmt 2>&1 | tee /tmp/wireframe-8-3-2-fmt.log
        make markdownlint 2>&1 | tee /tmp/wireframe-8-3-2-markdownlint.log
        make check-fmt 2>&1 | tee /tmp/wireframe-8-3-2-check-fmt.log
        make lint 2>&1 | tee /tmp/wireframe-8-3-2-lint.log
        make test 2>&1 | tee /tmp/wireframe-8-3-2-test.log
        make nixie 2>&1 | tee /tmp/wireframe-8-3-2-nixie.log

    Expected: all pass with zero warnings.

15. If any gate fails, inspect the corresponding log, fix the issue, and
    rerun that command before continuing.

## Validation and acceptance

Acceptance criteria:

- Budget enforcement: unit tests prove each dimension is enforced, partial
  buffers are freed on violation, and completed or purged assemblies reclaim
  headroom.
- Backward compatibility: all existing tests pass unchanged when
  `MemoryBudgets` is not configured.
- Behavioural coverage: `rstest-bdd` scenarios pass for enforcement behaviour.
- Documentation: design decisions recorded in ADR 0002; user guide updated;
  roadmap `8.3.2` marked done.

Quality criteria:

- Tests: all existing and new unit + behavioural tests pass via `make test`.
- Lint: `make lint` passes with no warnings.
- Formatting: `make fmt` and `make check-fmt` pass.
- Markdown: `make markdownlint` passes.
- Mermaid: `make nixie` passes.

## Idempotence and recovery

All edits are additive and safe to rerun. If an intermediate step fails:

- Keep local changes.
- Fix the specific failure.
- Rerun only the failed command, then continue the pipeline.

Avoid destructive git commands. If rollback is needed, revert only files
changed for this roadmap item.

## Artefacts and notes

Expected artefacts after completion:

- New: `src/message_assembler/budget.rs` (budget enforcement helpers).
- New: `src/message_assembler/budget_tests.rs` (budget enforcement unit
  tests).
- New: `tests/features/budget_enforcement.feature`.
- New: `tests/fixtures/budget_enforcement.rs`.
- New: `tests/steps/budget_enforcement_steps.rs`.
- New: `tests/scenarios/budget_enforcement_scenarios.rs`.
- Modified: `src/message_assembler/mod.rs`, `state.rs`, `error.rs`,
  `tests.rs`.
- Modified: `src/app/frame_handling/assembly.rs`, `src/app/connection.rs`.
- Modified: `src/app/memory_budgets.rs` (doc comment update).
- Modified: `tests/fixtures/mod.rs`, `tests/steps/mod.rs`,
  `tests/scenarios/mod.rs`.
- Modified: `docs/adr-002-streaming-requests-and-shared-message-assembly.md`,
  `docs/users-guide.md`, `docs/roadmap.md`.
- Gate logs: `/tmp/wireframe-8-3-2-*.log`.

## Interfaces and dependencies

At the end of this milestone, the following interfaces must exist.

In `src/message_assembler/error.rs`:

    pub enum MessageAssemblyError {
        // … existing variants …

        /// Accepting this frame would exceed the per-connection buffering budget.
        #[error("connection budget exceeded for key {key}: {attempted} bytes > {limit} bytes")]
        ConnectionBudgetExceeded {
            key: MessageKey,
            attempted: usize,
            limit: NonZeroUsize,
        },

        /// Accepting this frame would exceed the in-flight assembly byte budget.
        #[error("in-flight budget exceeded for key {key}: {attempted} bytes > {limit} bytes")]
        InFlightBudgetExceeded {
            key: MessageKey,
            attempted: usize,
            limit: NonZeroUsize,
        },
    }

In `src/message_assembler/state.rs`:

    impl MessageAssemblyState {
        /// Create a new assembly state manager with optional aggregate budgets.
        #[must_use]
        pub fn with_budgets(
            max_message_size: NonZeroUsize,
            timeout: Duration,
            connection_budget: Option<NonZeroUsize>,
            in_flight_budget: Option<NonZeroUsize>,
        ) -> Self { … }

        /// Total bytes buffered across all in-flight assemblies.
        #[must_use]
        pub fn total_buffered_bytes(&self) -> usize { … }
    }

In `src/app/frame_handling/assembly.rs`:

    pub(crate) fn new_message_assembly_state(
        fragmentation: Option<FragmentationConfig>,
        frame_budget: usize,
        memory_budgets: Option<MemoryBudgets>,
    ) -> MessageAssemblyState { … }

Dependencies:

- No new crates.
- Testing remains on `rstest` and `rstest-bdd` v0.5.0 already present in
  `Cargo.toml`.
