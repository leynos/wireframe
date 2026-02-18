# Unify test support under one cargo feature gate

This ExecPlan is a living document. The sections `Constraints`, `Tolerances`,
`Risks`, `Progress`, `Surprises & Discoveries`, `Decision Log`, and
`Outcomes & Retrospective` must be kept up to date as work proceeds.

Status: DRAFT

`PLANS.md` is not present in this repository at the time this plan was drafted.

## Purpose / big picture

The crate currently defines two similarly named features for test utilities:
`test-support` and `test-helpers`. Only `test-helpers` is used to gate
`src/test_helpers.rs` exports, while `test-support` is unused. This naming
split is confusing and makes feature discovery harder for users.

This plan standardizes on one public test utility gate named `test-support`,
removes the competing `test-helpers` gate, and updates all feature checks and
manifest references so behaviour is explicit and consistent.

## Constraints

- Use exactly one cargo feature name for exported test utilities:
  `test-support`.
- Preserve existing behaviour: test helper APIs remain available for test code
  (`cfg(test)`) and for consumers who opt in through the feature gate.
- Do not change runtime behaviour of non-test functionality.
- Do not introduce new dependencies.
- Keep documentation and examples accurate where feature flags are shown.

## Tolerances (exception triggers)

- Scope: if unifying the gate requires edits outside the expected set
  (`Cargo.toml`, `src/lib.rs`, `src/test_helpers.rs`, and docs/tests
  references) in more than 10 files, stop and escalate.
- Compatibility: if a required downstream consumer in this workspace depends on
  `test-helpers` and cannot be migrated in the same change, stop and escalate
  with the exact blocker.
- Validation: if `make lint` or `make test` fails after 3 repair loops, stop
  and escalate with grouped failure themes.
- Naming: if both feature names must coexist for policy reasons, stop and
  escalate because that conflicts with the single-gate objective.

## Risks

- Risk: Hidden references to `test-helpers` in scripts or docs may be missed.
  Severity: medium. Likelihood: medium. Mitigation: run repository-wide search
  before and after implementation and keep zero tolerated occurrences except
  explicit migration notes.

- Risk: Feature-gated exports may accidentally become unavailable in tests.
  Severity: high. Likelihood: low. Mitigation: preserve `cfg(test)` coverage
  and verify with full workspace tests.

- Risk: Users currently enabling `test-helpers` will break after rename.
  Severity: medium. Likelihood: medium. Mitigation: document the rename in
  changelog or migration notes as part of the implementation.

## Progress

- [x] (2026-02-18) Drafted ExecPlan and captured current mismatch:
  `Cargo.toml` defines both flags, `src/lib.rs` and `src/test_helpers.rs` gate
  exports with `test-helpers`.
- [ ] Replace feature declarations so only `test-support` remains.
- [ ] Update `cfg(feature = "...")` gates to use `test-support`.
- [ ] Update dev-dependency feature selection for local self-dependency.
- [ ] Update any user-facing docs that reference `test-helpers`.
- [ ] Run full validation gates and confirm no remaining `test-helpers`
  references.

## Surprises & Discoveries

- Observation: the crate self dev-dependency in `Cargo.toml` currently enables
  `features = ["test-helpers"]`, so the rename is not only about cfg guards.
  Evidence: `Cargo.toml:54`. Impact: manifest and cfg updates must ship
  together to keep tests building.

## Decision Log

- Decision: converge on `test-support` and remove `test-helpers` instead of
  keeping both names as aliases. Rationale: the request requires one gate
  across the codebase; aliases preserve confusion and prolong ambiguity.
  Date/Author: 2026-02-18 / Codex.

- Decision: keep `cfg(test)` in addition to `feature = "test-support"` for
  helper modules. Rationale: internal test targets should not require explicit
  feature flags. Date/Author: 2026-02-18 / Codex.

## Outcomes & Retrospective

Not started. Populate after implementation and validation complete.

## Context and orientation

Current known state:

- `Cargo.toml` declares both `test-support = []` and `test-helpers = []`.
- `src/lib.rs` exports `pub mod test_helpers;` under
  `#[cfg(any(test, feature = "test-helpers"))]`.
- `src/test_helpers.rs` uses crate-level
  `#![cfg(any(test, feature = "test-helpers"))]`.
