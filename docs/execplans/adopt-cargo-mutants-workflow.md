# Introduce mutation testing workflow with cargo-mutants

This ExecPlan (execution plan) is a living document. The sections `Constraints`,
`Tolerances`, `Risks`, `Progress`, `Surprises & Discoveries`, `Decision Log`,
and `Outcomes & Retrospective` must be kept up to date as work proceeds.

Status: IN PROGRESS (Stages A and C complete; Stages B and D outstanding)

## Purpose / big picture

After this change, the Wireframe project has a GitHub Actions workflow that runs
`cargo-mutants` on a daily schedule (and on demand for any branch). The
workflow uses a change-detection guard so that the expensive mutation step is a
cheap no-op when no relevant Rust source files changed on `main` in the
preceding 25 hours. When changes are detected, mutations are scoped to only the
changed files using repeated `--file` arguments. The root crate and
`wireframe_testing` (which is not a workspace member) are handled as separate
targeted invocations. Each run produces:

1. Downloadable artefacts containing the `mutants.out/` directory (and
   `wireframe_testing/mutants.out/` when `wireframe_testing` files changed),
   including `outcomes.json`.
2. A Markdown summary posted to the GitHub Actions job summary page listing
   counts (caught, missed, timed out) and a table of every surviving mutant
   with its file, line, and mutation description — or a skip message when no
   relevant changes were found.

To observe success: trigger the workflow manually via the GitHub Actions UI on
the `main` branch, wait for it to complete, then check the job summary for the
results table (or skip message) and download the artefacts.

## Constraints

- The workflow must not block pull requests or pushes. It is triggered only by
  `schedule` and `workflow_dispatch`.
- The workflow file must live at `.github/workflows/mutation-testing.yml`.
- The Rust toolchain setup must use the same shared action as `ci.yml`:
  `leynos/shared-actions/.github/actions/setup-rust@aebb3f5b831102e2a10ef909c83d7d50ea86c332`.
- Change detection must use `git log --since` with commit timestamps (not
  reflog-based `@{25.hours.ago}`) because fresh CI clones lack reflog state.
  The window is 25 hours (one hour wider than the daily cadence) to absorb
  GitHub cron start-time drift, and files absent from the checked-out tip
  must be filtered out before scoping.
- The detection step must monitor: `src/**/*.rs`,
  `wireframe_testing/src/**/*.rs`, `examples/**/*.rs`, `benches/**/*.rs`.
  Manifest-only changes (`Cargo.toml`, `Cargo.lock`) are excluded.
- `wireframe_testing` is not a workspace member, so it requires a separate
  `cargo mutants --dir wireframe_testing` invocation.
- Mutation runs must not use `--output`: `--output DIR` creates
  `mutants.out/` _within_ `DIR`, so the workflow relies on the defaults —
  `./mutants.out/` for the root crate and `wireframe_testing/mutants.out/`
  for the companion crate — and all artefact and summary paths must match.
- Run steps must treat `cargo mutants` exit codes `2` (missed mutants) and
  `3` (timeouts) as success, and exit codes `1`, `4`, and `70` as failures.
  The workflow is informational; survivors must not produce red runs.
- Mutation runs must pass `--in-place` (upstream CI recommendation) and the
  job must set `timeout-minutes: 300` as a backstop against unbounded full
  runs.
- The checkout step must set `persist-credentials: false`; the workflow only
  needs read access.
- The ADR at `docs/adr-007-mutation-testing-with-cargo-mutants.md` must be
  updated to reflect any corrections discovered during implementation
  (especially the jq field names in the workflow sketch).
- All Markdown files must pass `make markdownlint`.
- en-GB-oxendict spelling throughout documentation.

## Tolerances (exception triggers)

- Scope: if implementation requires changes to more than 5 files (net), stop
  and escalate.
- Interface: no public Rust API changes are expected or permitted.
- Dependencies: no new Cargo dependencies. `cargo-mutants` is installed at
  workflow runtime only.
- Iterations: if `make markdownlint` still fails after 3 correction attempts,
  stop and escalate.

