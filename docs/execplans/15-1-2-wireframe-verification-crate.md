# Add the `wireframe-verification` Stateright crate

This ExecPlan is a living document. The sections `Constraints`, `Tolerances`,
`Risks`, `Progress`, `Surprises & Discoveries`, `Decision Log`, and
`Outcomes & Retrospective` must be kept up to date as work proceeds.

Status: DRAFT

This document must be maintained in accordance with `AGENTS.md` at the
repository root, including quality gates, test policy, approval gating,
documentation style requirements, and the requirement to update roadmap and
design documentation when decisions change.

Implementation must not begin until this plan is explicitly approved by the
user.

## Purpose / big picture

Roadmap item 15.1.2 adds the repository’s first dedicated formal-verification
crate: `crates/wireframe-verification`. That crate is the home for Stateright
models and their shared test harness, without widening the published
`wireframe` API or mixing Stateright, Kani, and Verus into one package.

After this change:

- the root workspace explicitly includes `crates/wireframe-verification`,
- ordinary root-level `cargo build` and `cargo test` ergonomics remain focused
  on the main `wireframe` package via `default-members = ["."]`,
- `cargo test -p wireframe-verification` and `cargo test --workspace` both
  exercise a placeholder Stateright model through ordinary Rust tests,
- `rstest` unit and integration coverage protects the new crate and its shared
  harness,
- `rstest-bdd` behavioural coverage protects the updated workspace contract,
- `docs/formal-verification-methods-in-wireframe.md`,
  `docs/developers-guide.md`, and `docs/roadmap.md` all reflect the new state,
  while `docs/users-guide.md` is updated only if the work changes something a
  library consumer needs to know.

Observable success is:

- `cargo metadata --no-deps --format-version 1` reports the root package and
  `crates/wireframe-verification` as workspace members while keeping the root
  package as the only default member.
- `cargo test -p wireframe-verification` passes.
- `cargo test --workspace` passes.
- the placeholder model proves that the crate structure, Stateright
  dependency, and shared checker harness are all wired correctly, without
  pre-implementing roadmap items 15.1.3 through 15.3.x.

## Constraints

- Treat this item as infrastructure-only. Do not add Kani tooling, Verus
  tooling, Makefile formal targets, or continuous integration (CI) jobs; those
  belong to roadmap items 15.1.3 through 15.1.5.
- Keep the root `wireframe` package as the sole default workspace member in
  `Cargo.toml`.
- Add `crates/wireframe-verification` as a real workspace member rather than a
  path-only helper or a nested local workspace.
- Keep the new crate internal: `publish = false`, no new public API surface in
  the main `wireframe` package, and no user-facing re-exports.
- Follow the Stateright guidance in
  `docs/formal-verification-methods-in-wireframe.md`, especially:
  `### Why Stateright belongs in a separate verification crate`,
  `### Shared checker harness`, and `### Suggested Stateright file layout`.
- Keep the placeholder model semantic and intentionally small. It should model
  policy shape and harness mechanics, not re-implement Tokio or the production
  `ConnectionActor`.
- Every new Rust module must begin with a module-level `//!` comment, and all
  public items in the new crate must have Rustdoc comments.
- Unit and integration validation must use `rstest`, with shared fixtures
  where that improves clarity.
- Behavioural validation must use `rstest-bdd` in the existing root-level
  `tests/features/`, `tests/fixtures/`, `tests/steps/`, and `tests/scenarios/`
  layout. Avoid recursive `cargo test` subprocesses inside tests.
- Record any crate-layout or harness decisions in
  `docs/formal-verification-methods-in-wireframe.md`.
- Update `docs/developers-guide.md`, especially `## Cargo workspace semantics`
  and `### Workspace manifest test support`, with the new contributor workflow.
- Inspect `docs/users-guide.md` and update it only if the work changes
  something a library consumer should know. A no-op conclusion must still be
  recorded in the implementation notes.
- On completion of the feature, mark roadmap item 15.1.2 done in
  `docs/roadmap.md`.

## Tolerances (exception triggers)

- Scope: if the placeholder Stateright model starts to pull in real
  `ConnectionActor` logic, asynchronous runtime setup, or non-trivial protocol
  semantics that properly belong to 15.2.x or 15.3.x, stop and split the work
  back down.
- Workspace shape: if adding the new crate forces `default-members` to widen
  beyond `["."]`, stop and confirm whether that ergonomics regression is
  acceptable.
- Dependency shape: if the model appears to require helper dependencies beyond
  `stateright`, `wireframe`, and test-only support such as `rstest`, record the
  reason and confirm that the added surface is justified.
