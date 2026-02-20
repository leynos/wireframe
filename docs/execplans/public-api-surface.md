# Reshape the public API surface

This ExecPlan is a living document. The sections `Constraints`, `Tolerances`,
`Risks`, `Progress`, `Surprises & Discoveries`, `Decision Log`, and
`Outcomes & Retrospective` must be kept up to date as work proceeds.

Status: COMPLETE

`PLANS.md` is not present in this repository at the time this plan was drafted.

## Purpose / big picture

Wireframe exposes a very wide root API, which makes discovery and maintenance
harder and increases accidental coupling. This plan defines a tiered,
progressive-discovery API shape: a small root, focused domain modules, and
optional convenience prelude. Test-only helpers become private to tests (or to
`wireframe_testing`) rather than regular public API.

## Constraints

- Keep crate root intentionally small and stable in shape.
- Organize exports so users can discover core workflows from root, then drill
  into module-specific detail.
- Test-only modules and helpers must not be publicly reachable in normal builds.
- Backwards compatibility is not required for this work.
- `docs/v0-1-0-to-v0-2-0-migration-guide.md` must be updated with all breaking
  API surface changes introduced by this plan.

## Tolerances (exception triggers)

- Scope: if API reshaping requires edits in more than 60 files, stop and
  escalate.
- Interface: if root simplification breaks internal integration tests in ways
  that cannot be addressed with straightforward import updates, stop and
  escalate.
- Dependencies: if new dependencies are required solely for re-export hygiene,
  stop and escalate.
- Iterations: if `make test` fails after 3 complete repair loops, stop and
  escalate with grouped import/signature issues.
- Ambiguity: if two valid root layouts exist with materially different user
  ergonomics, stop and escalate with both options.

## Risks

- Risk: aggressive root pruning may reduce short-term ergonomics for existing
  users. Severity: medium Likelihood: high Mitigation: provide
  `wireframe::prelude` and a migration guide with concrete import mappings.

- Risk: hiding test helpers may disrupt internal or companion-crate tests.
  Severity: medium Likelihood: medium Mitigation: move needed helpers to
  `wireframe_testing` or `#[cfg(test)]` local modules with explicit ownership.

- Risk: module boundary edits may produce circular dependencies.
  Severity: high Likelihood: low Mitigation: perform export-surface changes
  first, then internal visibility tightening, with compile checks at each stage.

## Progress

- [x] (2026-02-18) Drafted ExecPlan for public API surface cleanup.
- [x] Define target root API map and progressive discovery tiers.
- [x] Refactor `src/lib.rs` exports to the target map.
- [x] Tighten module visibility (`pub` to `pub(crate)` where applicable).
- [x] Remove public test-only reachability from production builds.
- [x] Update migration guide and user docs.
- [x] Run full quality gates.

## Surprises & Discoveries

- Observation: Root re-export pruning required broad import rewrites across
  tests, examples, and client test modules. Evidence:
  `git diff --name-only | wc -l` returned `60` touched files. Impact: At the
  tolerance boundary, and required iterative compile repair loops before moving
  to full quality gates.

- Observation: `connection::test_support` was publicly reachable in non-test
  builds via `cfg(not(loom))`. Evidence: `src/connection/mod.rs` exported
  `pub mod test_support` with only `not(loom)` gating. Impact: Tightened to
  `cfg(all(not(loom), any(test, feature = "test-support")))` to keep it out of
  normal production builds.

## Decision Log

- Decision: Use three discovery tiers.
  Rationale: Tiered discovery keeps root simple while preserving depth.
  Date/Author: 2026-02-18 / Codex.

- Decision: Prefer module-based access over broad root re-export lists.
  Rationale: Module paths communicate conceptual ownership and reduce root
  clutter. Date/Author: 2026-02-18 / Codex.

- Decision: Keep crate-root exports to canonical error/result aliases and move
  high-frequency ergonomics to `wireframe::prelude`. Rationale: This preserves
  a stable minimal root while still offering optional convenience imports.
  Date/Author: 2026-02-19 / Codex.