## Risks

- Risk: The `cargo-mutants` `outcomes.json` schema may differ between
  versions; the jq filters could break on a future release. Severity: low
  Likelihood: low Mitigation: Defer version pin until after the first
  successful run confirms a known-good version, then pin it in the workflow and
  add a comment noting the version the jq filters were validated against.

- Risk: The `outcomes.json` field names discovered via source reading may not
  perfectly match runtime output (custom `Serialize` impls can diverge from
  field names). Severity: medium Likelihood: low Mitigation: Note the field
  names in a comment in the workflow. On first real run, verify the jq output
  and correct if needed.

## Progress

- [x] Stage A: Research and correct ADR sketch (2026-07-04: `outcomes.json`
  field names confirmed from source; `--output` semantics, exit codes,
  no-mutants behaviour, and `--in-place` guidance confirmed against the
  cargo-mutants handbook and source).
- [ ] Stage B: Create the workflow file.
- [x] Stage C: Update the ADR to reflect corrections (2026-07-04: output
  paths, exit-code policy, 25-hour window, deleted-file guard,
  `--in-place`, `timeout-minutes`, `persist-credentials`,
  `wireframe_testing` false-survivor caveat, cron drift and suspension
  risks all recorded in the ADR).
- [ ] Stage D: Validation and commit.

## Surprises & discoveries

- `origin/main@{24.hours.ago}` relies on reflog state, which is not available
  in fresh CI clones. The safer alternative is `git log --since` using commit
  timestamps.
- `wireframe_testing` is not a workspace member, so
  `cargo mutants --workspace` from the repo root does not cover it. A second
  invocation with `--dir wireframe_testing` is required.
- `cargo-mutants` supports repeated `--file` arguments, allowing mutation runs
  to be scoped to specific changed files rather than full-crate sweeps.
- There is no `build.rs` in this repository, so it does not need to be
  included in the change-detection patterns.
- `--output DIR` places `mutants.out/` _within_ `DIR` (confirmed from
  `OutputDir::new` in cargo-mutants source): `--output mutants.out` would
  produce `mutants.out/mutants.out/outcomes.json` and silently break the
  summary step. The defaults (`./mutants.out/`, and
  `wireframe_testing/mutants.out/` under `--dir`) are used instead.
- `cargo mutants` exits non-zero for informative outcomes: `2` = missed
  mutants, `3` = timeouts. Without explicit exit-code handling, every run
  with a surviving mutant would show as a failed workflow, contradicting
  the "purely informational" intent. Exit codes `1` (usage), `4` (baseline
  failing), `5`/`6` (`--in-diff` problems), and `70` (internal error)
  indicate genuine faults.
- If scoping matches no mutants (for example, every changed file was
  subsequently deleted), `cargo mutants` exits successfully with a warning,
  so stale `--file` arguments are benign — the tip-existence guard is
  belt-and-braces.
- The cargo-mutants handbook recommends `--in-place` for CI so runs reuse
  the existing build cache instead of copying the tree. `--in-place` is
  incompatible with `-j` parallelism, which this workflow does not use.
- `cargo-binstall` is provided by the shared `setup-rust` action (confirmed:
  `ci.yml` runs `cargo binstall` immediately after it), so no separate
  install step is needed.
- `wireframe_testing` has only a handful of in-crate tests; its logic is
  exercised mainly by the root crate's suite. Its mutation results will be
  noisy with false survivors — recorded as a known limitation in the ADR.

## Decision log

- 2026-03-31: Changed schedule from weekly to daily with a change-detection
  guard. The guard uses `git log --since="24 hours ago"` with commit
  timestamps. Rationale: daily cadence provides faster feedback; the guard
  makes quiet days a cheap no-op.
- 2026-03-31: Defined "code changes" as `src/**/*.rs`,
  `wireframe_testing/src/**/*.rs`, `examples/**/*.rs`, `benches/**/*.rs`.
  Excluded manifest-only changes (`Cargo.toml`, `Cargo.lock`). Rationale:
  `cargo-mutants` only mutates Rust source; manifest changes that affect
  mutation outcomes can be caught via manual dispatch.
