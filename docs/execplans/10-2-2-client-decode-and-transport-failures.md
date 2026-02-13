# Map client decode and transport failures to `WireframeError`

This ExecPlan is a living document. The sections `Constraints`, `Tolerances`,
`Risks`, `Progress`, `Surprises & Discoveries`, `Decision Log`, and
`Outcomes & Retrospective` must be kept up to date as work proceeds.

Status: COMPLETE

No `PLANS.md` file exists in the repository root at the time of writing.

## Purpose / big picture

Phase `10.2.2` closes a contract gap in the client request/response pipeline.
Today, client decode and transport failures are surfaced as direct
`ClientError` variants. This work makes those failures flow through
`WireframeError` variants, so client behaviour matches the server-side error
model (`transport` vs `protocol/decode`).

Success is observable when:

- Client runtime decode failures are exposed as
  `WireframeError::Protocol(...)`.
- Client runtime transport failures are exposed as `WireframeError::Io(...)`.
- Integration tests round-trip multiple message types through a sample server.
- Unit tests (`rstest`) and behavioural tests (`rstest-bdd` v0.5.0) pass.
- `docs/wireframe-client-design.md` records the error-mapping decision.
- `docs/users-guide.md` reflects public client error-handling changes.
- `docs/roadmap.md` marks `10.2.2` as done.

## Constraints

- Keep the existing client framing and serialization flow intact in
  `src/client/runtime.rs` and `src/client/messaging.rs`; only error mapping and
  tests/docs should change.
- Preserve preamble and lifecycle hook behaviour; this plan does not alter
  preamble protocol or hook ordering.
- Behavioural tests must use `rstest-bdd` version `0.5.0` and remain integrated
  with the current `tests/bdd/mod.rs` harness.
- Do not introduce new runtime dependencies beyond required test stack updates.
- Any public API shape changes in client error types must be documented in
  `docs/users-guide.md` in the same implementation.

## Tolerances (exception triggers)

- Scope: if implementation needs edits to more than 18 files, stop and
  re-evaluate before continuing.
- API: if method signatures beyond client error surfaces (`send`, `receive`,
  `call`, `send_envelope`, `receive_envelope`, `call_correlated`) must change,
  stop and record options in `Decision Log`.
- Dependencies: if work requires dependencies besides `rstest-bdd` /
  `rstest-bdd-macros` version alignment, stop and escalate.
- Iterations: if the same failing gate (`fmt`, `lint`, `test`) fails 3 times in
  a row without net progress, pause and document the blocker.
- Ambiguity: if `decode` errors cannot be cleanly modelled as protocol-level
  errors in `WireframeError`, stop and present alternatives.

## Risks

- Risk: Public error matching may break existing downstream code that expects
  `ClientError::Io` or `ClientError::Deserialize` directly. Severity: medium
  Likelihood: medium Mitigation: provide explicit migration notes in
  `docs/users-guide.md` and keep variant naming/descriptions precise.

- Risk: `rstest-bdd` `0.5.0` may require macro or runtime adjustments across
  existing behavioural tests. Severity: medium Likelihood: medium Mitigation:
  bump versions first, run behavioural suite early, then adjust fixtures/steps
  before feature-specific assertions.

- Risk: Transport failure tests can be flaky due to OS timing around socket
  closure. Severity: medium Likelihood: medium Mitigation: re-use proven
  disconnect patterns from existing client tests and prefer deterministic
  server shutdown points.

## Progress

- [x] (2026-02-12 08:59Z) Draft ExecPlan for roadmap item `10.2.2`.
- [x] (2026-02-12 09:13Z) Confirm final client error surface and mapping
      strategy.
- [x] (2026-02-12 09:15Z) Implement `WireframeError` mapping in client
      runtime/message pipeline.
- [x] (2026-02-12 09:18Z) Add/adjust unit tests (`rstest`) for decode and
      transport mappings.
- [x] (2026-02-12 09:20Z) Add integration tests for multi-message-type
      round trips.
- [x] (2026-02-12 09:23Z) Upgrade behavioural test dependencies to
      `rstest-bdd` `0.5.0` and extend scenarios.
- [x] (2026-02-12 09:24Z) Update `docs/wireframe-client-design.md` with design
      decisions.
- [x] (2026-02-12 09:25Z) Update `docs/users-guide.md` with public interface
      guidance.
- [x] (2026-02-12 09:25Z) Mark roadmap entry `10.2.2` done in
      `docs/roadmap.md`.
- [x] (2026-02-12 09:31Z) Run formatting, lint, and full test gates.

