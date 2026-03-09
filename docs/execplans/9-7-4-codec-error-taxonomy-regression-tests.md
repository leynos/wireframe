# 9.7.4 Add codec-error regression tests backed by `wireframe_testing`

This ExecPlan (execution plan) is a living document. The sections
`Constraints`, `Tolerances`, `Risks`, `Progress`, `Surprises & Discoveries`,
`Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work
proceeds.

Status: COMPLETE

## Purpose / big picture

Roadmap item 9.1.2 introduced the `CodecError` taxonomy, default recovery
policies, EOF classification, and codec-error observability labels. Those
behaviours are already covered by direct crate tests, but phase 9.7 is about
hardening the reusable testing surface. After this change, a contributor will
be able to use `wireframe_testing` to drive malformed or partial codec inputs,
assert the resulting taxonomy and recovery-policy decisions, and verify the
expected `wireframe_codec_errors_total{error_type=...,recovery_policy=...}`
labels without rebuilding ad hoc test scaffolding.

Observable success has three parts. First, new `rstest` regression tests in the
main crate's `tests/` directory fail before implementation and pass after the
helpers or fixtures are finished. Second, new `rstest-bdd` v0.5.0 behavioural
scenarios prove the same behaviour through the public testing surface. Third,
the relevant design documentation is updated, `docs/users-guide.md` is amended
if the public test-support interface changes, and `docs/roadmap.md` marks 9.7.4
as done only after all quality gates pass.

## Constraints

- Treat roadmap item 9.7.4 as a hardening task for the public
  `wireframe_testing` testing surface. The plan may add small, additive helper
  APIs, but it must not change the semantics of `CodecError`, `RecoveryPolicy`,
  or EOF handling defined by 9.1.2.
- `wireframe_testing` is a dev-dependency, not a workspace member. Executable
  tests for that crate must live under the repository-root `tests/` directory,
  even though the roadmap says "in `wireframe_testing`".
- Use `rstest` for the regression test matrix and `rstest-bdd` v0.5.0 for
  behavioural scenarios. Follow the existing four-file BDD pattern: `.feature`,
  fixture, steps, and scenario registration.
- Keep all touched Rust source files below 400 lines. Split modules early if a
  new or modified file approaches 350 lines.
- Do not add new external dependencies.
- Keep comments and documentation in en-GB-oxendict spelling.
- If any new public helper is exported from `wireframe_testing/src/lib.rs` or a
  consumer-visible behaviour changes, update `docs/users-guide.md`.
- Record any lasting design choice in the most relevant design document. For
  this work, that is expected to be `docs/wireframe-testing-crate.md`; update
  `docs/adr-004-pluggable-protocol-codecs.md` only if taxonomy or policy
  semantics themselves need clarification.
- Do not mark roadmap item 9.7.4 done until the new tests, documentation, and
  validation commands are all green.

## Tolerances (exception triggers)

- Scope: if implementation needs more than 12 files changed or more than 900
  net lines, stop and re-scope before continuing.
- Interface: if the work cannot be completed without changing an existing
  public API signature in `wireframe` or `wireframe_testing`, stop and
  escalate. Adding a narrowly scoped new helper is allowed; changing or
  removing an existing one is not.
- Semantics: if the new regressions reveal that the current
  `CodecError::default_recovery_policy()` mapping or EOF classification is
  wrong and must be changed, stop and escalate because that exceeds a pure
  regression task.
- Dependencies: if any new crate appears necessary, stop and escalate.
- Iterations: if the same root-cause failure survives three fix attempts, stop,
  document the options in `Decision Log`, and ask for direction.
- Documentation spread: if clarifying the feature requires edits to more than
  four Markdown documents, pause and confirm the intended documentation scope.

## Risks

- Risk: the phrase "regression tests in `wireframe_testing`" is ambiguous
  because that crate's internal `#[cfg(test)]` modules are not exercised by the
  workspace test targets. Severity: medium. Likelihood: high. Mitigation:
  implement executable regressions under root `tests/` while importing only
  public `wireframe_testing` APIs, and explain this explicitly in the plan and
  the final documentation note.

- Risk: `ObservabilityHandle` uses `metrics::with_local_recorder`, which is
  thread-local. Severity: medium. Likelihood: medium. Mitigation: keep
  observability-heavy tests on the current thread, mirror the pattern already
  used by existing BDD scenarios, and document the constraint in the new test
  fixture or helper docs if additional helpers are added.

