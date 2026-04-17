# Convert the root manifest into a hybrid workspace (roadmap 15.1.1)

This ExecPlan (execution plan) is a living document. The sections
`Constraints`, `Tolerances`, `Risks`, `Progress`, `Surprises & Discoveries`,
`Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work
proceeds.

Status: DRAFT

This draft must be approved before implementation begins. Do not treat this
file as permission to edit code or documentation beyond the plan itself.

## Purpose / big picture

Roadmap item `15.1.1` establishes the Cargo layout needed for later formal
verification work without disrupting the existing root crate or day-to-day
contributor workflow.

The current repository already appears to satisfy that contract: `Cargo.toml`
contains both `[package]` and `[workspace]`, `default-members = ["."]` is
present, the roadmap checkbox is already marked done, and the repo contains an
older implemented execplan at
`docs/execplans/10-1-1-convert-root-manifest-into-hybrid-workspace.md`.

That means this plan is not a blank-sheet implementation plan. It is a
reconciliation plan for the current roadmap numbering. The first job is to
audit the existing state, confirm whether `15.1.1` is already fully satisfied,
and then make only the smallest changes needed to align the current numbering,
supporting documentation, and validation evidence.

Observable success for the eventual approved implementation is:

1. `Cargo.toml` still declares an explicit hybrid workspace with the root
   `wireframe` package as the sole `default-members` entry.
2. `cargo metadata --no-deps --format-version 1`, `cargo build`, and
   `cargo test --workspace` all pass from the repository root.
3. Existing `rstest` and `rstest-bdd` coverage for the workspace contract
   still passes, and new tests are added only if the audit proves an actual gap.
4. The formal-verification design document and developers' guide use the
   current roadmap numbering and describe the staged workspace contract
   accurately.
5. `docs/users-guide.md` is updated only if the audit finds a real
   consumer-facing consequence.
6. The work finishes with the roadmap entry still correct as `[x]`, not
   toggled casually.

## Approval gate

After this draft is reviewed, stop and wait for explicit user approval before
making implementation edits. Silence is not approval.

## Constraints

- Treat this as a renumbered backfill for roadmap item `15.1.1` first, and an
  implementation task second. Start by reconciling the current tree with the
  already-completed `10.1.1` work.
- Preserve the existing root package metadata and public crate identity in
  `Cargo.toml`.
- Keep the root `wireframe` package as the sole `default-members` entry.
- Preserve the staged roadmap boundary: do not introduce
  `crates/wireframe-verification`, Kani install scripts, Verus layout changes,
  Makefile formal-verification targets, or continuous integration (CI) jobs
  owned by roadmap items `15.1.2` through `15.1.5`.
- Prefer reusing the existing workspace-contract tests in
  `tests/workspace_manifest.rs`, `tests/features/workspace_manifest.feature`,
  `tests/fixtures/workspace_manifest.rs`,
  `tests/steps/workspace_manifest_steps.rs`, and
  `tests/scenarios/workspace_manifest_scenarios.rs` instead of adding parallel
  coverage.
- Any unit or integration validation added by this work must use `rstest`.
- Any behavioural validation added by this work must use `rstest-bdd`.
- Do not run nested `cargo build` or `cargo test` inside Rust tests. Use those
  commands only as top-level validation steps.
- Update the relevant design document if the audit finds stale numbering or
  stale staging guidance: `docs/formal-verification-methods-in-wireframe.md`.
- Update `docs/developers-guide.md` with any contributor-facing guidance about
  the hybrid workspace or current roadmap numbering.
- Update `docs/users-guide.md` only if a library consumer needs to know
  something new.
- Keep Markdown in en-GB-oxendict spelling.
- Run validation through Makefile targets where applicable, using
  `set -o pipefail` and `tee` so truncated terminal output does not hide
  failures.

## Tolerances

- Scope: if the audit shows that `15.1.1` is already fully implemented and the
  only missing artefact is this renumbered execplan file, stop and ask whether
  the user wants a documentation-only archival pass or no further change.
- Roadmap boundary: if satisfying this item now would require creating
  `crates/wireframe-verification` or extending workspace members beyond the
  current staged layout, stop and escalate because that is `15.1.2` work.
- Churn: if the implementation would touch more than 8 files or more than
  400 net lines, stop and re-scope.
- Tests: if existing workspace-contract tests already prove the acceptance
  criteria, do not add duplicate tests merely to satisfy the plan structure.
- Validation: if repo-wide documentation gates fail for unrelated baseline
  reasons, record the exact failures, leave the roadmap status unchanged, and
  escalate with evidence.
- Environment: if `make lint` is still blocked by the missing `whitaker`
  wrapper or another known container issue, record the blocker and do not
  misreport the gate as passed.

## Risks

- Risk: the repository already contains a completed execplan under the old
  numbering, so a careless implementation could duplicate history or overwrite
  evidence. Severity: high. Likelihood: high. Mitigation: treat
  `docs/execplans/10-1-1-convert-root-manifest-into-hybrid-workspace.md` as
  source material and preserve it as the implementation record for the original
  work.

- Risk: `docs/formal-verification-methods-in-wireframe.md` and
  `docs/developers-guide.md` still refer to roadmap item `10.1.1`, which now
  disagrees with `docs/roadmap.md`. Severity: medium. Likelihood: high.
  Mitigation: audit every `10.1.x` reference and update only the references
  that truly map to the renumbered formal-verification phase.

- Risk: Cargo metadata for this repository includes the in-tree
  `wireframe_testing` helper crate in `workspace_members` even when
  `members = ["."]`. Severity: medium. Likelihood: high. Mitigation: preserve
  the documented contract that the root package remains the sole
  `default-members` entry, and keep tests focused on that stable behaviour.

- Risk: a fresh implementation pass could accidentally drift into `15.1.2`
  by "cleaning up" the workspace member list or introducing a placeholder
  verification crate. Severity: high. Likelihood: medium. Mitigation: make the
  staged boundary explicit in the plan and stop immediately if that boundary is
  threatened.

- Risk: validation evidence from the earlier `10.1.1` work may no longer be
  reproducible exactly because tooling has changed. Severity: medium.
  Likelihood: medium. Mitigation: re-run the current acceptance commands on the
  current tree and record any environment-limited failures precisely.

## Progress

- [x] (2026-04-17 00:00Z) Drafted this execplan after reviewing
  `docs/roadmap.md`, `Cargo.toml`, `Makefile`,
  `docs/formal-verification-methods-in-wireframe.md`,
  `docs/developers-guide.md`, the existing workspace-contract tests, and the
  earlier implemented execplan
  `docs/execplans/10-1-1-convert-root-manifest-into-hybrid-workspace.md`.
- [ ] Stage A: audit the current tree against the `15.1.1` acceptance
  criteria and record whether any real implementation delta remains.
- [ ] Stage B: reconcile stale roadmap numbering or staging language in the
  formal-verification documentation and developers' guide.
- [ ] Stage C: make the smallest code or test changes required, if the audit
  proves that the current tree does not fully satisfy `15.1.1`.
- [ ] Stage D: run the full validation set and record evidence.
- [ ] Stage E: update this execplan's status and retrospective after the
  approved implementation concludes.

## Surprises & Discoveries

- Discovery: `docs/roadmap.md` already marks `15.1.1` done, but the repo did
  not yet have a matching `docs/execplans/15-1-1-...` file. The only existing
  implementation record is
  `docs/execplans/10-1-1-convert-root-manifest-into-hybrid-workspace.md`.

- Discovery: `Cargo.toml` already contains the staged hybrid-workspace shape:
  `[workspace]`, `members = ["."]`, `default-members = ["."]`, and
  `resolver = "3"`.

- Discovery: the repository already has both `rstest` and `rstest-bdd`
  coverage for the workspace contract, so this item should begin from evidence
  review rather than new test authoring.

- Discovery: `docs/formal-verification-methods-in-wireframe.md` and
  `docs/developers-guide.md` still describe the same staged design using the
  earlier `10.1.1` and `10.1.2` numbering.

- Discovery: the design guide's Cargo explanation already documents the
  important subtlety that `wireframe_testing` can appear in
  `workspace_members`, even though the root crate remains the sole
  `default-members` entry.

## Decision Log

- Decision: frame this document as a reconciliation and numbering-alignment
  plan, not as a first implementation plan. Rationale: the current tree already
  appears to satisfy the core acceptance criteria, and a second
  "implementation" must not blindly redo completed work. Date/Author:
  2026-04-17 / Codex.

- Decision: preserve the older `10-1-1` execplan as the historical execution
  record and use this file as the current-numbered approval gate for any
  follow-up clean-up. Rationale: renumbering should not erase implementation
  history. Date/Author: 2026-04-17 / Codex.

- Decision: prefer an audit-first path that updates documentation references
  and validation evidence before considering any Cargo or test edits.
  Rationale: the most likely remaining gap is numbering drift, not missing
  workspace behaviour. Date/Author: 2026-04-17 / Codex.

## Context and orientation

The key repository areas for this work are:

- `Cargo.toml`, which now contains the hybrid root manifest.
- `docs/roadmap.md`, which owns the current `15.1.1` acceptance criteria.
- `docs/formal-verification-methods-in-wireframe.md`, which explains the
  staged workspace design and the later verification-crate split.
- `docs/developers-guide.md`, which documents contributor-facing Cargo
  semantics and the workspace-contract test support.
- `docs/users-guide.md`, which should change only if the audit finds a
  consumer-facing consequence.
- `tests/workspace_manifest.rs` and the `workspace_manifest` BDD files, which
  already express the intended contract.
- `docs/execplans/10-1-1-convert-root-manifest-into-hybrid-workspace.md`,
  which records the earlier implementation and validation evidence.

The most important nuance is the staged boundary:

1. `15.1.1` is only the hybrid root-manifest conversion with the root package
   preserved as the default member.
2. `15.1.2` introduces `crates/wireframe-verification` and extends the
   workspace member set.

Any approved implementation must keep those two roadmap items separate.

## Relevant documentation and skills

Primary documentation for this work:

- `docs/roadmap.md`
- `docs/formal-verification-methods-in-wireframe.md`
- `docs/developers-guide.md`
- `docs/users-guide.md`
- `docs/execplans/10-1-1-convert-root-manifest-into-hybrid-workspace.md`

Supporting documentation to keep in view while validating and extending test
coverage:

- `docs/rust-testing-with-rstest-fixtures.md`
- `docs/rstest-bdd-users-guide.md`
- `docs/rust-doctest-dry-guide.md`
- `docs/hardening-wireframe-a-guide-to-production-resilience.md`
- `docs/generic-message-fragmentation-and-re-assembly-design.md`
- `docs/multi-packet-and-streaming-responses-design.md`
- `docs/the-road-to-wireframe-1-0-feature-set-philosophy-and-capability-maturity.md`
- `reliable-testing-in-rust-via-dependency-injection.md`

Skills that should be used during the eventual implementation:

- `execplans` for maintaining this document as a living plan.
- `arch-crate-design` for workspace and internal-crate boundary decisions.
- `leta` for navigating the existing Cargo, test, and documentation symbols
  before changing them.
- `en-gb-oxendict-style` for documentation edits.
- `rust-router` if the audit uncovers a real Rust implementation delta.

## Plan of work

### Stage A: reconcile the current state with the roadmap item

Start by proving what is already true in the current tree.

Read:

- `Cargo.toml`
- `docs/roadmap.md`
- `docs/formal-verification-methods-in-wireframe.md`
- `docs/developers-guide.md`
- `docs/execplans/10-1-1-convert-root-manifest-into-hybrid-workspace.md`
- the workspace-contract tests under `tests/`

Produce a short audit note in this file covering:

1. which `15.1.1` acceptance criteria are already satisfied;
2. whether any Cargo or test delta remains;
3. which documentation references still use the old numbering; and
4. whether `docs/users-guide.md` genuinely needs an edit.

Go/no-go rule: if the audit finds that only numbering drift remains, keep the
implementation scope to documentation and evidence. Do not reopen the Cargo
layout itself.

### Stage B: align the documentation to the current roadmap numbering

If the audit confirms stale references, update the affected documents in place.

Expected targets are:

- `docs/formal-verification-methods-in-wireframe.md`
- `docs/developers-guide.md`

The edits should:

1. replace stale `10.1.1` and `10.1.2` references with `15.1.1` and `15.1.2`
   where those references clearly point at the formal-verification phase in the
   current roadmap;
2. preserve the existing staged explanation that `15.1.1` stops at the hybrid
   workspace and `15.1.2` adds `crates/wireframe-verification`; and
3. leave `docs/users-guide.md` unchanged unless the audit found a concrete
   consumer-facing implication.

If it would reduce future confusion without rewriting history, add a brief
archival note in the old `10-1-1` execplan that the roadmap item was later
renumbered to `15.1.1`. Do this only if the note is clearly historical and does
not distort the implementation record.

### Stage C: correct any real acceptance gap, but only if the audit proves one

Only enter this stage if Stage A proves that the current tree does not satisfy
the roadmap acceptance criteria.

Any implementation here must be minimal. Acceptable examples include:

- tightening a stale assertion in the existing workspace-contract tests;
- fixing contributor documentation that now contradicts the manifest; or
- repairing a small manifest detail if the current tree no longer matches the
  staged contract.

Unacceptable work in this stage includes:

- creating `crates/wireframe-verification`;
- changing the workspace member set to include the verification crate;
- adding Kani, Stateright, or Verus tooling;
- expanding CI or Makefile formal-verification targets.

### Stage D: validate the result and capture evidence

Run the acceptance commands from the repository root. Use `set -o pipefail` and
`tee` for every long-running command so the logs survive output truncation.

Required acceptance commands:

```sh
set -o pipefail && cargo metadata --no-deps --format-version 1 \
  2>&1 | tee /tmp/15-1-1-cargo-metadata.log
