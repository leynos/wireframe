# Introduce mutation testing workflow with cargo-mutants

This ExecPlan (execution plan) is a living document. The sections
`Constraints`, `Tolerances`, `Risks`, `Progress`, `Surprises & Discoveries`,
`Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work
proceeds.

Status: DRAFT

## Purpose / big picture

After this change, the Wireframe project has a GitHub Actions workflow that
runs `cargo-mutants` on a daily schedule (and on demand for any branch). The
workflow uses a change-detection guard so that the expensive mutation step is a
cheap no-op when no relevant Rust source files changed on `main` in the
preceding 24 hours. When changes are detected, mutations are scoped to only the
changed files using repeated `--file` arguments. The root crate and
`wireframe_testing` (which is not a workspace member) are handled as separate
targeted invocations. Each run produces:

1. Downloadable artefacts containing the `mutants.out/` directory (and
   `mutants-wt.out/` when `wireframe_testing` files changed), including
   `outcomes.json`.
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
  reflog-based `@{24.hours.ago}`) because fresh CI clones lack reflog state.
- The detection step must monitor: `src/**/*.rs`,
  `wireframe_testing/src/**/*.rs`, `examples/**/*.rs`, `benches/**/*.rs`.
  Manifest-only changes (`Cargo.toml`, `Cargo.lock`) are excluded.
- `wireframe_testing` is not a workspace member, so it requires a separate
  `cargo mutants --dir wireframe_testing` invocation.
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
  Likelihood: low Mitigation: Pin the `cargo-mutants` version in the workflow.
  Add a comment noting the version the jq filters were validated against.

- Risk: The `outcomes.json` field names discovered via source reading may not
  perfectly match runtime output (custom `Serialize` impls can diverge from
  field names). Severity: medium Likelihood: low Mitigation: Note the field
  names in a comment in the workflow. On first real run, verify the jq output
  and correct if needed.

## Progress

- [ ] Stage A: Research and correct ADR sketch.
- [ ] Stage B: Create the workflow file.
- [ ] Stage C: Update the ADR to reflect corrections.
- [ ] Stage D: Validation and commit.

## Surprises & discoveries

- `origin/main@{24.hours.ago}` relies on reflog state, which is not available
  in fresh CI clones. The safer alternative is `git log --since="24 hours ago"`
  using commit timestamps.
- `wireframe_testing` is not a workspace member, so
  `cargo mutants --workspace` from the repo root does not cover it. A second
  invocation with `--dir wireframe_testing` is required.
- `cargo-mutants` supports repeated `--file` arguments, allowing mutation runs
  to be scoped to specific changed files rather than full-crate sweeps.
- There is no `build.rs` in this repository, so it does not need to be
  included in the change-detection patterns.

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

## Plan of work

### Stage A: Research and correct ADR sketch

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

No code changes in this stage — just confirming the corrections above.

### Stage B: Create the workflow file

Create `.github/workflows/mutation-testing.yml` with the following structure:

- **Triggers:** `schedule` (cron `30 4 * * *`, daily 04:30 UTC) and
  `workflow_dispatch` with an optional `branch` input defaulting to `main`.
- **Job:** `mutants`, running on `ubuntu-latest`, with `contents: read`
  permissions.
- **Steps:**
  1. Checkout the repository at the selected branch with full history
     (`fetch-depth: 0`).
  2. Detect changed Rust files on `origin/main` in the last 24 hours using
     `git log --since="24 hours ago"` (commit timestamps, not reflog). Sort
     changes into root-crate files and `wireframe_testing` files. Set step
     outputs: `has_changes`, `root_files`, `wt_files`, `dispatch`. For
     `workflow_dispatch` runs, bypass the guard by setting `has_changes=true`
     and `dispatch=true` unconditionally (runs full, unscoped mutations).
  3. If no relevant changes on a scheduled run, write a skip message to
     `$GITHUB_STEP_SUMMARY` and skip all subsequent heavy steps.
  4. Setup Rust using the shared action (gated on `has_changes`).
  5. Install `cargo-mutants` via `cargo binstall --no-confirm cargo-mutants`
     (gated on `has_changes`).
  6. Run `cargo mutants` for root crate with `--file` args for changed files
     (gated on `root_files` being non-empty or `dispatch` being true).
  7. Run `cargo mutants --dir wireframe_testing` with `--file` args for
     changed `wireframe_testing` files (gated on `wt_files` being non-empty
     or `dispatch` being true).
  8. Upload separate artefacts for root and `wireframe_testing` reports.
  9. Post a Markdown summary to `$GITHUB_STEP_SUMMARY` using corrected jq
     filters (with `if: always()`), calling a `post_results` shell function
     for each report directory.

### Stage C: Update the ADR

Update the ADR at `docs/adr-007-mutation-testing-with-cargo-mutants.md` to
reflect:

- Daily schedule with change-detection guard.
- Revised workflow sketch with `fetch-depth: 0`, `detect` step, gated
  steps, split root/`wireframe_testing` invocations.
- Updated risk and limitation sections for the new approach.
- Resolved decisions section documenting the design choices.
- Corrected jq field names (`.outcomes[]`, `.scenario.Mutant.file`, etc.).

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

**Stage C — update ADR:**

Edit `docs/adr-007-mutation-testing-with-cargo-mutants.md`:

- Replace all `.[]` with `.outcomes[]` in the jq filters.
- Replace `.scenario == "Mutant"` with `.scenario != "Baseline"` (or an
  object-type check).
- Replace `.mutant.source_file` with `.scenario.Mutant.file`.
- Replace `.mutant.line` with `.scenario.Mutant.span.start.line`.
- Replace `.mutant.description` with `.scenario.Mutant.name`.
- Change schedule from weekly to daily with change-detection guard.
- Add workspace split handling (root crate vs `wireframe_testing`).
- Add "Resolved Decisions" section documenting design choices.
- Update risks and limitations for the revised approach.

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
- The workflow checks out with `fetch-depth: 0` and has a `detect` step that
  uses `git log --since="24 hours ago"` (not reflog).
- Heavy steps (`Setup Rust`, `Install cargo-mutants`, `Run mutation testing`)
  are gated on `steps.detect.outputs.has_changes == 'true'`.
- `workflow_dispatch` runs bypass the change-detection guard by setting
  `has_changes=true` unconditionally and running full (unscoped) mutations.
- Root crate and `wireframe_testing` mutation runs are separate steps with
  appropriate `--file` and `--dir` arguments.
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
changed=$(git log --since="24 hours ago" --name-only \
  --format="" origin/main -- '*.rs' | sort -u)
```

