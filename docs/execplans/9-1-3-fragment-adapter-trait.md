# 9.2.1 FragmentAdapter trait and fragmentation opt-in hardening

This ExecPlan is a living document. The sections `Constraints`, `Tolerances`,
`Risks`, `Progress`, `Surprises & Discoveries`, `Decision Log`, and
`Outcomes & Retrospective` must be kept up to date as work proceeds.

Status: COMPLETE

No `PLANS.md` exists in this repository as of 2026-02-12.

## Purpose / big picture

Wireframe already ships transport fragmentation primitives, but current app
integration still has open hardening gaps called out by roadmap item 9.2.1:
fragmentation is enabled by default, duplicate versus out-of-order handling is
not explicit, purge scheduling ownership is implicit, and there is no public
fragment-adapter abstraction tying these rules together. This plan introduces a
public `FragmentAdapter` trait and aligns runtime behaviour, so fragmentation
is explicitly opt-in, purge control is public, duplicate and out-of-order
policies are deterministic, and edge cases (zero-length fragments and index
overflow) are defined and tested.

Success is observable when:

- `WireframeApp::new()` no longer fragments unless the caller explicitly opts
  in.
- a public fragment adapter API exists and exposes purge control.
- interleaved fragment streams reassemble correctly, while duplicate and
  out-of-order series follow documented policies.
- unit tests (`rstest`), integration tests, and behavioural tests
  (`rstest-bdd` v0.5.0) cover the new rules.
- design docs, user-facing docs, and roadmap status are updated consistently.

## Constraints

- Keep existing routing, middleware, serializer, and codec public APIs stable
  except for the intentional fragmentation opt-in behaviour change.
- Do not introduce `unsafe`.
- Preserve current default codec behaviour and frame-length guardrails.
- Keep module-level `//!` comments and rustdoc examples for new public items.
- Use `rstest` fixtures/parameterization for new unit and integration tests.
- Use `rstest-bdd` v0.5.0 for behavioural test coverage required by this
  feature.
- Update `docs/users-guide.md` for any public API change made by this work.
- Record decisions in the fragmentation design document
  `docs/generic-message-fragmentation-and-re-assembly-design.md` (and companion
  docs when composition guidance changes).

## Tolerances (exception triggers)

- Scope: if implementation exceeds 20 files or 1,200 net changed lines, pause
  and confirm before continuing.
- Interface: if this work requires changing unrelated public APIs (client
  runtime, push queue APIs, or serializer trait signatures), pause and confirm.
- Dependencies: only `rstest-bdd` and `rstest-bdd-macros` may be upgraded (to
  0.5.0) in this milestone; any other new dependency requires escalation.
- Iterations: if `make lint` or `make test` fails three consecutive times for
  the same root cause, stop and reassess approach.
- Time: if any single stage exceeds one focused day of work without reaching
  stage acceptance criteria, log the blocker and re-scope before proceeding.
- Ambiguity: if `FragmentAdapter` naming conflicts with existing design text
  cannot be resolved by documentation updates alone, stop and request naming
  direction.

## Risks

- Risk: behavioural change from default-enabled to opt-in fragmentation may
  break existing tests/examples that relied on implicit defaults. Severity:
  high. Likelihood: medium. Mitigation: migrate call sites to explicit builder
  configuration and add dedicated regression tests for disabled/default and
  enabled modes.

- Risk: `rstest-bdd` 0.5.0 may introduce macro/runtime changes that affect the
  current scenario harness. Severity: medium. Likelihood: medium. Mitigation:
  upgrade dependency early in the branch, run `make test-bdd`, adapt
  step/scenario signatures before feature-specific additions.

- Risk: duplicate suppression policy can mask protocol bugs if duplicates are
  not observably tracked. Severity: medium. Likelihood: medium. Mitigation:
  define explicit duplicate semantics in types/errors/docs and add tests that
  assert suppression versus rejection outcomes.

- Risk: purge scheduling ownership can remain ambiguous between adapter and
  connection loop. Severity: medium. Likelihood: high. Mitigation: codify
  ownership in the `FragmentAdapter` contract and document exactly when
  Wireframe calls purge versus when callers may drive it.

## Progress

- [x] (2026-02-12 00:00Z) Draft ExecPlan for roadmap item 9.2.1.
- [x] (2026-02-12 00:25Z) Finalize `FragmentAdapter` API contract and error
  taxonomy updates.
- [x] (2026-02-12 00:37Z) Make fragmentation opt-in on `WireframeApp` builder
  defaults.
- [x] (2026-02-12 00:42Z) Expose public purge API and wire it through adapter
  implementation.