## Surprises & Discoveries

- Observation: Decode and connection-close mapping currently happens in
  `src/client/messaging.rs::receive_internal`, not in `src/client/runtime.rs`.
  Evidence: `receive()` and `receive_envelope()` both delegate to
  `receive_internal()`. Impact: Centralized mapping change can cover raw and
  envelope APIs together.

- Observation: Behavioural tests already use `rstest-bdd`, but repository pins
  `0.4.0`. Evidence: `Cargo.toml` dev-dependencies list `rstest-bdd = "0.4.0"`
  and `rstest-bdd-macros = "0.4.0"`. Impact: `10.2.2` includes a required
  dependency/version alignment task.

- Observation: `docs/behavioural-testing-in-rust-with-cucumber.md` is
  historical-only and directs readers to `docs/rstest-bdd-users-guide.md`.
  Evidence: Note at top of the Cucumber document. Impact: implementation
  guidance should follow the rstest-bdd guide.

- Observation: `clippy::expect_used` enforcement still applies to helper
  functions in integration tests even when test bodies use `expect`. Evidence:
  `make lint` failed in `tests/client_runtime.rs` until helper functions were
  refactored to return `Result`. Impact: New helper functions in integration
  tests should return `Result` and avoid `expect` internally.

## Decision Log

- Decision: Model decode and transport failures through `WireframeError`
  variants and expose that mapping from client runtime errors. Rationale:
  aligns client and server failure semantics and matches roadmap item `10.2.2`
  intent. Date/Author: 2026-02-12 / Codex

- Decision: Keep preamble/serialization/correlation mismatch failures as
  explicit `ClientError` variants, while mapping request/response pipeline
  decode and transport failures through `WireframeError`. Rationale: preserves
  targeted semantics for existing non-pipeline error paths and limits API
  churn. Date/Author: 2026-02-12 / Codex

- Decision: Record design updates in `docs/wireframe-client-design.md`.
  Rationale: this is the canonical client architecture/design document and is
  the most relevant place to capture pipeline error semantics. Date/Author:
  2026-02-12 / Codex

## Outcomes & retrospective

Implemented `10.2.2` end-to-end.

- Request/response decode and transport failures now map through
  `ClientError::Wireframe(...)` backed by `WireframeError` variants.
- Unit, integration, and behavioural coverage now assert this mapping.
- Integration tests now round-trip multiple message types through a sample
  server.
- Behavioural tests were upgraded to `rstest-bdd` `0.5.0` and extended with a
  decode-mapping scenario.
- Client design and user documentation were updated with migration guidance.
- `docs/roadmap.md` marks `10.2.2` complete.

## Context and orientation

The client runtime currently has two layers:

- `src/client/runtime.rs`: public `send`, `receive`, `call`, lifecycle
  utilities.
- `src/client/messaging.rs`: envelope-aware APIs and shared
  `receive_internal()` decode path.

Current failure modelling:

- `src/client/error.rs` maps request/response transport failures to
  `ClientError::Wireframe(WireframeError::Io(_))` and decode failures to
  `ClientError::Wireframe(WireframeError::Protocol( ClientProtocolError::Deserialize(_)))`.
- `src/response.rs` already defines generic `WireframeError<E>` with `Io`,
  `Protocol`, and `Codec` variants.

Behavioural/integration layout:

- Behavioural tests run from `tests/bdd/mod.rs`, with features in
  `tests/features/`, scenario entrypoints in `tests/scenarios/`, and fixtures
  in `tests/fixtures/`.
- Existing client runtime integration coverage is in `tests/client_runtime.rs`.
- Existing client runtime behavioural coverage is in
  `tests/features/client_runtime.feature` and
  `tests/fixtures/client_runtime.rs`.

Documentation touchpoints for this item:

- `docs/wireframe-client-design.md` for design rationale.
- `docs/users-guide.md` for public API/error-handling guidance.
- `docs/roadmap.md` to mark completion of `10.2.2`.

## Plan of work

### Stage A: Decide and scaffold error mapping (no broad refactor)

Confirm the final client error representation for pipeline failures and encode
it in `src/client/error.rs`. Keep the change focused on decode and transport
paths and avoid unrelated error redesign.

Go/no-go check: compile and run targeted client unit tests before touching
behavioural files.

### Stage B: Implement runtime mapping + unit coverage

Update `src/client/messaging.rs::receive_internal()` (and any shared send path
that surfaces transport errors) so decode and transport failures are emitted as
`WireframeError` variants. Extend `src/client/tests/error_handling.rs` (and
related client unit tests) with `rstest` cases proving variant mapping.