The corrected jq snippet for the summary step (for reference during
implementation):

```sh
# Count outcomes (parameterised by $dir)
caught=$(jq '[.outcomes[] | select(.scenario != "Baseline" and .summary == "CaughtMutant")] | length' "$dir/outcomes.json")
missed=$(jq '[.outcomes[] | select(.scenario != "Baseline" and .summary == "MissedMutant")] | length' "$dir/outcomes.json")
timeout=$(jq '[.outcomes[] | select(.scenario != "Baseline" and .summary == "Timeout")] | length' "$dir/outcomes.json")

# Table of survivors
jq -r '
  .outcomes[]
  | select(.scenario != "Baseline" and .summary == "MissedMutant")
  | .scenario.Mutant as $m
  | "| \($m.file) | \($m.span.start.line) | \($m.name) |"
' "$dir/outcomes.json"
```

## Interfaces and dependencies

No Rust code changes. No new Cargo dependencies.

Runtime dependency (installed in CI only): `cargo-mutants`, installed via
`cargo binstall`. No version pin in the initial rollout (per the ADR's
outstanding decisions); a pin can be added after the first successful run
confirms a known-good version.

GitHub Actions dependencies (all pre-existing in the repository):

- `actions/checkout@v5`
- `actions/upload-artifact@v4`
- `leynos/shared-actions/.github/actions/setup-rust@aebb3f5b831102e2a10ef909c83d7d50ea86c332`
