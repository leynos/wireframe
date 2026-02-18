# Enable and resolve doctests

This ExecPlan is a living document. The sections `Constraints`, `Tolerances`,
`Risks`, `Progress`, `Surprises & Discoveries`, `Decision Log`, and
`Outcomes & Retrospective` must be kept up to date as work proceeds.

Status: DRAFT

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
- [ ] Inventory all Rustdoc code blocks in `src/` and classify by item
      visibility and runtime profile.
- [ ] Enable doctests in `Cargo.toml` and add a dedicated command target for
      doctest execution.
- [ ] Resolve compile failures in all doctest blocks.
- [ ] Enforce doctest scope rules (no private/test item doctest examples).
- [ ] Compute and validate runnable-vs-`no_run` benchmark.
- [ ] Run quality gates and update user documentation.

## Surprises & Discoveries

- Observation: None yet.
  Evidence: Plan-only phase. Impact: None yet.

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

## Outcomes & Retrospective

Not started. Populate after implementation milestones complete.

## Context and orientation

Current state relevant to this plan:

- `Cargo.toml` disables doctests with `doctest = false` in `[lib]`.
- Public docs are spread across module-level docs and item-level docs in
  `src/`.
- The repository already contains extensive integration and behavioural tests in
  `tests/`; these are not doctest replacements.
- `docs/rust-doctest-dry-guide.md` documents project expectations for doctests
  and should be kept aligned with implementation reality.

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

Implementation should retain:

- A short inventory summary of doctest blocks by class.
- A benchmark summary with counts and percentages.
- Failure cluster notes for any blocks converted to `no_run`.

## Interfaces and dependencies

No new external dependencies are planned.

Expected interfaces at completion:

- `Cargo.toml` `[lib]` no longer disables doctests.
- `Makefile` includes a repeatable doctest command target.
- Public Rustdoc examples compile under `cargo test --doc --all-features`.

Revision note: Initial draft created on 2026-02-18 to plan doctest enablement,
coverage policy, and validation workflow.
