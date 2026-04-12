# Expand the client troubleshooting guide and validate misconfiguration diagnostics (17.4.2)

This ExecPlan (execution plan) is a living document. The sections
`Constraints`, `Tolerances`, `Risks`, `Progress`, `Surprises & Discoveries`,
`Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work
proceeds.

Status: COMPLETED

## Purpose / big picture

Roadmap item `17.4.2` is not greenfield documentation work. The user guide
already contains a short `Client troubleshooting` section, but it is only a
bullet list and does not yet give operators enough signal to distinguish the
three misconfiguration classes named in the roadmap: codec length mismatch,
preamble errors, and TLS issues.

After this change, a library consumer reading [`docs/users-guide.md`] will be
able to match an observed symptom to the most likely misconfiguration, confirm
that diagnosis from the current error surface, and apply the correct fix
without reverse-engineering the client internals. The documentation change must
be backed by executable evidence:

1. `rstest` unit/integration coverage proving the documented failure signals.
2. `rstest-bdd` v0.5.0 behavioural coverage proving the same signals through
   user-facing scenarios.
3. Design-document updates in [`docs/wireframe-client-design.md`] recording any
   behavioural or wording decisions taken during implementation.
4. A roadmap update in [`docs/roadmap.md`] marking `17.4.2` done only after all
   quality gates pass.

Observable success means:

- the troubleshooting section in [`docs/users-guide.md`] is expanded from a
  terse bullet list into structured guidance that covers symptoms, how to
  detect them, and the likely corrective action;
- the wording matches the current reachable error variants and does not imply a
  non-existent native TLS API;
- targeted `rstest` and `rstest-bdd` scenarios fail before the documentation
  alignment work and pass after it; and
- full repository validation passes, including Markdown and doctest quality
  gates.

## Constraints

- Scope is limited to roadmap item `17.4.2`.
- Existing public client APIs must remain source-compatible. This milestone is
  about diagnostics and documentation, not a broader client redesign.
- The final documentation must update [`docs/users-guide.md`] even if no public
  API signatures change, because the observable client failure surface is part
  of the public contract.
- Record all design decisions taken during implementation in
  [`docs/wireframe-client-design.md`].
- Mark [`docs/roadmap.md`] entry `17.4.2` done only after all tests, lint, and
  documentation quality gates pass.
- Unit coverage must use `rstest`.
- Behavioural coverage must use `rstest-bdd` v0.5.0 and follow the repository's
  `feature + fixture + steps + scenarios` layout.
- Step-function parameter names in `rstest-bdd` tests must match the fixture
  function names exactly; do not use underscore-prefixed fixture names.
- Prefer extending existing client runtime and client preamble test harnesses
  rather than creating a parallel troubleshooting-only test framework.
- Documentation must use en-GB-oxendict spelling and remain consistent with the
  style guide in [`docs/documentation-style-guide.md`].
- Do not document `ClientError::PreambleWrite` as a user-observable signal
  unless implementation first makes that variant reachable from the actual
  preamble path.
- TLS remains future work in roadmap item `18.3.1`, so the troubleshooting
  section may describe TLS-related deployment mistakes and their symptoms, but
  must not imply first-party TLS transport support exists today.
- No new external dependencies should be introduced for this milestone.

## Tolerances (exception triggers)

- Scope: if implementation requires more than 16 files changed, stop and
  escalate.
- Size: if the net change exceeds roughly 1,000 lines before validation-log
  noise, stop and escalate.
- Interface: if a public API signature must change, stop and escalate.
- Dependency: if realistic TLS coverage appears to require a new TLS crate,
  certificate bundle, or test-only external service, stop and escalate. The
  default plan is to model TLS mismatch with deterministic non-Wireframe bytes
  over plain TCP.
- Semantics: if resolving the `PreambleWrite` inconsistency requires a broader
  client-error redesign, stop and escalate with options instead of widening the
  milestone implicitly.
- Iterations: if the same test or lint gate fails three times after focused
  fixes, stop and escalate.
- Time: if any single stage exceeds four hours elapsed, stop and escalate.

## Risks

- Risk: the user guide already has a troubleshooting section, so a partial edit
  can leave duplicated or contradictory guidance. Severity: medium. Likelihood:
  high. Mitigation: rewrite the existing section in one pass and verify every
  client troubleshooting mention in [`docs/users-guide.md`] and
  [`docs/wireframe-client-design.md`] stays aligned.

- Risk: `ClientError::PreambleWrite` exists in the enum but is not currently
  emitted by the preamble exchange implementation. Severity: high. Likelihood:
  high. Mitigation: make an explicit decision during Stage A: either wire
  write-side I/O failures into `PreambleWrite` and add tests, or constrain the
  documentation to the variants that are actually reachable today.

- Risk: TLS is only mentioned in the roadmap and has no first-party client API,
  so it is easy to accidentally write misleading docs. Severity: high.
  Likelihood: medium. Mitigation: frame TLS content as "wrong port / missing
  TLS terminator / protocol mismatch" troubleshooting only, and record that
  wording decision in the design document.

- Risk: existing client runtime tests already observe codec-limit mismatch as
  `ClientError::Wireframe(WireframeError::Io(_))`, which is less specific than
  downstream users may expect. Severity: medium. Likelihood: high. Mitigation:
  the troubleshooting prose should explicitly teach users to pair the transport
  error with payload size and server-side frame-limit logs rather than
  promising a dedicated client-side "frame too large" variant.

- Risk: `rstest-bdd` fixture wiring is strict and small naming mistakes can
  invalidate the whole behavioural suite. Severity: medium. Likelihood: medium.
  Mitigation: extend the existing `client_runtime` and `client_preamble`
  worlds, keep fixture names stable, and use targeted `cargo test --test bdd`
  filters before running the full suite.

## Progress

- [x] (2026-03-21 00:00Z) Drafted the initial ExecPlan for roadmap item
  `17.4.2`.
- [x] (2026-03-21 00:45Z) Stage A: confirmed that
  [`src/client/preamble_exchange.rs`] still reports write-side preamble
  failures through `ClientError::PreambleEncode`, not `PreambleWrite`, and
  aligned the docs to the reachable error surface instead of widening client
  behaviour.
- [x] (2026-03-21 00:55Z) Stage B: added `rstest`-style integration coverage
  for TLS-like wrong-protocol bytes in [`tests/client_runtime.rs`] and invalid
  preamble acknowledgement bytes in [`tests/client_preamble.rs`].
- [x] (2026-03-21 01:05Z) Stage C: extended the existing BDD suites in
  [`tests/features/client_runtime.feature`] and
  [`tests/features/client_preamble.feature`], plus their fixture, step, and
  scenario wiring, to cover the same diagnostics behaviourally.
- [x] (2026-03-21 01:15Z) Stage D: rewrote the troubleshooting guidance in
  [`docs/users-guide.md`], aligned [`docs/wireframe-client-design.md`], and
  added an explicit `17.4.2` decision record.
- [x] (2026-03-21 01:35Z) Stage E: ran the Rust and doc quality gates, updated
  [`docs/roadmap.md`], and recorded the final validation outcome below. Full
  `make markdownlint` still reports unrelated legacy MD029 failures in older
  execplans; targeted lint for the touched docs passes.

## Surprises & Discoveries

- The repository already contains a short troubleshooting section in
  [`docs/users-guide.md`] around the existing client documentation. This
  milestone therefore expands and validates existing guidance rather than
  inventing it from scratch.

- The existing runtime tests already cover the codec-length-mismatch symptom.
  [`tests/client_runtime.rs`] includes
  `client_surfaces_oversized_frame_failures_as_wireframe_io`, and the matching
  behavioural scenario already exists in
  [`tests/features/client_runtime.feature`].

- The existing preamble tests already cover timeout behaviour.
  [`tests/client_preamble.rs`] includes
  `client_preamble_timeout_triggers_failure`, and the matching behavioural
  scenario already exists in [`tests/features/client_preamble.feature`].

- [`src/client/error.rs`] declares `ClientError::PreambleWrite`, but
  [`src/client/preamble_exchange.rs`] currently maps `write_preamble(...)`
  failures to `ClientError::PreambleEncode` because `write_preamble` returns
  `EncodeError`, including `EncodeError::Io`. This mismatch must be resolved
  before the troubleshooting docs are expanded.

- TLS support is not yet a built-in client feature. [`docs/roadmap.md`] still
  lists `18.3.1` ("Provide built-in middleware or guides for implementing TLS")
  as future work, so `17.4.2` must stay within documentation and diagnostics
  for deployment mismatches.

- The initial full-suite `make test` run failed once in the existing
  `client::tests::send_streaming::invokes_error_hook_on_transport_failure` test
  with `expected transport error, got Ok`. An isolated rerun of that test
  passed, and a subsequent full `make test` rerun passed cleanly, so the
  failure behaved like a transient baseline flake rather than a regression from
  this work.

## Decision Log

- Decision: keep this milestone focused on documentation accuracy plus testable
  diagnostics, not on introducing a new troubleshooting helper API. Rationale:
  the roadmap item lives under docs/adoption, and the existing test harnesses
  already expose the needed observable behaviour. Date/Author: 2026-03-21 /
  planning phase.

- Decision: extend existing client runtime and client preamble tests instead of
  creating a new standalone troubleshooting suite. Rationale: the current
  harnesses already model codec mismatch and preamble failures, which keeps the
  new work small and aligned with repository patterns. Date/Author: 2026-03-21
  / planning phase.

- Decision: treat TLS troubleshooting as wrong-protocol detection on plain TCP
  unless implementation discovers an already-supported transport adaptor.
  Rationale: the roadmap explicitly says built-in TLS is future work, so the
  docs must teach users how to recognize a TLS mismatch without pretending that
  `WireframeClient` has native TLS configuration today. Date/Author: 2026-03-21
  / planning phase.

- Decision: keep `ClientError::PreambleWrite` out of the troubleshooting prose
  and instead document the currently emitted variants (`PreambleTimeout`,
  `PreambleRead`, and `PreambleEncode`). Rationale: `write_preamble(...)`
  returns `bincode::EncodeError`, including write-side I/O, so the current
  client path does not emit `ClientError::PreambleWrite`. Date/Author:
  2026-03-21 / implementation.

## Outcomes & Retrospective

Shipped outcomes:

- [`docs/users-guide.md`] now expands `Client troubleshooting` into structured
  guidance for codec length mismatches, preamble timeout/read/encode failures,
  TLS or wrong-protocol port mismatches, correlation mismatches, streaming
  contention, and transport disconnects.
- [`docs/wireframe-client-design.md`] now mirrors that wording, records that
  `PreambleWrite` is not currently user-observable, and adds a dedicated
  `17.4.2` decision record.
- [`docs/roadmap.md`] marks `17.4.2` complete.
- New executable evidence now covers the roadmap's missing cases:
  - [`tests/client_runtime.rs`] adds
    `client_surfaces_tls_protocol_mismatch_as_wireframe_io`;
  - [`tests/client_preamble.rs`] adds
    `client_invalid_preamble_response_surfaces_preamble_read`;
  - [`tests/features/client_runtime.feature`] and
    [`tests/features/client_preamble.feature`] add matching BDD scenarios,
    with supporting fixture, step, and scenario updates.

Validation summary:

- Passed: `make fmt`
- Passed: `make nixie`
- Passed: `make check-fmt`
- Passed: `make lint`
- Passed: `make test` (after one transient rerun)
- Passed: `make test-doc`
- Passed: `make doctest-benchmark`
- Passed: targeted `markdownlint-cli2` on
  [`docs/users-guide.md`], [`docs/wireframe-client-design.md`],
  [`docs/roadmap.md`], and this ExecPlan
- Known unrelated baseline: full `make markdownlint` still fails on legacy
  MD029 ordered-list numbering in older execplans outside the scope of `17.4.2`

## Context and orientation

The relevant repository areas are already in place:

- [`docs/users-guide.md`] contains the user-facing client guide and the
  existing `Client troubleshooting` subsection.
- [`docs/wireframe-client-design.md`] contains the client design record,
  including an existing troubleshooting bullet list that must stay aligned with
  the user guide.
- [`docs/roadmap.md`] tracks roadmap item `17.4.2`, which should remain
  unchecked until the end of implementation.
- [`src/client/error.rs`] defines the public client error variants that the
  docs must describe accurately.
- [`src/client/preamble_exchange.rs`] defines which preamble variants are
  currently emitted.
- [`tests/client_runtime.rs`] and [`tests/client_preamble.rs`] are the existing
  `rstest`-style integration tests for runtime and preamble behaviour.
- [`tests/features/client_runtime.feature`] and
  [`tests/features/client_preamble.feature`] are the existing Gherkin features.
- [`tests/fixtures/client_runtime.rs`],
  [`tests/steps/client_runtime_steps.rs`],
  [`tests/scenarios/client_runtime_scenarios.rs`],
  [`tests/fixtures/client_preamble.rs`],
  [`tests/steps/client_preamble_steps.rs`], and
  [`tests/scenarios/client_preamble_scenarios.rs`] provide the current
  `rstest-bdd` wiring that should be extended.

The current documentation and implementation already suggest the three
misconfiguration classes this milestone must cover:

1. Codec length mismatch:
   client `ClientCodecConfig::max_frame_length` is larger than the server's
   `buffer_capacity`, so larger requests appear to fail only once payload size
   crosses the server limit.
2. Preamble errors:
   negotiation can fail before framing starts because the server does not
   reply, replies with invalid bytes, or the callback mishandles the exchange.
3. TLS issues:
   a user points a plain `WireframeClient` at a port that expects TLS or sits
   behind a TLS terminator misconfiguration, so the first bytes exchanged do
   not match Wireframe expectations.

## Implementation stages

## Stage A: audit and lock the observable error surface

Start by reconciling the public docs with the emitted client errors.

1. Re-read [`src/client/error.rs`] and [`src/client/preamble_exchange.rs`] and
   confirm which variants are truly observable from the current preamble flow.
2. Decide whether to:
   - keep `PreambleWrite` in the public error enum and make it reachable; or
   - leave behaviour unchanged and remove `PreambleWrite` from troubleshooting
     guidance.
3. Record the chosen direction in [`docs/wireframe-client-design.md`] before
   rewriting the user guide so the design record and user-facing docs move in
   lockstep.

Expected outcome: implementation has a single, explicit truth for the
troubleshooting copy and the tests do not need to guess at unreachable states.

## Stage B: add failing `rstest` coverage first

Extend existing integration tests instead of creating new harnesses.

1. In [`tests/client_runtime.rs`], convert the current oversized-frame test into
   a small `#[rstest]`-driven diagnostic contract if doing so improves reuse,
   or add adjacent parameterized cases that pin the documented codec-mismatch
   symptom: `ClientError::Wireframe(WireframeError::Io(_))` on oversize
   requests once the server frame cap is exceeded.
