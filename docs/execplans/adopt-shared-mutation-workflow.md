# Adopt the shared mutation-testing reusable workflow

This ExecPlan (execution plan) is a living document. The sections
`Constraints`, `Tolerances`, `Risks`, `Progress`,
`Surprises & Discoveries`, `Decision Log`, and
`Outcomes & Retrospective` must be kept up to date as work proceeds.

Status: IN PROGRESS (implementation pre-approved: the requester asked to
"plan and implement" in one instruction, which stands in for the usual
approval gate)

## Purpose / big picture

Wireframe currently runs mutation testing through a bespoke GitHub
Actions workflow (`.github/workflows/mutation-testing.yml`, merged as
PR #563 per ADR-007). That logic has since been generalized into a
reusable workflow, `mutation-cargo.yml` in `leynos/shared-actions`
(merged as PR #319 there), which adds sharded full runs, tested Python
helper scripts, and a merged shard summary. After this change, wireframe
becomes a thin caller of the shared workflow — the estate's reference
caller — and stops carrying its own detection/summary shell. The caller
also enables `--all-features` and scaffolding excludes, resolving
wireframe issue #571 (feature-gated tests were compiled out during
mutation runs, producing ~14 false survivors; example/test-support
files polluted the survivors table).

To observe success: after merge, the Actions tab still shows a
"Mutation testing" workflow; a scheduled run on a quiet day skips in
seconds; a manual dispatch fans out across six shards and posts one
merged summary; and serde-bridge mutants (`src/message.rs`,
`src/serializer.rs`) no longer appear as survivors because their
feature-gated tests now run.

## Constraints

- The workflow file stays at `.github/workflows/mutation-testing.yml`
  with the workflow name "Mutation testing", so history and any saved
  links remain continuous.
- The caller must pin `leynos/shared-actions` by full commit SHA
  (47aea18960d24f33aedc4782ec6b73e365418313, the PR #319 merge commit).
- The caller job must grant exactly `contents: read` and
  `id-token: write` (the shared workflow's OIDC source resolution
  requires the latter; a reusable workflow cannot exceed the caller's
  grant).
- Behavioural parity with ADR-007 must hold: daily 04:30 UTC schedule,
  manual dispatch, informational-only (no PR gating), 25-hour
  detection window, `wireframe_testing` as a separate target.
- No Rust code changes. Docs edits limited to ADR-007 and
  `docs/developers-guide.md`.
- All Markdown must pass `make markdownlint` and `make nixie`;
  en-GB-oxendict spelling.
- Red-Green-Refactor is unavailable: wireframe has no workflow test
  harness, and the change's runtime surface is GitHub Actions. The
  nearest observable substitutes are static validation (YAML parse,
  `actionlint`, `zizmor`) plus a post-merge dispatch run (see
  Validation and acceptance).

## Tolerances (exception triggers)

- Scope: more than 4 files changed (net), stop and escalate.
- Interface: any change to the shared workflow itself is out of scope;
  if the caller cannot express something without one, stop and
  escalate.
- Dependencies: none may be added.
- Iterations: if static validation still fails after 3 fix attempts,
  stop and escalate.
- Ambiguity: if the merged shared workflow's inputs do not match this
  plan's expectations (checked in Stage A), stop and present options.

## Risks

- Risk: `--all-features` changes the baseline test set for mutation
  runs; an unexpected feature interaction could fail the baseline
  (exit 4) on the first real run.
  Severity: low. Likelihood: low.
  Mitigation: `make test` already runs
  `cargo test --all-targets --all-features` in CI, so the feature set
  is exercised on every push; the loom-gated advanced tests
  additionally require `--cfg loom` and stay inert. Validated for real
  by the post-merge dispatch run.
- Risk: the pinned shared workflow has a defect only visible in real
  runs (its act tests cover the guard path only).
  Severity: medium. Likelihood: low.
  Mitigation: this adoption is Stage G of the shared-actions plan —
  wireframe is the designated pilot, and the dispatch run after merge
  is the acceptance test. Rolling back is a one-commit revert to the
  bespoke workflow.

## Progress

- [x] (2026-07-05) Stage A: verify the merged shared workflow's inputs
  and the PR #319 merge SHA; confirm `make test` uses
  `--all-features`; confirm wireframe features (`serializer-serde`
  non-default; `advanced-tests` inert without `--cfg loom`).
- [x] (2026-07-05) Stage B: bespoke workflow replaced with the thin
  caller pinned to 47aea1896.
- [x] (2026-07-05) Stage C: ADR-007 status amended (Accepted;
  implementation superseded by the shared workflow, sketch retained as
  history); developers-guide mutation section rewritten for the caller.
- [x] (2026-07-05) Stage D: YAML parse, actionlint, and zizmor all
  clean; markdownlint and nixie green; CodeRabbit review clean; single
  commit pushed.
- [x] (2026-07-06) Review round: added
  `tests/workflow_contract.rs` — six contract checks over the caller
  (full-SHA pin with hex validation, least-privilege permissions with
  the OIDC scope, ref-keyed queueing concurrency, retained triggers,
  the issue-571 configuration, and the companion-crate target). The
  pin guard was negative-tested: repointing the pin at `@main` fails
  `caller_pins_the_shared_workflow_by_full_commit_sha`; restoring the
  SHA passes all six.
- [ ] Stage E (post-merge, outside this branch): dispatch a manual run;
  confirm shard fan-out, merged summary, and the disappearance of the
  serde-bridge false survivors; then update the estate rollout plan
  and the shared-actions ExecPlan Stage G.

## Surprises & discoveries

- Observation: none yet beyond Stage A confirmations.

## Decision log

- Decision: drop the bespoke workflow's `branch` dispatch input.
  Rationale: the Actions UI "Run workflow" control already selects the
  ref for `workflow_dispatch`, and the reusable workflow checks out the
  triggering ref; the input was a redundancy that also complicated the
  concurrency key. The concurrency group now keys on `github.ref`.
  Date/Author: 2026-07-05, planning agent.
- Decision: set `extra-args: "--all-features"` on the caller.
  Rationale: matches `make test` (the CI baseline) and resolves issue
  #571's false-survivor class — the serde-bridge round-trip tests are
  gated on the non-default `serializer-serde` feature and were
  compiled out of mutation baselines.
  Date/Author: 2026-07-05, planning agent.
- Decision: set `exclude-globs` to `src/codec/examples.rs`,
  `src/test_helpers.rs`, `src/connection/test_support.rs`.
  Rationale: issue #571's second class — illustrative codecs and
  test-support scaffolding whose survivors are noise. The test-only
  snapshot accessors in `src/connection/state.rs` are *not* excluded:
  that file carries genuine production logic.
  Date/Author: 2026-07-05, planning agent.
- Decision: Red-Green-Refactor replaced by static validation plus a
  post-merge dispatch run. Rationale: recorded in Constraints; there is
  no local execution harness for GitHub Actions in this repository.
  Date/Author: 2026-07-05, planning agent.
- Decision: add `tests/workflow_contract.rs` per review feedback,
  raising the changed-file count to 5 against the 4-file scope
  tolerance. Rationale: the reviewer's testing check asked for an
  automated contract check over the caller boundary; a std-only Rust
  integration test (using `include_str!`, so no new dependencies)
  covers exactly the properties the reviewer named — inputs, ref
  handling, and concurrency — and runs under `make test`. The tolerance
  breach is accepted as the review directive itself constitutes the
  escalation response. Executing the workflow locally remains out of
  scope: the shared workflow's behaviour is unit- and act-tested in
  leynos/shared-actions, and duplicating that harness here would test
  someone else's code.
  Date/Author: 2026-07-06, planning agent.
- Decision: decline renaming the ExecPlan to the `NNN-N-N-` pattern.
  Rationale: that prefix maps plans to roadmap items, and this work has
  no roadmap item (the roadmap's only mutation reference concerns
  zero-copy mutation paths, 13.2.2). Repository precedent explicitly
  includes unprefixed plans for non-roadmap work:
  `adopt-cargo-mutants-workflow.md`, `code-base-audit-2026-06-05.md`,
  `migrate-from-cucumber-to-rstest-bdd.md`, and others.
  Date/Author: 2026-07-06, planning agent.
- Decision: decline adding mutation-testing coverage to
  `docs/users-guide.md`. Rationale: that guide addresses consumers of
  the wireframe library; mutation testing is contributor tooling, and
  its canonical home — `docs/developers-guide.md` — was updated in this
  change with precisely the content the review's resolution text asked
  for (shared workflow, `--all-features`, excluded paths, and the
  manual-dispatch sharded run model).
  Date/Author: 2026-07-06, planning agent.

## Outcomes & retrospective

(To be completed.)

## Context and orientation

Wireframe is a Rust async networking library. Its mutation testing was
introduced by ADR-007
(`docs/adr-007-mutation-testing-with-cargo-mutants.md`) as a bespoke
workflow at `.github/workflows/mutation-testing.yml`: a single job that
detects Rust files changed on `main` in the last 25 hours (skipping
cheaply when none), runs `cargo mutants` scoped to those files (a
second invocation covers the non-workspace `wireframe_testing` crate),
treats exit codes 2/3 as informative, and posts a jq-built summary.
Its first full dispatch run measured 1,821 mutants at roughly 15
single-job hours — beyond GitHub's job ceiling — which motivated the
sharded reusable workflow now available as
`leynos/shared-actions/.github/workflows/mutation-cargo.yml` (see that
repository's `docs/mutation-cargo-workflow.md` for the caller guide and
inputs table). The shared workflow reproduces the bespoke behaviour and
adds: shard fan-out for full dispatch runs (`shard-count`, default 6),
a merged cross-shard summary, `exclude-globs`, `extra-args`, and a
pinned cargo-mutants version (27.1.0) whose report parser is tested.

`wireframe_testing` is a companion crate outside the root workspace;
its coverage lives mainly in the root crate's tests, so its survivors
table is advisory (documented in ADR-007 and the shared caller guide).

Wireframe issue #571 requests two mutation-run configuration changes,
both expressible as caller inputs: run feature-gated tests
(`--all-features`) and exclude scaffolding paths.

## Plan of work

Stage A (done): confirm the merge SHA of shared-actions PR #319
(47aea1896…), the shared workflow's input names, and that
`--all-features` matches wireframe's CI baseline.

Stage B: overwrite `.github/workflows/mutation-testing.yml` with a
thin caller — triggers unchanged (daily cron 04:30 UTC plus bare
`workflow_dispatch`), root `permissions: {}`, a
`mutation-testing-${{ github.ref }}` concurrency group with
`cancel-in-progress: false`, and one job `mutation` granting
`contents: read` and `id-token: write`, using the pinned shared
workflow with `extra-crate-dirs: "wireframe_testing"`, the three
exclude globs, and `extra-args: "--all-features"`. All other inputs
keep the shared defaults, which match ADR-007's values.

Stage C: amend `docs/adr-007-mutation-testing-with-cargo-mutants.md` —
status note recording that the bespoke implementation is superseded by
the shared reusable workflow (decision unchanged, implementation
relocated), with the caller shown and the sketch retained as history.
Update the "Mutation testing" section of `docs/developers-guide.md` to
describe the caller, the shard behaviour on dispatch runs, and the
issue #571 configuration.

Stage D: validate and commit (single commit), then push and request a
CodeRabbit review via the scrutineer gate-runner.

Stage E (post-merge): manual dispatch; verify shard fan-out and merged
summary; confirm serde-bridge survivors are gone; update
`~/docs/mutation-testing-rollout-plan.md` and close issue #571 if the
run confirms the fix.

## Concrete steps

All commands run from the repository root
(`~/Projects/wireframe.worktrees/adopt-shared-mutation-workflow`).

Validate the workflow file:

```sh
python3 -c "import yaml; yaml.safe_load(open('.github/workflows/mutation-testing.yml'))"
actionlint .github/workflows/mutation-testing.yml
uvx zizmor .github/workflows/mutation-testing.yml
```

Expected: no output from the YAML parse; actionlint exits 0 with no
findings; zizmor reports "No findings to report."

Validate the docs:

```sh
make markdownlint
make nixie
```

Expected: `Summary: 0 error(s)` and all diagrams validated.

Commit (file-based message, per house convention) and push the branch.

## Validation and acceptance

Quality criteria: the workflow file parses, passes actionlint and
zizmor with no findings, and expresses exactly the inputs in Stage B;
`make markdownlint` and `make nixie` pass; CodeRabbit review reports no
unaddressed concerns.

Acceptance is behavioural and lands in two parts. Pre-merge: static
validation above. Post-merge (Stage E): on a quiet day the scheduled
run's `detect` job reports `has_changes=false` and the run finishes in
under a minute; a manual dispatch produces six `mutants` shard jobs for
the root target plus one for `wireframe_testing`, a
`mutation-report-*` artefact per shard, and a single merged summary
listing per-target counts — with no `src/message.rs` or
`src/serializer.rs` serde-bridge survivors and no
`src/codec/examples.rs` / test-support entries.

## Idempotence and recovery

Every step is a plain file edit; re-running validation is read-only.
Rollback at any point is `git revert` of the single commit, restoring
the bespoke workflow verbatim.

## Artifacts and notes

The caller (Stage B target state):

```yaml
jobs:
  mutation:
    permissions:
      contents: read
      id-token: write
    uses: leynos/shared-actions/.github/workflows/mutation-cargo.yml@47aea18960d24f33aedc4782ec6b73e365418313
    with:
      extra-crate-dirs: "wireframe_testing"
      exclude-globs: "src/codec/examples.rs,src/test_helpers.rs,src/connection/test_support.rs"
      extra-args: "--all-features"
```

## Interfaces and dependencies

No Rust interfaces change. The single external dependency is
`leynos/shared-actions/.github/workflows/mutation-cargo.yml`, pinned at
commit `47aea18960d24f33aedc4782ec6b73e365418313`; its input contract
is documented in that repository's `docs/mutation-cargo-workflow.md`.