- 2026-03-31: Split mutation runs into root crate and `wireframe_testing`
  invocations. Rationale: `wireframe_testing` is not a workspace member and
  requires `--dir wireframe_testing` for `cargo-mutants` to find it.
- 2026-07-04: Dropped `--output` from both invocations and aligned artefact
  and summary paths with the default output locations. Rationale:
  `--output DIR` nests `mutants.out/` inside `DIR`, so the sketched
  `--output mutants.out` would have hidden the report from the summary jq
  and produced doubly nested artefacts.
- 2026-07-04: Treat exit codes `2` and `3` as success in the run steps;
  `1`, `4`, and `70` still fail the job. Rationale: the workflow is
  informational — survivors are its deliverable, not a failure — but broken
  invocations and failing baselines must stay visible as red runs.
- 2026-07-04: Widened the change-detection window from 24 to 25 hours and
  added a tip-existence guard (`[ -f "$f" ]`) before scoping. Rationale:
  GitHub cron start times drift by up to an hour, which would silently drop
  commits landing in the gap; deleted or renamed files should not reach
  `--file`. Diffing against the last successful run's SHA was considered
  and rejected as disproportionate complexity for an informational
  workflow.
- 2026-07-04: Added `--in-place` to both invocations, `timeout-minutes: 300`
  on the job, and `persist-credentials: false` on checkout. Rationale:
  upstream recommends `--in-place` in CI for build-cache reuse; full
  dispatch runs are otherwise unbounded on a 2-core runner; the workflow
  needs only read access.
- 2026-07-04: Kept the `wireframe_testing` invocation despite expected
  false-survivor noise, recording the caveat in the ADR's Known Risks and
  the possible removal in Outstanding Decisions. Rationale: the results
  remain useful for the crate's genuinely in-crate logic, and dropping the
  run is trivial later if triage noise proves too high.

## Outcomes & retrospective

(To be completed.)

## Context and orientation

This section describes the repository layout relevant to this task.

**Wireframe** is a Rust async networking library for binary protocols. Its
source lives at the repository root with `Cargo.toml` and `src/`. The project
uses a `Makefile` for build, lint, test, and documentation targets.

Key paths:

- `.github/workflows/ci.yml` — Main CI pipeline (format, lint, test, coverage
  upload). Runs on push/PR to `main`.
- `.github/workflows/advanced-tests.yml` — Daily scheduled and manually
  dispatchable workflow running Loom-based concurrency tests. Structural
  reference for the new workflow (daily cron trigger pattern).
- `docs/adr-007-mutation-testing-with-cargo-mutants.md` — The ADR proposing
  this work. Contains a workflow sketch with jq filters that need correction.
- `docs/documentation-style-guide.md` — Formatting, spelling, and ADR
  conventions.
- `AGENTS.md` — Coding standards and commit quality gates.

**cargo-mutants `outcomes.json` structure** (determined from source reading of
`cargo-mutants` v25.x):

The top-level object has an `outcomes` array. Each element has:

- `scenario`: either the string `"Baseline"` or an object containing a
  `"Mutant"` key whose value is the mutant descriptor.
- `summary`: one of `"Success"`, `"CaughtMutant"`, `"MissedMutant"`,
  `"Unviable"`, `"Timeout"`, `"Failure"`.

The mutant descriptor object has:

- `name`: human-readable description (e.g.
  `"replace foo::bar with Default::default()"`).
- `file`: repository-relative path with forward slashes.
- `span`: object with `start` and `end`, each containing `line` and `column`.
- `replacement`: the replacement expression.
- `genre`: the mutation genre.
- `package`: the Cargo package name.

Therefore the correct jq filter for surviving mutants is:

```plaintext
.outcomes[]
| select(.scenario != "Baseline" and .summary == "MissedMutant")
| .scenario.Mutant as $m
| "| \($m.file) | \($m.span.start.line) | \($m.name) |"
```

And the correct count filters use `.outcomes[]` (not `.[]`).

