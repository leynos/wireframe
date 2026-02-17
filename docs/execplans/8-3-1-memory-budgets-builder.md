# Add `WireframeApp::memory_budgets(...)` builder method (8.3.1)

This ExecPlan is a living document. The sections `Constraints`, `Tolerances`,
`Risks`, `Progress`, `Surprises & Discoveries`, `Decision Log`, and
`Outcomes & Retrospective` must be kept up to date as work proceeds.

Status: COMPLETE

No `PLANS.md` exists in this repository as of 2026-02-17.

## Purpose / big picture

Roadmap item `8.3.1` introduces a first-class way for application builders to
set per-connection memory budgets on `WireframeApp`.

After this change, library consumers can configure a single, explicit
`memory_budgets(...)` builder step that captures:

- bytes buffered per message;
- bytes buffered per connection; and
- bytes buffered across in-flight assemblies.

This milestone is configuration surface only. Enforcement, soft back-pressure,
hard-cap abort behaviour, and derived defaults are implemented in roadmap items
`8.3.2` through `8.3.5`.

Success is observable when:

- `WireframeApp::memory_budgets(...)` exists as a public builder method;
- unit tests (`rstest`) prove defaults and builder composition semantics;
- behavioural tests (`rstest-bdd` v0.5.0) prove the public builder API is
  usable in scenario style;
- design decisions are recorded in the relevant design document(s);
- `docs/users-guide.md` documents the new public builder surface; and
- `docs/roadmap.md` marks `8.3.1` as done after all quality gates pass.

## Constraints

- Scope is strictly roadmap item `8.3.1`: add the builder configuration
  surface, not enforcement logic.
- Preserve current runtime behaviour when budgets are not configured.
- Do not implement soft-limit back-pressure or hard-cap termination in this
  milestone.
- Do not add new external dependencies.
- Keep API additions minimal and explicit; avoid broad public-surface drift.
- Keep source modules under the 400-line project guidance.
- Validate with `rstest` unit tests and `rstest-bdd` behavioural tests.
- Record the API-shape decision in a relevant design document.
- Update `docs/users-guide.md` for the new library-consumer-facing method.
- Mark roadmap item `8.3.1` done only after validation gates pass.

## Tolerances (exception triggers)

- Scope: if this work requires touching more than 12 files or exceeds 500 net
  LOC, stop and escalate.
- Interface: if implementing `memory_budgets(...)` requires additional public
  APIs beyond the configuration value type and builder method, stop and
  escalate.
- Dependencies: if any new crate is required, stop and escalate.
- Ambiguity: if ADR/design docs do not provide enough clarity on the config
  shape, stop and present options with trade-offs.
- Iterations: if the same failing test/lint issue persists after 3 focused
  attempts, stop and escalate.
- Time: if any stage exceeds 3 hours elapsed effort, stop and escalate.

## Risks

- Risk: API shape may overfit future enforcement internals.
  Severity: medium Likelihood: medium Mitigation: keep the config type narrowly
  scoped to the three documented budget dimensions and avoid
  enforcement-coupled fields.

- Risk: behavioural tests may become superficial because enforcement is out of
  scope for `8.3.1`. Severity: medium Likelihood: medium Mitigation: make BDD
  scenarios explicitly verify public builder usability and composition with
  existing builder flows.

- Risk: introducing budgets may accidentally reset or conflict with existing
  fragmentation configuration paths. Severity: medium Likelihood: low
  Mitigation: add unit tests proving codec/builder transitions preserve budget
  configuration unless explicitly changed.

## Progress

- [x] (2026-02-17 00:49Z) Drafted ExecPlan for roadmap item `8.3.1`.
- [x] (2026-02-17 22:31Z) Added `MemoryBudgets` type and threaded optional
      `memory_budgets` state through `WireframeApp` storage/rebuild paths.
- [x] (2026-02-17 22:31Z) Added `WireframeApp::memory_budgets(...)` builder
      method in `src/app/builder/config.rs`.
