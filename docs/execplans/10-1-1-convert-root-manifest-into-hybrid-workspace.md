# Convert the root manifest into a hybrid workspace

This ExecPlan is a living document. The sections `Constraints`, `Tolerances`,
`Risks`, `Progress`, `Surprises & Discoveries`, `Decision Log`, and
`Outcomes & Retrospective` must be kept up to date as work proceeds.

Status: DRAFT

This document must be maintained in accordance with `AGENTS.md` at the
repository root, including quality gates, test policy, and documentation style
requirements.

## Purpose / big picture

Roadmap item 10.1.1 establishes the Cargo layout needed for formal verification
work without disrupting the existing root package or normal contributor
workflow.

After this change:

- the repository root `Cargo.toml` is a hybrid manifest containing both
  `[package]` and `[workspace]`,
- the root `wireframe` package remains the default workspace member, so plain
  root `cargo build`, `cargo test`, `cargo check`, and `cargo clippy` keep
  their current ergonomics,
- the repository is ready for follow-on items that add
  `crates/wireframe-verification`, Kani, Stateright, and Verus support.

Observable success is:

- `cargo metadata --no-deps --format-version 1` reports a workspace rooted at
  the repository and shows the root package as the only default member for this
  stage.
- `cargo build` passes from the repository root.
- `cargo test --workspace` passes from the repository root.
- New `rstest` integration coverage and `rstest-bdd` behavioural coverage
  protect the workspace-default-member contract without recursively invoking
  nested `cargo test` runs.
- `docs/formal-verification-methods-in-wireframe.md` records the staged
  workspace decision, `docs/developers-guide.md` explains the new contributor
  workflow, `docs/users-guide.md` is updated if consumer-facing guidance
  changes, and `docs/roadmap.md` marks 10.1.1 done.

## Constraints

- Preserve the existing root package metadata and public crate identity in
  `Cargo.toml`.
- Keep the root package as the default workspace member for all non-workspace
  cargo commands.
- Treat 10.1.1 as infrastructure-only. Do not implement the verification crate,
  Kani and Verus tooling, Makefile verification targets, or CI jobs owned by
  roadmap items 10.1.2 through 10.1.5 unless a minimal compatibility shim is
  strictly required.
- Prefer no new dependencies. In particular, avoid introducing a crate solely
  to inspect Cargo metadata unless a smaller local helper is clearly
  insufficient.
- Unit and integration validation must use `rstest`, reusing fixtures where
  shared repository-metadata setup exists.
- Behavioural validation must use `rstest-bdd` with the existing
  feature/fixture/steps/scenario layout under `tests/`.
- Behavioural tests must not recursively run `cargo build` or `cargo test`
  inside `cargo test`; reserve those commands for top-level validation.
- Record the staging decision in the relevant design document:
  `docs/formal-verification-methods-in-wireframe.md`.
- Update `docs/developers-guide.md` with any contributor-facing guidance about
  hybrid workspace semantics and command usage.
- Update `docs/users-guide.md` if, and only if, the workspace conversion
  changes something a library consumer needs to know.
- On completion of the feature, mark roadmap item 10.1.1 done in
  `docs/roadmap.md`.

## Tolerances (exception triggers)

- Scope: if keeping 10.1.1 separate from 10.1.2 proves impossible without
  creating a placeholder `crates/wireframe-verification` package, stop and
  decide explicitly whether to collapse roadmap boundaries or relax the member
  list temporarily.
- Dependencies: if metadata validation appears to require a new parser crate,
  first attempt a minimal local parser or stable string-based assertions; only
  add a dependency with recorded rationale.
- Tooling spillover: if the workspace conversion forces Makefile or CI changes
  beyond documentation updates, stop and reassess because 10.1.4 and 10.1.5 own
  that plumbing.
- Test stability: if metadata-based `rstest-bdd` coverage is flaky across
  three hardening attempts, fall back to a simpler manifest-observation
  scenario and document the limitation.
- Validation: if `cargo test --workspace` surfaces unrelated pre-existing
  failures, record them separately and only proceed once it is clear whether
  10.1.1 caused the regression.

## Risks

- Risk: the formal-verification guide currently shows the final end-state
  member list (`[".", "crates/wireframe-verification"]`), while the roadmap
  splits workspace conversion and verification-crate creation into separate
  items. Severity: high. Likelihood: high. Mitigation: document 10.1.1 as a
  staged workspace conversion and update the design document to explain when
  the verification crate joins the workspace.