- Risk: the repository already has direct codec-error tests
  (`tests/codec_error.rs`, `tests/features/codec_error.feature`). New coverage
  could duplicate those tests instead of validating the `wireframe_testing`
  surface. Severity: medium. Likelihood: high. Mitigation: focus the new suite
  on cases that explicitly consume `wireframe_testing` helpers such as
  malformed-frame fixtures, codec-aware decode helpers, partial-frame drivers,
  and the observability handle.

- Risk: current example codecs such as `HotlineFrameCodec` surface plain
  `io::Error` messages for some malformed-wire cases instead of constructing
  `CodecError` values directly. Severity: medium. Likelihood: medium.
  Mitigation: use the default `LengthDelimitedFrameCodec` where the regression
  needs to pin `CodecError` and EOF taxonomy, and use Hotline fixtures only
  where the regression is about the reusable malformed-wire test surface.

## Progress

- [x] (2026-03-06) Read the roadmap item, 9.1.2 ExecPlan, ADR-004, the
  hardening and testing guides, and the current `wireframe_testing` public
  surface.
- [x] (2026-03-09) Stage A: add `rstest` regression coverage in
  `tests/test_codec_error_regressions.rs`.
- [x] (2026-03-09) Stage B: confirm the existing helper surface is sufficient;
  no new public `wireframe_testing` helper API was required.
- [x] (2026-03-09) Stage C: add `rstest-bdd` scenarios and a dedicated world in
  `tests/features/codec_error_regressions.feature`,
  `tests/fixtures/codec_error_regressions.rs`,
  `tests/steps/codec_error_regressions_steps.rs`, and
  `tests/scenarios/codec_error_regressions_scenarios.rs`.
- [x] (2026-03-09) Stage D: update `docs/wireframe-testing-crate.md` and mark
  `docs/roadmap.md` item 9.7.4 done.
- [x] (2026-03-09) Stage E: run formatting, lint, unit, behavioural, and
  documentation quality gates and capture evidence.

## Surprises & Discoveries

- `wireframe_testing` already exposes the building blocks most of this feature
  needs: malformed Hotline fixtures, codec encode/decode helpers, partial-frame
  drivers, and `ObservabilityHandle`.
- The existing behavioural suite for codec errors lives in
  `tests/features/codec_error.feature`, but that world mostly builds
  `CodecError` values directly or talks to the decoder without exercising the
  `wireframe_testing` helper crate as a reusable consumer surface.
- `docs/users-guide.md` already documents the `CodecError` taxonomy,
  `RecoveryPolicyHook`, and the codec-error metric labels, so the user-guide
  delta should stay small unless a new public test helper is exported.
- The generic `wireframe_testing::decode_frames_with_codec` helper erases the
  typed decoder error into `io::Error` text, so typed EOF regressions need to
  call the default codec's decoder directly and use `wireframe_testing` only
  for framing helpers and observability capture.

## Decision Log

- Decision: interpret roadmap item 9.7.4 as "add executable regressions backed
  by `wireframe_testing` public APIs", not "place tests under
  `wireframe_testing/src`". Rationale: that is the only way for the regressions
  to run in the current workspace structure while still hardening the helper
  crate's consumer-facing surface. Date/Author: 2026-03-06 / Codex.

- Decision: prefer the default `LengthDelimitedFrameCodec` for taxonomy and EOF
  assertions, and use Hotline fixtures only for malformed-wire helper coverage.
  Rationale: the default codec is where `CodecError` and structured EOF
  handling are guaranteed today; the Hotline fixtures remain useful for the
  reusable malformed-input test surface but do not fully encode the taxonomy.
  Date/Author: 2026-03-06 / Codex.

- Decision: start without adding public helpers.
  Rationale: the repository already has `decode_frames_with_codec`,
  `drive_with_partial_frames`, `drive_with_partial_codec_frames`,
  `valid_hotline_wire`, `oversized_hotline_wire`, `truncated_hotline_header`,
  `truncated_hotline_payload`, and `ObservabilityHandle`. Only introduce a new
  helper if the red tests show repeated boilerplate that obscures the intended
  regression. Date/Author: 2026-03-06 / Codex.

- Decision: do not add a new public helper just to preserve typed `EofError`
  values. Rationale: the generic helper API is intentionally codec-agnostic and
  normalizes failures to `io::Error`; preserving typed EOF values would require
  a more opinionated helper than this roadmap item justifies. The implemented
  regressions therefore combine `wireframe_testing` framing helpers and
  observability capture with direct `LengthDelimitedFrameCodec` decoder calls.
  Date/Author: 2026-03-09 / Codex.