**cargo-mutants behavioural facts** (confirmed 2026-07-04 against the
cargo-mutants handbook at <https://mutants.rs/> and the upstream source):

- `--output DIR` creates `mutants.out/` _within_ `DIR`; the default is
  `mutants.out/` in the source directory being mutated (for `--dir
  wireframe_testing`, that is `wireframe_testing/mutants.out/`).
- Exit codes: `0` all caught, `1` usage error, `2` missed mutants, `3`
  timeouts, `4` baseline tests already failing, `5`/`6` `--in-diff`
  problems, `70` internal error.
- If no mutants match the scoping arguments, `cargo mutants` warns and
  exits `0`.
- `--in-place` mutates the source tree directly (restoring after each
  mutant) instead of copying it, reusing the build cache; recommended for
  CI. Incompatible with `-j` parallelism.
- `--in-diff` and `--shard` exist as future refinements (line-level scoping
  and parallel splitting respectively); both are recorded as outstanding
  decisions in the ADR.

## Plan of work

### Stage A: Research and correct ADR sketch (complete)

Identify the corrections needed to the ADR's workflow sketch:

1. The top-level JSON is an object with an `outcomes` array, not a bare array.
   All jq filters must use `.outcomes[]` instead of `.[]`.
2. The `scenario` field for mutants is not the string `"Mutant"` — it is an
   object `{"Mutant": {...}}`. The baseline scenario is the string
   `"Baseline"`. The correct filter for mutant outcomes is
   `select(.scenario != "Baseline")` or `select(.scenario | type == "object")`.
3. The mutant's file path is `.scenario.Mutant.file` (not
   `.mutant.source_file`).
4. The mutant's line is `.scenario.Mutant.span.start.line` (not
   `.mutant.line`).
5. The mutant's description is `.scenario.Mutant.name` (not
   `.mutant.description`).