- Decision: Preserve the `test-support` feature for integration-test workflows
  while removing default-build exposure of connection test helpers. Rationale:
  Internal and companion tests continue to work without exposing test-only
  helpers in standard library builds. Date/Author: 2026-02-19 / Codex.

## Outcomes & Retrospective

Completed implementation outcomes:

- Root API was reduced to canonical error/result aliases with detailed APIs
  now discovered through module namespaces.
- `wireframe::prelude` was introduced as an optional ergonomics layer for
  common imports.
- `connection::test_support` is no longer reachable in normal builds.
- Migration and user documentation now describe module-based imports and
  include explicit before/after mappings for removed root re-exports.

Validation outcomes:

- `make fmt` passed.
- `make check-fmt` passed.
- `make lint` passed.
- `make test` passed.
- `make markdownlint` passed.
- `make nixie` passed.

## Context and orientation

Current state:

- `src/lib.rs` exports many modules and large re-export groups directly from
  root.
- Some test-oriented helpers are conditionally exported with cargo features.
- `src/connection/mod.rs` exposes a `test_support` module under
  `cfg(not(loom))`, which is broader than test-only scope.

Target state:

- Root surfaces only the primary concepts and canonical errors/results.
- Detailed APIs live under clearly owned modules (`app`, `server`, `client`,
  `codec`, `fragment`, `message_assembler`, and similar).
- Optional `prelude` includes high-frequency traits/types only.
- Test support is private to tests or externalized to `wireframe_testing`.

Likely touched files:

- `src/lib.rs`
- module `mod.rs` files across `src/`
- `src/connection/mod.rs`
- `docs/users-guide.md`
- `docs/v0-1-0-to-v0-2-0-migration-guide.md`

## Plan of work

Stage A designs the target API map. Create a before/after inventory of root
exports, each tagged with tier (`root`, `module`, `prelude`, `internal`).

Stage B applies root simplification. Remove or relocate broad re-exports from
`src/lib.rs`, retaining only the smallest coherent front door.

Stage C enforces visibility boundaries. Convert internal modules and helpers to
`pub(crate)` or `#[cfg(test)]` as appropriate. Ensure test support no longer
appears in normal builds.

Stage D adds ergonomics. Introduce or refine a curated `prelude` for common
imports and update docs so examples use intended discovery paths.

Stage E ships migration documentation and validates the full build/test matrix.

## Concrete steps

Run all commands from repository root (`/home/user/project`).

1. Capture current public API map.

   `rg -n "^pub mod |^pub use " src/lib.rs src/*/mod.rs`

2. Apply root export and visibility changes.

   `make check-fmt`

3. Verify crate compiles and tests pass.

   `make lint` `make test`

4. Validate migration and user docs.

   `make fmt` `make markdownlint` `make nixie`

Expected success indicators:

- Root export list is materially smaller and conceptually grouped.
- Test-only helpers are no longer reachable in normal builds.
- Migration guide includes import-path before/after mappings.

## Validation and acceptance

Acceptance criteria:

- Root API is intentionally small and documented.
- Progressive discovery path exists and is documented (`root` -> `module` ->
  optional `prelude`).
- Test-only modules are inaccessible in non-test builds.
- `docs/v0-1-0-to-v0-2-0-migration-guide.md` is updated for all breaking
  import/path changes.
- `make check-fmt`, `make lint`, and `make test` pass.
- `make fmt`, `make markdownlint`, and `make nixie` pass for doc changes.

## Idempotence and recovery

API map generation and compile checks are re-runnable. If a visibility change
causes excessive breakage, revert that module boundary only, then reapply with
smaller steps and immediate compile validation.

## Artifacts and notes

Implementation should preserve:

- Before/after root export inventory.
- Migration mapping list for renamed or relocated paths.
- Notes on any moved test helpers and their new ownership.

## Interfaces and dependencies

No new external dependencies are planned.

Expected interface shape:

- `wireframe::` root exposes only canonical error/result aliases.
- `wireframe::<module>::...` is the default path for specialized APIs.
- `wireframe::prelude::*` is optional convenience, not mandatory coupling.

Revision note: Initial draft created on 2026-02-18 to plan public API surface
simplification, test-surface privacy, and migration documentation.
