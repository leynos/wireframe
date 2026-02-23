# Publish client echo-login example and client API documentation

This ExecPlan is a living document. The sections `Constraints`, `Tolerances`,
`Risks`, `Progress`, `Surprises & Discoveries`, `Decision Log`, and
`Outcomes & Retrospective` must be kept up to date as work proceeds.

Status: DRAFT

This document must be maintained in accordance with `AGENTS.md` at the
repository root, including quality gates, test policy, and documentation style
requirements.

## Purpose / big picture

Roadmap item 10.4 requires two concrete outcomes:

- A runnable client example that connects to the `echo` server, sends a login
  request, and decodes the echoed acknowledgement.
- Expanded client documentation with configuration tables, lifecycle diagrams,
  and troubleshooting guidance for the client APIs.

After this work, a new user can run a documented command sequence, observe a
successful login round trip end-to-end, and use the documentation to configure
and debug client connections without reading source code.

Observable success is:

- `cargo run --example client_echo_login --features examples` successfully
  exchanges a login message with the echo server and prints a decoded
  acknowledgement.
- New `rstest` unit/integration coverage and `rstest-bdd` behavioural coverage
  validate the example contract.
- `docs/users-guide.md` and `docs/wireframe-client-design.md` contain
  configuration tables, lifecycle diagrams, troubleshooting guidance, and the
  design decision record for the example contract.
- `docs/roadmap.md` marks 10.4.1 and 10.4.2 done.

## Constraints

- Do not introduce new dependencies.
- Preserve existing public API signatures in `src/client/`.
- Keep the example under the existing examples feature gate
  (`required-features = ["examples"]`).
- Unit/integration tests must use `rstest` fixtures or parameterized cases
  where shared setup exists.
- Behavioural tests must use `rstest-bdd` v0.5.0 with the existing
  feature/fixture/steps/scenario layout in `tests/`.
- Documentation changes must follow `docs/documentation-style-guide.md`:
  sentence-case headings, wrapped prose, Mermaid with screen-reader lead-ins,
  and en-GB-oxendict spelling.
- Update `docs/users-guide.md` for any public interface guidance required by
  the feature.
- Record implementation decisions in the relevant design document
  (`docs/wireframe-client-design.md`).
- On feature completion, mark roadmap entries 10.4.1 and 10.4.2 done in
  `docs/roadmap.md`.

## Tolerances (exception triggers)

- Scope: if implementation requires more than 14 files or 500 net lines,
  pause and escalate.
- Interface: if satisfying 10.4 requires changing existing public API
  signatures in `src/client/`, pause and escalate.
- Dependencies: if a new crate is required, pause and escalate.
- Behavioural ambiguity: if "acknowledgement" cannot be represented clearly
  with the existing echo contract, pause and present options.
- Test stability: if new behavioural tests remain flaky after 3 hardening
  iterations, pause and escalate with failure evidence.
- Validation: if quality gates still fail after 5 fix attempts, pause and
  escalate.

## Risks

- Risk: ambiguous acknowledgement semantics with a pure echo server.
  Severity: medium. Likelihood: medium. Mitigation: define and document the
  contract explicitly: for this example, acknowledgement is the echoed login
  payload decoded as the expected type.

- Risk: behavioural tests can become brittle if step text overlaps existing
  client-runtime steps. Severity: low. Likelihood: medium. Mitigation: reuse
  the existing client-runtime world and add uniquely worded steps for login
  acknowledgement.

- Risk: documentation drift between `docs/users-guide.md` and
  `docs/wireframe-client-design.md`. Severity: medium. Likelihood: medium.
  Mitigation: update both documents in one stage and include mirrored tables
  and lifecycle terminology.

- Risk: Mermaid validation failures from malformed diagrams.
  Severity: low. Likelihood: low. Mitigation: keep diagrams minimal and
  validate with `make nixie` before final gating.

## Progress

- [x] (2026-02-23 00:00Z) Drafted ExecPlan for roadmap items 10.4.1 and 10.4.2.
- [ ] Stage A complete: confirm contract, target files, and test locations.
- [ ] Stage B complete: runnable example added and registered.
- [ ] Stage C complete: `rstest` unit/integration coverage added.
- [ ] Stage D complete: `rstest-bdd` behavioural coverage added.
- [ ] Stage E complete: user guide and client design docs expanded with tables,
      diagrams, troubleshooting, and decision notes.