- [x] (2026-02-12 01:03Z) Implement duplicate suppression and out-of-order
  handling policy.
- [x] (2026-02-12 01:11Z) Define zero-length fragment behaviour and
  index-overflow handling.
- [x] (2026-02-12 01:35Z) Add/upgrade unit, integration, and behavioural
  tests.
- [x] (2026-02-12 02:00Z) Update design docs, user guide, and roadmap
  checkboxes.
- [x] (2026-02-12 02:20Z) Run formatting, lint, and full test gates.

## Surprises & Discoveries

- Observation: the fragmentation design doc still states default-enabled
  behaviour, while roadmap 9.2.1 now requires explicit opt-in. Evidence:
  `docs/generic-message-fragmentation-and-re-assembly-design.md` section 3.4
  versus `docs/roadmap.md` section 9.2. Impact: this milestone must include
  design-document corrections, not only code/test edits.

- Observation: behavioural testing guidance for this repository now lives in
  `docs/rstest-bdd-users-guide.md`. Evidence: current testing policy and
  roadmap requirements for `rstest-bdd`. Impact: behavioural testing updates
  for this feature should follow that guide's conventions and version updates.

- Observation: the first version of the interleaved transport integration test
  assumed deterministic response ordering and intermittently timed out.
  Evidence: `tests/fragment_transport.rs` failures during local `make test`
  runs. Impact: the assertion strategy was changed to drain both responses and
  compare an order-independent payload set, which matches scheduler reality.

## Decision Log

- Decision: introduce a public `FragmentAdapter` trait plus one default
  implementation backed by existing `Fragmenter` and `Reassembler` primitives.
  Rationale: satisfies roadmap wording while minimizing rewrite risk by reusing
  proven internals. Date/Author: 2026-02-12 / Codex.

- Decision: change app-level fragmentation from implicit default to explicit
  opt-in at builder time. Rationale: aligns with roadmap hardening goal and
  prevents hidden performance and behavioural costs for protocols that do not
  need fragmentation. Date/Author: 2026-02-12 / Codex.

- Decision: codify duplicate suppression and out-of-order rejection as separate
  outcomes. Rationale: duplicate retransmissions can be tolerated safely, while
  true out-of-order delivery should fail deterministically and clear partial
  state. Date/Author: 2026-02-12 / Codex.

- Decision: make `with_codec(...)` clear fragmentation state and require a
  fresh explicit `enable_fragmentation()` call. Rationale: fragmentation
  settings depend on codec max-frame details; retaining old settings after
  codec replacement can silently produce mismatched thresholds. Date/Author:
  2026-02-12 / Codex.

## Outcomes & Retrospective

Implemented outcomes:

- Added a public `FragmentAdapter` trait and `DefaultFragmentAdapter`
  implementation in `src/fragment/adapter.rs`, and exported them through
  `src/fragment/mod.rs` and `src/lib.rs`.
- Shifted runtime integration to the new adapter contract while preserving app
  call-site behaviour through the `src/app/fragmentation_state.rs` alias layer.
- Made fragmentation opt-in by default in `WireframeApp` builder construction
  and introduced `enable_fragmentation()` for explicit activation.
- Added caller-driven purge access through the adapter API and documented purge
  ownership in design and user docs.
- Defined duplicate suppression as non-fatal (`Duplicate`) and preserved
  out-of-order rejection semantics with deterministic state cleanup.
- Added/updated coverage for opt-in defaults, interleaved reassembly, duplicate
  suppression, out-of-order fragments, zero-length fragments, and index
  overflow across unit, integration, and behavioural test suites.
- Updated roadmap 9.2.1 and companion design/user documents to reflect final
  behaviour and composition order.

Retrospective:

- Separating duplicate from out-of-order outcomes reduced ambiguity in both code
  and tests and made policy documentation straightforward.
- Explicit opt-in defaults prevent hidden transport costs for users that do not
  need fragmentation, but this increases migration burden for existing builder
  call sites; the new builder tests now guard this contract.

## Context and orientation

Current transport fragmentation internals live in:

- `src/fragment/fragmenter.rs` (outbound splitting).
- `src/fragment/reassembler.rs` (inbound assembly and timeout purge).
- `src/fragment/series.rs` (index ordering rules).
- `src/app/fragmentation_state.rs` (connection-scoped wrapper currently used by
  app frame handling).

Current app integration points are:

- `src/app/builder/core.rs` and `src/app/builder/codec.rs`, where
  `default_fragmentation(...)` currently auto-enables fragmentation.