- [x] (2026-02-17 22:31Z) Added `rstest` unit tests for defaults, setter
      behaviour, and composition with `with_codec`/`serializer`.
- [x] (2026-02-17 22:31Z) Added `rstest-bdd` feature, fixture, steps, and
      scenarios for memory-budget configuration behaviour.
- [x] (2026-02-17 22:31Z) Updated `docs/adr-002-...`, `docs/users-guide.md`,
      and marked `docs/roadmap.md` item `8.3.1` as done.
- [x] (2026-02-17 22:31Z) Ran full quality gates with `tee` logs.

## Surprises & Discoveries

- Observation: the repository already uses `rstest-bdd` `0.5.0` in
  `Cargo.toml`, so no dependency migration is needed for this feature.
  Evidence: `Cargo.toml` dev-dependencies. Impact: behavioural-test work can
  focus on scenario coverage only.

- Observation: no dedicated memory budget type or builder storage currently
  exists in `src/app/`. Evidence: `WireframeApp` fields in
  `src/app/builder/core.rs` and builder methods in `src/app/builder/config.rs`.
  Impact: this milestone must introduce a new configuration value and wire it
  through builder reconstruction paths.

- Observation: `docs/behavioural-testing-in-rust-with-cucumber.md` is retained
  for history; current behavioural guidance lives in
  `docs/rstest-bdd-users-guide.md`. Evidence: note at top of
  `docs/behavioural-testing-in-rust-with-cucumber.md`. Impact: behavioural test
  implementation should follow rstest-bdd patterns, while still keeping the
  historical reference document consistent.

- Observation: strict clippy configuration rejects structs whose fields share a
  common prefix or suffix. Evidence: `make lint` failures from
  `clippy::struct-field-names` while introducing `MemoryBudgets`. Impact:
  internal field names were revised to mixed names while preserving public
  getter names and API semantics.

## Decision Log

- Decision: introduce a dedicated `MemoryBudgets` value type and pass it into
  `WireframeApp::memory_budgets(...)` instead of accepting three bare `usize`
  arguments. Rationale: avoids primitive-obsession/integer-soup APIs and gives
  a stable, documented configuration unit for future enforcement stages.
  Date/Author: 2026-02-17 / Codex.

- Decision: keep `8.3.1` focused on configuration surface only; enforcement and
  derived-default runtime semantics remain in `8.3.2`-`8.3.5`. Rationale:
  aligns with roadmap sequencing and avoids mixing behavioural changes across
  milestones. Date/Author: 2026-02-17 / Codex.

- Decision: avoid lint suppressions for field-name warnings and instead rename
  internal fields to satisfy `clippy::struct-field-names`. Rationale: project
  policy requires keeping lints strict unless suppression is a last resort.
  Date/Author: 2026-02-17 / Codex.

## Outcomes & Retrospective

Completed.

- `MemoryBudgets` was added as a dedicated public app configuration value with
  non-zero byte caps and documented accessors.
- `WireframeApp` now stores optional memory budgets and exposes
  `memory_budgets(...)` as a fluent builder method.
- Type-changing builder paths preserve configured budgets.
- New `rstest` unit tests and `rstest-bdd` behavioural scenarios validate the
  configuration surface.
- Documentation was updated and roadmap item `8.3.1` was marked done.
- Full quality gates passed with captured logs.

## Context and orientation

`WireframeApp` is defined in `src/app/builder/core.rs` and currently stores
routing, middleware, codec, fragmentation, and optional message-assembler
settings. Builder configuration methods live in `src/app/builder/config.rs`,
`src/app/builder/codec.rs`, and `src/app/builder/protocol.rs`.

Inbound message assembly currently derives limits in
`src/app/frame_handling/assembly.rs` from fragmentation config or frame budget,
but there is no per-connection memory-budget configuration object yet.

Relevant requirements and rationale are captured in:

- `docs/roadmap.md` section `8.3.1`.
- `docs/adr-002-streaming-requests-and-shared-message-assembly.md` section
  "Configurable per-connection memory budgets and back-pressure".