- Test design: if the behavioural coverage cannot be expressed cleanly by
  extending the existing workspace-manifest BDD slice after three hardening
  attempts, stop and agree a narrower behavioural contract instead of
  improvising a second ad hoc harness.
- Validation: if `cargo test --workspace`, `make lint`, or `make test`
  surface unrelated pre-existing failures, record them separately and proceed
  only once it is clear whether 15.1.2 caused the regression.
- Documentation drift: if the formal-verification guide still contains old
  phase numbering or staging language that conflicts with the implemented
  state, update that document as part of this item rather than leaving the
  mismatch behind.

## Risks

- Risk: the formal-verification guide still contains older roadmap numbering in
  some sections, even though the current roadmap tracks this work under 15.1.x.
  Severity: high. Likelihood: high. Mitigation: update the guide during this
  item so the design document and roadmap describe the same rollout.

- Risk: `cargo metadata` already reports the in-tree helper crate
  `wireframe_testing` in `workspace_members`, so the 15.1.2 assertions must
  distinguish between declared members, actual metadata, and default members.
  Severity: high. Likelihood: high. Mitigation: extend the existing
  workspace-manifest helpers rather than inventing a second metadata parser.

- Risk: Stateright is new to the repository, so the first harness may become
  over-abstract or over-concrete. Severity: medium. Likelihood: medium.
  Mitigation: keep the placeholder model deliberately tiny and verify only the
  minimum safety and reachability properties needed to prove the crate wiring.

- Risk: the new crate could accidentally become user-visible through root-crate
  documentation or re-exports. Severity: medium. Likelihood: low. Mitigation:
  keep the crate internal, inspect `docs/users-guide.md` deliberately, and do
  not add root-crate re-exports.

- Risk: root-level `cargo test` ergonomics could become confusing once the
  workspace gains a second real member. Severity: medium. Likelihood: medium.
  Mitigation: update `docs/developers-guide.md` with explicit command guidance
  and keep BDD coverage focused on the root-only default-member contract.

## Progress

- [x] (2026-04-21) Reviewed roadmap item 15.1.2, the current root
      `Cargo.toml`, the workspace-manifest test support, the historical
      10.1.1 ExecPlan, and the formal-verification design guidance.
- [x] (2026-04-21) Drafted this approval-gated ExecPlan and wrote it to
      `docs/execplans/15-1-2-wireframe-verification-crate.md`.
- [ ] Approval gate: await explicit user approval before implementing any code
      or non-plan documentation changes.
- [ ] Stage A: confirm the precise workspace delta from 15.1.1 to 15.1.2 and
      lock the new crate’s minimal manifest and file layout.
- [ ] Stage B: scaffold `crates/wireframe-verification` with a placeholder
      Stateright model and shared verification harness.
- [ ] Stage C: add `rstest` coverage in the new crate and update the
      root-level `rstest-bdd` workspace-manifest slice for the new contract.
- [ ] Stage D: update the design and contributor documentation, inspect the
      user guide, and mark roadmap item 15.1.2 done.
- [ ] Stage E: run the full validation gates and record any environment-limited
      results.

## Surprises & Discoveries

- Discovery: the current repository does not contain a renumbered
  `docs/execplans/15-1-1-convert-root-manifest-into-hybrid-workspace.md` file.
  The concrete prerequisite record that exists in-tree is
  `docs/execplans/10-1-1-convert-root-manifest-into-hybrid-workspace.md`.

- Discovery: `Cargo.toml` is currently in the staged 15.1.1 shape:
  `[workspace]`, `members = ["."]`, `default-members = ["."]`, `resolver = "3"`.

- Discovery: the existing workspace-manifest BDD and helper layer already load
  `Cargo.toml` and `cargo metadata`, so 15.1.2 should extend those checks
  rather than duplicating them.

- Discovery: `docs/developers-guide.md` already has a dedicated
  `## Cargo workspace semantics` section that explains the staged 15.1.1
  contract. That section is the right place to document the new two-member
  workspace and the continued root-only default member.

- Discovery: the formal-verification guide already recommends the exact crate
  split needed here, including a suggested `wireframe-verification` manifest
  and file layout. The main missing work is repository integration, not design
  invention.

## Decision Log

- Decision: use the new crate only to prove the Stateright crate boundary and
  harness pattern, not to deliver meaningful model coverage of the production
  connection actor yet. Rationale: roadmap item 15.1.2 is the infrastructure
  slice that enables later verification work; deeper modelling belongs to later
  formal-verification items. Date/Author: 2026-04-21 / Codex.

