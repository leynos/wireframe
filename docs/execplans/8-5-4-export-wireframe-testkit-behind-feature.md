# 8.5.4 Export testkit utilities as `wireframe::testkit` behind a feature

This ExecPlan (execution plan) is a living document. The sections
`Constraints`, `Tolerances`, `Risks`, `Progress`, `Surprises & Discoveries`,
`Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work
proceeds.

Status: DRAFT

## Purpose / big picture

Roadmap item `8.5.4` closes the public-surface loop for the phase-8 streaming
and assembly work. The utilities added in `8.5.1`, `8.5.2`, and `8.5.3` already
exist for downstream tests in the `wireframe_testing` companion crate, but ADR
0002 also allows a first-party export as `wireframe::testkit` behind a
dedicated Cargo feature so library consumers can opt into these helpers from
the main crate without paying for them by default.

After this change, a downstream crate that enables `wireframe`'s `testkit`
feature should be able to write tests like:

```rust,no_run
use std::{num::NonZeroUsize, time::Duration};

use wireframe::testkit::{
    SlowIoConfig,
    SlowIoPacing,
    assert_message_assembly_completed_for_key,
    drive_with_fragments,
};
```

Success is observable when all of the following are true:

1. `wireframe` exposes a new `testkit` module only when the `testkit` feature
   is enabled.
2. The phase-8.5 helper families are usable from `wireframe::testkit` in new
   `rstest` integration tests.
3. A focused `rstest-bdd` flow proves the feature-gated export from the public
   crate surface, not just the old `wireframe_testing` path.
4. Existing `wireframe_testing` users remain source-compatible for the covered
   helpers.
5. `docs/users-guide.md`,
   `docs/adr-002-streaming-requests-and-shared-message-assembly.md`,
   and `docs/roadmap.md` reflect the delivered feature.

## Constraints

- Do not introduce a Cargo dependency cycle. `wireframe_testing` already
  depends on `wireframe`, so `wireframe` must not depend on `wireframe_testing`.
- Keep the new feature opt-in. `wireframe::testkit` must not be part of the
  default feature set.
- Scope this roadmap item to the phase-8.5 helpers:
  partial-frame or fragment drivers, slow-I/O helpers, deterministic reassembly
  assertions, and the small supporting result or error types those APIs require.
- Preserve source compatibility for the existing `wireframe_testing` public
  APIs that correspond to those phase-8.5 helpers. A consumer already using
  `wireframe_testing::drive_with_partial_frames` or
  `wireframe_testing::reassembly::*` must not need code changes.
- Keep runtime behaviour outside the new `testkit` feature unchanged.
- No single Rust source file may exceed 400 lines.
- Use `rstest` for focused integration tests and `rstest-bdd` for behavioural
  validation, following the repository's existing four-file BDD structure.
- Update `docs/users-guide.md` for the new public interface exposed to library
  consumers.
- Record the implementation decision in the relevant design document. For this
  item that means ADR 0002, because it is the source that explicitly mentions
  `wireframe::testkit`.
- Mark roadmap item `8.5.4` done only after the required tests and
  documentation updates are complete.

## Tolerances (exception triggers)

- Architecture: if the only viable implementation would require
  `wireframe -> wireframe_testing -> wireframe`, stop and escalate rather than
  forcing the cycle through a build trick.
- Scope: if satisfying this item requires migrating non-8.5 utilities such as
  observability helpers, codec fixtures, or unrelated test support into
  `wireframe::testkit`, stop and confirm the expanded scope first.
- Compatibility: if preserving the current `wireframe_testing` source surface
  for the covered helpers proves impossible, stop and escalate before making a
  breaking change.
- Dependencies: if implementation requires a new third crate solely to break
  the dependency cycle, stop and review that architectural change with the user.
- Size: if the implementation grows beyond 22 touched files or 1,400 net
  lines, stop and reassess. This item should stay focused on the export path,
  compatibility shims, tests, and documentation.
- Validation: if the targeted `rstest` and `rstest-bdd` coverage still fails
  after 5 fix iterations, stop and capture the last failing output.

## Risks

- Risk: the obvious implementation, re-exporting `wireframe_testing` from the
  main crate, is impossible because it creates a Cargo cycle. Severity: high.
  Likelihood: high. Mitigation: make `wireframe::testkit` the source of truth
  and reduce `wireframe_testing` to a compatibility facade for the covered APIs.

- Risk: moving helper implementations into the main crate could accidentally
  broaden the feature to include more than roadmap item `8.5.4` requires.
  Severity: medium. Likelihood: medium. Mitigation: explicitly limit the new
  root module to the phase-8.5 families and leave later `wireframe_testing`
  additions in place.

- Risk: doc examples for `wireframe::testkit` may fail when built without the
  feature. Severity: medium. Likelihood: high. Mitigation: gate module docs and
  examples carefully and validate with `make test-doc`, which uses
  `--all-features`.

- Risk: integration tests may appear to pass while behavioural coverage still
  exercises only `wireframe_testing`. Severity: medium. Likelihood: medium.
  Mitigation: add a dedicated BDD fixture and step flow that imports
  `wireframe::testkit` directly.

- Risk: the self-dev-dependency entry for `wireframe` currently enables
  `test-support` and `pool`, but not `testkit`. Severity: medium. Likelihood:
  high. Mitigation: update the dev-dependency feature list and add focused
  tests that will fail if the re-export is missing.

## Progress

- [x] (2026-03-21 00:00Z) Read the roadmap item, ADR 0002, the referenced
  design and testing guides, and the adjacent phase-8.5 ExecPlans.
- [x] (2026-03-21 00:00Z) Inspected the current crate layout, feature flags,
  and `wireframe_testing` exports.
- [x] (2026-03-21 00:00Z) Identified the central architectural constraint:
  `wireframe` cannot depend on `wireframe_testing` without creating a Cargo
  cycle.
- [x] (2026-03-21 00:00Z) Drafted this ExecPlan.
- [ ] Implement the `testkit` feature and the new `wireframe::testkit` module
  in the main crate.
- [ ] Convert the covered `wireframe_testing` exports into compatibility
  re-exports or thin wrappers over the new root module.
- [ ] Add `rstest` integration tests covering the new root export.
- [ ] Add `rstest-bdd` behavioural coverage that imports `wireframe::testkit`
  directly.
- [ ] Update ADR 0002, `docs/users-guide.md`, and `docs/roadmap.md`.
- [ ] Run the required validation suite and record outcomes.

## Surprises & Discoveries

- Observation: the phase-8.5 helpers already exist and are public, but only
  through `wireframe_testing`. The missing work is export topology and feature
  gating, not helper design.

- Observation: `wireframe_testing/Cargo.toml` depends on the root crate via
  `wireframe = { path = ".." }`, which makes a direct re-export from
  `wireframe` impossible.

- Observation: the main crate's self-dev-dependency currently enables
  `test-support` and `pool`, so new integration tests that use
  `wireframe::testkit` will need the `testkit` feature added there as well.

- Observation: the behavioural test target `tests/bdd/mod.rs` only runs with
  the `advanced-tests` feature, so `make test` alone is not enough to satisfy
  the roadmap's `rstest-bdd` requirement for this item.

## Decision Log

- Decision: make `wireframe::testkit` the implementation source of truth for
  the phase-8.5 utilities, and keep `wireframe_testing` as a compatibility
  facade for those same items. Rationale: it satisfies the roadmap item without
  creating a Cargo cycle and avoids duplicated helper logic. Date/Author:
  2026-03-21 / Codex

- Decision: scope `wireframe::testkit` to the phase-8.5 helper families rather
  than attempting to absorb the full `wireframe_testing` crate. Rationale: the
  roadmap item is specific, and later helper families such as observability or
  codec fixtures are separate pieces of work with their own documentation and
  validation history. Date/Author: 2026-03-21 / Codex

- Decision: add dedicated root-surface tests instead of rewriting the earlier
  phase-8.5 tests wholesale. Rationale: focused new tests prove the export
  contract while existing tests continue guarding the helper behaviour and
  compatibility path. Date/Author: 2026-03-21 / Codex

## Outcomes & Retrospective

Not started yet. This section must be updated after implementation with:

- the final `wireframe::testkit` API that shipped;
- the compatibility strategy used in `wireframe_testing`;
- the exact validation commands run and their outcomes; and
- any follow-up work or limitations intentionally left out of `8.5.4`.

## Context and orientation

The current implementation is split across two crates:

- `wireframe/` is the published main library. Its root module list is in
  `src/lib.rs`, and its feature flags are in `Cargo.toml`.
- `wireframe_testing/` is a companion crate used by the repository's
  integration tests and by downstream crates that want a separate testing
  helper package.

Relevant files today:

- `Cargo.toml`:
  root feature flags and the self-dev-dependency used by integration tests.
- `src/lib.rs`:
  current root exports. There is no `pub mod testkit;` yet.
- `wireframe_testing/src/lib.rs`:
  broad public re-export surface for test helpers.
- `wireframe_testing/src/helpers.rs`:
  module registry for partial-frame, fragment, codec, payload, and slow-I/O
  drivers.
- `wireframe_testing/src/helpers/partial_frame.rs`:
  chunked partial-frame drivers added in `8.5.1`.
- `wireframe_testing/src/helpers/fragment_drive.rs`:
  fragment-feeding drivers added in `8.5.1`.
- `wireframe_testing/src/helpers/slow_io.rs`:
  slow reader and writer helpers added in `8.5.2`.
- `wireframe_testing/src/reassembly/`:
  deterministic reassembly assertions added in `8.5.3`.
- `wireframe_testing/src/integration_helpers.rs`:
  shared `TestError` and `TestResult` types currently used by the assertion
  helpers.
- `docs/adr-002-streaming-requests-and-shared-message-assembly.md`:
  source requirement that explicitly says the utilities may be re-exported as
  `wireframe::testkit` behind a feature.
- `docs/users-guide.md`:
  current user-facing documentation for the companion test helpers.
- `docs/wireframe-testing-crate.md`:
  design-facing description of the helper families already delivered.

The important architectural fact is that `wireframe_testing` already depends on
`wireframe`, so any implementation based on "just add
`wireframe_testing = { optional = true }` to the main crate and re-export it"
must be rejected immediately because Cargo will detect a cycle.

## Plan of work

### Stage A: Establish the new root-surface contract

Add a dedicated Cargo feature in the main crate:

```toml
[features]
testkit = []
```

Then add a new gated root module in `src/lib.rs`:

```rust
#[cfg(feature = "testkit")]
pub mod testkit;
```

The module should be documented as optional test support for downstream
protocol crates, not as runtime production API.

Before moving any code, write down the exact surface to export from
`wireframe::testkit`:

- partial-frame and fragment drivers from `8.5.1`;
- `SlowIoConfig`, `SlowIoPacing`, and slow-I/O drivers from `8.5.2`;
- reassembly assertion helpers and expectation types from `8.5.3`; and
- the minimal shared result or error types needed by those assertion helpers.

Do not widen this list during implementation without updating the
`Decision Log`.

### Stage B: Move the phase-8.5 implementations into the main crate

Create a new `src/testkit/` tree in the main crate. Split it so the 400-line
cap remains easy to satisfy. A likely shape is:

```plaintext
src/testkit/
  mod.rs
  helpers.rs
  integration_helpers.rs
  reassembly/
    mod.rs
    assert_helpers.rs
    fragment.rs
    message.rs
    message_error.rs