- `docs/generic-message-fragmentation-and-re-assembly-design.md` section
  `9.3 Memory budget integration`.
- `docs/the-road-to-wireframe-1-0-feature-set-philosophy-and-capability-maturity.md`
  hardening subsection on memory caps.
- `docs/hardening-wireframe-a-guide-to-production-resilience.md` for resilience
  constraints.

Testing topology for this feature:

- Unit tests use `rstest` in crate-local modules (for example,
  `src/app/builder/core.rs`).
- Behavioural tests use `rstest-bdd` fixtures/scenarios under `tests/features/`,
  `tests/fixtures/`, `tests/steps/`, and `tests/scenarios/`.

## Plan of work

### Stage A: add configuration model and builder surface

Create a dedicated memory-budget configuration type and wire it into
`WireframeApp` storage.

Expected edits:

- Add a new module, `src/app/memory_budgets.rs`, defining a public
  `MemoryBudgets` value type with clear field names and constructor/getter
  methods.
- Export the type from `src/app/mod.rs`.
- Add an optional `memory_budgets` field to `WireframeApp` in
  `src/app/builder/core.rs` and thread it through `Default` and
  `rebuild_with_params`/`RebuildParams`.
- Add `WireframeApp::memory_budgets(...)` in `src/app/builder/config.rs`.

Go/no-go: if adding this method forces unplanned public APIs or runtime
behaviour changes, stop and update `Decision Log` before proceeding.

### Stage B: add unit coverage (`rstest`)

Add focused unit tests for configuration behaviour:

- defaults: no memory budgets configured on `WireframeApp::new()`;
- setter: `memory_budgets(...)` stores the configured value;
- composition: type-changing builder paths (`with_codec`, `serializer`) preserve
  the configured budgets.

Place tests in existing builder test modules (preferred:
`src/app/builder/core.rs` test module) to allow direct inspection of
crate-visible builder fields.

Go/no-go: if tests suggest runtime enforcement changes are needed to assert the
feature, stop and keep scope at configuration semantics only.

### Stage C: add behavioural coverage (`rstest-bdd` v0.5.0)

Add a small behaviour feature that validates public API usability in scenario
form (for example, builder creation and composition with existing builder
methods).

Expected edits (or equivalent minimal set):

- `tests/features/memory_budgets.feature`
- `tests/fixtures/memory_budgets.rs`
- `tests/steps/memory_budgets_steps.rs`
- `tests/scenarios/memory_budgets_scenarios.rs`
- registration updates in `tests/fixtures/mod.rs`, `tests/steps/mod.rs`, and
  `tests/scenarios/mod.rs`

Keep scenarios explicit that this stage validates configuration surface, not
budget enforcement behaviour.

### Stage D: documentation, roadmap, and quality gates

Update docs to reflect the new public interface and record design decisions:

- update `docs/adr-002-streaming-requests-and-shared-message-assembly.md` with
  the concrete `memory_budgets(...)` API shape adopted in code;
- update `docs/users-guide.md` with builder usage examples and scope note that
  enforcement follows in later roadmap items;
- if needed for consistency, add a brief alignment update in
  `docs/generic-message-fragmentation-and-re-assembly-design.md`.

After code and doc updates, mark roadmap entry `8.3.1` as done in
`docs/roadmap.md` and run full gates.

Go/no-go: do not mark roadmap done until all gates pass.

## Concrete steps

Run from repository root (`/home/user/project`).

1. Implement the API surface and tests.

    rg -n "struct WireframeApp|impl<S, C, E, F> WireframeApp" src/app
    rg -n "message_assembler|fragmentation" src/app/builder

2. Run focused tests first.

    set -o pipefail
    cargo test --all-features src::app::builder 2>&1 | tee /tmp/wireframe-8-3-1-unit.log

    set -o pipefail
    cargo test --test bdd --all-features memory_budgets 2>&1 | tee /tmp/wireframe-8-3-1-bdd.log

Expected transcript snippets:

