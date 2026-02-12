# Integrate MessageAssembler into the inbound connection path (8.2.5/8.2.6)

This ExecPlan is a living document. The sections `Constraints`, `Tolerances`,
`Risks`, `Progress`, `Surprises & Discoveries`, `Decision Log`, and
`Outcomes & Retrospective` must be kept up to date as work proceeds.

Status: DRAFT

No `PLANS.md` exists in this repository as of 2026-02-12.

## Purpose / big picture

Wireframe already exposes a `MessageAssembler` trait and a
`MessageAssemblyState`, but the inbound runtime path does not invoke them yet.
After this work, inbound packets will be processed in the required order:
transport decode, transport reassembly, protocol message assembly, then handler
routing.

Success is observable when a configured assembler can combine interleaved
message-key streams on a live connection, reject continuity violations
predictably, purge stale partial assemblies on timeout, and dispatch only
completed payloads to handlers. Unit tests (`rstest`) and behavioural tests
(`rstest-bdd` v0.5.0) must prove these behaviours.

## Constraints

- Preserve the composition order from ADR 0002:
  decode -> transport reassembly -> message assembly -> handler dispatch.
- Keep transport fragmentation logic in `src/app/fragmentation_state.rs`
  separate from protocol message assembly logic in `src/message_assembler/` and
  `src/app/frame_handling/`.
- Do not change `MessageAssembler` trait method signatures unless explicitly
  approved.
- Do not add new external dependencies beyond updating existing
  `rstest-bdd` crates to v0.5.0.
- Maintain current behaviour when no assembler is configured.
- Keep all new or modified source files under 400 lines.
- Use `rstest` for unit tests and `rstest-bdd` v0.5.0 for behavioural tests.
- Record design decisions in a relevant design document (`docs/adr-002-...`
  and supporting design docs where needed).
- Update `docs/users-guide.md` for any library-consumer visible behaviour
  change.
- Mark `docs/roadmap.md` items 8.2.5 and 8.2.6 as done only after all quality
  gates pass.

## Tolerances (exception triggers)

- Scope: if implementation requires more than 18 files or more than 900 net
  LOC, stop and escalate.
- Interface: if public API changes beyond removing the existing
  "not yet integrated" caveat are required, stop and escalate.
- Dependencies: if work requires adding any crate beyond the planned
  `rstest-bdd` version bump, stop and escalate.
- Behavioural ambiguity: if metadata delivery semantics to handlers cannot be
  completed without additional API changes, stop and present options.
- Iterations: if the same failing test/lint issue persists after 3 focused
  fix attempts, stop and escalate.
- Time: if any single stage takes more than 4 hours elapsed effort, stop and
  escalate with status and options.

## Risks

- Risk: The term "connection actor inbound path" is ambiguous because
  `src/connection/` is outbound-focused, while inbound routing lives in
  `src/app/connection.rs`. Severity: medium Likelihood: high Mitigation:
  Implement in `src/app/connection.rs` and record this naming clarification in
  the design docs.

- Risk: Message assembly error handling may diverge from existing inbound
  decode-failure policy. Severity: high Likelihood: medium Mitigation: Route
  parse/continuity failures through the same deserialization failure tracker
  semantics (`InvalidData` threshold handling).

- Risk: Timeout behaviour can become flaky in behavioural tests if tied to wall
  clock timing. Severity: medium Likelihood: medium Mitigation: Use
  deterministic fixture-controlled time progression where possible; keep
  network waits bounded and explicit.

- Risk: Upgrading `rstest-bdd` from 0.4.0 to 0.5.0 may require macro or step
  registration adjustments. Severity: medium Likelihood: medium Mitigation:
  perform the version bump first and fix compile-time macro breakage before
  changing runtime logic.

## Progress

- [x] (2026-02-12 00:00Z) Drafted ExecPlan for roadmap items 8.2.5 and 8.2.6.
- [ ] Confirm and document the exact inbound integration seam in
      `src/app/connection.rs` and `src/app/frame_handling/`.
- [ ] Upgrade behavioural test dependencies to `rstest-bdd` v0.5.0 and restore
      green compilation.
- [ ] Implement message assembly application after transport reassembly.
- [ ] Add `rstest` unit tests for interleaving, ordering violations, and
      timeout purging in the inbound runtime path.
- [ ] Add `rstest-bdd` v0.5.0 behavioural tests that exercise the same runtime
      behaviours through the test harness.
- [ ] Update design docs and `docs/users-guide.md` for the new integrated
      behaviour.
- [ ] Mark roadmap entries 8.2.5 and 8.2.6 as done.
- [ ] Run full quality gates (`fmt`, markdown lint, Rust format check, lint,
      tests) and capture logs with `tee`.

## Surprises & discoveries

- Observation: `WireframeApp::with_message_assembler` is currently a stored
  configuration only; inbound code in `src/app/connection.rs` does not yet call
  the hook. Evidence: no `message_assembler` usage in inbound frame handling.
  Impact: 8.2.5 requires runtime integration work, not only API exposure.

