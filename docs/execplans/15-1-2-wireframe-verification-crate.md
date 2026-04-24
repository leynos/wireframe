# Add the internal `wireframe-verification` crate

This ExecPlan is a living document. The sections `Constraints`, `Tolerances`,
`Risks`, `Progress`, `Surprises & Discoveries`, `Decision Log`, and
`Outcomes & Retrospective` must be kept up to date as work proceeds.

Status: IMPLEMENTED (environment-limited validation)

This document must be maintained in accordance with `AGENTS.md` at the
repository root, including quality gates, test policy, and documentation style
requirements.

## Purpose / big picture

Roadmap item 15.1.2 adds `crates/wireframe-verification` as the dedicated home
for Stateright models and shared bounded-checker helpers.

After this change:

- the root workspace explicitly includes
  `crates/wireframe-verification`,
- the root `wireframe` package remains the only default workspace member, so
  ordinary root `cargo test`, `cargo check`, and `cargo clippy` still target
  the main crate by default,
- contributors can run `cargo test -p wireframe-verification` and get a
  passing placeholder Stateright model backed by a shared harness that later
  roadmap items can extend toward the real `ConnectionActor`.

Observable success is:

- `cargo metadata --no-deps --format-version 1` reports
  `wireframe-verification` in `workspace_members`,
- `cargo test -p wireframe-verification` passes,
- the workspace-manifest regression tests and BDD coverage reflect the
  post-15.1.2 contract,
- `docs/formal-verification-methods-in-wireframe.md`,
  `docs/developers-guide.md`, and `docs/roadmap.md` reflect the implemented
  state.

## Constraints

- Keep the root `wireframe` package as the sole `default-members` entry in the
  root `Cargo.toml`.
- Keep the new crate internal with `publish = false`.
- Treat 15.1.2 as crate-and-doc infrastructure only. Do not add Makefile
  targets, continuous integration plumbing, Kani tooling, or Verus tooling
  owned by later roadmap items.
- Keep the first Stateright model intentionally small and semantic. It should
  prove the crate and harness shape, not attempt a full `ConnectionActor`
  reimplementation.
- Use `rstest` for the new crate's automated tests.
- Keep each Rust file under the 400-line repository cap.
- Preserve the existing `wireframe_testing` helper-crate explanation in the
  developer docs while extending it with the new verification-crate behaviour.

## Tolerances (exception triggers)

- If adding the verification crate forces plain root-level Cargo commands to
  stop targeting the root `wireframe` package by default, stop and reassess.
- If the Stateright crate requires Makefile or CI changes to be testable
  locally, stop and defer that plumbing to roadmap items 15.1.4 and 15.1.5.
- If the placeholder model cannot be expressed without depending on unstable
  or undocumented Stateright APIs, stop and record the blocker.
- If repository quality gates reveal unrelated pre-existing failures, record
  them separately and continue only once it is clear whether 15.1.2 caused the
  regression.

## Risks

- Risk: the formal-verification guide still shows illustrative snippets that
  may drift from the currently available Stateright release. Severity: medium.
  Likelihood: medium. Mitigation: update the guide to match the version that
  actually resolves in this repository.

- Risk: the guide's example `wireframe` dependency disables default features,
  but the current root crate does not compile under that configuration.
  Severity: medium. Likelihood: high. Mitigation: use the normal `wireframe`
  feature set for this infrastructure step and record the mismatch as a future
  follow-up for the main crate rather than forcing unrelated feature work into
  15.1.2.

- Risk: workspace-manifest tests written for 10.1.1 can become false negatives
  once the verification crate lands. Severity: high. Likelihood: high.
  Mitigation: update both the `rstest` and `rstest-bdd` coverage in the same
  change as the manifest edit.

## Progress

- [x] (2026-04-24 00:00 UTC) Recalled project memory, reviewed the formal
      verification guide, the existing hybrid-workspace tests, and the root
      manifest.
- [x] (2026-04-24 00:10 UTC) Restored this missing 15.1.2 ExecPlan as the
      living implementation record for the work.
- [x] (2026-04-24 00:20 UTC) Added `crates/wireframe-verification`,
      introduced the shared Stateright harness, and added the placeholder
      connection-model tests.
- [x] (2026-04-24 00:30 UTC) Updated the root workspace membership and flipped
      the manifest-contract tests and BDD scenario to the post-15.1.2 state.
