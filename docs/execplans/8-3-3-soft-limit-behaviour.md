# Implement soft-limit inbound read pausing for memory budgets (8.3.3)

This ExecPlan is a living document. The sections `Constraints`, `Tolerances`,
`Risks`, `Progress`, `Surprises & Discoveries`, `Decision Log`, and
`Outcomes & Retrospective` must be kept up to date as work proceeds.

Status: DRAFT

No `PLANS.md` exists in this repository as of 2026-02-23.

## Purpose / big picture

Roadmap item `8.3.3` requires soft-limit behaviour for per-connection memory
budgets: when inbound buffered bytes approach configured aggregate caps,
Wireframe should apply back-pressure by pausing socket reads rather than
immediately consuming the next frame.

After this change, operators who configure `WireframeApp::memory_budgets(...)`
will get two complementary protections on inbound assembly paths:

- soft pressure: paced reads under high buffered-byte pressure; and
- hard cap: deterministic rejection and cleanup when a limit is exceeded
  (already implemented in `8.3.2`).

Success is observable when:

- inbound reads are paused under soft pressure using the configured budgets;
- the pause path is covered by `rstest` unit tests;
- behaviour is covered by `rstest-bdd` (`0.5.0`) scenarios using a live inbound
  runtime fixture;
- design decisions are recorded in
  `docs/adr-002-streaming-requests-and-shared-message-assembly.md`;
- `docs/users-guide.md` explains the soft-limit runtime behaviour for library
  consumers; and
- `docs/roadmap.md` marks `8.3.3` as done only after all quality gates pass.

## Constraints

- Scope is strictly roadmap item `8.3.3`: implement soft-limit read pausing for
  inbound assembly pressure.
- Do not regress or re-scope hard-cap enforcement semantics from `8.3.2`.
- Preserve runtime behaviour when `memory_budgets` is not configured.
- Keep public API changes to zero unless unavoidable. If a public API change is
  required, stop and escalate.
- Do not add new external dependencies.
- Keep all modified or new source files at or below the 400-line repository
  guidance.
- Validate with `rstest` unit tests and `rstest-bdd` behavioural tests.
- Follow testing guidance from:
  `docs/rust-testing-with-rstest-fixtures.md`,
  `docs/reliable-testing-in-rust-via-dependency-injection.md`, and
  `docs/rstest-bdd-users-guide.md`.
- Record implementation decisions in the relevant design document(s), primarily
  ADR 0002.
- Update `docs/users-guide.md` for any consumer-visible behavioural change.
- Mark roadmap item `8.3.3` done only after all gates pass.

## Tolerances (exception triggers)

- Scope: if implementation requires changes to more than 14 files or more than
  700 net LOC, stop and escalate.
- Interface: if implementing `8.3.3` requires a new public builder method,
  changed public function signatures, or new public types, stop and escalate.
- Dependencies: if any new crate is required, stop and escalate.
- Semantics ambiguity: if the project requires a specific soft-limit threshold
  or pause cadence not documented in ADR/design docs, stop and present options
  with trade-offs before finalizing behaviour.
- Iterations: if the same failing gate persists after 3 focused attempts, stop
  and escalate.
- Time: if any single stage exceeds 4 hours elapsed effort, stop and escalate.

## Risks

- Risk: `src/app/inbound_handler.rs` is currently 383 lines, so adding soft
  limit logic may exceed the 400-line file cap. Severity: high Likelihood: high
  Mitigation: place threshold calculation and pause-decision logic in a new
  helper module under `src/app/frame_handling/`, keeping inbound loop changes
  minimal.

- Risk: `src/message_assembler/state.rs` is already 400 lines.
  Severity: medium Likelihood: medium Mitigation: avoid adding logic there for
  this item; consume existing `total_buffered_bytes()` query from the inbound
  runtime instead.

- Risk: full read suspension can deadlock in buffered multi-frame assembly if no
  bytes can be reclaimed without reading additional frames. Severity: high
  Likelihood: medium Mitigation: implement soft pressure as paced polling
  (bounded read pauses per loop iteration), not indefinite suspension until
  below threshold.