- `src/app/connection.rs`, which instantiates optional fragmentation state and
  calls purge on read-timeout ticks.
- `src/app/frame_handling/response.rs` and
  `src/app/frame_handling/reassembly.rs`, which apply fragmentation and
  reassembly around handler processing.

Current tests and fixtures already cover part of the domain:

- `src/fragment/tests.rs` for primitive/unit coverage.
- `tests/fragment_transport.rs` and `tests/fragment_transport/*` for transport
  integration coverage.
- `tests/features/fragment.feature`, `tests/steps/fragment_steps.rs`, and
  `tests/scenarios/fragment_scenarios.rs` for behavioural coverage.

Documentation currently needing alignment:

- `docs/generic-message-fragmentation-and-re-assembly-design.md` (adapter
  contract, duplicate/out-of-order policy, purge ownership, opt-in semantics).
- `docs/multi-packet-and-streaming-responses-design.md` (layer composition
  order references).
- `docs/the-road-to-wireframe-1-0-feature-set-philosophy-and-capability-maturity.md`
  (hardening narrative alignment).
- `docs/hardening-wireframe-a-guide-to-production-resilience.md` (hardening
  narrative alignment).
- `docs/users-guide.md` (public configuration surface and behaviour changes).
- `docs/roadmap.md` (mark 9.2.1 sub-items done on completion).

## Plan of work

### Stage A: contract and policy definition (no behavioural changes yet)

Define the `FragmentAdapter` public contract in `src/fragment`, including purge
methods and explicit result/error shapes for duplicate suppression,
out-of-order fragments, zero-length fragments, and overflow paths. Update the
fragmentation design document first, so code follows an agreed contract.

Go/no-go:

- Go when design docs and trait signatures agree on ownership of purge
  scheduling and policy terms.
- No-go if naming or ownership remains ambiguous after doc updates.

### Stage B: wire adapter into app path and enforce opt-in

Implement the default adapter using existing `Fragmenter` + `Reassembler`
logic, then switch app frame handling to use the adapter contract. Change
builder defaults, so fragmentation is disabled unless explicitly configured.
Update builder docs/comments and any helper defaults that currently turn
fragmentation on implicitly.

Go/no-go:

- Go when `WireframeApp::new()` has no fragmentation state by default and
  explicit config paths still work.
- No-go if disabling defaults causes unbounded regressions outside
  fragmentation-related tests.

### Stage C: policy enforcement and test expansion

Implement duplicate suppression versus out-of-order rejection behaviour in the
fragment series/reassembler path. Add explicit zero-length and overflow
handling tests. Expand integration tests for interleaved reassembly and opt-in
semantics. Upgrade behavioural tests to `rstest-bdd` v0.5.0 and add scenarios
that prove the new policies.

Go/no-go:

- Go when new tests fail before implementation and pass after changes.
- No-go if policy cannot be expressed without broad unrelated API changes.

### Stage D: documentation, roadmap completion, and hardening gates

Update user-facing and design docs to match implemented behaviour and
composition order. Mark roadmap 9.2.1 checklist items as done. Run full
formatting, lint, and test gates.

Go/no-go:

- Go when docs and code behaviour match and all quality gates pass.
- No-go if any gate fails; fix root causes before finalizing.

## Concrete steps

1. Add/adjust fragmentation adapter API surface:
   `src/fragment/adapter.rs` (new), `src/fragment/mod.rs`, `src/lib.rs`.

2. Refactor connection-facing adapter implementation:
   `src/app/fragmentation_state.rs`, `src/app/frame_handling/core.rs`,
   `src/app/frame_handling/reassembly.rs`,
   `src/app/frame_handling/response.rs`, `src/app/connection.rs`.

3. Enforce opt-in builder behaviour:
   `src/app/builder_defaults.rs`, `src/app/builder/core.rs`,
   `src/app/builder/codec.rs`, `src/app/builder/config.rs`, plus affected
   doctests and call sites.

4. Implement policy-specific fragment logic and tests:
   `src/fragment/series.rs`, `src/fragment/reassembler.rs`,
   `src/fragment/error.rs`, `src/fragment/tests.rs`.

5. Expand integration and behavioural suites:
   `tests/fragment_transport.rs`, `tests/fragment_transport/rejection.rs`,
   `tests/fragment_transport/eviction.rs`, `tests/common/fragment_helpers.rs`,
   `tests/features/fragment.feature`, `tests/steps/fragment_steps.rs`,
   `tests/scenarios/fragment_scenarios.rs`, `tests/fixtures/fragment/mod.rs`,
   `tests/fixtures/fragment/reassembly.rs`.