- unit: `test result: ok.` with new builder-memory-budget tests listed.
- bdd: scenario names from `memory_budgets.feature` pass.

1. Run repository quality gates.

    set -o pipefail
    make fmt 2>&1 | tee /tmp/wireframe-8-3-1-fmt.log

    set -o pipefail
    make markdownlint 2>&1 | tee /tmp/wireframe-8-3-1-markdownlint.log

    set -o pipefail
    make check-fmt 2>&1 | tee /tmp/wireframe-8-3-1-check-fmt.log

    set -o pipefail
    make lint 2>&1 | tee /tmp/wireframe-8-3-1-lint.log

    set -o pipefail
    make test 2>&1 | tee /tmp/wireframe-8-3-1-test.log

    set -o pipefail
    make nixie 2>&1 | tee /tmp/wireframe-8-3-1-nixie.log

2. If any command fails, inspect the corresponding `/tmp/wireframe-8-3-1-*.log`
   file, fix the issue, and rerun that command before continuing.

## Validation and acceptance

Acceptance criteria:

- Public API: `WireframeApp::memory_budgets(...)` is available to consumers.
- Configuration semantics: unit tests prove default and setter/composition
  behaviour.
- Behavioural coverage: rstest-bdd scenarios pass for public builder usage.
- Documentation: design decision and user guide updates are present and
  accurate.
- Roadmap: `docs/roadmap.md` marks `8.3.1` done only after gates pass.

Quality criteria:

- Tests: focused unit + behavioural tests and full `make test` pass.
- Lint/type/docs: `make lint` passes with no warnings.
- Formatting: `make fmt` and `make check-fmt` pass.
- Markdown: `make markdownlint` passes.
- Mermaid validation: `make nixie` passes.

## Idempotence and recovery

All edits are additive and safe to rerun. If an intermediate step fails:

- keep local changes;
- fix the specific failure; and
- rerun only the failed command, then continue the pipeline.

Avoid destructive git commands. If rollback is needed, revert only files
changed for this roadmap item.

## Artifacts and notes

Expected artifacts after completion:

- New/updated app configuration files for `MemoryBudgets` and builder wiring.
- New `rstest` unit tests covering memory-budget builder behaviour.
- New `rstest-bdd` feature/fixture/steps/scenario for builder API behaviour.
- Updated design docs (`ADR 0002` and related alignment notes),
  `docs/users-guide.md`, and `docs/roadmap.md`.
- Gate logs:
  `/tmp/wireframe-8-3-1-impl-{fmt,markdownlint,check-fmt,lint,test,nixie}.log`.

## Interfaces and dependencies

At the end of this milestone, the following interfaces must exist.

In `src/app/memory_budgets.rs`:

    pub struct MemoryBudgets { … }

    impl MemoryBudgets {
        pub const fn new(
            bytes_per_message: std::num::NonZeroUsize,
            bytes_per_connection: std::num::NonZeroUsize,
            bytes_in_flight: std::num::NonZeroUsize,
        ) -> Self { … }

        pub const fn bytes_per_message(&self) -> std::num::NonZeroUsize { … }
        pub const fn bytes_per_connection(&self) -> std::num::NonZeroUsize { … }
        pub const fn bytes_in_flight(&self) -> std::num::NonZeroUsize { … }
    }

In `src/app/builder/config.rs`:

    impl<S, C, E, F> WireframeApp<S, C, E, F> {
        #[must_use]
        pub fn memory_budgets(self, budgets: crate::app::MemoryBudgets) -> Self { … }
    }

In `src/app/builder/core.rs`:

- `WireframeApp` stores optional memory budget configuration.
- Type-changing rebuild paths preserve this configuration.

Dependencies:

- No new crates.
- Testing remains on `rstest` and `rstest-bdd` v0.5.0 already present in
  `Cargo.toml`.

## Revision note

- Updated this ExecPlan from `DRAFT` to `COMPLETE`.
- Recorded implemented work, discovered lint constraints, and final outcomes.
- Replaced planned log paths with the actual `-impl` gate logs used for
  verification.
