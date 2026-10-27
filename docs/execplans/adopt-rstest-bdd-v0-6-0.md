# Migrate Wireframe's rstest-bdd consumers to v0.6.0

This ExecPlan is a living document. Keep `Constraints`, `Tolerances`, `Risks`,
`Progress`, `Surprises & discoveries`, `Decision log`,
`Outcomes & retrospective`, `Conformance basis`, and `Verification plan`
current throughout the migration.

Status: COMPLETE. Draft PR [#811](https://github.com/leynos/wireframe/pull/811)
is open for review.

## Purpose / big picture

Wireframe already uses `rstest-bdd` for its behavioural suite. This migration
will make that suite run against the published v0.6.0 API while preserving its
existing Gherkin scenarios, strict compile-time validation, and scenario
behaviour. After the migration, `make test-bdd` and the repository's canonical
format, lint, unit, integration, and documentation gates will pass on the
repository's pinned toolchain.

The baseline suite has 45 feature files, 181 scenario declarations, and 24
async scenario functions. The candidate adds one feature file and two focused
regression scenarios, for 46 files and 183 scenario declarations. The main BDD
target therefore has 172 scenarios; the separate pooled-client target remains
at 11. Nine existing scenario functions use the `TestResult<World>` alias for
fallible fixtures. The v0.6.0 macro recognizes spelled `Result<T, E>` and
`StepResult<T, E>` fixture parameters, not arbitrary aliases, so these
parameters must become owned, explicit `Result<World, TestError>` values. Their
scenario return types must explicitly be `Result<(), TestError>`. The baseline
is commit `b194531cde8a7a8f88ea7c98e24c91b91657138d` on 2026-10-23.

## Context and orientation

The direct consumers are the `bdd` and `bdd_pool` test targets in the root
`Cargo.toml`. Gherkin files live under `tests/features/`; their step functions,
fixtures, and scenario bindings live under `tests/steps/`, `tests/fixtures/`,
and `tests/scenarios/`. The `wireframe_testing` package maintains an
independent lockfile but has no direct `rstest-bdd` dependency.

The imported upstream documents are authoritative and must remain byte-for-
byte intact. Wireframe's previous adapted guide is preserved separately at
`docs/wireframe-rstest-bdd-users-guide.md`.

## Constraints

- Preserve existing scenarios, fixture values, error propagation, skip
  behaviour, strict compile-time validation, and deliberate feature choices.
- Do not use v0.6.1 beta or v0.7.0 APIs. In particular, do not adopt v0.7
  guard-based `StepContext` borrowing or claim its lifecycle guarantees.
- Preserve the existing `#[from(rstest_bdd_harness_context)]` spelling unless
  the pinned v0.6.0 source proves a different form is available and needed.
- Imported guide bytes remain unchanged. Record immutable provenance,
  checksums, and upstream link mapping in this plan.
- Do not modify submodules, vendored code, or unrelated work.
- Retain the mainline toolchain pin and components unless validation proves a
  further change is needed for the target dependency.
- The user authorized any required MSRV or dated-nightly changes.
- Do not merge or publish a release. The final remote action is a draft PR.

## Tolerances

- Expected edits are limited to the root BDD dependency and applicable
  manifests/lockfiles, BDD tests and fixtures, documentation, and toolchain or
  CI settings required by a demonstrated compiler-floor change.
- Do not weaken compile-time validation, add old/new compatibility facades,
  or make a stable repository nightly-only.
- Add focused regression coverage for aliased step errors and successful
  fixture overrides, while keeping existing feature scenarios intact.
- Report and investigate any test or scenario inventory difference before
  accepting it.
- Run gates sequentially through the repository's exclusive `scrutineer`
  gate runner and keep each command's exit status with its log.

## Risks

- **R-1: Existing fallible fixtures become false-green or fail to bind.**
  v0.6 unwraps an owned `Result<T, E>` fixture before inserting `T` into
  `StepContext`; the old step signatures accept `&mut TestResult<World>`.
  Mitigation: migrate the affected steps to `&mut World`, make scenario return
  types explicitly fallible, and test both failure propagation and successful
  value injection.
- **R-2: A test previously passed despite a failed step.** The v0.6 runtime
  dispatch recognizes aliases of `Result`; a failing scenario must remain
  visibly failing rather than be suppressed or ignored.
- **R-3: Upstream docs contain later-release material and relative links.**
  Mitigation: keep the copies untouched, map relative references to the
  immutable upstream tree, and apply only v0.6.0 API evidence.
- **R-4: The published dependency raises the compiler floor.** Upstream BDD
  packages require Rust 1.88, while existing root code uses
  `AtomicU64::try_update`, stable since Rust 1.95. Declare the root's actual
  floor as 1.95 and verify it with the stable compiler.
- **R-5: Unrelated host builds contend for the shared Cargo cache and compiler
  admission.** Queue normally, retain logs, and do not create isolated caches
  or kill processes.

## Progress

- [x] (2026-10-23) Read repository instructions and checked the clean baseline
  worktree.
- [x] (2026-10-23) Renamed the local branch to
  `adopt-rstest-bdd-v0-6-0`; confirmed no existing branch or PR with that name.
- [x] (2026-10-23) Fetched the v0.6.0 tag and verified it resolves to the
  requested immutable commit before importing either guide.
- [x] (2026-10-23) Imported both authoritative documents and preserved the
  previously adapted Wireframe guide separately.
- [x] (2026-10-23) Recorded 45 feature files and 181 actual scenario
  declarations/annotations, with no outlines, examples blocks, skip tags, or
  ignored BDD scenarios. Twenty-four scenario functions are explicitly async.
  The main target has 170 scenarios; 11 pooled-client scenarios are in a
  separate target. One additional textual `#[scenario]` match is a comment.
- [x] (2026-10-23) Baseline `make test-bdd` passed: 205 tests total, comprising
  170 scenario tests and 35 fixture unit tests, with zero failures, ignored
  tests, or harness skips. The log is
  `/tmp/test-bdd-7a74dac1-96d7-4892-9e35-8d2ad29c242c-adopt-rstest-bdd-v0-6-0-baseline.out`.
- [x] (2026-10-23) The separate `bdd_pool` target baseline passed all 11
  scenarios, with zero failures, ignored tests, or filtered tests. The log is
  `/tmp/test-bdd-pool-7a74dac1-96d7-4892-9e35-8d2ad29c242c-adopt-rstest-bdd-v0-6-0-baseline.out`.
- [x] (2026-10-23) An independent `build-limits-status` query timed out during
  the baseline run. The active gate continued receiving compiler slots and
  completed without an admission refusal.
- [x] (2026-10-23) Added two focused alias regressions and confirmed both fail
  for the expected v0.5.0 behaviours: the aliased error does not panic, and the
  successful alias leaves the fixture at its original value. Evidence is
  `/tmp/test-bdd-v0-5-alias-regressions-retry-7a74dac1-96d7-4892-9e35-8d2ad29c242c.out`.
- [x] (2026-10-23) Updated the root BDD dependencies and lockfile with targeted
  `cargo update --precise 0.6.0` commands. Strict compile-time validation stays
  enabled. Upstream BDD package metadata requires Rust 1.88, but the first lint
  run showed existing root code calls `AtomicU64::try_update`, stable since
  Rust 1.95. The root package floor is therefore 1.95 for both library and test
  targets; the existing dated nightly already exceeds it.
- [x] (2026-10-23) Migrated all nine aliased fallible-fixture scenario
  parameters to owned explicit `Result<World, TestError>` values and
  `Result<(), TestError>` scenario returns; step definitions now borrow the
  unwrapped mutable world. Retained the old setup-error context at fixture
  creation.
- [x] (2026-10-23) Completed the applicability matrix from the full owned
  source inventory. Only aliased step-result dispatch and fallible fixture
  handling require source changes; direct runtime/report/language-server APIs,
  fixture renames, custom harnesses, and Tokio runtime declarations are absent.
- [x] (2026-10-23) Verified the focused v0.6 regression pair. The third run
  passed both cases (aliased error propagation and successful fixture
  replacement): `2 passed; 0 failed; 0 ignored; 205 filtered out`. Log:
  `/tmp/test-bdd-v0-6-alias-regressions-retry-3-7a74dac1-96d7-4892-9e35-8d2ad29c242c.out`.
  The earlier compile attempt and assertion correction remain documented below
  for the evidence trail.
- [x] (2026-10-23) Recorded the focused gate's intermediate findings. The first
  v0.6 compile failed with 18 errors and no tests run. Its log is
  `/tmp/test-bdd-v0-6-alias-regressions-7a74dac1-96d7-4892-9e35-8d2ad29c242c.out`.
  The v0.6 macro consumes each owned `Result` with `?` before fixture
  insertion, so the scenario body must not use that parameter; generated code
  also requires the fixture function in scope. The next run passed the aliased
  error case and the `Then` step observed the successful replacement, but a
  scenario-body assertion read its original fixture argument and failed after
  the steps. Log:
  `/tmp/test-bdd-v0-6-alias-regressions-retry-2-7a74dac1-96d7-4892-9e35-8d2ad29c242c.out`.
  The assertion now lives only in the `Then` step, where it checks the mutable
  `StepContext` fixture; the scenario body merely consumes its required fixture
  parameter. The third run passed as recorded above.
- [x] (2026-10-24) Ran the candidate `make test-bdd` after correcting the
  fixture clock and runtime scheduling. All 207 tests passed: 172 BDD scenarios
  and 35 fixture unit tests, with zero failures, ignored tests, or filtered
  tests. This is an increase of two scenarios and one feature file from
  baseline, exactly matching the two alias regression cases; no existing
  scenarios were removed. The log is
  `/tmp/test-bdd-wireframe-adopt-rstest-bdd-v0-6-0-rerun7.out`.
- [x] (2026-10-23) Recorded the initial `make lint` failure before any later
  gate ran: Clippy rejected `AtomicU64::try_update` at
  `src/client/pool/lease.rs:212` and `src/fragment/fragmenter.rs:73` because
  the candidate floor was 1.88 although both calls require 1.95. Log:
  `/tmp/lint-7a74dac1-96d7-4892-9e35-8d2ad29c242c-adopt-rstest-bdd-v0-6-0.out`.
- [x] (2026-10-23) The second `make lint` run passed rustdoc and cleared the
  MSRV findings, then identified two `unnecessary_wraps` cases in BDD steps.
  The lifecycle close step now returns unit; the override regression checks its
  initial fixture before returning `Ok(replacement)`, preserving meaningful
  `Result<T, E>` coverage. Evidence:
  `/tmp/lint-7a74dac1-96d7-4892-9e35-8d2ad29c242c-adopt-rstest-bdd-v0-6-0-rerun.out`.
- [x] Validate the Rust 1.95 floor and the current mainline nightly with the
  compiler-coupled gates.
- [x] Run canonical gates sequentially, inspect final dependency graphs and
  lockfiles, and explain every inventory change.
- [x] (2026-10-23) Ran `make fmt` successfully after the source migration.
  Both imported v0.6.0 guide checksums remain exact; formatter log:
  `/tmp/fmt-wireframe-adopt-rstest-bdd-v0-6-0.out`.
- [x] (2026-10-23) After the two Clippy findings were corrected, the final
  sequential gate rerun passed `make fmt` and `make check-fmt`. Final rustdoc
  lint passed in 4m08s and all-target Clippy passed in 16m01s after queued
  compiler admission. Cargo emitted an existing warning for an unused
  `wireframe` dependency in `crates/wireframe-verification/Cargo.toml:12`. Two
  `build-limits-status` probes returned `EAGAIN` and timeout errors, recorded
  to the user, while queued compiler requests continued advancing. Whitaker used
  `nightly-2026-05-28` and reported the existing unused `.cargo/config.toml`
  key `unstable.embed-metadata`, then failed without Dylint findings when the
  supervisor refused a compiler launch for the `middleware` test. The gate
  stopped before later checks. Its log is
  `/tmp/lint-7a74dac1-96d7-4892-9e35-8d2ad29c242c-adopt-rstest-bdd-v0-6-0-final.out`.
- [x] (2026-10-24) After a fresh supervisor status reported healthy, retried
  the gate sequence. Its first `make check-fmt` stopped because the plan needed
  Markdown table formatting; no later gates ran. `make fmt` then passed, and
  both imported guide hashes still match the immutable v0.6.0 copies.
- [x] (2026-10-24) The restarted `make check-fmt` passed. `make lint` passed
  workspace rustdoc, all-target Clippy, and Whitaker using
  `nightly-2026-05-28`. Clippy emitted the existing unused-dependency warning at
  `crates/wireframe-verification/Cargo.toml:12`; Cargo also reported the
  existing unused `.cargo/config.toml` key `unstable.embed-metadata`. Log:
  `/tmp/lint-7a74dac1-96d7-4892-9e35-8d2ad29c242c-adopt-rstest-bdd-v0-6-0-rerun3.out`.
- [x] (2026-10-24) The following `make test-bdd` run executed 207 tests and
  exposed 28 failures. The previously ignored step errors had hidden existing
  scenario setup/assertion defects: client, preamble, hook, codec, and panic
  scenarios created a separate Tokio runtime per step, while inbound assembly
  fixtures did not yield consistently between frame writes and virtual-time
  advances. The send-streaming transport-failure setup also closed gracefully
  before a small write that could still fit the socket buffer. Log:
  `/tmp/test-bdd-7a74dac1-96d7-4892-9e35-8d2ad29c242c-adopt-rstest-bdd-v0-6-0-rerun3.out`.
- [x] (2026-10-24) The next BDD run, after preserving one runtime per scenario
  and returning owned errors from fallible fixtures, passed 201 of 207 tests.
  Six timing scenarios still failed: budget cleanup
  (`completed_assembly_reclaims_budget`, `timeout_purge_reclaims_budget`),
  memory back-pressure (`soft_pressure_delays_completion`), budget transitions
  (`recovery_from_soft_limit`), inbound assembly timeout
  (`message_assembly_inbound_timeout`), and client preamble timeout
  (`client_preamble_timeout_failure`). Log:
  `/tmp/test-bdd-7a74dac1-96d7-4892-9e35-8d2ad29c242c-adopt-rstest-bdd-v0-6-0-rerun4.out`.
- [x] (2026-10-24) The subsequent full BDD run passed 202 of 207 tests, with
  five failures remaining: `timeout_purge_reclaims_budget`,
  `recovery_from_soft_limit`, `client_preamble_timeout_failure`,
  `soft_pressure_delays_completion`, and `message_assembly_inbound_timeout`.
  Its log is `/tmp/test-bdd-wireframe-adopt-rstest-bdd-v0-6-0-rerun5.out`. The
  matching post-edit `make check-fmt` passed (125 files unchanged); log:
  `/tmp/check-fmt-wireframe-adopt-rstest-bdd-v0-6-0-rerun5-post-edit.out`.
- [x] (2026-10-24) A focused timeout experiment replaced only the inbound
  assembly fixture's Tokio virtual-time advance with a real wait. The exact
  `message_assembly_inbound_timeout` scenario then passed (1 passed, 206
  filtered). `MessageAssemblyState` compares `std::time::Instant`, so advancing
  Tokio time alone cannot expire its stored deadline. The experiment fixture
  was restored after the run. Evidence: `/tmp/h1-message-assembly-timeout.out`.
- [x] (2026-10-24) Source review found the slow preamble server dropped its
  accepted TCP stream before sleeping, causing EOF instead of the timeout
  expected by the scenario. The current change retains that stream through the
  delay. The timeout fixtures now use wall-clock waits for assembly state, and
  frame-processing waits yield the current-thread runtime after the parse
  signal so soft-pressure timers can be scheduled before virtual time moves.
- [x] (2026-10-24) Reran all BDD scenarios after correcting the remaining
  setup defects. The run passed 207/207, including both previously failing
  timing scenarios, aliased error propagation, and successful fixture
  replacement. `make check-fmt` also passed with 126 Rust and Markdown files
  unchanged. Logs are
  `/tmp/test-bdd-wireframe-adopt-rstest-bdd-v0-6-0-rerun7.out` and
  `/tmp/check-fmt-wireframe-adopt-rstest-bdd-v0-6-0-rerun7.out`.
- [x] (2026-10-24) The delayed candidate BDD rerun completed successfully after
  the shared Cargo package-cache lock cleared: 207 tests passed. The imported
  document hashes were rechecked and still match the pinned upstream files.
  `build-limits-status` probes also timed out with
  `build-limits-status: cannot query the supervisor at
  /run/user/1000/buildlimits/supervisor.sock: timed out`
  (exit 1). No process was signalled; queued compiler work continued.
- [x] (2026-10-24) `origin/main` advanced from `b194531c` to `7bdc49e4` while
  implementation was in progress. Preserved the entire candidate with an
  untracked-inclusive stash, fast-forwarded the branch, and reapplied it. The
  stash remains available as recovery evidence; overlapping manifest, lockfile
  and developer-guide changes auto-merged without conflicts.
- [x] (2026-10-24) The post-implementation Whitaker run reached all-target test
  checks and found only that `budget_cleanup` occupied 405 lines, over the
  repository's 400-line limit. Rustdoc and Clippy had completed; no later gate
  ran. Split the fixture self-tests into
  `tests/fixtures/budget_cleanup_tests.rs`; the production fixture is now 357
  lines and the test module 48 lines. Its lint rerun remains pending.
- [x] (2026-10-24) During the final lint rerun, the supervisor status probe
  returned
  `build-limits-status: cannot query the supervisor at
  /run/user/1000/buildlimits/supervisor.sock: timed out`.
  The probe output was piped through `head` without `pipefail`, so its own
  exit code was not captured. The active rustdoc command remained queued; no
  process was signalled, retried, or bypassed.
- [x] (2026-10-27) The interrupted lint invocation had no terminal result.
  Replayed the archived candidate onto current `origin/main` at `5a3fbfa9`,
  which advances the repository pin to `nightly-2026-10-27`. The selected
  compiler reports `rustc 1.101.0-nightly (16d1ffac1 2026-10-26)`; the
  component list is unchanged. The shared compiler supervisor reported healthy.
- [x] (2026-10-27) On that base, `make fmt`, `make check-fmt`, `make lint`,
  `make test-bdd`, `make test`, `make test-workflow-contracts`, and
  `make test-doc` passed. The BDD run again passed all 207 tests, and the
  workflow contracts passed 259 tests. The root unit suite passed 587 tests.
  The imported guide hashes remained unchanged after formatting. Lint reported
  the existing unused `wireframe` dependency and `unstable.embed-metadata`
  configuration warnings; it passed Clippy and Whitaker.
- [x] (2026-10-27) The separate `make test-ui` gate exposed a diagnostic
  snapshot mismatch under the new mainline nightly. The same six imports remain
  unresolved and the same configuration notes remain present; only underline
  and label placement changed. Updated the single reviewed Loom UI snapshot,
  without changing the fixture or its assertions. The subsequent gate passed.
  Failure log:
  `/tmp/test-ui-7a74dac1-96d7-4892-9e35-8d2ad29c242c-adopt-rstest-bdd-v0-6-0-final2.out`.
- [x] (2026-10-27) The corrected `make test-ui`, `make test-feature-gate`,
  and `make test-loom` gates passed. The standalone consumer confirmed both
  disabled accessors are absent and the enabled and public APIs compile. Rust
  1.95.0 passed `cargo check --workspace --all-targets --all-features --locked`
  with warnings denied. The pooled BDD target passed all 11 existing scenarios.
  `make test-doc` passed with four existing ignored doctests and excludes
  `wireframe_testing` as documented in the Makefile.
- [x] (2026-10-27) The lockfile inventory contains only v0.6.0 BDD-family
  packages and no prereleases; the independent `wireframe_testing` lockfile
  contains no BDD package. The v0.6 macro brings in the base harness and its
  `cargo_metadata` dependency, upgrades Gherkin to 0.16 and its typed-builder
  dependencies to 0.23, and unifies `cap-std` on the existing 4.0.3 version.
  Older 0.5-only dependencies were removed. No harness was added directly.
- [x] (2026-10-27) `make markdownlint` stopped at its spelling prerequisite on
  three Oxford-spelling corrections in this plan. Corrected those words, and
  the rerun of `make markdownlint` and `make nixie` passed.
- [x] (2026-10-27) Locked inverse Cargo trees confirm the runtime and macros
  are root development dependencies and the v0.6.0 base harness is a transitive
  dependency of the macros. Both imported guide hashes still match.
- [x] (2026-10-27) Committed the reviewed migration as `80b316d6`, pushed the
  requested branch, and opened draft PR
  [#811](https://github.com/leynos/wireframe/pull/811). The branch tracks
  `origin/adopt-rstest-bdd-v0-6-0`.
- [x] (2026-10-27) The optional CodeGraph branch-context query returned
  `Too many open files`; the independent source review and required gates
  completed separately.

## Imported-document provenance

The upstream repository is `https://github.com/leynos/rstest-bdd`. The fetched
tag `v0.6.0` resolves to commit `72fb22635670e456545ca368805ba4c1c9d7bd69`,
matching the commit supplied by the user. Both files were read with
`git show <commit>:<path>` from that immutable commit.

| Original path at the pinned commit | Local destination                           | SHA-256                                                            |
| ---------------------------------- | ------------------------------------------- | ------------------------------------------------------------------ |
| `docs/users-guide.md`              | `docs/rstest-bdd-users-guide.md`            | `1e9f4d1b6607fdf83df979676f1a682218cd6751381d374cb98b701778b798b2` |
| `docs/v0-6-0-migration-guide.md`   | `docs/rstest-bdd-v0-6-0-migration-guide.md` | `6e76c10028f9962134f6158732cf1645ae1b0beba3018bccd3bc3e88b2c38985` |

The prior Wireframe-adapted `docs/rstest-bdd-users-guide.md` was committed
before this task and is preserved at
`docs/wireframe-rstest-bdd-users-guide.md`. Its local Rust-version paragraph
was updated to reflect the new root package floor; the upstream copies are
separate and unchanged. Imported relative references map as follows; no
imported bytes are edited to rewrite links.

| Relative upstream reference                                 | Local or immutable upstream destination                                                                                                                               |
| ----------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `v0-6-0-migration-guide.md` from the user guide             | `docs/rstest-bdd-v0-6-0-migration-guide.md`                                                                                                                           |
| `users-guide.md` from the migration guide                   | `docs/rstest-bdd-users-guide.md`                                                                                                                                      |
| `developers-guide.md`                                       | [upstream developers' guide](https://github.com/leynos/rstest-bdd/blob/72fb22635670e456545ca368805ba4c1c9d7bd69/docs/developers-guide.md)                             |
| `rstest-bdd-design.md`                                      | [upstream design](https://github.com/leynos/rstest-bdd/blob/72fb22635670e456545ca368805ba4c1c9d7bd69/docs/rstest-bdd-design.md)                                       |
| `adr-006-fallible-scenario-functions.md`                    | [upstream ADR 006](https://github.com/leynos/rstest-bdd/blob/72fb22635670e456545ca368805ba4c1c9d7bd69/docs/adr-006-fallible-scenario-functions.md)                    |
| `adr-007-harness-context-injection.md`                      | [upstream ADR 007](https://github.com/leynos/rstest-bdd/blob/72fb22635670e456545ca368805ba4c1c9d7bd69/docs/adr-007-harness-context-injection.md)                      |
| `adr-009-consistent-implicit-fixture-name-normalization.md` | [upstream ADR 009](https://github.com/leynos/rstest-bdd/blob/72fb22635670e456545ca368805ba4c1c9d7bd69/docs/adr-009-consistent-implicit-fixture-name-normalization.md) |
| `adr-012-guard-based-stepcontext-borrowing.md`              | [upstream ADR 012](https://github.com/leynos/rstest-bdd/blob/72fb22635670e456545ca368805ba4c1c9d7bd69/docs/adr-012-guard-based-stepcontext-borrowing.md); v0.7-only   |

The target source is authoritative where the copied guide includes later
material: the v0.6.0 commit contains `#[harness_context]`, but Wireframe does
not use harness context injection; v0.7 guard-based borrowing is not part of
this migration. The tagged source and published v0.6.0 package copies of
`crates/rstest-bdd-macros/src/codegen/wrapper/equivalence_tests.rs` are
byte-identical (SHA-256
`a05fa5e5158e72e3a11742c43b307643e107238acc2bbf1d5b84c77d89274119`); its
equivalence test proves `#[from(rstest_bdd_harness_context)]` is accepted. The
tagged and published `crates/rstest-bdd/tests/result_fixture.rs` copies are
also byte-identical (SHA-256
`afefd64ad7f9c4e729ab5d7b8ed8fae3fa9eb4fc4d4281fc66988cedcdcbfe28`); its
scenario follows the guide's owned `Result<T, E>` parameter and fallible
`Result<(), E>` return without reading the fixture parameter in the scenario
body. The crates.io package metadata independently reports Rust 1.88 for
`rstest-bdd`, `rstest-bdd-macros`, and `rstest-bdd-harness-tokio`. Existing
`AtomicU64::try_update` calls in `src/client/pool/lease.rs` and
`src/fragment/fragmenter.rs` are stable since Rust 1.95, as confirmed by
`clippy::incompatible-msrv`; the root package therefore declares 1.95. Rust
1.95.0 is installed locally. The baseline compiler is
`rustc 1.101.0-nightly (1a2c4797e 2026-10-21)`, selected by
`rust-toolchain.toml` channel `nightly-2026-10-22`. The workspace has four
members, and `tests/feature_gate` is a standalone package with its own
manifest. The root and `wireframe_testing` lockfiles are maintained
independently. No baseline package declared `rust-version`; the candidate root
package now declares 1.95, while other package floors remain unchanged.

## Applicability matrix

| Upstream change area                                                   | Wireframe evidence                                                                           | Action                                                                       |
| ---------------------------------------------------------------------- | -------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------- |
| Owned fallible fixture unwrapping and scenario `Result<(), E>` returns | Nine scenarios use `TestResult<World>` fixture aliases                                       | Spell `Result<World, TestError>` and return explicit `Result<(), TestError>` |
| Aliased step `Result<T, E>` dispatch                                   | Step functions use the `TestResult` alias; `TestError` derives `Display` through `thiserror` | Verify propagation and `Ok(T)` fixture override with regressions             |
| Fixture-name normalization and explicit `#[from(...)]` names           | No underscore-prefixed BDD scenario parameters or `#[from]` selectors found                  | Not applicable                                                               |
| `InsertOutcome`, bypass reporting, and direct `StepContext` APIs       | No direct callers found in owned test source                                                 | Not applicable unless further inventory finds use                            |
| Reporting or language-server API changes                               | No direct report, snapshot, or language-server integration found                             | Not applicable                                                               |
| Tokio harness                                                          | 24 existing async scenarios use explicit `#[tokio::test(flavor = "current_thread")]`         | Keep genuinely async tests external to the synchronous harness               |
| Harness-context injection                                              | No harness-context markers found in Wireframe tests                                          | Not applicable                                                               |
| v0.7 guard borrowing and lifecycle guarantees                          | Outside the target release                                                                   | Do not adopt                                                                 |

## Decision log

- **D-1:** Use published v0.6.0 dependencies with the repository's plain
  semver requirement convention, then update lockfiles through targeted Cargo
  resolution. Do not treat `0.6.0` as an exact pin.
- **D-2:** Keep the 24 explicitly asynchronous scenarios on their existing
  Tokio test attributes. The v0.6 Tokio harness executes synchronous scenario
  closures and is not a replacement for async steps that can yield.
- **D-3:** Preserve the former Wireframe guide at a new name and make the
  imported upstream guide the canonical `rstest-bdd` user guide path.
- **D-4:** Declare Rust 1.95 for the root package. The v0.6.0 BDD dependency
  graph requires only Rust 1.88, but existing `AtomicU64::try_update` calls
  require 1.95. Retain the mainline nightly pin and components; mainline later
  advanced the pin from `nightly-2026-10-22` to `nightly-2026-10-27`.

## Conformance basis

The migration follows the root `AGENTS.md`, the accepted scope in
`docs/adr-003-replace-cucumber-with-rstest-bdd.md`, and the immutable upstream
v0.6.0 user and migration guides recorded above. The repository's current BDD
architecture and test policy are the governing local design; no new public
runtime interface is planned.

## Verification plan

Capture baseline and final feature-file, scenario-annotation, async-scenario,
and expected-skip counts. Record the baseline commit and full `rustc -Vv`
identity. Validate dependency constraints and resolved versions for each
workspace and standalone manifest, and inspect `cargo tree` for duplicate or
stale BDD-family packages.

Run the repository's canonical formatting, lint, behavioural, unit,
integration, documentation, UI, feature-gate, and applicable verification
commands sequentially through `scrutineer`. Preserve pipeline status in every
log. The final evidence must include the failing-before/passing-after
regression for an aliased step error and a successful fallible fixture value,
plus any unavailable check and every residual failure.

## Outcomes & retrospective

Implementation and validation are complete. The two new regression scenarios
account for the BDD inventory increase. Draft PR #811 is open for review.
