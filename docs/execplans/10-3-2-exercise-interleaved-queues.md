# Exercise Interleaved Push Queue Fairness and Rate-Limit Symmetry

This ExecPlan is a living document. The sections `Constraints`, `Tolerances`,
`Risks`, `Progress`, `Surprises & Discoveries`, `Decision Log`, and
`Outcomes & Retrospective` must be kept up to date as work proceeds.

Status: DRAFT

No `PLANS.md` file exists in the repository root at the time of writing.

## Purpose / big picture

Roadmap item `10.3.2` closes the remaining parity gap in client streaming
validation. The server already contains high- and low-priority push queues,
fairness controls, and shared push rate limiting. Existing tests validate each
piece independently. This work proves those guarantees remain symmetrical when
traffic is interleaved and observed through the client streaming surface.

After this change, a maintainer can point to deterministic unit tests and
behavioural tests that exercise:

- fairness across interleaved high/low push queues; and
- shared rate limiting across both priorities under interleaved load.

Observable completion signals:

- new `rstest` unit tests pass for interleaved fairness and rate symmetry;
- new `rstest-bdd` (`0.5.0`) scenarios pass for client-observed behaviour;
- relevant design documentation records decisions taken during implementation;
- `docs/users-guide.md` reflects any consumer-visible interface or behaviour
  updates; and
- `docs/roadmap.md` marks `10.3.2` as done.

## Constraints

- Keep implementation aligned with `docs/roadmap.md` item `10.3.2`; do not
  pull unrelated roadmap work into this change set.
- Use `rstest` for unit coverage and `rstest-bdd` `0.5.0` for behavioural
  coverage.
- Prefer extending existing client streaming test infrastructure rather than
  introducing a second, parallel harness unless reuse is infeasible.
- Do not add new external dependencies.
- Keep public client method signatures stable unless a hard blocker is
  discovered and explicitly recorded in `Decision Log`.
- Preserve deterministic tests by using virtual time where timing sensitivity
  exists.
- Record design decisions in the most relevant design docs touched by this
  feature.
- Update `docs/users-guide.md` for any consumer-visible behaviour or interface
  change.
- Mark roadmap entry `10.3.2` done only after tests and gates pass.
- Satisfy project quality gates and doc tooling requirements before completion.

## Tolerances (exception triggers)

- Scope: if implementation requires changes in more than 16 files, stop and
  re-evaluate decomposition.
- API: if satisfying `10.3.2` requires adding/changing public client methods
  or error variants, stop and document options before continuing.
- Runtime semantics: if tests reveal that interleaved push traffic cannot be
  exercised without redefining the `ResponseStream` correlation contract, stop
  and escalate with alternatives.
- Determinism: if timing-sensitive tests remain flaky after two iterations with
  virtual time, stop and revise strategy.
- Dependencies: if new crates appear necessary, stop and escalate.
- Iterations: if the same gate (`check-fmt`, `lint`, `test`, `markdownlint`,
  `nixie`) fails three consecutive times without net progress, stop and record
  blocker details.

## Risks

- Risk: tests may accidentally duplicate existing server-only fairness/rate
  coverage without proving client-observed parity. Mitigation: define explicit
  parity assertions in the test matrix before coding, then map each assertion
  to a concrete scenario.

- Risk: client streaming currently rejects frames with unexpected correlation
  identifiers, which can collide with unsolicited push semantics. Mitigation:
  for parity scenarios, use explicitly correlated test frames and document this
  boundary in design notes.

- Risk: rate-limit tests can become scheduler-sensitive and flaky.
  Mitigation: use `tokio::time::pause` and `advance` in unit-level checks and
  keep behavioural assertions outcome-focused rather than wall-clock precise.

- Risk: behavioural fixture complexity may grow quickly.
  Mitigation: evolve `tests/fixtures/client_streaming.rs` incrementally and
  keep helper functions small and purpose-named.

## Progress

- [x] (2026-02-19) Gathered context from roadmap, client/runtime code, existing
      fairness/rate tests, and referenced design/testing documents.
- [x] (2026-02-19) Drafted ExecPlan for roadmap item `10.3.2`.
- [ ] Define final parity assertion matrix and map each assertion to
      unit/behavioural tests.
- [ ] Implement/extend `rstest` unit tests for interleaved fairness and
      shared-rate symmetry.
- [ ] Implement/extend `rstest-bdd` scenarios and fixtures for client-observed
      parity.
- [ ] Apply minimal runtime/test-harness adjustments required by failing
      parity tests.
- [ ] Update design docs and users guide with final decisions/behaviour.
- [ ] Mark roadmap item `10.3.2` done.
- [ ] Run all quality and documentation gates with captured logs.

## Surprises & Discoveries