2. In [`tests/client_preamble.rs`], keep the existing timeout case and add a
   second preamble-read failure case where the server sends deterministic
   non-Wireframe bytes (preferably TLS-record-like bytes) that cause the
   success callback's `read_preamble` path to fail with
   `ClientError::PreambleRead`.
3. Only if Stage A chooses to make `PreambleWrite` reachable, add a write-side
   regression case proving the mapped variant.

Red/green expectation:

```plaintext
Before implementation:
- at least one new preamble/TLS-oriented test fails because the current
  behaviour or docs are incomplete/misaligned.

After implementation:
- all new and existing client runtime/preamble tests pass.
```

Targeted commands during this stage:

```sh
set -o pipefail
cargo test --test client_runtime 2>&1 | tee /tmp/17-4-2-client-runtime.log
```

```sh
set -o pipefail
cargo test --test client_preamble 2>&1 | tee /tmp/17-4-2-client-preamble.log
```

## Stage C: add behavioural coverage with `rstest-bdd` v0.5.0

Mirror the same diagnostics at the scenario level using the existing worlds.

1. Extend [`tests/features/client_runtime.feature`] with a scenario that states
   the codec mismatch symptom in user language, for example that a client with
   a larger frame cap than the server sees a transport error on oversized
   payloads.
2. Extend [`tests/features/client_preamble.feature`] with a scenario for an
   invalid preamble reply or TLS-like endpoint so the user-visible behaviour is
   expressed as a troubleshooting example.