Additional corrections confirmed 2026-07-04 (see "cargo-mutants behavioural
facts" above): no `--output`, exit-code handling in the run steps,
`--in-place`, the 25-hour window, the tip-existence guard,
`timeout-minutes`, and `persist-credentials: false`.

No code changes in this stage — just confirming the corrections above.

### Stage B: Create the workflow file

Create `.github/workflows/mutation-testing.yml` with the following structure:

- **Triggers:** `schedule` (cron `30 4 * * *`, daily 04:30 UTC) and
  `workflow_dispatch` with an optional `branch` input defaulting to `main`.
- **Job:** `mutants`, running on `ubuntu-latest`, with `contents: read`
  permissions and `timeout-minutes: 300`.
- **Steps:**
  1. Checkout the repository at the selected branch with full history
     (`fetch-depth: 0`) and `persist-credentials: false`.
  2. Detect changed Rust files on `origin/main` in the last 25 hours using
     `git log --since="25 hours ago"` (commit timestamps, not reflog),
     skipping files absent from the checked-out tip (`[ -f "$f" ]`). Sort
     changes into root-crate files and `wireframe_testing` files using a
     `while read` loop (not `for f in $var` word-splitting). Set step
     outputs: `has_changes`, `root_files`, `wt_files`, `dispatch`. For
     `workflow_dispatch` runs, bypass the guard by setting `has_changes=true`
     and `dispatch=true` unconditionally (runs full, unscoped mutations).
  3. If no relevant changes on a scheduled run, write a skip message to
     `$GITHUB_STEP_SUMMARY` and skip all subsequent heavy steps.
  4. Setup Rust using the shared action (gated on `has_changes`).
  5. Install `cargo-mutants` via `cargo binstall --no-confirm cargo-mutants`
     (gated on `has_changes`; `cargo-binstall` is provided by the shared
     action).
  6. Run `cargo mutants --in-place` for the root crate with `--file` args
     for changed files (gated on `root_files` being non-empty or `dispatch`
     being true). Wrap the invocation in exit-code handling: `0`, `2`, and
     `3` succeed; anything else fails the step. Output lands in
     `./mutants.out/` (no `--output`).
  7. Run `cargo mutants --in-place --dir wireframe_testing` with `--file`
     args for changed `wireframe_testing` files (gated on `wt_files` being
     non-empty or `dispatch` being true), with the same exit-code handling.
     Output lands in `wireframe_testing/mutants.out/`.
  8. Upload separate artefacts for root (`mutants.out/`) and
     `wireframe_testing` (`wireframe_testing/mutants.out/`) reports.
  9. Post a Markdown summary to `$GITHUB_STEP_SUMMARY` using corrected jq
     filters (with `if: always()`), calling a `post_results` shell function
     for each report directory (`mutants.out` and
     `wireframe_testing/mutants.out`).

### Stage C: Update the ADR (complete)

Update the ADR at `docs/adr-007-mutation-testing-with-cargo-mutants.md` to
reflect:

- Daily schedule with change-detection guard (25-hour window, tip-existence
  guard).
- Revised workflow sketch with `fetch-depth: 0`,
  `persist-credentials: false`, `timeout-minutes: 300`, `detect` step,
  gated steps, split root/`wireframe_testing` invocations using default
  output locations, `--in-place`, and exit-code handling.
- Updated risk and limitation sections for the new approach, including
  `wireframe_testing` false survivors, cron drift, no-catch-up, and
  scheduled-workflow suspension.
- Resolved decisions section documenting the design choices; outstanding
  decisions for `--in-diff`, `--shard`, and whether to keep the
  `wireframe_testing` run.
- Corrected jq field names (`.outcomes[]`, `.scenario.Mutant.file`, etc.).

Completed 2026-07-04 alongside Stage A.

### Stage D: Validation and commit

1. Run `make markdownlint` — expect 0 errors.
2. Verify the YAML is syntactically valid by parsing it with a YAML tool or
   checking that `yq` / `python -c 'import yaml; ...'` succeeds.
3. Commit the new workflow file and the updated ADR as a single atomic commit.

## Concrete steps

All commands run from the repository root.

**Stage B — create workflow file:**

Write `.github/workflows/mutation-testing.yml` with the content described
above, including the `detect` step, gated `Setup Rust` / `Install` /
`Run mutation testing` steps, split root/`wireframe_testing` invocations,
separate artefact uploads, and the `post_results` shell function in the summary
step.

**Stage C — update ADR (complete 2026-07-04):**

The ADR at `docs/adr-007-mutation-testing-with-cargo-mutants.md` already
carries the corrected jq field names, daily schedule with guard, workspace
split, and Resolved Decisions section, plus the 2026-07-04 corrections:
default output locations (no `--output`), exit-code policy, 25-hour window
with tip-existence guard, `--in-place`, `timeout-minutes: 300`,
`persist-credentials: false`, and the expanded risks (including
`wireframe_testing` false survivors and cron drift/suspension). No further
ADR edits are expected in this task; Stage B must implement the workflow
exactly as the ADR sketch now describes.

**Stage D — validate:**

```sh
make markdownlint
```

Expected output:

```plaintext
Summary: 0 error(s)
```

Validate YAML syntax:

```sh
python3 -c "import yaml; yaml.safe_load(open('.github/workflows/mutation-testing.yml'))"
```

Expected: no output (success).

Commit (gated by `make markdownlint` passing):

```sh
git add .github/workflows/mutation-testing.yml \
       docs/adr-007-mutation-testing-with-cargo-mutants.md
git commit
```

## Validation and acceptance

**Quality criteria (what "done" means):**

- `make markdownlint` passes with 0 errors.
- `.github/workflows/mutation-testing.yml` is syntactically valid YAML.
- The workflow file contains both `schedule` (daily) and `workflow_dispatch`
  triggers.
- The workflow checks out with `fetch-depth: 0` and
  `persist-credentials: false`, and has a `detect` step that uses
  `git log --since="25 hours ago"` (not reflog) with a tip-existence guard.
- The job sets `timeout-minutes: 300`.
- Heavy steps (`Setup Rust`, `Install cargo-mutants`, `Run mutation testing`)
  are gated on `steps.detect.outputs.has_changes == 'true'`.
- `workflow_dispatch` runs bypass the change-detection guard by setting
  `has_changes=true` unconditionally and running full (unscoped) mutations.
- Root crate and `wireframe_testing` mutation runs are separate steps with
  appropriate `--file` and `--dir` arguments, both passing `--in-place`,
  neither passing `--output`, and both wrapping the invocation in the
  exit-code handling (`0`/`2`/`3` succeed; all else fails).
- Artefact and summary paths reference `mutants.out/` and
  `wireframe_testing/mutants.out/`.
- The jq filters in both the workflow and the ADR use the corrected field
  names (`.outcomes[]`, `.scenario.Mutant.file`, etc.).

**Quality method (how to check):**

- `make markdownlint` — automated.
- YAML syntax check — `python3 -c "import yaml; ..."` or equivalent.
- Manual review of the jq filters against the field name reference in the
  Context section above.
- Manual review of the change-detection step logic.

**Observable behaviour after merge:**

- Navigate to Actions → "Mutation testing" in the GitHub UI.
- On a day with no recent `main` changes, the daily run shows
  "Mutation testing skipped" in the job summary and completes in seconds.
- Click "Run workflow" (manual dispatch), select a branch, and trigger.
  Manual runs bypass the change-detection guard and always run mutations.
- After completion, the job summary shows caught/missed/timeout counts and a
  table of surviving mutants (per crate).
- Separate `mutation-report-root` and `mutation-report-wireframe-testing`
  artefacts are available for download (when applicable).

## Idempotence and recovery

All steps are idempotent. The workflow file can be overwritten and
re-committed. The ADR edits are textual replacements that produce the same
result if applied more than once. `make markdownlint` is read-only and safe to
re-run.

## Artefacts and notes

The change-detection snippet (for reference during implementation):

```sh
# Use commit timestamps (not reflog) — safe in fresh CI clones.
# 25-hour window absorbs GitHub cron start-time drift.
git log --since="25 hours ago" --name-only \
  --format="" origin/main -- '*.rs' | sort -u
# Consume with a `while IFS= read -r f` loop, skipping entries where
# `[ -f "$f" ]` fails (deleted or renamed since the change).
```

The exit-code wrapper for the run steps (for reference during
implementation):

```sh
set +e
cargo mutants --in-place --timeout-multiplier 3 $args
status=$?
set -e
case "$status" in
  0|2|3) exit 0 ;;  # all caught / missed mutants / timeouts: informative
  *) exit "$status" ;;  # usage error, failing baseline, internal error
esac
```

The corrected jq snippet for the summary step (for reference during
implementation):

```sh
# Count outcomes (parameterised by $dir)
caught=$(jq '
  [.outcomes[]
   | select(.scenario != "Baseline"
            and .summary == "CaughtMutant")]
  | length' "$dir/outcomes.json")
missed=$(jq '
  [.outcomes[]
   | select(.scenario != "Baseline"
            and .summary == "MissedMutant")]
  | length' "$dir/outcomes.json")
timeout=$(jq '
  [.outcomes[]
   | select(.scenario != "Baseline"
            and .summary == "Timeout")]
  | length' "$dir/outcomes.json")

# Table of survivors
jq -r '
  .outcomes[]
  | select(.scenario != "Baseline"
           and .summary == "MissedMutant")
  | .scenario.Mutant as $m
  | "| \($m.file) | \($m.span.start.line) | \($m.name) |"
' "$dir/outcomes.json"
```

## Interfaces and dependencies

No Rust code changes. No new Cargo dependencies.

Runtime dependency (installed in CI only): `cargo-mutants`, installed via
`cargo binstall`. No version pin in the initial rollout; a pin should be added
after the first successful run confirms a known-good version (see the risk
mitigation in the Risks section above).

GitHub Actions dependencies (all pre-existing in the repository):

- `actions/checkout@v5`
- `actions/upload-artifact@v4`
- `leynos/shared-actions/.github/actions/setup-rust@aebb3f5b831102e2a10ef909c83d7d50ea86c332`
