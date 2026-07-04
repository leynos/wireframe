# Architectural decision record (ADR) 007: mutation testing with cargo-mutants

## Status

Proposed.

## Date

2026-03-31.

## Context and Problem Statement

Wireframe maintains line and branch coverage via `cargo-llvm-cov` in CI
(ADR-006, `ci.yml`), and uploads results to CodeScene. Coverage percentage,
however, only measures which lines execute — not whether the test suite
actually detects faults in the exercised code. A function can be fully
"covered" yet have no assertion that would fail if its logic were altered.

Mutation testing addresses this gap by systematically introducing small
semantic changes (mutants) into the source and verifying that the test suite
catches each one. Surviving mutants — mutants that do not cause any test to
fail — highlight areas where test assertions are weak or missing.

The project needs a lightweight, low-friction mechanism for running mutation
testing periodically, recording surviving mutants, and feeding those results
back into test-improvement work.

## Decision Drivers

- Detect weak assertions that line coverage alone cannot reveal.
- Keep the feedback loop actionable: surviving mutants should be easy to
  triage and convert into concrete test-improvement tasks.
- Avoid blocking the existing CI pipeline — mutation runs are slow and should
  not gate pull requests.
- Allow manual runs on arbitrary branches for ad hoc validation.
- Minimize maintenance burden: prefer a single-binary tool that understands
  Cargo workspaces natively.

## Options Considered

### Option A: cargo-mutants