3. Update the matching fixture, step, and scenario files without changing the
   fixture names. Reuse the existing world objects and add the minimum state
   necessary to assert the newly documented error.

Expected outcome: the behavioural suite reads like the final troubleshooting
guide and proves the documented misconfiguration story from the public API.

Targeted commands during this stage:

```sh
set -o pipefail
cargo test --test bdd --all-features -- client_runtime 2>&1 | tee /tmp/17-4-2-bdd-runtime.log
```

```sh
set -o pipefail
cargo test --test bdd --all-features -- client_preamble 2>&1 | tee /tmp/17-4-2-bdd-preamble.log
```

## Stage D: rewrite the documentation and record the decisions

Once the tests describe the intended behaviour, rewrite the client
troubleshooting content.

1. Replace the short bullet list in [`docs/users-guide.md`] with structured
   subsections for:
   - codec length mismatch;
   - preamble errors; and
   - TLS / wrong-protocol endpoint issues.
2. For each subsection, include:
   - the observable symptom;
   - the error variant or runtime behaviour to look for;
   - the confirmation step (for example, comparing
     `ClientCodecConfig::max_frame_length` with server `buffer_capacity`);
   - the likely fix.
3. Update the troubleshooting bullets in [`docs/wireframe-client-design.md`] so
   they match the user guide exactly enough to avoid drift, while also
   recording any Stage A decision about `PreambleWrite` or TLS wording.
