# Enable and resolve doctests

This ExecPlan is a living document. The sections `Constraints`, `Tolerances`,
`Risks`, `Progress`, `Surprises & Discoveries`, `Decision Log`, and
`Outcomes & Retrospective` must be kept up to date as work proceeds.

Status: COMPLETE

`PLANS.md` is not present in this repository at the time this plan was drafted.

## Purpose / big picture

Wireframe currently sets `doctest = false` in `Cargo.toml`, which means many
Rustdoc examples are never compile-validated. This plan enables doctests and
brings all public API examples to a compile-clean state. Success is observable
when `cargo test --doc` passes, doctests are scoped to public API
documentation, and the repository tracks a measurable runnable-vs-`no_run`
benchmark.

## Constraints

- Doctests must be enabled for the library crate in `Cargo.toml`.
- All doctest code blocks must compile under supported feature combinations.
- Doctest examples must not be attached to private items or test-only helper
  modules.
- Doctest examples must not be attached to `#[cfg(test)]` test functions.
- Behavioural integration tests and unit tests remain separate from doctests;
  doctests document public API usage, not internal test scaffolding.
- No new runtime-only external services (network/database) are required for
  doctest execution.

## Tolerances (exception triggers)

- Scope: if enabling doctests requires changes in more than 40 files, stop and
  escalate.
- Interface: if this work requires public API signature changes, stop and
  escalate.
- Dependencies: if a new dependency is required only to make doctests pass,
  stop and escalate.
- Iterations: if `cargo test --doc --all-features` fails after 3 full fix
  loops, stop and escalate with failure clusters.
- Ambiguity: if runnable-vs-`no_run` policy cannot satisfy stakeholder intent,
  stop and escalate with options.

## Risks

- Risk: examples that open sockets or spawn long-running tasks may hang under
  doctest execution. Severity: medium Likelihood: high Mitigation: convert
  those blocks to `no_run` while preserving compile checks.

- Risk: examples in private modules may accidentally become doctests once
  doctests are enabled. Severity: medium Likelihood: medium Mitigation: audit
  private/test modules and strip runnable example blocks from non-public item
  docs.

- Risk: feature-gated API examples may fail when doctests run under default
  features. Severity: medium Likelihood: medium Mitigation: annotate examples
  with explicit feature guards and validate under both default and
  `--all-features` runs.

## Progress

- [x] (2026-02-18) Drafted ExecPlan with success criteria and milestones.
- [x] (2026-02-18) Inventoried Rustdoc code fences in `src/` and classified
      by runnable class:
      runnable 128, `no_run` 51, `ignore`/`compile_fail` 6, non-Rust/text 7.
- [x] (2026-02-18) Enabled doctests in `Cargo.toml` and added
      `make test-doc`.
- [x] (2026-02-18) Resolved doctest compile failures across builder, client,
      extractor, response, server, and runtime API docs.
- [x] (2026-02-18) Enforced scope rules by rewriting private/test-helper
      examples (`src/server/test_util.rs`, `src/connection/state.rs`,
      `src/connection/frame.rs`, `src/server/runtime/accept.rs`) to
      non-doctest `text` fences.
- [x] (2026-02-18) Added `scripts/doctest-benchmark.sh` and
      `make doctest-benchmark`; benchmark now passes thresholds.
- [x] (2026-02-18) Ran full Rust and Markdown quality gates and updated
      user-facing doctest guidance.

## Surprises & Discoveries

- Observation: Enabling doctest execution surfaced 27 failing examples in the
  first pass, mostly type inference in generic builder docs plus a few stale
  API snippets. Evidence: `/tmp/doc-pre.log`. Impact: required broad
  documentation repairs before policy work.

- Observation: Several server binding examples panic when executed because they
  require a Tokio runtime context. Evidence: failures in
  `src/server/config/binding.rs` doctests. Impact: those examples were
  correctly reclassified as `no_run`.

- Observation: Initial benchmark result failed policy at 64% runnable / 35%
  `no_run`. Evidence: early output from `scripts/doctest-benchmark.sh`. Impact:
  converted deterministic state/extractor examples from `no_run` to runnable to
  restore the target balance.

## Decision Log

- Decision: Use an explicit benchmark of at least 70% runnable doctests and no
  more than 30% `no_run` doctests. Rationale: This preserves high executable
  documentation coverage while keeping side-effect-heavy examples
  compile-checked only. The wording request about `no_run` percentage is
  ambiguous; this interpretation optimizes usefulness. Date/Author: 2026-02-18
  / Codex.

- Decision: Treat doctest ownership as public API documentation only.
  Rationale: This prevents mixing testing concerns with user-facing docs and
  aligns with Rustdoc guidance. Date/Author: 2026-02-18 / Codex.

- Decision: Use explicit closure return types (`|| -> WireframeApp { ... }`) in
  server docs to stabilize type inference for generic builders. Rationale: this
  resolved repeated `E0283` inference failures without changing public
  signatures. Date/Author: 2026-02-18 / Codex.

- Decision: Promote deterministic, side-effect-free examples from `no_run` to
  runnable in `src/session.rs` and `src/extractor/request.rs`. Rationale:
  improves executable documentation coverage and satisfies the runnable/no_run
  benchmark policy while preserving reliability. Date/Author: 2026-02-18 /
  Codex.