- [x] (2026-04-24 01:25 UTC) Finished documentation updates, ran the required
      validation, and recorded the final results and baseline blockers.

## Surprises & Discoveries

- Discovery: the draft ExecPlan file discussed in the previous session was not
  present in the repository, so 15.1.2 implementation had to restore the living
  document before coding could proceed.

- Discovery: the current crates.io release for Stateright is `0.31.0`, not the
  `0.30` shown in the formal-verification guide example.

- Discovery: the reduced-feature `wireframe` dependency example does not
  compile today. The main crate uses `tokio_util::io::StreamReader`, so it
  still needs the `tokio-util` `io` feature when default features are disabled.
  The verification crate therefore uses the normal `wireframe` feature set for
  now.

- Discovery: the first placeholder model accidentally encoded
  "at most one multi-packet terminator ever" rather than "at most one
  terminator per multi-packet session". A Stateright counterexample exposed
  that difference immediately, and the model was corrected by resetting the
  per-session terminator counter when a new multi-packet session starts.

## Decision Log

- Decision: implement a small semantic placeholder model rather than jumping
  straight to a detailed `ConnectionActor` translation. Rationale: 15.1.2 owns
  the crate boundary and shared harness, while later roadmap items own the
  higher-fidelity model. Date/Author: 2026-04-24 / Codex.

- Decision: use `stateright = "0.31.0"` because that is the current published
  release that resolves in this repository today. Rationale: repository docs
  should follow the version contributors can actually install. Date/Author:
  2026-04-24 / Codex.

- Decision: keep `default-members = ["."]` unchanged while extending
  `members` to include `crates/wireframe-verification`. Rationale: preserve the
  day-to-day ergonomics established in 15.1.1 and match the roadmap's staged
  workspace design. Date/Author: 2026-04-24 / Codex.

- Decision: depend on `wireframe` with its normal default feature set from the
  verification crate. Rationale: the reduced-feature example in the design doc
  is not yet compatible with the current main crate, and fixing that would
  exceed 15.1.2 scope. Date/Author: 2026-04-24 / Codex.

## Outcomes & Retrospective

- Implemented `crates/wireframe-verification` with a shared bounded Stateright
  harness, a placeholder semantic connection model, and two `rstest`
  integration tests that prove both the harness and the placeholder model.

- Updated the root workspace to include
  `crates/wireframe-verification` while keeping `default-members = ["."]`, and
  flipped the workspace-manifest `rstest` and `rstest-bdd` coverage from the
  old "verification crate absent" expectation to the new
  "verification crate present without widened defaults" contract.

- Updated `docs/developers-guide.md`,
  `docs/formal-verification-methods-in-wireframe.md`, and `docs/roadmap.md`
  so the repository documentation matches the implemented workspace state and
  the current Stateright version.

- Validation results:
  `make fmt` passed.
  `make check-fmt` passed.
  `make markdownlint` passed.
  `make nixie` passed.
  `cargo test -p wireframe-verification` passed.
  `cargo clippy -p wireframe-verification --all-targets -- -D warnings`
  passed.
  `cargo test --test workspace_manifest` passed.
  `cargo test --test bdd --features advanced-tests workspace_manifest` passed.
  `make test` passed.
  `make test-doc` passed.
  `make doctest-benchmark` passed.

- Environment-limited or baseline blockers:
  `make lint` failed in the Whitaker phase on pre-existing `expect` usage
  findings in the main crate, including `src/fairness.rs`,
  `src/fragment/payload.rs`, `src/message_assembler/tests.rs`,
  `src/message_assembler/state_tests.rs`,
  `src/message_assembler/budget_tests.rs`, and
  `src/server/runtime/tests.rs`.
  `cargo test --workspace` reached the newly added verification crate and the
  root crate successfully, but then failed on pre-existing `wireframe_testing`
  doctest inference and visibility errors in
  `wireframe_testing/src/client_pair.rs`,
  `wireframe_testing/src/helpers/codec_drive.rs`,
  `wireframe_testing/src/helpers/drive.rs`,
  `wireframe_testing/src/helpers/payloads.rs`,
  `wireframe_testing/src/helpers/runtime.rs`, and
  `wireframe_testing/src/multi_packet.rs`.