- Observation: `src/connection/` is centred on outbound frame delivery and does
  not own inbound decode/reassembly. Evidence: `src/connection/mod.rs` actor
  loop handles push/response streams, while inbound decode happens in
  `src/app/connection.rs`. Impact: roadmap wording must be interpreted as
  inbound path in `WireframeApp` connection handling.

- Observation: the workspace currently pins `rstest-bdd = "0.4.0"` in
  `Cargo.toml`. Evidence: `Cargo.toml` dev-dependencies. Impact: this feature
  must include the requested upgrade to v0.5.0.

## Decision log

- Decision: Integrate message assembly in `src/app/connection.rs` using
  `frame_handling` helpers, immediately after `reassemble_if_needed` and before
  handler lookup. Rationale: this is the existing inbound choke point and
  preserves ADR 0002 layer ordering. Date/Author: 2026-02-12 / Codex.

- Decision: Keep route selection (`Envelope.id`) and correlation inheritance
  semantics unchanged; message assembly changes only payload readiness and
  buffering behaviour for this milestone. Rationale: avoids unplanned public
  API expansion while satisfying 8.2.5/8.2.6 scope. Date/Author: 2026-02-12 /
  Codex.

- Decision: Treat message assembly parse/continuity failures as inbound
  deserialization failures under the existing threshold policy. Rationale:
  aligns with hardening guidance and keeps invalid inbound data handling
  deterministic. Date/Author: 2026-02-12 / Codex.

## Outcomes & retrospective

Not started yet.

## Context and orientation

The current inbound runtime pipeline is implemented in `src/app/connection.rs`:

- `decode_envelope` converts codec frames into `Envelope` values.
- `frame_handling::reassemble_if_needed` applies transport-level fragment
  reassembly (`src/app/frame_handling/reassembly.rs`).
- Completed envelopes are dispatched directly to handlers via
  `frame_handling::forward_response`.

`MessageAssembler` support exists but is not wired into this pipeline:

- Hook trait and header model: `src/message_assembler/mod.rs` and
  `src/message_assembler/header.rs`.
- Multiplexing and continuity state machine: `src/message_assembler/state.rs`
  and `src/message_assembler/series.rs`.
- Builder configuration surface: `WireframeApp::with_message_assembler` in
  `src/app/builder/protocol.rs`.

Design references that must stay aligned:

- `docs/adr-002-streaming-requests-and-shared-message-assembly.md`
- `docs/generic-message-fragmentation-and-re-assembly-design.md`
- `docs/multi-packet-and-streaming-responses-design.md`
- `docs/the-road-to-wireframe-1-0-feature-set-philosophy-and-capability-maturity.md`
- `docs/hardening-wireframe-a-guide-to-production-resilience.md`
- `docs/rust-testing-with-rstest-fixtures.md`
- `docs/reliable-testing-in-rust-via-dependency-injection.md`
- `docs/rstest-bdd-users-guide.md`
- `docs/rust-doctest-dry-guide.md`

Testing topology to extend:

- Unit tests under `src/app/` and `src/message_assembler/` use `rstest`.
- Behavioural tests live under `tests/features/`, `tests/fixtures/`,
  `tests/steps/`, and `tests/scenarios/` and run through the `bdd` test target.

## Plan of work

Stage A focuses on test harness readiness and interface stability. Upgrade
`rstest-bdd` and `rstest-bdd-macros` to v0.5.0, resolve any compile-time macro
or fixture wiring updates, and ensure existing behavioural tests are green
before runtime changes. Go/no-go: do not start runtime integration until the
BDD harness compiles on v0.5.0.

Stage B adds inbound message assembly plumbing. Introduce a dedicated helper in
`src/app/frame_handling/` (for example `assembly.rs`) that consumes complete
post-fragmentation envelopes, invokes the configured `MessageAssembler`, feeds
`MessageAssemblyState`, and returns either `None` (assembly still in progress)
or a completed envelope for dispatch. Keep this helper small and explicitly
route failures through existing deserialization failure accounting.

Stage C integrates the helper into connection processing. Extend connection
state in `src/app/connection.rs` to hold optional message assembly runtime
state, invoke the new helper after transport reassembly, and purge expired
assemblies on idle read timeouts alongside existing fragmentation purging.
Go/no-go: if composition order or failure policy differs from ADR 0002, stop
and update the Decision Log before proceeding.

Stage D adds tests. Unit tests (`rstest`) must cover: interleaved assembly
across keys in inbound flow, ordering-violation rejection in inbound flow, and
idle timeout purge behaviour. Behavioural tests (`rstest-bdd` v0.5.0) must
cover the same user-visible behaviours through feature scenarios and step
fixtures.

Stage E updates documentation and roadmap status. Remove outdated caveats in
`docs/users-guide.md`, add implementation decisions to ADR/design docs, and
mark roadmap entries 8.2.5 and 8.2.6 as done only after all gates pass.

## Concrete steps

1. Baseline and dependency upgrade.

   - Edit `Cargo.toml` dev-dependencies:
     - `rstest-bdd = "0.5.0"`
     - `rstest-bdd-macros = { version = "0.5.0", features =`
       `["strict-compile-time-validation"] }`
   - Refresh lockfile:

       cargo update -p rstest-bdd --precise 0.5.0
       cargo update -p rstest-bdd-macros --precise 0.5.0