- Existing coverage already validates core mechanics separately:
  `tests/connection_actor_fairness.rs` covers fairness and `tests/push.rs`
  covers shared rate limiting across priorities.
- Client streaming behavioural coverage exists but currently stops at frame
  ordering, clean termination, mismatch handling, and disconnect handling; it
  does not yet exercise high/low queue interleaving or rate-limit symmetry.
- `ResponseStream` enforces per-frame correlation checks; parity scenarios must
  account for this to avoid false failures unrelated to fairness/rate logic.
- Qdrant MCP project-memory endpoints were not available in this session (no
  discoverable MCP resources/templates), so external project notes could not be
  retrieved.

## Decision Log

- Decision: treat `10.3.2` primarily as parity validation work first
  (tests + docs), with production code changes only if those tests expose an
  actual behavioural gap. Rationale: existing server-side primitives are
  already present; roadmap text emphasizes exercising/proving symmetry.
  Date/Author: 2026-02-19 / Codex.

- Decision: extend existing client streaming BDD assets
  (`client_streaming` feature/fixture/steps/scenarios) instead of creating a
  parallel feature namespace. Rationale: keeps behavioural coverage cohesive
  and lowers maintenance cost. Date/Author: 2026-02-19 / Codex.

- Decision: use virtual time in timing-sensitive unit tests and outcome-based
  assertions in BDD scenarios. Rationale: deterministic CI behaviour without
  over-coupling BDD tests to scheduler details. Date/Author: 2026-02-19 / Codex.

## Outcomes & retrospective

Pending implementation.

Completion notes will summarize:

- what parity guarantees were proven;
- whether runtime code changed or test-only changes sufficed;
- what design decisions were recorded; and
- any residual risks or follow-up roadmap links.

## Context and orientation

Relevant implementation and test paths:

- `src/client/response_stream.rs`
- `src/client/streaming.rs`
- `src/client/tests/streaming.rs`
- `src/client/tests/streaming_infra.rs`
- `src/connection/mod.rs`
- `src/connection/drain.rs`
- `src/fairness.rs`
- `src/push/queues/mod.rs`
- `src/push/queues/handle.rs`
- `tests/connection_actor_fairness.rs`
- `tests/push.rs`
- `tests/multi_packet_streaming.rs`
- `tests/features/client_streaming.feature`
- `tests/fixtures/client_streaming.rs`
- `tests/steps/client_streaming_steps.rs`
- `tests/scenarios/client_streaming_scenarios.rs`
- `tests/scenarios/mod.rs`
- `tests/steps/mod.rs`
- `tests/fixtures/mod.rs`

Relevant documentation:

- `docs/roadmap.md`
- `docs/multi-packet-and-streaming-responses-design.md`
- `docs/wireframe-client-design.md`
- `docs/the-road-to-wireframe-1-0-feature-set-philosophy-and-capability-maturity.md`
- `docs/hardening-wireframe-a-guide-to-production-resilience.md`
- `docs/users-guide.md`
- `docs/rust-testing-with-rstest-fixtures.md`
- `docs/reliable-testing-in-rust-via-dependency-injection.md`
- `docs/rstest-bdd-users-guide.md`
- `docs/rust-doctest-dry-guide.md`

## Plan of work

### Stage A: lock the parity contract and test matrix

Define the exact behaviours that constitute "symmetry" for `10.3.2` and bind
each to test coverage:

- fairness symmetry: low-priority traffic must make forward progress during
  high-priority bursts under configured fairness thresholds; and
- rate-limit symmetry: one shared limiter budget must govern both priorities,
  so traffic on one side reduces immediate capacity on the other.

Produce a short assertion matrix in code comments or test module docs tying
assertions to specific tests.

Go/no-go checkpoint: matrix exists and each assertion has at least one planned
`rstest` case and one planned BDD scenario (where behaviour is externally
observable).

### Stage B: extend unit tests (`rstest`) for interleaved parity

Add focused unit-level client-streaming parity tests, reusing existing
streaming helpers where possible:

- interleaved high/low queue frames preserve fairness-driven progression;
- shared limiter budget affects both high and low pushes symmetrically; and
- interleaving does not break stream termination/correlation invariants.

Preferred touch points:

- `src/client/tests/streaming.rs` for new `#[rstest]` cases;
- `src/client/tests/streaming_infra.rs` for reusable server/harness helpers.

Use deterministic virtual time for rate-sensitive cases.

Go/no-go checkpoint: targeted client streaming unit tests pass before BDD
changes begin.

### Stage C: extend behavioural tests (`rstest-bdd` 0.5.0)

Expand behavioural coverage to express parity in user-observable language:

- add scenarios to `tests/features/client_streaming.feature` for interleaved
  high/low fairness outcomes;