- Risk: timing-sensitive behaviour tests can become flaky.
  Severity: medium Likelihood: medium Mitigation: use Tokio paused time
  (`tokio::time::pause`/`advance`) in a dedicated BDD world and assert
  immediate-vs-delayed availability with deterministic checks (`try_recv`
  before time advance, eventual receive after advance).

## Progress

- [x] (2026-02-23 19:10Z) Drafted ExecPlan for roadmap item `8.3.3`.
- [ ] Stage A: add soft-limit policy helper and unit coverage.
- [ ] Stage B: integrate paced read pausing into inbound loop.
- [ ] Stage C: add behavioural coverage (`rstest-bdd` v0.5.0).
- [ ] Stage D: documentation and roadmap updates.
- [ ] Stage E: run all quality gates and finalize.

## Surprises & Discoveries

- Observation: hard-cap budget enforcement was already completed in `8.3.2`.
  Evidence: `src/message_assembler/state.rs` and
  `docs/execplans/8-3-2-budget-enforcement.md`. Impact: `8.3.3` should add
  pacing/back-pressure only, not duplicate hard-cap rejection logic.

- Observation: inbound read polling currently happens in
  `WireframeApp::process_stream` (`src/app/inbound_handler.rs`), not in
  `src/connection/`. Evidence: `src/server/connection_spawner.rs` invokes
  `app.handle_connection_result`, and the read loop is in
  `src/app/inbound_handler.rs`. Impact: soft-limit read pausing must be
  implemented in the app inbound path.

- Observation: current user guide memory-budget section documents hard-cap
  rejection semantics but not explicit soft-limit pacing details. Evidence:
  `docs/users-guide.md` section "Per-connection memory budgets". Impact: docs
  must be updated so consumers understand runtime behaviour under near-cap
  pressure.

## Decision Log

- Decision: implement soft pressure as paced read pausing before `framed.next`
  in the inbound loop, driven by current buffered bytes and configured
  aggregate budgets. Rationale: this satisfies "pause reads" semantics while
  avoiding structural changes to assembly-state ownership. Date/Author:
  2026-02-23 / Codex.

- Decision: derive soft pressure from the minimum active aggregate cap
  (`bytes_per_connection`, `bytes_in_flight`) and compare without integer
  division to satisfy strict clippy settings. Rationale: the smaller cap is the
  earliest risk boundary, and avoiding integer division keeps lint policy
  intact. Date/Author: 2026-02-23 / Codex.

- Decision: keep soft-limit policy internal (no new public API) for `8.3.3`.
  Rationale: roadmap scope calls for runtime behaviour, and existing
  `memory_budgets(...)` configuration is sufficient. Date/Author: 2026-02-23 /
  Codex.

## Outcomes & Retrospective

Not started. This section will be completed after implementation and quality
validation.

## Context and orientation

Current budget plumbing:

- `src/app/memory_budgets.rs` defines `MemoryBudgets` and `BudgetBytes`.
- `src/app/builder/config.rs` exposes `WireframeApp::memory_budgets(...)`.
- `src/app/frame_handling/assembly.rs::new_message_assembly_state` threads
  budgets into `MessageAssemblyState::with_budgets(...)`.
- `src/message_assembler/state.rs` enforces hard caps and exposes
  `total_buffered_bytes()`.

Current inbound read path:

- `WireframeApp::process_stream` in `src/app/inbound_handler.rs` drives
  `timeout(timeout_dur, framed.next()).await` in a loop.
- decoded envelopes flow through decode -> transport reassembly -> message
  assembly -> handler dispatch.

Relevant docs to keep aligned:

- `docs/roadmap.md` (`8.3.3` target row).
- `docs/adr-002-streaming-requests-and-shared-message-assembly.md`.
- `docs/generic-message-fragmentation-and-re-assembly-design.md`.
- `docs/multi-packet-and-streaming-responses-design.md`.
- `docs/the-road-to-wireframe-1-0-feature-set-philosophy-and-capability-maturity.md`.
- `docs/hardening-wireframe-a-guide-to-production-resilience.md`.
- `docs/users-guide.md`.
- `docs/rust-doctest-dry-guide.md` (for doc snippet safety expectations).

Testing topology to use:

- unit tests in crate modules using `rstest`;
- behavioural tests via `tests/features/`, `tests/fixtures/`, `tests/steps/`,
  `tests/scenarios/` with `rstest-bdd` v0.5.0;
- quality gates through Makefile targets.

## Plan of work

### Stage A: implement a soft-limit policy helper with unit tests

Create a new helper module for inbound soft-pressure decisions, keeping
`src/app/inbound_handler.rs` under the file-size limit.

Expected edits:

- Add `src/app/frame_handling/backpressure.rs` with:
  - a function that computes whether soft pressure is active using:
    `buffered_bytes` from `MessageAssemblyState` and configured aggregate caps;
  - a private soft-threshold rule based on the smallest active aggregate cap;
  - a pause-duration constant (internal) used by inbound loop pacing.
- Register helper exports in `src/app/frame_handling/mod.rs`.
- Add `src/app/frame_handling/backpressure_tests.rs` with `rstest` coverage.

Unit tests must cover at least:

- no budgets configured -> never pause;
- below threshold -> no pause;
- at/above threshold -> pause;
- dual-budget case uses the smallest cap as the governing threshold;
- helper remains pure and deterministic (no async/timing in policy unit tests).

Go/no-go: if policy requires public API changes, stop and escalate.

### Stage B: integrate paced pausing into inbound read loop

Modify `src/app/inbound_handler.rs` to consult the new soft-pressure helper
before polling `framed.next()`.

Integration behaviour:

- if soft pressure is active, pause reads for the configured duration,
  purge expired assembly/fragment state, and continue loop;
- otherwise proceed with normal `timeout(..., framed.next())` processing;
- preserve existing decode/reassemble/assemble/dispatch ordering and error
  handling.

Keep loop changes small and explicitly logged (debug-level) for observability.

Go/no-go: if this pushes `src/app/inbound_handler.rs` past 400 lines, extract
additional local logic into `src/app/frame_handling/` helpers before continuing.

### Stage C: add behavioural tests (`rstest-bdd` v0.5.0)

Add a dedicated behavioural suite for soft-limit read pausing.

Expected edits:

- `tests/features/memory_budget_backpressure.feature`
- `tests/fixtures/memory_budget_backpressure.rs`
- `tests/steps/memory_budget_backpressure_steps.rs`
- `tests/scenarios/memory_budget_backpressure_scenarios.rs`
- register modules in:
  - `tests/fixtures/mod.rs`
  - `tests/steps/mod.rs`
  - `tests/scenarios/mod.rs`

Scenario coverage target:

- under soft pressure, inbound payload completion is not immediately dispatched
  before virtual time advances;
- after advancing virtual time, the delayed read resumes and the payload is
  delivered;
- when buffered bytes are comfortably below threshold, dispatch proceeds without
  pressure-induced delay.

Go/no-go: if timing assertions are flaky under virtual time, refactor the world
fixture to use deterministic availability checks (`try_recv`) plus explicit
`advance` steps.

### Stage D: documentation and roadmap updates

Update documentation to reflect implemented behaviour and decisions.

Required updates:

- `docs/adr-002-streaming-requests-and-shared-message-assembly.md`:
  add `8.3.3` implementation decisions and soft-limit semantics.
- `docs/users-guide.md`:
  update "Per-connection memory budgets" to explain soft-limit read pausing and
  its relation to hard-cap rejection.
- `docs/generic-message-fragmentation-and-re-assembly-design.md`:
  align section 9.3 wording with concrete runtime behaviour (if needed).
- `docs/roadmap.md`:
  mark `8.3.3` as done only after all gates pass.

### Stage E: quality gates and final verification

Run all required gates and capture logs with `tee`.

No roadmap checkbox update until every gate is green.

## Concrete steps

Run from repository root (`/home/user/project`).

1. Implement Stage A helper + unit tests.

2. Run focused unit tests for helper and inbound assembly behaviour:

       set -o pipefail
       cargo test --lib frame_handling 2>&1 | tee /tmp/wireframe-8-3-3-unit-a.log
       cargo test --lib message_assembler 2>&1 | tee /tmp/wireframe-8-3-3-unit-b.log