- Decision: keep behavioural coverage rooted in the existing
  workspace-manifest BDD slice rather than adding a new `.feature` family for
  this item. Rationale: the user-visible behaviour here is primarily workspace
  membership and command ergonomics, which the existing slice already models.
  Date/Author: 2026-04-21 / Codex.

- Decision: treat `docs/formal-verification-methods-in-wireframe.md` as the
  authoritative design document for any crate-layout or harness choices made
  during implementation. Rationale: the roadmap item explicitly points there,
  and later formal-verification items will depend on the same design record.
  Date/Author: 2026-04-21 / Codex.

## Outcomes & Retrospective

Not started yet. Completion should leave the repository with a small internal
verification crate, passing `rstest` and `rstest-bdd` coverage, updated design
and contributor documentation, and the roadmap entry marked done. This section
must be rewritten with actual outcomes, log paths, and validation results once
implementation is complete.

## Context and orientation

The most relevant repository areas for this item are:

- `Cargo.toml`, which currently defines the hybrid root manifest with only the
  root package in `members` and `default-members`.
- `docs/roadmap.md`, which owns the 15.1.2 acceptance criteria.
- `docs/formal-verification-methods-in-wireframe.md`, especially
  `### Why Stateright belongs in a separate verification crate`,
  `### Shared checker harness`, `### Suggested Stateright file layout`, and
  `## Concrete first tasks`.
- `docs/developers-guide.md`, especially `## Cargo workspace semantics` and
  `### Workspace manifest test support`.
- `tests/common/workspace_manifest_support.rs`,
  `tests/fixtures/workspace_manifest.rs`,
  `tests/features/workspace_manifest.feature`,
  `tests/steps/workspace_manifest_steps.rs`, and
  `tests/scenarios/workspace_manifest_scenarios.rs`, which already encode the
  staged 15.1.1 workspace contract.

The new crate should follow the suggested design shape from the formal
verification guide:

- `crates/wireframe-verification/Cargo.toml`
- `crates/wireframe-verification/src/lib.rs`
- `crates/wireframe-verification/src/connection_model/mod.rs`
- `crates/wireframe-verification/src/connection_model/model.rs`
- `crates/wireframe-verification/src/connection_model/state.rs`
- `crates/wireframe-verification/src/connection_model/action.rs`
- `crates/wireframe-verification/src/connection_model/properties.rs`
- `crates/wireframe-verification/tests/verification_harness.rs`
- `crates/wireframe-verification/tests/connection_actor.rs`

The placeholder model does not need to be a faithful `ConnectionActor`
simulation. It needs only enough state, actions, and properties to prove that
the Stateright dependency compiles, the harness pattern is sound, and the crate
can host later models cleanly.

## Relevant documentation and skills

The implementer should keep these documents open while working:

- `docs/roadmap.md`
- `docs/formal-verification-methods-in-wireframe.md`
- `docs/developers-guide.md`
- `docs/users-guide.md`
- `docs/generic-message-fragmentation-and-re-assembly-design.md`
- `docs/multi-packet-and-streaming-responses-design.md`
- `docs/the-road-to-wireframe-1-0-feature-set-philosophy-and-capability-maturity.md`
- `docs/hardening-wireframe-a-guide-to-production-resilience.md`
- `docs/rust-testing-with-rstest-fixtures.md`
- `docs/reliable-testing-in-rust-via-dependency-injection.md`
- `docs/rstest-bdd-users-guide.md`
- `docs/rust-doctest-dry-guide.md`
- `docs/execplans/10-1-1-convert-root-manifest-into-hybrid-workspace.md`

The most relevant skills for implementation are:

- `execplans`, to keep this document current while the work proceeds.
- `leta`, to navigate the workspace-manifest helpers, test fixtures, and new
  crate structure without guesswork.
- `rust-router`, to route any Rust implementation or API questions to the
  smallest appropriate Rust skill.

## Approval gate

This plan stops here until the user approves it. The first implementation turn
must begin by changing `Status: DRAFT` to `Status: APPROVED` or
`Status: IN PROGRESS`, then updating `Progress`, `Decision Log`, and
`Surprises & Discoveries` as the work proceeds.

## Plan of work

### Stage A: confirm the 15.1.2 contract

Start by auditing the precise delta from 15.1.1:

1. Confirm that the root manifest still uses the staged workspace shape from
   the prerequisite item.
2. Confirm that the existing workspace-manifest tests still encode
   "verification crate absent" and therefore need to be updated for 15.1.2.
3. Decide the minimal manifest for `crates/wireframe-verification`, keeping it
   internal and aligned with the formal-verification guide.
4. Decide whether the placeholder model lives entirely under a
   `connection_model` module or whether one tiny shared support module is also
   required to keep files under the 400-line limit.