## Outcomes & Retrospective

- Doctests enabled (`Cargo.toml` now sets `doctest = true`).
- Repeatable doctest workflow added:
  `make test-doc` and `make doctest-benchmark`.
- Doctest acceptance target met:
  `cargo test --doc --all-features` passes (`179 passed`, `0 failed`,
  `4 ignored` + `2 compile_fail` checks).
- Runnable-vs-`no_run` benchmark met:
  runnable 128 (71%), `no_run` 51 (28%), ignored/compile_fail 6, non-rust/text
  7.
- Scope policy enforced for private/test-only helpers by converting their code
  blocks to non-doctest fences.
- Quality gates all pass:
  `make fmt`, `make check-fmt`, `make lint`, `make test`, `make test-doc`,
  `make doctest-benchmark`, `make markdownlint`, `make nixie`.

## Context and orientation

Current state after implementation:

- `Cargo.toml` enables doctests with `doctest = true` in `[lib]`.
- Public docs remain spread across module-level and item-level docs in `src/`,
  with doctest ownership scoped to the public API.
- The repository still keeps integration and behavioural tests in `tests/`;
  these remain distinct from doctests.
- `docs/rust-doctest-dry-guide.md` now includes Wireframe-specific policy and
  benchmark commands.

Key files expected to change:

- `Cargo.toml`
- `Makefile`
- Rustdoc-bearing modules under `src/`
- `docs/rust-doctest-dry-guide.md`
- `docs/users-guide.md` (if guidance text requires updates)

## Plan of work

Stage A performs inventory and policy assignment before changing behaviour.
Generate a report of all Rust code blocks in Rustdoc comments and classify each
as `runnable`, `no_run`, `ignore`, or `compile_fail`, with item visibility
(public/private/test-only) and module path.

Stage B enables doctests and introduces a repeatable verification command path.
Set `doctest = true` and add a dedicated Make target (`make test-doc`) that
runs `cargo test --doc --all-features` with warnings denied.

Stage C resolves compile failures and applies runtime policy. Pure examples
should be runnable unless there is a concrete side effect risk. Examples that
perform networking, rely on timing races, or represent non-deterministic flows
should use `no_run` and still compile.

Stage D enforces scope boundaries. Remove or rewrite example blocks attached to
private items and `#[cfg(test)]` modules so doctest burden stays on public API
surface only.

Stage E locks observability and docs. Add benchmark reporting (runnable vs
`no_run`) to a scriptable check, then update documentation with the policy and
how to interpret exceptions.

Each stage ends with validation before advancing.

## Concrete steps

Run all commands from repository root (`/home/user/project`).

1. Inventory Rustdoc code blocks and classify candidates.

   `rg -n "^\s*///|^\s*//!|```rust" src docs`

2. Enable doctests and add an explicit Doctest Make target.

   `make check-fmt`

3. Execute doctests with all features.

   `RUSTFLAGS="-D warnings" cargo test --doc --all-features`

4. Execute standard Rust quality gates after fixes.

   `make check-fmt` `make lint` `make test`

5. Execute markdown quality gates if docs were changed.

   `make fmt` `make markdownlint` `make nixie`

Expected success indicators:

- Doctest command exits 0.
- No doctest compile errors remain.
- Benchmark report confirms runnable ratio >= 70% and `no_run` ratio <= 30%.

## Validation and acceptance

Acceptance criteria:

- `cargo test --doc --all-features` passes.
- All public Rustdoc examples compile.
- Runnable benchmark target is met: at least 70% runnable.
- `no_run` benchmark bound is met: no more than 30% `no_run`.
- No doctest-bearing code blocks remain on private functions or test-only
  modules.
- `make check-fmt`, `make lint`, and `make test` pass.
- `make fmt`, `make markdownlint`, and `make nixie` pass if docs changed.

## Idempotence and recovery

All steps are idempotent. Re-running inventory and doctest commands is safe. If
a doctest fix introduces regressions, revert only the affected file and re-run
the stage-specific command before continuing.

## Artifacts and notes

Implementation artifacts retained:

- Inventory and failure clustering logs:
  `/tmp/doc-pre.log`, `/tmp/doc-pass4.log`.
- Benchmark logs:
  `/tmp/doctest-benchmark.log`, `/tmp/wireframe-doctest-benchmark.log`.
- Quality gate logs:
  `/tmp/wireframe-make-fmt.log`, `/tmp/wireframe-check-fmt.log`,
  `/tmp/wireframe-lint.log`, `/tmp/wireframe-test.log`,
  `/tmp/wireframe-test-doc.log`, `/tmp/wireframe-markdownlint.log`,
  `/tmp/wireframe-nixie.log`.

## Interfaces and dependencies

No new external dependencies are planned.

Expected interfaces at completion:

- `Cargo.toml` `[lib]` no longer disables doctests.
- `Makefile` includes a repeatable doctest command target.
- Public Rustdoc examples compile under `cargo test --doc --all-features`.

Revision note: Initial draft created on 2026-02-18 to plan doctest enablement,
coverage policy, and validation workflow.