- Risk: path dependencies such as `wireframe = { path = ".", ... }` in
  `dev-dependencies` and `wireframe_testing` may behave differently once the
  root manifest also acts as a workspace. Severity: medium. Likelihood: medium.
  Mitigation: validate `cargo metadata`, `cargo build`, and
  `cargo test --workspace` early, before adding regression tests or docs.

- Risk: repository-metadata tests can become brittle if they assert absolute
  package IDs or filesystem paths. Severity: medium. Likelihood: medium.
  Mitigation: assert only stable properties such as package names, relative
  member counts, and the presence of the root package in default members.

- Risk: contributors may incorrectly assume that plain `cargo test` now
  includes future verification crates. Severity: medium. Likelihood: medium.
  Mitigation: update `docs/developers-guide.md` with explicit command
  semantics, including when to use `--workspace`.

## Progress

- [x] (2026-04-10) Drafted the ExecPlan, reviewed the roadmap item, reviewed
      the formal-verification design guidance, and confirmed the current repo
      is still a single-package root with `wireframe_testing` outside any
      workspace.
- [ ] Stage A: confirm the staged workspace contract and target file list.
- [ ] Stage B: convert the root manifest into a hybrid workspace.
- [ ] Stage C: add `rstest` and `rstest-bdd` regression coverage for workspace
      invariants.
- [ ] Stage D: update design and contributor documentation, inspect the user
      guide, and mark the roadmap item done.
- [ ] Stage E: run the full validation and documentation gates.

## Surprises & Discoveries

- Discovery: `docs/formal-verification-methods-in-wireframe.md` already
  recommends a hybrid workspace and explicitly calls out
  `default-members = ["."]`, but its example shows the later end-state member
  list that includes `crates/wireframe-verification`.

- Discovery: `wireframe_testing` is currently a separate path crate and not a
  workspace member, which means a hybrid workspace conversion must avoid
  accidentally implying that all path crates join the workspace automatically.

- Discovery: the `Makefile` already describes `typecheck` as a "workspace
  typecheck" even though the repository is not yet a workspace. The
  documentation update should align contributor language with the actual Cargo
  layout.

## Decision Log

- Decision: implement 10.1.1 as a staged workspace conversion. Add
  `[workspace]`, `default-members = ["."]`, and `resolver = "3"` in the root
  manifest during 10.1.1, but defer adding `crates/wireframe-verification` to
  `members` until 10.1.2 creates the crate, unless Cargo forces a placeholder
  package earlier. Rationale: the roadmap explicitly separates the workspace
  conversion from verification-crate creation, so the implementation should
  preserve that granularity and document the staged rollout in the design doc.
  Date/Author: 2026-04-10 / Codex.

- Decision: use `cargo metadata` as the regression-test oracle for workspace
  semantics, while keeping `cargo build` and `cargo test --workspace` as
  top-level validation commands rather than nested test subprocesses.
  Rationale: metadata inspection is stable, fast, and avoids recursive build
  execution from inside `cargo test`. Date/Author: 2026-04-10 / Codex.

## Outcomes & Retrospective

(To be completed when implementation lands.)

## Context and orientation

Relevant repository areas:

- `Cargo.toml` currently defines only the root `wireframe` package.
- `wireframe_testing/Cargo.toml` depends on the root crate by path but is not a
  workspace member today.
- `docs/roadmap.md` owns the 10.1.1 acceptance criteria.
- `docs/formal-verification-methods-in-wireframe.md` is the relevant design
  document and already defines the eventual hybrid-workspace end state.
- `docs/developers-guide.md` is the correct place for contributor-facing Cargo
  workflow guidance.
- `docs/users-guide.md` must be inspected for any consumer-facing fallout,
  although none is expected.
- `tests/` already contains both `rstest` integration tests and the
  `rstest-bdd` structure under `tests/features/`, `tests/fixtures/`,
  `tests/steps/`, and `tests/scenarios/`.

The most important design nuance for this item is the distinction between the
**stage-1 workspace shape** and the **final formal-verification end state**.
Item 10.1.1 should only make the root manifest workspace-aware and keep the
root package as the default member. Item 10.1.2 should then add the dedicated
verification crate and extend the member list.

## Plan of work

### Stage A: confirm the staged workspace contract

Resolve the staging question before editing files:

1. Verify that the root manifest can become a hybrid workspace without adding a
   placeholder verification crate.
2. Lock the 10.1.1 contract to "hybrid workspace now, verification member in
   10.1.2" unless Cargo proves otherwise.
3. Identify the minimal file set for this item:
   `Cargo.toml`, design docs, developer docs, optional user docs, roadmap, and
   new regression-test files.

Go/no-go: proceed only once the staged contract is written into this ExecPlan
and ready to be mirrored into the formal-verification design doc.

### Stage B: convert the root manifest