- The crate self dev-dependency in `Cargo.toml` enables
  `features = ["test-helpers"]`.

Target state:

- `Cargo.toml` has one test helper feature entry: `test-support`.
- All source-level feature cfg checks reference `test-support`.
- Dev-dependency feature selection references `test-support`.
- Repository search shows no active `test-helpers` references, except optional
  migration/changelog text if included.

Likely touched files:

- `Cargo.toml`
- `src/lib.rs`
- `src/test_helpers.rs`
- `CHANGELOG.md` and/or migration docs if feature rename is documented there

## Plan of work

Stage A is inventory and rename planning. Capture every `test-helpers` and
`test-support` usage in source, manifests, and docs. Confirm there are no other
behavioural gates hidden behind macros or generated code.

Stage B applies the feature unification. Remove the `test-helpers` feature
declaration, keep `test-support`, and update all cfg gates and dev-dependency
references to the single name.

Stage C updates user-facing references. Where the old flag is documented,
switch to `test-support` and add a concise migration note if this is a breaking
interface for users.

Stage D validates behaviour and quality gates. Run Rust and Markdown validation
commands with logged output and confirm feature-gated helper availability still
matches expectations.

## Concrete steps

Run all commands from repository root (`/home/user/project`).

1. Baseline all feature-name occurrences.

    rg -n "test-helpers|test-support" Cargo.toml src tests docs README.md \
      CHANGELOG.md

2. Implement rename to a single gate.

   - Edit `Cargo.toml`:
     - Remove `test-helpers = []`.
     - Keep `test-support = []`.
     - Change self dev-dependency feature list to `["test-support"]`.
   - Edit `src/lib.rs` and `src/test_helpers.rs`:
     - Replace `feature = "test-helpers"` with
       `feature = "test-support"`.
   - Update docs/changelog references as needed.

3. Re-run occurrence search and confirm there are no active old-name
   references.

    rg -n "test-helpers|test-support" Cargo.toml src tests docs README.md \
      CHANGELOG.md

4. Run formatting, lint, and test quality gates with captured logs.

    set -o pipefail; make check-fmt 2>&1 | tee /tmp/single-test-support-check-fmt.log
    set -o pipefail; make lint 2>&1 | tee /tmp/single-test-support-lint.log
    set -o pipefail; make test 2>&1 | tee /tmp/single-test-support-test.log

5. Run documentation gates if docs were touched.

    set -o pipefail; make fmt 2>&1 | tee /tmp/single-test-support-fmt.log
    set -o pipefail; make markdownlint 2>&1 | tee /tmp/single-test-support-markdownlint.log
    set -o pipefail; make nixie 2>&1 | tee /tmp/single-test-support-nixie.log

Expected success indicators:

- Exactly one test helper feature is present in `Cargo.toml`.
- Source cfg gates use only `test-support`.
- `make check-fmt`, `make lint`, and `make test` pass.
- If docs changed, `make fmt`, `make markdownlint`, and `make nixie` pass.

## Validation and acceptance

Acceptance criteria:

- Repository search finds no active `feature = "test-helpers"` in source.
- `Cargo.toml` features table declares only `test-support` for this purpose.
- Test helpers remain available under `cfg(test)` and under
  `--features test-support`.
- Full quality gates pass with no lint, formatting, or test failures.
- Any user-facing feature flag references are updated to `test-support`.

Verification commands:

    rg -n 'feature = "test-helpers"|test-helpers = \\[\\]' Cargo.toml src
    cargo test --all-targets --all-features
    cargo check --all-targets --no-default-features --features test-support

## Idempotence and recovery

All searches and validation commands are safe to re-run. If rename changes
cause unexpected breakage, restore only the affected file to the previous
committed state, reapply the rename in smaller commits, and re-run checks after
each file change.

## Artifacts and notes

Record in implementation notes:

- Before/after `rg` output for feature names.
- Paths changed for feature gate migration.
- Any downstream compatibility notes for users migrating from `test-helpers`.

## Interfaces and dependencies

No new dependencies are planned. The only intentional interface change is the
feature name used to expose test helper APIs to non-test builds: `test-support`.

Revision note: Initial draft created on 2026-02-18 for feature-flag ergonomics
cleanup around test utility exports.