- [ ] Stage F complete: roadmap updated and all gates pass.

## Surprises & Discoveries

- None yet. Update this section during implementation whenever behaviour differs
  from the assumptions in this draft.

## Decision Log

- Decision: implement 10.4 behavioural coverage by extending the existing
  client-runtime BDD suite instead of creating a separate BDD domain.
  Rationale: this keeps setup reuse high, avoids duplicate worlds, and keeps
  roadmap 10.x client validation centralized. Date/Author: 2026-02-23 / Codex.

- Decision: define login acknowledgement in the runnable example as the echoed
  login message decoded by the client. Rationale: this honours the explicit
  requirement to use the `echo` server while still demonstrating typed decode
  on the client side. Date/Author: 2026-02-23 / Codex.

## Outcomes & Retrospective

Pending implementation.

This section must be completed at delivery with:

- shipped files,
- validation evidence,
- roadmap status confirmation,
- and lessons learned.

## Context and orientation

The relevant repository areas are:

- `examples/echo.rs`: existing runnable echo server.
- `Cargo.toml`: explicit `[[example]]` registrations under the `examples`
  feature gate.
- `src/client/runtime.rs`, `src/client/messaging.rs`, `src/client/streaming.rs`:
  current client API surface documented for users.
- `tests/client_runtime.rs`: integration tests for client runtime behaviour.
- `tests/features/client_runtime.feature` plus
  `tests/fixtures/client_runtime.rs`, `tests/steps/client_runtime_steps.rs`,
  `tests/scenarios/client_runtime_scenarios.rs`: existing client runtime BDD
  coverage.
- `docs/users-guide.md`: public consumer guide that already documents client
  runtime and streaming APIs.
- `docs/wireframe-client-design.md`: client design source of truth and the
  correct location for decision rationale.
- `docs/roadmap.md`: roadmap checklist containing 10.4.1 and 10.4.2.

Terminology used in this plan:

- "Echo server" means the server sends back the same decoded message payload.
- "Acknowledgement" in 10.4.1 means the client receives and decodes the echoed
  login payload as a successful reply.

## Plan of work

### Stage A: contract confirmation and scaffolding map (no code changes)

Confirm the exact files to touch and the example contract wording that will be
used in code and docs. Verify that extending the existing client-runtime tests
is sufficient for both unit and behavioural validation.

Go/no-go: proceed only when contract wording is fixed and file list is bounded.

### Stage B: runnable example implementation

Add `examples/client_echo_login.rs` as a runnable client example that:

- connects to `127.0.0.1:7878`,
- sends a typed login request,
- decodes the echoed response as the login acknowledgement,
- and prints a clear success line.

Register the new example in `Cargo.toml` with
`required-features = ["examples"]`.

Keep the example focused and executable without hidden setup. It should assume
`examples/echo.rs` is running, and state that requirement in module docs and in
user-facing docs.

Go/no-go: example compiles and runs successfully against the echo server.

### Stage C: unit/integration validation with `rstest`

Extend `tests/client_runtime.rs` with a focused test for the login
request/acknowledgement round trip. Use `#[rstest]` parameterization for at
least two usernames to prove typed decode is not hard-coded.

Prefer existing helper infrastructure (`spawn_sample_echo_server`) unless a
small helper extraction meaningfully improves clarity.

Go/no-go: new unit/integration coverage fails before implementation and passes
after implementation.

### Stage D: behavioural validation with `rstest-bdd` v0.5.0

Extend the client runtime BDD suite:

- add scenario text to `tests/features/client_runtime.feature`,
- add fixture/world behaviour in `tests/fixtures/client_runtime.rs`,
- add step definitions in `tests/steps/client_runtime_steps.rs`,
- wire scenario in `tests/scenarios/client_runtime_scenarios.rs`.

Re-use existing fixture runtime patterns and avoid creating a new BDD world for
this single behaviour.

Go/no-go: new behavioural scenario fails before implementation and passes after
implementation under the existing `bdd` test target.

### Stage E: documentation and design-decision updates

Update `docs/users-guide.md` and `docs/wireframe-client-design.md` with:

- configuration tables for client builder/runtime knobs relevant to connect,
  framing, preamble, lifecycle hooks, and messaging APIs,
- lifecycle diagrams (Mermaid) showing connect, optional preamble exchange,
  setup hook, request/response, error hook, close, and teardown,