Edit the root manifest to add the workspace section while preserving the root
package.

Planned manifest changes:

- add `[workspace]`,
- set `members = ["."]` for 10.1.1,
- set `default-members = ["."]`,
- set `resolver = "3"`.

Then validate the raw Cargo behaviour immediately:

- `cargo metadata --no-deps --format-version 1`
- `cargo build`
- `cargo test --workspace`

If the hybrid layout changes package resolution, path dependency behaviour, or
test target discovery, fix those regressions before moving on to test or docs
work.

Go/no-go: continue only when the repository builds and the workspace metadata
matches the staged 10.1.1 contract.

### Stage C: add `rstest` regression coverage

Add a focused integration test, likely `tests/workspace_manifest.rs`, that uses
`#[rstest]` fixtures to gather and reuse repository metadata.

The test should validate stable invariants only:

- the repository now exposes a workspace,
- the root package is a workspace member,
- the root package is the default member,
- 10.1.1 does not unexpectedly widen default-member coverage.

Prefer a small local metadata helper in `tests/common/` if more than one test
needs to inspect the `cargo metadata` output. Do not invoke nested
`cargo build` or `cargo test` from these tests.

Go/no-go: continue only when the new `rstest` coverage passes reliably in
isolation.

### Stage D: add `rstest-bdd` behavioural coverage

Add a small BDD slice that expresses the contributor-facing behaviour of the
new layout, for example:

- feature file under `tests/features/workspace_manifest.feature`,
- fixture world under `tests/fixtures/workspace_manifest.rs`,
- step definitions under `tests/steps/workspace_manifest_steps.rs`,
- scenario bindings under `tests/scenarios/workspace_manifest_scenarios.rs`,
- module wiring in `tests/fixtures/mod.rs`, `tests/steps/mod.rs`, and
  `tests/scenarios/mod.rs`.

The scenario should describe the observable contract in plain language: the
repository acts as a hybrid workspace, while the root package remains the
default target for ordinary root-level cargo commands. The implementation
should inspect metadata or other stable manifest observations, not recursively
re-run the full build.

Go/no-go: continue only when the new BDD coverage compiles and passes alongside
the existing feature suite.

### Stage E: documentation and roadmap updates

Update the design and contributor documentation once the manifest and tests are
stable.

Required documentation work:

- update `docs/formal-verification-methods-in-wireframe.md` so the
  "Root `Cargo.toml` changes" section explains the staged rollout:
  `members = ["."]` in 10.1.1, then `[".", "crates/wireframe-verification"]` in
  10.1.2;
- update `docs/developers-guide.md` with hybrid-workspace semantics and clear
  guidance on when to use plain cargo commands versus `--workspace`;
- inspect `docs/users-guide.md` and update it only if the workspace change
  alters anything a library consumer needs to know;
- mark 10.1.1 done in `docs/roadmap.md` after all gates pass.

Go/no-go: proceed to final validation only when the docs reflect the actual
implemented contract.

### Stage F: validation and quality gates

Run targeted checks first, then the full repository gates. Because long output
is truncated in this environment, every command should use `set -o pipefail`
and `tee` to a log file.

Targeted validation:

- `cargo test --test workspace_manifest`
- `cargo test --test bdd workspace_manifest`
- `cargo metadata --no-deps --format-version 1`
- `cargo build`
- `cargo test --workspace`

Repository gates:

- `make fmt`
- `make check-fmt`
- `make lint`
- `make test`
- `make test-doc`
- `make doctest-benchmark`
- `make markdownlint`
- `make nixie`

Record the resulting log paths in `Outcomes & Retrospective` when the feature
is implemented.

## Concrete file plan

Expected edits for implementation:

- `Cargo.toml`
- `docs/formal-verification-methods-in-wireframe.md`
- `docs/developers-guide.md`
- `docs/users-guide.md` if required
- `docs/roadmap.md`
- `tests/workspace_manifest.rs`
- `tests/features/workspace_manifest.feature`
- `tests/fixtures/workspace_manifest.rs`
- `tests/steps/workspace_manifest_steps.rs`
- `tests/scenarios/workspace_manifest_scenarios.rs`
- `tests/fixtures/mod.rs`
- `tests/steps/mod.rs`
- `tests/scenarios/mod.rs`

Files that should remain out of scope for 10.1.1 unless a blocker appears:

- `crates/wireframe-verification/**`
- `tools/kani/**`
- `tools/verus/**`
- `scripts/install-kani.sh`
- `scripts/install-verus.sh`
- `scripts/run-verus.sh`
- `.github/workflows/**`
- `Makefile` verification targets owned by 10.1.4