Go/no-go: proceed only once the contract is still clearly "new workspace
member, same default member, no Makefile or CI formal targets yet".

### Stage B: scaffold the verification crate

Create the new workspace member and its minimal file tree.

1. Update the root `Cargo.toml` so `members` includes
   `"crates/wireframe-verification"` while `default-members` remains `["."]`.
2. Add `crates/wireframe-verification/Cargo.toml` with `publish = false`,
   `edition = "2024"`, the Stateright dependency from the design guide, a path
   dependency on the root `wireframe` crate, and `rstest` test support.
3. Add `src/lib.rs` with crate-level and module-level documentation explaining
   that this crate hosts internal Stateright models and harnesses.
4. Add a small `connection_model` module tree matching the suggested layout,
   keeping each file focused and below the repository line-count guideline.
5. Implement a placeholder model that uses direct `stateright::Model`
   integration rather than the actor API, with a tiny state machine that can
   prove one or two safety properties and one or two reachability properties.

Acceptance signal for this stage: `cargo test -p wireframe-verification`
compiles and runs a Stateright-backed test successfully.

### Stage C: add regression coverage

Protect both the crate internals and the workspace contract.

1. Add `rstest`-based coverage in the new crate’s tests for the shared checker
   harness and the placeholder model configuration. Use fixtures to avoid
   repeating model construction.
2. Update the root-level workspace-manifest support and fixture helpers so
   they can assert that the verification crate now appears in metadata while
   the root package remains the only default member.
3. Update `tests/features/workspace_manifest.feature`,
   `tests/steps/workspace_manifest_steps.rs`, and
   `tests/scenarios/workspace_manifest_scenarios.rs` to describe the new 15.1.2
   behaviour.
4. Keep behavioural tests observational. They should inspect manifest and
   metadata state, not run nested top-level validation commands.

Acceptance signal for this stage: the new crate’s `rstest` tests pass, and the
root `bdd` target continues to prove the workspace contract through
`rstest-bdd`.

### Stage D: update design and contributor documentation

Record the architectural choice and the changed contributor workflow.

1. Update `docs/formal-verification-methods-in-wireframe.md` so it reflects
   the current roadmap numbering and the fact that
   `crates/wireframe-verification` now exists as the Stateright home.
2. Update `docs/developers-guide.md` so `## Cargo workspace semantics`
   explains the difference between plain root commands and `--workspace` now
   that the verification crate is real, and so
   `### Workspace manifest test support` explains the updated assertions.
3. Inspect `docs/users-guide.md`. If this infrastructure-only change affects no
   consumer-facing behaviour, leave the file unchanged and record that
   conclusion in the implementation notes. If any consumer-facing command or
   package-layout guidance has become relevant, document it narrowly.
4. Mark roadmap item 15.1.2 done in `docs/roadmap.md` only after the code and
   tests have passed.

Acceptance signal for this stage: the design and contributor docs describe the
new crate accurately, and the roadmap reflects completion.

### Stage E: validate and capture evidence

Run the repository gates using Makefile targets where they exist, with
`set -o pipefail` and `tee` to capture logs for review.

Minimum validation sequence:

1. `cargo test -p wireframe-verification`
2. `cargo test --workspace`
3. `make fmt`
4. `make check-fmt`
5. `make markdownlint`
6. `make nixie`
7. `make lint`
8. `make test`
9. `make test-doc`
10. `make doctest-benchmark`

Record the log paths and any environment-limited failures in
`Outcomes & Retrospective`. If a known environment issue blocks a gate, record
the exact command, the exact error, and any narrower confirming command that
did pass.

## Validation notes

Use dedicated log files under `/tmp/` so the final implementation record can
name them explicitly. A good pattern is one log per command, for example:

- `/tmp/15-1-2-wireframe-verification-cargo-test.log`
- `/tmp/15-1-2-wireframe-verification-workspace-test.log`
- `/tmp/15-1-2-wireframe-verification-make-fmt.log`
- `/tmp/15-1-2-wireframe-verification-make-lint.log`
- `/tmp/15-1-2-wireframe-verification-make-test.log`

If `make markdownlint` fails on unrelated baseline Markdown issues, first run a
targeted `markdownlint-cli2` pass on the changed files so the new work is still
validated, then record the baseline blocker separately. Do not silently skip
the full-repository command.

## Evidence to capture during implementation

When the plan is implemented, keep the evidence concise and observable:

- the `cargo metadata` facts that prove the new workspace member and unchanged
  default member,
- the names of the new `rstest` tests and the updated `rstest-bdd` scenario,
- the exact documentation sections updated,
- the validation log paths and final pass or block results.
