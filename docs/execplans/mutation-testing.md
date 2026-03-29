# Introduce mutation testing workflow with cargo-mutants

This ExecPlan (execution plan) is a living document. The sections
`Constraints`, `Tolerances`, `Risks`, `Progress`, `Surprises & Discoveries`,
`Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work
proceeds.

Status: DRAFT

## Purpose / big picture

After this change, the Wireframe project has a GitHub Actions workflow that
runs `cargo-mutants` on a weekly schedule (and on demand for any branch). Each
run produces:

1. A downloadable artefact containing the full `mutants.out/` directory
   (including `outcomes.json`).
2. A Markdown summary posted to the GitHub Actions job summary page listing
   counts (caught, missed, timed out) and a table of every surviving mutant
   with its file, line, and mutation description.

To observe success: trigger the workflow manually via the GitHub Actions UI on
the `main` branch, wait for it to complete, then check the job summary for the
results table and download the artefact.

## Constraints

- The workflow must not block pull requests or pushes. It is triggered only by
  `schedule` and `workflow_dispatch`.
- The workflow file must live at `.github/workflows/mutation-testing.yml`.
- The Rust toolchain setup must use the same shared action as `ci.yml`:
  `leynos/shared-actions/.github/actions/setup-rust@aebb3f5b831102e2a10ef909c83d7d50ea86c332`.
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

(None yet.)

## Decision log

(None yet.)

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
- `.github/workflows/advanced-tests.yml` — Scheduled and manually
  dispatchable workflow running Loom-based concurrency tests. Good structural
  reference for the new workflow.
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

- **Triggers:** `schedule` (cron `30 4 * * 1`, Monday 04:30 UTC) and
  `workflow_dispatch` with an optional `branch` input defaulting to `main`.
- **Job:** `mutants`, running on `ubuntu-latest`, with `contents: read`
  permissions.
- **Steps:**
  1. Checkout the repository at the selected branch.
  2. Setup Rust using the shared action (same pin as `ci.yml`).
  3. Install `cargo-mutants` via `cargo binstall --no-confirm cargo-mutants`.
  4. Run `cargo mutants --timeout-multiplier 3 --output mutants.out`.
  5. Upload `mutants.out/` as an artefact (with `if: always()`).
  6. Post a Markdown summary to `$GITHUB_STEP_SUMMARY` using corrected jq
     filters (with `if: always()`).

The summary step extracts caught, missed, and timeout counts from
`.outcomes[]`, then renders surviving mutants as a table.

### Stage C: Update the ADR

Update the jq filters in the workflow sketch in
`docs/adr-007-mutation-testing-with-cargo-mutants.md` to match the corrected
field names. This keeps the ADR accurate as a reference. Also update the ADR
status from `Proposed` to `Accepted` with today's date and a brief summary.

### Stage D: Validation and commit

1. Run `make markdownlint` — expect 0 errors.
2. Verify the YAML is syntactically valid by parsing it with a YAML tool or
   checking that `yq` / `python -c 'import yaml; ...'` succeeds.
3. Commit the new workflow file and the updated ADR as a single atomic commit.

## Concrete steps

All commands run from the repository root.

**Stage B — create workflow file:**

Write `.github/workflows/mutation-testing.yml` with the content described
above. No commands to run yet; this is a file creation step.

**Stage C — update ADR:**

Edit `docs/adr-007-mutation-testing-with-cargo-mutants.md`:

- Replace all `.[]` with `.outcomes[]` in the jq filters.
- Replace `.scenario == "Mutant"` with `.scenario != "Baseline"` (or an
  object-type check).
- Replace `.mutant.source_file` with `.scenario.Mutant.file`.
- Replace `.mutant.line` with `.scenario.Mutant.span.start.line`.
- Replace `.mutant.description` with `.scenario.Mutant.name`.
- Change status to `Accepted (2026-03-22)`.

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
- The workflow file contains both `schedule` and `workflow_dispatch` triggers.
- The jq filters in both the workflow and the ADR use the corrected field
  names (`.outcomes[]`, `.scenario.Mutant.file`, etc.).
- The ADR status is updated to `Accepted`.

**Quality method (how to check):**

- `make markdownlint` — automated.
- YAML syntax check — `python3 -c "import yaml; ..."` or equivalent.
- Manual review of the jq filters against the field name reference in the
  Context section above.

**Observable behaviour after merge:**

- Navigate to Actions → "Mutation testing" in the GitHub UI.
- Click "Run workflow", select a branch (or leave as `main`), and trigger.
- After completion, the job summary shows caught/missed/timeout counts and a
  table of surviving mutants.
- The `mutation-report` artefact is available for download.

## Idempotence and recovery

All steps are idempotent. The workflow file can be overwritten and
re-committed. The ADR edits are textual replacements that produce the same
result if applied more than once. `make markdownlint` is read-only and safe to
re-run.

## Artefacts and notes

The corrected jq snippet for the summary step (for reference during
implementation):

```sh
# Count outcomes
caught=$(jq '[.outcomes[] | select(.scenario != "Baseline" and .summary == "CaughtMutant")] | length' mutants.out/outcomes.json)
missed=$(jq '[.outcomes[] | select(.scenario != "Baseline" and .summary == "MissedMutant")] | length' mutants.out/outcomes.json)
timeout=$(jq '[.outcomes[] | select(.scenario != "Baseline" and .summary == "Timeout")] | length' mutants.out/outcomes.json)

# Table of survivors
jq -r '
  .outcomes[]
  | select(.scenario != "Baseline" and .summary == "MissedMutant")
  | .scenario.Mutant as $m
  | "| \($m.file) | \($m.span.start.line) | \($m.name) |"
' mutants.out/outcomes.json
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