- troubleshooting guidance for common client failures (frame length mismatch,
  preamble timeout/failure, correlation mismatch, stream borrow constraints,
  transport disconnects),
- runnable example commands and expected observable output,
- and explicit design decision notes in `docs/wireframe-client-design.md`
  describing why acknowledgement is represented as an echoed decode.

Keep wording and terminology aligned between both documents.

Go/no-go: docs are internally consistent and pass Markdown and Mermaid
validation.

### Stage F: roadmap closure and full validation

Mark both roadmap checkboxes complete in `docs/roadmap.md`:

- 10.4.1 done
- 10.4.2 done

Run all required gates and capture output logs. Do not finish until every gate
passes.

## Concrete steps

All commands run from repository root (`/home/user/project`). Use
`set -o pipefail` and `tee` for every long-running gate.

1. Baseline and targeted checks during implementation:

    set -o pipefail
    cargo test --test client_runtime -- --nocapture | tee /tmp/10-4-1-client-runtime-targeted.log

    set -o pipefail
    make test-bdd | tee /tmp/10-4-1-bdd.log

2. Full Rust quality gates before completion:

    set -o pipefail
    make check-fmt | tee /tmp/10-4-1-check-fmt.log

    set -o pipefail
    make lint | tee /tmp/10-4-1-lint.log

    set -o pipefail
    make test | tee /tmp/10-4-1-test.log

    set -o pipefail
    make test-doc | tee /tmp/10-4-1-test-doc.log

    set -o pipefail
    make doctest-benchmark | tee /tmp/10-4-1-doctest-benchmark.log

3. Documentation quality gates:

    set -o pipefail
    make fmt | tee /tmp/10-4-1-fmt.log

    set -o pipefail
    make markdownlint | tee /tmp/10-4-1-markdownlint.log

    set -o pipefail
    make nixie | tee /tmp/10-4-1-nixie.log

4. Manual runtime proof for the example:

   Terminal A command:

    cargo run --example echo --features examples

   Terminal B command:

    cargo run --example client_echo_login --features examples

Expected manual proof: Terminal B prints a decoded acknowledgement message and
exits successfully.

## Validation and acceptance

Acceptance criteria:

- Example exists and is runnable via Cargo examples with the `examples`
  feature.
- Client connects to echo server, sends login request, decodes typed
  acknowledgement.
- New `rstest` integration coverage validates login round-trip behaviour.
- New `rstest-bdd` scenario validates the same behaviour at feature level.
- `docs/users-guide.md` and `docs/wireframe-client-design.md` include
  configuration tables, lifecycle diagrams, and troubleshooting guidance.
- Design decision rationale is recorded in `docs/wireframe-client-design.md`.
- `docs/roadmap.md` marks 10.4.1 and 10.4.2 as done.
- All gates in "Concrete steps" pass.

Quality method:

- Automated: `make check-fmt`, `make lint`, `make test`, `make test-doc`,
  `make doctest-benchmark`, `make fmt`, `make markdownlint`, `make nixie`.
- Manual: run `echo` and `client_echo_login` examples concurrently and confirm
  acknowledgement output.

## Idempotence and recovery

- All edits are additive and repeatable.
- Re-running tests and quality gates is safe.
- If manual example run fails due to port contention on `127.0.0.1:7878`, stop
  the existing process and retry.
- If BDD compile-time validation fails due to ambiguous step matching, rename
  new steps with a client-runtime-specific prefix and rerun.

## Artifacts and notes

Capture concise evidence in commit/PR notes:

- command log file paths under `/tmp/10-4-1-*.log`,
- terminal output line showing decoded acknowledgement,
- and final roadmap diff lines marking 10.4.1/10.4.2 complete.

## Interfaces and dependencies

Planned interface additions/changes:

- New runnable example binary registration in `Cargo.toml` and
  `examples/client_echo_login.rs`.
- No changes to existing public client API signatures.
- Test interface additions limited to test files under `tests/`.
- Documentation interface updates limited to guidance content in
  `docs/users-guide.md` and `docs/wireframe-client-design.md`.

Dependency policy:

- Reuse existing crates (`tokio`, `wireframe`, `rstest`, `rstest-bdd`).
- Do not add external dependencies.

Revision note (2026-02-23): Initial draft created for roadmap items 10.4.1 and
10.4.2. This revision defines acknowledgement semantics for the echo contract,
scopes file targets, and sets mandatory validation gates for implementation.