[`cargo-mutants`](https://github.com/sourcefrog/cargo-mutants) is a
purpose-built mutation testing tool for Rust. It generates mutants by rewriting
source files, runs the test suite against each, and produces a structured JSON
report (`mutants.out/outcomes.json`) listing caught, missed (survived),
unviable, and timed-out mutants. It supports workspace filtering, parallelism,
and incremental runs.

- Single binary, installable via `cargo install` or `cargo binstall`.
- JSON output is straightforward to archive and post-process.
- Actively maintained with Rust edition 2024 support.

### Option B: custom property-based testing expansion

Expand the existing `rstest` / `proptest` suite to cover more boundary
conditions. This improves test quality but does not systematically identify
which existing code paths lack meaningful assertions.

### Option C: manual audit

Periodically audit coverage reports by hand. This is labour-intensive,
error-prone, and does not scale.

| Topic                | cargo-mutants     | Property testing   | Manual audit |
| -------------------- | ----------------- | ------------------ | ------------ |
| Systematic detection | Yes               | Partial            | No           |
| Automation           | Full              | Partial            | None         |
| Maintenance cost     | Low (single tool) | Medium (new tests) | High         |
| Blocks PR pipeline   | No (scheduled)    | Yes (runs in CI)   | No           |
| Actionable output    | JSON + Markdown   | Test failures only | Prose        |

_Table 1: Comparison of mutation testing approaches._

## Decision Outcome / Proposed Direction

Introduce `cargo-mutants` as a scheduled GitHub Actions workflow with manual
dispatch support. The workflow runs daily but uses a change-detection guard so
that the expensive mutation step is a cheap no-op when no relevant Rust source
files changed on `main` in the preceding 24 hours.

### Schedule and change detection

1. **Trigger:** Runs on a daily cron schedule against `main`, and is manually
   dispatchable on any branch via `workflow_dispatch` with a configurable
   branch input (defaulting to `main`).
2. **Change detection:** After checkout (with full history), compute the set of
   Rust source files changed on `origin/main` in the last 25 hours using commit
   timestamps:

   ```sh
   git log --since="25 hours ago" --name-only --format="" \
     origin/main -- '*.rs'
   ```

   This is preferred over `origin/main@{25.hours.ago}` because that syntax
   relies on reflog state, which is not available in fresh CI clones. The
   window is 25 hours rather than 24 because GitHub cron start times drift
   (runs routinely begin 15–60 minutes late); the extra hour means a commit
   landing just after one run still falls inside the next run's window, at
   the cost of occasionally re-testing a file. Files that no longer exist at
   the checked-out tip (deleted or renamed within the window) are filtered
   out before being passed to `--file`. The result is stored as a step
   output (`has_changes=true|false`) and the heavy mutation step is gated
   with
   `if: steps.detect.outputs.has_changes == 'true'`. Manual `workflow_dispatch`
   runs bypass the guard by setting `has_changes=true` unconditionally and
   running a full (unscoped) mutation against both the root crate and
   `wireframe_testing`.
3. **Skip summary:** When no relevant changes are found on a scheduled run, the
   workflow writes a short skip message to `$GITHUB_STEP_SUMMARY` and exits
   cleanly.

### What counts as "code changes"

The detection step monitors the following path patterns:

- `src/**/*.rs` — root crate source.
- `wireframe_testing/src/**/*.rs` — companion testing crate source.
- `examples/**/*.rs` — example programs.
- `benches/**/*.rs` — benchmarks.

Manifest-only changes (`Cargo.toml`, `Cargo.lock`) are excluded.
`cargo-mutants` filters Rust source files, so manifest-only changes do not
produce meaningful mutations. If a manifest change alters build behaviour in a
way that affects mutation outcomes, a manual `workflow_dispatch` run can be
triggered.

### Workspace split handling

`wireframe_testing` is not a workspace member, so a root-level
`cargo mutants --workspace` would not cover it. The workflow handles this by
running up to two targeted invocations:

- **Root crate:** When changed files fall under `src/`, `examples/`, or
  `benches/`, run `cargo mutants` from the repository root with repeated
  `--file` arguments scoping the run to the specific changed files.
- **`wireframe_testing`:** When changed files fall under
  `wireframe_testing/src/`, run a second invocation via
  `cargo mutants --dir wireframe_testing` with `--file` arguments for the
  changed files within that crate.

### Execution, artefacts, and summary

1. **Execution:** Installs `cargo-mutants` via `cargo binstall` (the shared
   `setup-rust` action provides `cargo-binstall`), then runs the scoped
   invocations described above with `--in-place` (the upstream CI
   recommendation, which reuses the existing build cache instead of copying
   the tree) and a per-mutant timeout multiplier to bound execution time. The
   job carries an explicit `timeout-minutes` ceiling as a backstop against
   runaway full runs.
2. **Output locations:** `cargo-mutants` creates `mutants.out/` inside the
   source directory it operates on. Note that `--output DIR` places
   `mutants.out/` _within_ `DIR` (it does not rename the directory), so the
   workflow relies on the defaults instead: the root run writes
   `./mutants.out/` and the `wireframe_testing` run writes
   `wireframe_testing/mutants.out/`.
3. **Exit-code handling:** `cargo mutants` exits non-zero for informative
   outcomes as well as genuine failures: `0` all caught, `1` usage error, `2`
   mutants missed, `3` tests timed out, `4` baseline tests already failing,
   `70` internal error. Because this workflow is informational, exit codes
   `2` and `3` are treated as success (the survivors are the deliverable, not
   a failure), while `1`, `4`, and `70` still fail the job so genuine faults
   remain visible.
4. **Artefact:** Uploads the `mutants.out/` directory (or directories) as
   GitHub Actions artefacts, preserving `outcomes.json` (machine-readable) and
   the human-readable log. When both root and `wireframe_testing` runs occur,
   separate artefacts are uploaded.
5. **Summary:** Posts a Markdown summary to the GitHub Actions job summary
   (`$GITHUB_STEP_SUMMARY`) listing surviving mutants with file, line, and
   mutation description — directly usable as a test-improvement backlog.

The workflow does not gate pull requests or pushes. It is purely informational.
Surviving mutants are triaged manually and converted into test-improvement
tasks on the roadmap as warranted.

### Workflow sketch

```yaml
name: Mutation testing

on:
  schedule:
    - cron: "30 4 * * *" # Daily, 04:30 UTC
  workflow_dispatch:
    inputs:
      branch:
        description: "Branch to test"
        required: false
        default: "main"

jobs:
  mutants:
    runs-on: ubuntu-latest
    timeout-minutes: 300
    permissions:
      contents: read
    env:
      CARGO_TERM_COLOR: always
    steps:
      - uses: actions/checkout@v5
        with:
          ref: ${{ github.event.inputs.branch || 'main' }}
          fetch-depth: 0
          persist-credentials: false

      - name: Detect changed Rust files
        id: detect
        run: |
          # Manual dispatch bypasses change detection — always run.
          if [ "${{ github.event_name }}" = "workflow_dispatch" ]; then
            echo "has_changes=true" >> "$GITHUB_OUTPUT"
            echo "root_files=" >> "$GITHUB_OUTPUT"
            echo "wt_files=" >> "$GITHUB_OUTPUT"
            echo "dispatch=true" >> "$GITHUB_OUTPUT"
            exit 0
          fi

          # Use commit timestamps (not reflog) — safe in fresh CI clones.
          # The 25-hour window absorbs GitHub cron start-time drift.
          # git log already filters to *.rs; case below matches prefixes
          # including nested directories (e.g. src/foo/bar.rs). Files
          # deleted or renamed since the change are skipped.
          root_files=""
          wt_files=""
          while IFS= read -r f; do
            [ -f "$f" ] || continue
            case "$f" in
              src/*|examples/*|benches/*)
                root_files="${root_files:+$root_files }$f" ;;
              wireframe_testing/src/*)
                wt_files="${wt_files:+$wt_files }$f" ;;
            esac
          done < <(git log --since="25 hours ago" --name-only \
            --format="" origin/main -- '*.rs' | sort -u)

          if [ -z "$root_files" ] && [ -z "$wt_files" ]; then
            echo "has_changes=false" >> "$GITHUB_OUTPUT"
            echo "## Mutation testing skipped" >> "$GITHUB_STEP_SUMMARY"
            echo "" >> "$GITHUB_STEP_SUMMARY"
            echo "No Rust source changes on \`main\` in the last" \
              "24 hours." >> "$GITHUB_STEP_SUMMARY"
          else
            echo "has_changes=true" >> "$GITHUB_OUTPUT"
            echo "root_files=$root_files" >> "$GITHUB_OUTPUT"
            echo "wt_files=$wt_files" >> "$GITHUB_OUTPUT"
          fi

      - name: Setup Rust
        if: steps.detect.outputs.has_changes == 'true'
        uses: leynos/shared-actions/.github/actions/setup-rust@aebb3f5b831102e2a10ef909c83d7d50ea86c332

      - name: Install cargo-mutants
        if: steps.detect.outputs.has_changes == 'true'
        run: cargo binstall --no-confirm cargo-mutants

      - name: Run mutation testing (root crate)
        if: >-
          steps.detect.outputs.has_changes == 'true'
          && (steps.detect.outputs.root_files != ''
              || steps.detect.outputs.dispatch == 'true')
        run: |
          args=""
          for f in ${{ steps.detect.outputs.root_files }}; do
            args="$args --file $f"
          done
          # Exit 2 (missed mutants) and 3 (timeouts) are informative
          # outcomes, not failures; 1, 4, and 70 are genuine faults.
          set +e
          # shellcheck disable=SC2086
          cargo mutants --in-place --timeout-multiplier 3 $args
          status=$?
          set -e
          case "$status" in
            0|2|3) exit 0 ;;
            *) exit "$status" ;;
          esac

      - name: Run mutation testing (wireframe_testing)
        if: >-
          steps.detect.outputs.has_changes == 'true'
          && (steps.detect.outputs.wt_files != ''
              || steps.detect.outputs.dispatch == 'true')
        run: |
          args=""
          for f in ${{ steps.detect.outputs.wt_files }}; do
            # Strip the wireframe_testing/ prefix for --file.
            args="$args --file ${f#wireframe_testing/}"
          done
          set +e
          # shellcheck disable=SC2086
          cargo mutants --in-place --timeout-multiplier 3 \
            --dir wireframe_testing $args
          status=$?
          set -e
          case "$status" in
            0|2|3) exit 0 ;;
            *) exit "$status" ;;
          esac

      - name: Upload mutation report (root)
        if: >-
          always()
          && (steps.detect.outputs.root_files != ''
              || steps.detect.outputs.dispatch == 'true')
        uses: actions/upload-artifact@v4
        with:
          name: mutation-report-root
          path: mutants.out/

      - name: Upload mutation report (wireframe_testing)
        if: >-
          always()
          && (steps.detect.outputs.wt_files != ''
              || steps.detect.outputs.dispatch == 'true')
        uses: actions/upload-artifact@v4
        with:
          name: mutation-report-wireframe-testing
          path: wireframe_testing/mutants.out/

      - name: Post summary
        if: >-
          always()
          && steps.detect.outputs.has_changes == 'true'
        run: |
          post_results() {
            local label="$1" dir="$2"
            if [ ! -f "$dir/outcomes.json" ]; then return; fi
            echo "## Mutation testing results ($label)" \
              >> "$GITHUB_STEP_SUMMARY"
            echo "" >> "$GITHUB_STEP_SUMMARY"
            caught=$(jq '[.outcomes[]
              | select(.scenario != "Baseline"
                       and .summary == "CaughtMutant")]
              | length' "$dir/outcomes.json")
            missed=$(jq '[.outcomes[]
              | select(.scenario != "Baseline"
                       and .summary == "MissedMutant")]
              | length' "$dir/outcomes.json")
            timeout=$(jq '[.outcomes[]
              | select(.scenario != "Baseline"
                       and .summary == "Timeout")]
              | length' "$dir/outcomes.json")
            echo "- **Caught:** ${caught}" >> "$GITHUB_STEP_SUMMARY"
            echo "- **Missed (survived):** ${missed}" \
              >> "$GITHUB_STEP_SUMMARY"
            echo "- **Timeout:** ${timeout}" >> "$GITHUB_STEP_SUMMARY"
            echo "" >> "$GITHUB_STEP_SUMMARY"
            if [ "$missed" -gt 0 ]; then
              echo "### Surviving mutants" >> "$GITHUB_STEP_SUMMARY"
              echo "" >> "$GITHUB_STEP_SUMMARY"
              echo "| File | Line | Mutation |" \
                >> "$GITHUB_STEP_SUMMARY"
              echo "| ---- | ---- | ------- |" \
                >> "$GITHUB_STEP_SUMMARY"
              jq -r '
                .outcomes[]
                | select(.scenario != "Baseline"
                         and .summary == "MissedMutant")
                | .scenario.Mutant as $m
                | "| \($m.file) | \($m.span.start.line) | \($m.name) |"
              ' "$dir/outcomes.json" >> "$GITHUB_STEP_SUMMARY"
            fi
          }

          post_results "root crate" "mutants.out"
          post_results "wireframe_testing" "wireframe_testing/mutants.out"
```

## Known Risks and Limitations

- **Execution time:** Mutation testing is inherently slow. The daily schedule
  with change-detection gating and per-file `--file` scoping cap the cost to
  only recently changed code, but runs touching many files may still take
  significant wall-clock time. Parallelism (`-j`) can help but increases runner
  resource consumption.
- **False survivors:** Some mutants survive because the mutated behaviour is
  semantically equivalent to the original (e.g. replacing `x + 0` with `x`).
  These require human triage and cannot be eliminated automatically.
- **`wireframe_testing` false survivors:** The companion crate's logic is
  exercised chiefly by the root crate's test suite; `wireframe_testing`
  itself carries only a small number of in-crate tests. Because
  `cargo mutants --dir wireframe_testing` runs only that package's own
  tests, most mutants there will appear to survive even when the root
  crate's tests would catch the fault. The `wireframe_testing` results
  table is therefore advisory until the crate gains meaningful in-crate
  coverage; treat its survivors with scepticism during triage.
- **Runner cost:** Scheduled runs consume GitHub Actions minutes. The daily
  schedule starts every day, but the change-detection guard ensures the
  expensive mutation step only runs when `main` received relevant Rust source
  changes in the preceding 24 hours. On quiet days the workflow exits in
  seconds.
- **Change detection edge cases:** The `git log --since` guard uses commit
  timestamps, not author timestamps. Force-pushed or rebased commits may carry
  original timestamps outside the 25-hour window, and merge commits whose
  constituent commits were created earlier are similarly invisible (the
  repository's squash-merge convention makes this unlikely in practice).
  There is also no catch-up: if a scheduled run fails or is skipped, commits
  from that window are never mutation-tested. A manual `workflow_dispatch`
  run bypasses the guard entirely, providing the fallback in all these
  cases. Diffing against the SHA of the last successful run (via the GitHub
  API) would close the gap completely but was rejected as disproportionate
  for an informational workflow.
- **Scheduled-workflow suspension:** GitHub automatically disables cron
  triggers on repositories with no activity for 60 days. On a quiet
  repository the workflow silently stops running until re-enabled — an
  acceptable failure mode, since no code changes means nothing new to
  mutate.
- **Manifest-only changes:** Changes to `Cargo.toml` or `Cargo.lock` without
  accompanying Rust source changes are not detected. If a manifest change
  affects mutation outcomes (e.g. enabling a feature that changes conditional
  compilation), a manual run is required.
- **Tool stability:** `cargo-mutants` is actively maintained but not yet 1.0.
  Breaking changes to its JSON output format would require updating the summary
  script.

## Outstanding Decisions

- Whether to set a mutation score threshold that, if breached, posts a GitHub
  issue automatically.
- Exact `--timeout-multiplier` value — needs calibration against the current
  test suite runtime.
- Whether to adopt `--in-diff` (line-level scoping from a diff) in place of
  file-level `--file` scoping for scheduled runs, once the initial workflow
  has proven itself. `--in-diff` tests only mutants on changed lines, which
  is cheaper but misses mutants elsewhere in a changed file.
- Whether full `workflow_dispatch` runs need `--shard` support to split the
  work across parallel jobs, should single-job runs approach the
  `timeout-minutes` ceiling.
- Whether to keep the `wireframe_testing` invocation given its false-survivor
  noise (see Known Risks), or to drop it until the crate has meaningful
  in-crate test coverage.

## Resolved Decisions

- **Scope filtering:** Mutations are scoped to files changed in the preceding
  24 hours using repeated `--file` arguments rather than running against the
  full workspace. This was chosen to keep daily runs fast and focused.
- **Workspace split:** `wireframe_testing` is not a workspace member, so it
  requires a separate `cargo mutants --dir wireframe_testing` invocation.
  Changed files under `wireframe_testing/src/` trigger this second run
  automatically.
- **Schedule cadence:** Changed from weekly to daily. The change-detection
  guard ensures that the expensive mutation step only runs when relevant source
  changes landed on `main`, making quiet days a cheap no-op.
- **Change detection mechanism:** Uses `git log --since="25 hours ago"` with
  commit timestamps rather than `origin/main@{25.hours.ago}` (which relies on
  reflog state unavailable in fresh CI clones). The window exceeds the
  24-hour cadence by one hour to absorb GitHub cron start-time drift;
  occasional double-testing of a file is preferred over silently dropping a
  commit. Files no longer present at the checked-out tip are filtered out
  before scoping.
- **Exit-code policy:** `cargo mutants` exit codes `2` (missed mutants) and
  `3` (timeouts) are treated as success because the workflow is
  informational — survivors are its output, not a failure. Exit codes `1`
  (usage error), `4` (baseline tests failing), and `70` (internal error)
  still fail the job so genuine faults surface as red runs.
- **Output directories:** The workflow relies on the default output
  locations (`./mutants.out/` for the root crate,
  `wireframe_testing/mutants.out/` for the companion crate) because
  `--output DIR` creates `mutants.out/` _within_ `DIR` rather than renaming
  it, which would otherwise nest the report one level deeper than the
  summary and artefact steps expect.
- **Build strategy:** Runs use `--in-place`, the upstream recommendation for
  CI, so mutation builds reuse the runner's existing build cache instead of
  copying the source tree. The job also sets `timeout-minutes: 300` as a
  hard backstop against unbounded full runs.
- **Code change definition:** The detection monitors `src/**/*.rs`,
  `wireframe_testing/src/**/*.rs`, `examples/**/*.rs`, and `benches/**/*.rs`.
  Manifest-only changes (`Cargo.toml`, `Cargo.lock`) are excluded because
  `cargo-mutants` only mutates Rust source files.
- **Manual dispatch bypass:** `workflow_dispatch` runs skip the
  change-detection guard entirely, providing a fallback for manifest changes or
  ad hoc validation.