4. If implementation changes any observable client behaviour while resolving
   the docs mismatch, make sure [`docs/users-guide.md`] explains that behaviour
   to library consumers.

The documentation must be explicit that TLS guidance is about diagnosing a
misconfigured transport boundary today, not about enabling built-in TLS on the
client.

## Stage E: run the full quality gates and close the roadmap item

After code and docs are complete, run the full validation suite from the
repository root. Use `tee` and `set -o pipefail` so failures are not hidden.

```sh
set -o pipefail
make fmt 2>&1 | tee /tmp/17-4-2-fmt.log
```

```sh
set -o pipefail
make markdownlint MDLINT=/root/.bun/bin/markdownlint-cli2 2>&1 | tee /tmp/17-4-2-markdownlint.log
```

```sh
set -o pipefail
make check-fmt 2>&1 | tee /tmp/17-4-2-check-fmt.log
```

```sh
set -o pipefail
make lint 2>&1 | tee /tmp/17-4-2-lint.log
```

```sh
set -o pipefail
make test 2>&1 | tee /tmp/17-4-2-test.log
```

```sh
set -o pipefail
make test-doc 2>&1 | tee /tmp/17-4-2-test-doc.log
```

```sh
set -o pipefail
make doctest-benchmark 2>&1 | tee /tmp/17-4-2-doctest-benchmark.log
```

```sh
set -o pipefail
make nixie 2>&1 | tee /tmp/17-4-2-nixie.log
```

Only after these pass should implementation:

1. mark `17.4.2` as done in [`docs/roadmap.md`];
2. update the `Progress` and `Outcomes & Retrospective` sections in this
   ExecPlan; and
3. summarize the final evidence in the commit message and handoff note.

## Acceptance checklist

Implementation is complete when all of the following are true:

1. [`docs/users-guide.md`] contains a substantive client troubleshooting
   section covering codec length mismatch, preamble errors, and TLS-related
   deployment mistakes.
2. The troubleshooting prose matches the actual emitted client behaviour and
   does not mention unreachable variants without first fixing them.
3. [`docs/wireframe-client-design.md`] records the wording and behavioural
   decisions taken for the troubleshooting guidance.
4. `rstest` coverage exists for the documented failure classes.
5. `rstest-bdd` coverage exists for the same failure classes.
6. [`docs/roadmap.md`] marks `17.4.2` done.
7. All validation commands in Stage E pass.