## Outcomes & Retrospective

Completed on 2026-03-09.

The implemented regression suite adds:

- `tests/test_codec_error_regressions.rs` with `rstest` coverage for
  representative taxonomy-to-policy mappings, typed EOF classification for the
  default codec, custom hook override labelling, and malformed Hotline fixture
  regressions; and
- a matching `rstest-bdd` suite that exercises the same behaviours through a
  dedicated world fixture and `ObservabilityHandle`.

No new public helper API was added to `wireframe_testing`, so
`docs/users-guide.md` did not need an update. The lasting design note was
recorded in `docs/wireframe-testing-crate.md`, and `docs/roadmap.md` now marks
9.7.4 done.

Validation commands and outcomes recorded for Stage E:

- `make fmt`
- `make check-fmt`
- `make lint`
- `make test`
- `make markdownlint MDLINT=/root/.bun/bin/markdownlint-cli2`

All five commands passed on 2026-03-09. No `docs/users-guide.md` update was
needed because the implementation did not change the public consumer API; it
only added regression coverage and a production bug fix that preserves the
documented `CodecError` contract.

## Context and orientation

The relevant production taxonomy lives in:

- `src/codec/error.rs` for `CodecError`, `FramingError`, `ProtocolError`, and
  `EofError`.
- `src/codec/recovery/` for `RecoveryPolicy`, `RecoveryPolicyHook`,
  `CodecErrorContext`, and `RecoveryConfig`.
- `src/metrics.rs` for `wireframe_codec_errors_total` and
  `inc_codec_error(error_type, recovery_policy)`.

The current reusable testing surface lives in:

- `wireframe_testing/src/helpers/codec_ext.rs` for generic codec
  encode/decode helpers.
- `wireframe_testing/src/helpers/partial_frame.rs` for chunked partial-frame
  delivery helpers.
- `wireframe_testing/src/helpers/codec_fixtures.rs` for malformed Hotline wire
  fixtures.
- `wireframe_testing/src/observability/mod.rs` and
  `wireframe_testing/src/observability/assertions.rs` for log and metric
  capture.

Existing direct coverage already present in the repository:

- `tests/codec_error.rs` verifies the public taxonomy directly.
- `tests/features/codec_error.feature` plus the fixture in
  `tests/fixtures/codec_error/` verify recovery defaults and EOF decoding
  behaviours.
- `tests/codec_fixtures.rs`, `tests/codec_test_harness.rs`, and
  `tests/test_observability_harness.rs` verify the helper crate in isolation.

The missing piece is a regression suite that combines those testing utilities
to pin the 9.1.2 semantics through the helper crate's public surface. That is
what 9.7.4 adds.

## Plan of work

### Stage A: create failing regression coverage first

Add a new `rstest`-based integration test file, expected to be named
`tests/test_codec_error_regressions.rs`. The file should be written first and
should fail before any helper additions are made. Keep the test matrix focused
on the 9.1.2 contract:

- taxonomy-to-policy mapping for representative cases:
  `OversizedFrame -> framing/drop`,
  `InvalidLengthEncoding -> framing/disconnect`,
  `UnknownMessageType -> protocol/drop`, `io::Error -> io/disconnect`,
  `CleanClose -> eof/disconnect`;
- EOF classification produced by the default codec for clean close, mid-header,
  and mid-frame inputs;
- regression coverage proving that `ObservabilityHandle` can record and assert
  the `error_type` and `recovery_policy` labels derived from those cases; and
- at least one malformed-wire regression that uses a `wireframe_testing`
  fixture or helper rather than bespoke byte construction.

Use `#[rstest]` parameterization instead of repeated hand-written tests. Tests
that rely on `ObservabilityHandle` should use
`#[tokio::test(flavor = "current_thread")]` when async code is involved.

### Stage B: add only the minimum missing test-support API

If Stage A shows that the existing helper surface is insufficient, add the
smallest possible additive API in `wireframe_testing`. Candidate additions are
limited to one of these shapes:

1. a tiny helper that builds or classifies expected codec-error labels from a
   `CodecError`;
2. a small malformed-length fixture for the default length-delimited codec if
   the regression cannot be expressed cleanly with current partial-frame
   helpers; or
3. a narrow observability assertion convenience that removes repeated
   label-construction boilerplate.

Do not add a new abstraction layer or a generic "codec error harness". This is
supposed to be a regression hardening task, not a framework rewrite.

If a public helper is added, update:

- `wireframe_testing/src/lib.rs` re-exports;
- helper module documentation and doctests; and
- `docs/users-guide.md` so consumers know the helper exists.

### Stage C: add behavioural coverage through `rstest-bdd`

Create a dedicated feature, expected to be named
`tests/features/codec_error_regressions.feature`, plus matching files in
`tests/fixtures/`, `tests/steps/`, and `tests/scenarios/`. Keep the scenarios
behaviour-focused and routed through the helper crate. Recommended scenarios:

1. "Oversized payload is classified as framing/drop and counted in
   observability".
2. "Clean EOF at frame boundary maps to eof/disconnect without partial-data
   loss".
3. "Partial header and partial payload closures are distinguished".
4. "A custom recovery hook can override the default policy and the overridden
   label is what the observability handle sees".

Reuse the repository's established BDD conventions:

- the fixture parameter name in every step must match the fixture function name
  exactly;
- world fixtures should use `#[rustfmt::skip]` before `#[fixture]` if they
  would otherwise be collapsed into single-line functions; and
- use current-thread Tokio tests for scenarios that record metrics.

### Stage D: update documentation and roadmap

Update the most relevant design document with any design choice that survives
implementation. The default target is `docs/wireframe-testing-crate.md`,
because this roadmap item is about how the helper crate should be used for
codec-error regressions. Update `docs/adr-004-pluggable-protocol-codecs.md`
only if implementation reveals a semantic clarification about taxonomy,
recovery defaults, or EOF interpretation that belongs in the codec ADR.

Audit `docs/users-guide.md`. If no public API or consumer-visible behaviour
changed, record that explicitly in `Outcomes & Retrospective` and leave the
file untouched. If a new helper or changed interface was exported, add the
minimum user-facing documentation necessary to explain it.

When and only when the feature is implemented and validated, change the 9.7.4
checkbox in `docs/roadmap.md` from `[ ]` to `[x]`.

### Stage E: validate red, green, and full gates

Run the new regressions in red/green order using `tee` and `set -o pipefail` so
failures are preserved in the logs.

Red phase:

```sh
set -o pipefail && cargo test --test test_codec_error_regressions 2>&1 | tee /tmp/9-7-4-red-rstest.log
set -o pipefail && cargo test --test bdd --all-features -- codec_error_regression 2>&1 | tee /tmp/9-7-4-red-bdd.log
```

Green and full validation phase:

```sh
set -o pipefail && make fmt 2>&1 | tee /tmp/9-7-4-fmt.log
set -o pipefail && make markdownlint MDLINT=/root/.bun/bin/markdownlint-cli2 2>&1 | tee /tmp/9-7-4-markdownlint.log
set -o pipefail && make check-fmt 2>&1 | tee /tmp/9-7-4-check-fmt.log
set -o pipefail && make lint 2>&1 | tee /tmp/9-7-4-lint.log
set -o pipefail && make test 2>&1 | tee /tmp/9-7-4-test.log
```

If any touched Markdown file contains Mermaid diagrams, also run:

```sh
set -o pipefail && make nixie 2>&1 | tee /tmp/9-7-4-nixie.log
```

Success criteria:

- the new `rstest` file passes;
- the new BDD scenarios pass;
- no pre-existing codec-error or helper-crate tests regress;
- formatting, lint, documentation lint, and full tests are all green; and
- the roadmap entry is updated only after those conditions are met.

## Expected file map

The likely change set should stay close to this list:

- `tests/test_codec_error_regressions.rs`
- `tests/features/codec_error_regressions.feature`
- `tests/fixtures/codec_error_regressions.rs`
- `tests/steps/codec_error_regressions_steps.rs`
- `tests/scenarios/codec_error_regressions_scenarios.rs`
- `tests/fixtures/mod.rs`
- `tests/steps/mod.rs`
- `tests/scenarios/mod.rs`
- optionally one or two small `wireframe_testing/src/helpers/*.rs` or
  `wireframe_testing/src/observability/*.rs` files if Stage B proves they are
  necessary
- `wireframe_testing/src/lib.rs` only if a new helper is re-exported
- `docs/wireframe-testing-crate.md`
- `docs/users-guide.md` only if the public interface changes
- `docs/adr-004-pluggable-protocol-codecs.md` only if codec semantics need a
  clarified design note
- `docs/roadmap.md`

## Approval gate

This file is the draft phase only. Do not begin implementation until the user
explicitly approves the plan or requests revisions. If implementation later
discovers that one of the tolerances would be exceeded, stop, update this
document, and escalate before continuing.