```

The goal is not a byte-for-byte file move. Instead:

1. Copy or adapt the current phase-8.5 helper implementations from
   `wireframe_testing/src/helpers/partial_frame.rs`,
   `wireframe_testing/src/helpers/fragment_drive.rs`,
   `wireframe_testing/src/helpers/slow_io.rs`,
   `wireframe_testing/src/reassembly/`, and the required pieces of
   `wireframe_testing/src/integration_helpers.rs`.
2. Adjust module paths so the helpers use the main crate's own modules rather
   than importing them through the companion crate.
3. Keep the public names stable where possible so existing docs and tests map
   cleanly to the new root surface.
4. Keep any support that is not needed for these 8.5 helpers out of the new
   root module. For example, observability helpers and codec fixtures stay in
   `wireframe_testing`.

At the end of this stage, `wireframe::testkit::*` should compile and be
documented, but `wireframe_testing` may still use its old internal
implementation temporarily.

### Stage C: Turn `wireframe_testing` into a compatibility facade

After the root module exists, simplify the covered `wireframe_testing` exports
so they re-export from `wireframe::testkit` instead of owning duplicate logic.

Concretely:

1. Update `wireframe_testing/Cargo.toml` so its `wireframe` dependency enables
   the new `testkit` feature.
2. Re-export the moved phase-8.5 items from `wireframe_testing/src/lib.rs` and
   any affected intermediate modules.
3. Remove or reduce duplicated implementations in the companion crate once the
   root module is the single source of truth.
4. Keep unchanged any `wireframe_testing` APIs that are outside the 8.5 scope.

This stage is successful when existing imports such as
`wireframe_testing::drive_with_partial_frames` and
`wireframe_testing::reassembly::assert_message_assembly_error` still compile,
but they are just facades over `wireframe::testkit`.

### Stage D: Add focused `rstest` integration coverage

Add one new integration test file in the repository root, for example
`tests/testkit_exports.rs`, that imports from `wireframe::testkit` directly.

Use `rstest` to cover at least one representative case from each family:

1. A partial-frame or fragment driver round trip.
2. A slow-I/O configuration and paced drive.
3. A message-assembly assertion helper returning `TestResult<()>`.
4. Negative feature-shape expectations that prove the helper diagnostics are
   still deterministic.

Keep the test file narrowly focused on the new root export contract. It should
not repeat every behavioural case already covered by the earlier phase-8.5
tests.

### Stage E: Add focused `rstest-bdd` coverage for the root export

Create a small behavioural flow dedicated to the new `wireframe::testkit`
surface. Follow the repository's standard structure:

- `tests/features/testkit_exports.feature`
- `tests/fixtures/testkit_exports.rs`
- `tests/steps/testkit_exports_steps.rs`
- `tests/scenarios/testkit_exports_scenarios.rs`

This BDD flow should import `wireframe::testkit` directly and prove at least
these observable behaviours:

1. a protocol test can drive fragmented or partial input through the public
   root module;
2. a protocol test can pace I/O through the public root module; and
3. a protocol test can assert reassembly outcomes through the public root
   module without panicking.

Do not retrofit the old phase-8.5 BDD suites unless that becomes necessary to
keep the code healthy. The purpose here is to prove the new export path.

### Stage F: Update documentation and roadmap state

Update these files together so the documented story stays coherent:

- `docs/adr-002-streaming-requests-and-shared-message-assembly.md`
  Add an implementation note explaining how `wireframe::testkit` was delivered
  and why the main crate became the source of truth for the 8.5 helpers.
- `docs/users-guide.md`
  Add or update examples so library consumers see the new `wireframe::testkit`
  path, the `testkit` Cargo feature, and the continued availability of
  `wireframe_testing` if they prefer the companion crate.
- `docs/wireframe-testing-crate.md`
  Update the relationship section so it no longer implies the companion crate
  owns all implementations for the covered helpers.
- `docs/roadmap.md`
  Mark `8.5.4` done only after validation passes.

If the implementation settles on a scope boundary such as "only the 8.5 helper
families moved; observability remains companion-crate-only", record that
explicitly in ADR 0002.

### Stage G: Validation and evidence capture

Run focused tests first so failures are easy to interpret, then the full gates.
Use `set -o pipefail` and `tee` for every long-running command.

Focused validation:

```sh
set -o pipefail
cargo test --test testkit_exports --features testkit \
  2>&1 | tee /tmp/8-5-4-testkit-exports.log