set -o pipefail && cargo build \
  2>&1 | tee /tmp/15-1-1-cargo-build.log
set -o pipefail && cargo test --workspace \
  2>&1 | tee /tmp/15-1-1-cargo-test-workspace.log
```

If any test or documentation files change, also run the focused regression
checks that already cover this feature:

```sh
set -o pipefail && cargo test --test workspace_manifest \
  2>&1 | tee /tmp/15-1-1-workspace-manifest-test.log
set -o pipefail && cargo test --test bdd --features advanced-tests \
  workspace_manifest 2>&1 | tee /tmp/15-1-1-workspace-manifest-bdd.log
```

For documentation gates, run:

```sh
set -o pipefail && make fmt \
  2>&1 | tee /tmp/15-1-1-make-fmt.log
set -o pipefail && make markdownlint \
  2>&1 | tee /tmp/15-1-1-make-markdownlint.log
set -o pipefail && make nixie \
  2>&1 | tee /tmp/15-1-1-make-nixie.log
```

If Stage C touched Rust code or tests, also run the normal code-quality gates:

```sh
set -o pipefail && make check-fmt \
  2>&1 | tee /tmp/15-1-1-make-check-fmt.log
set -o pipefail && make lint \
  2>&1 | tee /tmp/15-1-1-make-lint.log
set -o pipefail && make test \
  2>&1 | tee /tmp/15-1-1-make-test.log
```

Record all outcomes in this file, including any environment-limited failures.

### Stage E: close the plan cleanly

When the approved implementation finishes:

1. update `Progress`;
2. add final notes to `Surprises & Discoveries`;
3. record any changes in `Decision Log`;
4. write the exact validation outcomes in `Outcomes & Retrospective`; and
5. leave `docs/roadmap.md` correct as `[x]` unless the audit proves that the
   checkbox was premature.

## Outcomes & Retrospective

Not started yet. This plan exists to obtain approval before any implementation
or reconciliation work begins.