Go/no-go check: `cargo test` passes for client unit modules before integration
suite expansion.

### Stage C: Integration + behavioural verification

Add integration tests that spin up a sample server and round-trip multiple
message types through the client pipeline. Then extend behavioural coverage
using `rstest-bdd` `0.5.0` for user-observable failure mapping outcomes.

Go/no-go check: integration tests and behavioural tests pass locally.

### Stage D: Documentation + roadmap closure

Update design and user documentation to reflect the new error surface. Mark
`10.2.2` done in `docs/roadmap.md` only after all tests and quality gates pass.

Go/no-go check: documentation lint/format gates pass; roadmap status changed in
same change-set.

## Concrete steps

1. Update behavioural test dependencies to `0.5.0` in `Cargo.toml`:
   `rstest-bdd` and `rstest-bdd-macros`.

2. Implement pipeline error mapping in `src/client/error.rs` and
   `src/client/messaging.rs`. Keep mapping rules explicit:
   - transport I/O -> `WireframeError::Io`
   - decode/deserialization/closed-peer during receive pipeline ->
     `WireframeError::Protocol(...)`

3. Add `rstest`-driven unit coverage in `src/client/tests/error_handling.rs`
   and any adjacent client test modules.

4. Add integration tests in `tests/client_runtime.rs` or a focused new file
   such as `tests/client_decode_transport.rs`:
   - round-trip at least three distinct message types through one sample server
   - assert decode failure maps to the `WireframeError` protocol branch
   - assert transport failure maps to the `WireframeError` I/O branch

5. Extend behavioural tests (`tests/features/client_runtime.feature` and related
   step/fixture/scenario modules, or a dedicated new feature set) to assert
   observable mapping semantics using `rstest-bdd` `0.5.0`.

6. Record design decisions in `docs/wireframe-client-design.md` under client
   request/response error handling.

7. Update `docs/users-guide.md` with:
   - the revised error taxonomy for client receive/call paths
   - migration guidance for callers matching previous `ClientError` variants

8. Mark roadmap item done in `docs/roadmap.md`:
   - change `10.2.2` from `[ ]` to `[x]` after successful validation.

9. Run quality gates from repository root with logs captured:

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

10. Review log tails for each gate and ensure non-zero exits are resolved before
    considering the item complete.

## Validation and acceptance

Acceptance criteria:

- Client decode and transport errors from request/response pipeline are mapped
  through `WireframeError` variants and asserted in tests.
- Unit tests using `rstest` validate mapping branches and hook behaviour.
- Integration tests prove round-trip support for multiple message types through
  a sample server and cover failure mapping.
- Behavioural tests execute with `rstest-bdd` `0.5.0` and verify observable
  client outcomes.
- `docs/wireframe-client-design.md` and `docs/users-guide.md` reflect the new
  behaviour.
- `docs/roadmap.md` marks `10.2.2` as done.
- `make fmt`, `make markdownlint`, `make check-fmt`, `make lint`, and
  `make test` all succeed.

## Idempotence and recovery

- All edits are additive or local refactors; commands are safe to re-run.
- If a gate fails, fix the reported issue and re-run only the failed gate, then
  re-run full validation before completion.
- If behavioural tests fail after dependency bump, pin the exact break in
  `Decision Log`, patch fixtures/steps, and re-run `make test`.

## Artifacts and notes

Expected touched files (minimum set):

- `src/client/error.rs`
- `src/client/messaging.rs`
- `src/client/tests/error_handling.rs`
- `tests/client_runtime.rs` (or `tests/client_decode_transport.rs`)
- behavioural test files under `tests/features/`, `tests/fixtures/`,
  `tests/steps/`, and `tests/scenarios/` as needed
- `Cargo.toml`
- `docs/wireframe-client-design.md`
- `docs/users-guide.md`
- `docs/roadmap.md`

## Interfaces and dependencies

Targeted interface outcome:

- Client request/response pipeline failures carry `WireframeError` semantics:
  transport failures in `WireframeError::Io` and decode failures in
  `WireframeError::Protocol(...)`.
- Public client error documentation explicitly states how to match these cases.

Dependency outcome:

- `rstest-bdd = "0.5.0"`
- `rstest-bdd-macros = "0.5.0"` (with current compile-time validation feature
  policy preserved)

## Revision note

Updated from draft to complete after implementation, test suite expansion,
documentation updates, and successful validation gates.