```

```sh
set -o pipefail
cargo test --test bdd --features "advanced-tests testkit" -- testkit_exports \
  2>&1 | tee /tmp/8-5-4-bdd.log
```

Repository gates:

```sh
set -o pipefail
make fmt 2>&1 | tee /tmp/8-5-4-fmt.log
```

```sh
set -o pipefail
make markdownlint MDLINT=/root/.bun/bin/markdownlint-cli2 \
  2>&1 | tee /tmp/8-5-4-markdownlint.log
```

```sh
set -o pipefail
make check-fmt 2>&1 | tee /tmp/8-5-4-check-fmt.log
```

```sh
set -o pipefail
make lint 2>&1 | tee /tmp/8-5-4-lint.log
```

```sh
set -o pipefail
make test 2>&1 | tee /tmp/8-5-4-test.log
```

```sh
set -o pipefail
make test-doc 2>&1 | tee /tmp/8-5-4-test-doc.log
```

```sh
set -o pipefail
make doctest-benchmark 2>&1 | tee /tmp/8-5-4-doctest-benchmark.log
```

```sh
set -o pipefail
make nixie 2>&1 | tee /tmp/8-5-4-nixie.log
```

Expected outcome:

- the focused `rstest` and `rstest-bdd` commands exit `0`;
- the new `wireframe::testkit` examples compile under `make test-doc`;
- the existing `wireframe_testing` consumers still pass through the full suite;
  and
- the roadmap checkbox is updated only after those conditions hold.

## Acceptance checklist

This item is complete only when a novice can follow the steps above and observe
all of the following:

1. `wireframe::testkit` exists only with `feature = "testkit"`.
2. The exported surface covers the phase-8.5 helper families listed in this
   plan.
3. `wireframe_testing` remains source-compatible for the same helpers.
4. The focused `rstest` and `rstest-bdd` coverage exercises the root export,
   not only the companion crate path.
5. Documentation tells consumers when to use `wireframe::testkit` and when
   `wireframe_testing` remains useful.
6. `docs/roadmap.md` marks `8.5.4` as done.