2. Add inbound assembly helper module.

   - Create or extend `src/app/frame_handling/assembly.rs` with:
     - a function that accepts `Envelope`, optional assembler reference,
       mutable `MessageAssemblyState`, and failure tracker context;
     - strict payload slicing checks (`header_len`, `metadata_len`, `body_len`);
     - first-frame and continuation handling through `FirstFrameInput` and
       `accept_continuation_frame`;
     - conversion of completed assembly output into an `Envelope` payload.
   - Export helper from `src/app/frame_handling/mod.rs`.

3. Wire runtime state in connection handling.

   - Update `src/app/connection.rs` to:
     - hold optional inbound message assembly state per connection;
     - call assembly helper after `reassemble_if_needed` and before route
       lookup;
     - purge expired message assemblies during read-timeout ticks;
     - keep no-assembler path as pass-through.

4. Add or extend unit tests with `rstest`.

   - Add targeted tests in `src/app/frame_handling/tests.rs` and/or
     `src/app/connection/tests.rs`:
     - interleaved keyed assembly dispatches only completed messages;
     - out-of-order continuation is rejected and does not dispatch;
     - timeout purge evicts stale partial state and later continuation fails as
       missing-first-frame.

5. Add behavioural tests with `rstest-bdd` v0.5.0.

   - Add feature file, fixture, steps, and scenario registrations under:
     - `tests/features/`
     - `tests/fixtures/`
     - `tests/steps/`
     - `tests/scenarios/`
   - Ensure scenarios cover interleaving, ordering violations, and timeout
     behaviour through an app-level inbound flow.

6. Update docs and roadmap.

   - Update `docs/adr-002-streaming-requests-and-shared-message-assembly.md`
     with the concrete inbound integration and failure-handling decisions.
   - Update supporting design docs listed above to remove "planned" wording for
     8.2.5 and document the implemented composition.
   - Update `docs/users-guide.md` to describe active inbound integration and
     expected behaviour for configured assemblers.
   - Update `docs/roadmap.md` to mark 8.2.5 and 8.2.6 as done.

## Validation and acceptance

Acceptance conditions:

- Inbound runtime applies message assembly after transport fragmentation.
- Interleaved keyed assemblies work in inbound dispatch path.
- Ordering violations are rejected deterministically and follow existing
  invalid-data handling policy.
- Timeout purging removes stale partial assemblies.
- Unit coverage uses `rstest`; behavioural coverage uses `rstest-bdd` v0.5.0.
- Design docs, user guide, and roadmap are updated and consistent.

Run from repository root, capturing complete logs:

    set -o pipefail
    make fmt 2>&1 | tee /tmp/wireframe-fmt.log

    set -o pipefail
    make markdownlint 2>&1 | tee /tmp/wireframe-markdownlint.log

    set -o pipefail
    make check-fmt 2>&1 | tee /tmp/wireframe-check-fmt.log

    set -o pipefail
    make lint 2>&1 | tee /tmp/wireframe-lint.log

    set -o pipefail
    make test 2>&1 | tee /tmp/wireframe-test.log

Targeted checks before full suite:

    set -o pipefail
    cargo test --test bdd --all-features message_assembly 2>&1 | tee /tmp/wireframe-bdd-message-assembly.log

    set -o pipefail
    cargo test --all-features connection_fragmentation 2>&1 | tee /tmp/wireframe-connection-fragmentation.log

If Mermaid diagrams are touched, also run:

    set -o pipefail
    make nixie 2>&1 | tee /tmp/wireframe-nixie.log

## Idempotence and recovery

All plan steps are additive and can be re-run safely. If a stage fails:

- fix the specific issue;
- re-run only the affected command group first;
- then re-run full gates.

Avoid destructive Git operations. If rollback is needed, revert only the
specific files introduced by this feature branch.

## Artifacts and notes

Expected deliverables:

- inbound message assembly integration in `src/app/connection.rs` and
  `src/app/frame_handling/`;
- `rstest` unit tests for inbound interleaving, ordering violation, timeout;
- `rstest-bdd` v0.5.0 behavioural scenarios for the same behaviours;
- updated ADR/design docs and user guide;
- roadmap entries 8.2.5 and 8.2.6 checked as complete.

## Interfaces and dependencies

Public API expectations after completion:

- `WireframeApp::with_message_assembler(...)` remains the configuration entry
  point and is now operational in inbound runtime processing.
- `WireframeApp::message_assembler()` remains available for inspection.
- Existing message assembler public types remain stable:
  `MessageAssembler`, `ParsedFrameHeader`, `FrameHeader`, `FirstFrameHeader`,
  `ContinuationFrameHeader`, `MessageAssemblyState`, and related error types.

Dependency expectation:

- Behavioural test stack uses `rstest-bdd` v0.5.0 and matching
  `rstest-bdd-macros` v0.5.0.

## Revision note

Initial draft created for roadmap items 8.2.5 and 8.2.6. This version defines
scope, constraints, staged execution, validation commands, documentation
updates, and the final roadmap completion step.