6. Upgrade behavioural-test dependencies to v0.5.0 and adapt harness if needed:
   `Cargo.toml`, `Cargo.lock`, and any `rstest-bdd` API call sites.

7. Update documentation and roadmap:
   `docs/generic-message-fragmentation-and-re-assembly-design.md`,
   `docs/multi-packet-and-streaming-responses-design.md`,
   `docs/the-road-to-wireframe-1-0-feature-set-philosophy-and-capability-maturity.md`,
    `docs/hardening-wireframe-a-guide-to-production-resilience.md`,
   `docs/users-guide.md`, and `docs/roadmap.md`.

8. Run validation commands from repository root, capturing full logs:

       set -o pipefail
       timeout 300 make fmt 2>&1 | tee /tmp/wireframe-fmt.log

       set -o pipefail
       timeout 300 make markdownlint 2>&1 | tee /tmp/wireframe-markdownlint.log

       set -o pipefail
       timeout 300 make check-fmt 2>&1 | tee /tmp/wireframe-check-fmt.log

       set -o pipefail
       timeout 300 make lint 2>&1 | tee /tmp/wireframe-lint.log

       set -o pipefail
       timeout 300 make test-bdd 2>&1 | tee /tmp/wireframe-test-bdd.log

       set -o pipefail
       timeout 300 make test 2>&1 | tee /tmp/wireframe-test.log

   If Mermaid diagrams are edited, also run:

       set -o pipefail
       timeout 300 make nixie 2>&1 | tee /tmp/wireframe-nixie.log

## Validation and acceptance

Acceptance is complete when all statements below are true:

- Default app behaviour: creating `WireframeApp::new()` without explicit
  fragmentation configuration does not fragment traffic.
- Opt-in behaviour: explicit builder configuration enables fragmentation and
  preserves round-trip correctness for large payloads.
- Purge API: callers can invoke a documented public purge method on the
  adapter/reassembly path and observe stale state eviction.
- Duplicate/out-of-order policy: duplicate fragments are handled per documented
  suppression policy; out-of-order fragments trigger deterministic rejection
  and cleanup.
- Edge cases: zero-length fragments and fragment index overflow semantics are
  defined and covered by tests.
- Interleaving: integration tests verify interleaved fragment streams
  reassemble correctly.
- Behavioural coverage: `rstest-bdd` scenarios validate fragment-series policy
  and any new public behaviour, running under v0.5.0 dependencies.
- Documentation parity: user guide and design docs describe the same behaviour
  that tests verify.
- Roadmap update: `docs/roadmap.md` item 9.2.1 and all requested sub-items are
  checked as done.

## Idempotence and recovery

All changes are additive or local refactors and can be safely reapplied. If any
stage fails:

- revert only the files touched in that stage;
- keep prior completed stages intact;
- rerun the stage-local tests first, then full gates.

If dependency upgrade causes widespread unrelated breakage, pin the updated
version in branch commits, fix compatibility incrementally, and only then
resume feature-specific edits.

## Artifacts and notes

Validation evidence logs:

- `/tmp/wireframe-fmt.log` (`make fmt`)
- `/tmp/wireframe-markdownlint.log` (`make markdownlint`)
- `/tmp/wireframe-check-fmt.log` (`make check-fmt`)
- `/tmp/wireframe-lint.log` (`make lint`)
- `/tmp/wireframe-test-bdd.log` (`make test-bdd`)
- `/tmp/wireframe-test.log` (`make test`)

## Interfaces and dependencies

Target public interface shape (exact naming may be adjusted during Stage A):

    pub trait FragmentAdapter: Send + Sync {
        fn fragment<E: crate::fragment::Fragmentable>(
            &self,
            packet: E,
        ) -> Result<Vec<E>, crate::fragment::FragmentationError>;
        fn reassemble<E: crate::fragment::Fragmentable>(
            &mut self,
            packet: E,
        ) -> Result<Option<E>, crate::fragment::FragmentAdapterError>;
        fn purge_expired(&mut self) -> Vec<crate::fragment::MessageId>;
    }

Target behavioural dependency changes:

- `rstest-bdd = "0.5.0"`
- `rstest-bdd-macros = { version = "0.5.0", ... }`

These remain `dev-dependencies` only.

## Revision note (2026-02-12)

Initial draft created for roadmap item 9.2.1 with explicit constraints,
tolerances, staged implementation flow, and required testing/documentation
updates.

Completed update: plan status advanced to COMPLETE, required milestones were
checked off, outcomes and policy decisions were recorded, and artefacts were
captured for reproducibility.
