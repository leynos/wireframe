# Architectural decision record (ADR) 007: mutation testing with cargo-mutants

## Status

Proposed.

## Date

2026-03-22.

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
purpose-built mutation testing tool for Rust. It generates mutants by
rewriting source files, runs the test suite against each, and produces a
structured JSON report (`mutants.out/outcomes.json`) listing caught, missed
(survived), unviable, and timed-out mutants. It supports workspace filtering,
parallelism, and incremental runs.

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

| Topic                  | cargo-mutants     | Property testing    | Manual audit  |
| ---------------------- | ----------------- | ------------------- | ------------- |
| Systematic detection   | Yes               | Partial             | No            |
| Automation             | Full              | Partial             | None          |
| Maintenance cost       | Low (single tool) | Medium (new tests)  | High          |
| Blocks PR pipeline     | No (scheduled)    | Yes (runs in CI)    | No            |
| Actionable output      | JSON + Markdown   | Test failures only  | Prose         |

_Table 1: Comparison of mutation testing approaches._

## Decision Outcome / Proposed Direction

Introduce `cargo-mutants` as a scheduled GitHub Actions workflow with manual
dispatch support. The workflow:

1. **Trigger:** Runs on a weekly cron schedule against `main`, and is manually
   dispatchable on any branch via `workflow_dispatch` with a configurable
   branch input (defaulting to `main`).
2. **Execution:** Installs `cargo-mutants` via `cargo binstall`, then runs it
   against the workspace with a timeout per mutant to bound execution time.
3. **Artefact:** Uploads the `mutants.out/` directory as a GitHub Actions
   artefact, preserving `outcomes.json` (machine-readable) and the
   human-readable log.
4. **Summary:** Posts a Markdown summary to the GitHub Actions job summary
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
    - cron: "30 4 * * 1" # Weekly, Monday 04:30 UTC
  workflow_dispatch:
    inputs:
      branch:
        description: "Branch to test"
        required: false
        default: "main"

jobs:
  mutants:
    runs-on: ubuntu-latest
    permissions:
      contents: read
    env:
      CARGO_TERM_COLOR: always
    steps:
      - uses: actions/checkout@v5
        with:
          ref: ${{ github.event.inputs.branch || 'main' }}

      - name: Setup Rust
        uses: leynos/shared-actions/.github/actions/setup-rust@aebb3f5b831102e2a10ef909c83d7d50ea86c332

      - name: Install cargo-mutants
        run: cargo binstall --no-confirm cargo-mutants

      - name: Run mutation testing
        run: cargo mutants --timeout-multiplier 3 --output mutants.out

      - name: Upload mutation report
        if: always()
        uses: actions/upload-artifact@v4
        with:
          name: mutation-report
          path: mutants.out/

      - name: Post summary
        if: always()
        run: |
          if [ -f mutants.out/outcomes.json ]; then
            echo "## Mutation testing results" >> "$GITHUB_STEP_SUMMARY"
            echo "" >> "$GITHUB_STEP_SUMMARY"
            caught=$(jq '[.outcomes[] | select(.scenario != "Baseline" and .summary == "CaughtMutant")] | length' mutants.out/outcomes.json)
            missed=$(jq '[.outcomes[] | select(.scenario != "Baseline" and .summary == "MissedMutant")] | length' mutants.out/outcomes.json)
            timeout=$(jq '[.outcomes[] | select(.scenario != "Baseline" and .summary == "Timeout")] | length' mutants.out/outcomes.json)
            echo "- **Caught:** ${caught}" >> "$GITHUB_STEP_SUMMARY"
            echo "- **Missed (survived):** ${missed}" >> "$GITHUB_STEP_SUMMARY"
            echo "- **Timeout:** ${timeout}" >> "$GITHUB_STEP_SUMMARY"
            echo "" >> "$GITHUB_STEP_SUMMARY"
            if [ "$missed" -gt 0 ]; then
              echo "### Surviving mutants" >> "$GITHUB_STEP_SUMMARY"
              echo "" >> "$GITHUB_STEP_SUMMARY"
              echo "| File | Line | Mutation |" >> "$GITHUB_STEP_SUMMARY"
              echo "| ---- | ---- | ------- |" >> "$GITHUB_STEP_SUMMARY"
              jq -r '
                .outcomes[]
                | select(.scenario != "Baseline" and .summary == "MissedMutant")
                | .scenario.Mutant as $m
                | "| \($m.file) | \($m.span.start.line) | \($m.name) |"
              ' mutants.out/outcomes.json >> "$GITHUB_STEP_SUMMARY"
            fi
          fi
```

## Known Risks and Limitations

- **Execution time:** Mutation testing is inherently slow. The weekly schedule
  and per-mutant timeout cap the cost, but runs on large workspaces may still
  take significant wall-clock time. Parallelism (`-j`) can help but increases
  runner resource consumption.
- **False survivors:** Some mutants survive because the mutated behaviour is
  semantically equivalent to the original (e.g. replacing `x + 0` with `x`).
  These require human triage and cannot be eliminated automatically.
- **Runner cost:** Scheduled runs consume GitHub Actions minutes. The weekly
  cadence balances insight against cost; this can be adjusted.
- **Tool stability:** `cargo-mutants` is actively maintained but not yet 1.0.
  Breaking changes to its JSON output format would require updating the
  summary script.

## Outstanding Decisions

- Whether to filter mutations to specific crates or modules in the initial
  rollout, or to run against the full workspace from the start.
- Whether to set a mutation score threshold that, if breached, posts a GitHub
  issue automatically.
- Exact `--timeout-multiplier` value — needs calibration against the current
  test suite runtime.