- add scenario(s) validating shared rate-limit effects across priorities;
- extend fixture logic in `tests/fixtures/client_streaming.rs` to support these
  modes with predictable outputs;
- add corresponding steps/scenario wiring in:
  `tests/steps/client_streaming_steps.rs`,
  `tests/scenarios/client_streaming_scenarios.rs`, and module registries.

Prefer asserting externally visible outcomes (delivered ordering/progression
and eventual completion) over internals.

Go/no-go checkpoint: `make test-bdd` passes with new scenarios.

### Stage D: apply minimal implementation fixes only if required

If new parity tests expose a genuine runtime gap, implement the smallest change
that restores symmetry while preserving existing API contracts. Keep changes
localized and covered by regression tests added in Stages B/C.

If fixes imply public API or behavioural contract changes beyond tolerance,
stop and escalate via `Decision Log` before proceeding.

Go/no-go checkpoint: all new and existing parity tests pass without introducing
non-determinism.

### Stage E: update design docs, users guide, and roadmap

Record final design decisions and testing rationale in the relevant design
documents (primary targets:
`docs/multi-packet-and-streaming-responses-design.md` and/or
`docs/wireframe-client-design.md`).

Update `docs/users-guide.md` for any consumer-visible behaviour/interface
impact. If no public interface changes are needed, add a concise note
clarifying that parity validation strengthened guarantees without changing
client API calls.

After all validation passes, mark roadmap item `10.3.2` as done in
`docs/roadmap.md`.

Go/no-go checkpoint: docs accurately reflect implemented behaviour and roadmap
status change is included in the same final change set.

### Stage F: run full quality gates with captured logs

Run required gates from repository root with `tee` and `pipefail`:

    set -o pipefail
    make fmt 2>&1 | tee /tmp/wireframe-10-3-2-fmt.log

    set -o pipefail
    make check-fmt 2>&1 | tee /tmp/wireframe-10-3-2-check-fmt.log

    set -o pipefail
    make lint 2>&1 | tee /tmp/wireframe-10-3-2-lint.log

    set -o pipefail
    make test-bdd 2>&1 | tee /tmp/wireframe-10-3-2-test-bdd.log

    set -o pipefail
    make test 2>&1 | tee /tmp/wireframe-10-3-2-test.log

    set -o pipefail
    make markdownlint 2>&1 | tee /tmp/wireframe-10-3-2-markdownlint.log

    set -o pipefail
    make nixie 2>&1 | tee /tmp/wireframe-10-3-2-nixie.log

Go/no-go checkpoint: every command exits `0`.

## Concrete implementation checklist

1. Add/extend parity assertion helpers in client streaming unit test
   infrastructure.
2. Add `rstest` cases in `src/client/tests/streaming.rs` for:
   - fairness under interleaved high/low pushes;
   - shared rate-limit symmetry across priorities;
   - invariants (termination and correlation) under interleaving.
3. Extend `client_streaming` BDD feature and fixture stack to mirror these
   behaviours.
4. If needed, implement minimal runtime/test harness fixes and attach regression
   tests.
5. Update design docs with decisions, update users guide as required, and mark
   roadmap `10.3.2` done.
6. Run all quality/documentation gates and capture logs.

## Validation and acceptance

Feature acceptance criteria:

- Unit tests (`rstest`) explicitly prove interleaved high/low fairness and
  shared rate-limit symmetry.
- Behavioural tests (`rstest-bdd` `0.5.0`) demonstrate parity as externally
  observable client behaviour.
- Existing related suites (`tests/push.rs`,
  `tests/connection_actor_fairness.rs`, `tests/multi_packet_streaming.rs`)
  continue to pass.
- Design decisions are recorded in relevant docs.
- `docs/users-guide.md` reflects consumer-facing changes (or explicitly records
  that no interface change was required).
- `docs/roadmap.md` marks `10.3.2` complete.
- `make check-fmt`, `make lint`, `make test`, `make markdownlint`, and
  `make nixie` all exit `0`.

## Idempotence and recovery

The plan is additive and re-runnable:

- tests and docs edits can be re-applied safely;
- quality gates can be re-run until green; and
- no destructive data migrations or irreversible operations are required.

If a stage fails:

1. fix the immediate cause;
2. re-run that stage's checkpoint command(s); then
3. continue to the next stage only after passing results.

## Notes for implementers

- Keep step definitions and fixtures small; extract helper methods rather than
  creating deeply branching step logic.
- Avoid coupling BDD scenarios to implementation-only details (for example,
  internal counters) unless those details are the explicit behaviour under test.
- Use clear, searchable naming for new tests and scenarios so future roadmap
  audits can locate `10.3.2` evidence quickly.