3. Integrate Stage B inbound loop pausing and re-run focused unit tests:

       set -o pipefail
       cargo test --lib inbound_handler 2>&1 | tee /tmp/wireframe-8-3-3-unit-c.log

4. Add Stage C behavioural suite and run targeted BDD scenarios:

       set -o pipefail
       cargo test --test bdd --all-features memory_budget_backpressure 2>&1 | tee /tmp/wireframe-8-3-3-bdd.log

5. Update Stage D docs and roadmap.

6. Run full quality gates:

       set -o pipefail
       make fmt 2>&1 | tee /tmp/wireframe-8-3-3-fmt.log
       make markdownlint 2>&1 | tee /tmp/wireframe-8-3-3-markdownlint.log
       make check-fmt 2>&1 | tee /tmp/wireframe-8-3-3-check-fmt.log
       make lint 2>&1 | tee /tmp/wireframe-8-3-3-lint.log
       make test 2>&1 | tee /tmp/wireframe-8-3-3-test.log
       make nixie 2>&1 | tee /tmp/wireframe-8-3-3-nixie.log

7. If any gate fails, fix only the failing area and rerun the failing command
   until green, then rerun affected downstream gates.

## Validation and acceptance

Acceptance criteria:

- Soft-limit behaviour: inbound reads are paced under memory pressure before
  hard cap violation.
- Hard-cap behaviour from `8.3.2` remains intact (no regressions).
- Unit tests (`rstest`) validate policy and inbound integration points.
- Behavioural tests (`rstest-bdd` 0.5.0) validate delayed-vs-resumed read
  behaviour through scenario steps.
- Design documentation records final soft-limit decisions.
- User guide reflects consumer-visible runtime behaviour.
- `docs/roadmap.md` marks `8.3.3` done.

Quality criteria:

- tests: `make test` passes;
- lint: `make lint` passes with no warnings;
- formatting: `make fmt` and `make check-fmt` pass;
- markdown: `make markdownlint` passes;
- mermaid validation: `make nixie` passes.

## Idempotence and recovery

All planned edits are additive and safe to rerun.

If a step fails:

- preserve local changes;
- inspect the relevant `/tmp/wireframe-8-3-3-*.log` file;
- apply the minimal fix;
- rerun only the failed command first, then downstream gates.

Avoid destructive git commands. If rollback is required, revert only files
changed for `8.3.3`.

## Artefacts and notes

Expected artefacts after completion:

- New: `src/app/frame_handling/backpressure.rs`.
- New: `src/app/frame_handling/backpressure_tests.rs`.
- Modified: `src/app/frame_handling/mod.rs`.
- Modified: `src/app/inbound_handler.rs`.
- New: `tests/features/memory_budget_backpressure.feature`.
- New: `tests/fixtures/memory_budget_backpressure.rs`.
- New: `tests/steps/memory_budget_backpressure_steps.rs`.
- New: `tests/scenarios/memory_budget_backpressure_scenarios.rs`.
- Modified: `tests/fixtures/mod.rs`.
- Modified: `tests/steps/mod.rs`.
- Modified: `tests/scenarios/mod.rs`.
- Modified: `docs/adr-002-streaming-requests-and-shared-message-assembly.md`.
- Modified: `docs/users-guide.md`.
- Modified: `docs/generic-message-fragmentation-and-re-assembly-design.md`
  (if wording requires alignment).
- Modified: `docs/roadmap.md` (`8.3.3` checkbox).
- Gate logs: `/tmp/wireframe-8-3-3-*.log`.

## Interfaces and dependencies

No new external dependencies are required.

Internal interfaces expected at the end of this milestone:

In `src/app/frame_handling/backpressure.rs`:

    pub(crate) fn should_pause_inbound_reads(
        state: Option<&crate::message_assembler::MessageAssemblyState>,
        budgets: Option<crate::app::MemoryBudgets>,
    ) -> bool

In `src/app/inbound_handler.rs`:

- `process_stream` consults `should_pause_inbound_reads(...)` before polling
  `framed.next()` and applies an async sleep when soft pressure is active.

In behavioural tests:

- a new rstest-bdd world validates soft-limit pause/resume semantics with
  deterministic virtual-time control.
