# 9.6.1 Codec performance benchmarks for throughput, latency, and allocations

This execution plan (ExecPlan) is a living document. The sections
`Constraints`, `Tolerances`, `Risks`, `Progress`, `Surprises and discoveries`,
`Decision log`, and `Outcomes and retrospective` must be kept up to date as
work proceeds.

Status: COMPLETE (2026-02-23)

No `PLANS.md` exists in this repository as of 2026-02-23.

## Purpose / big picture

Roadmap item `9.6.1` closes the remaining benchmark work in phase 9 by adding
repeatable codec measurements for throughput, latency, fragmentation overhead,
and allocation baselines. The benchmark surface must cover both the default
`LengthDelimitedFrameCodec` and one custom codec, and it must produce outputs
that can be compared across future runs to detect regressions.

After this work, maintainers can observe success by running the benchmark
command and seeing:

- Encode and decode latency and throughput for small and large payloads across
  `LengthDelimitedFrameCodec` and `HotlineFrameCodec`.
- A measured ratio comparing fragmented and unfragmented outbound paths.
- Allocation baseline counters for payload wrapping and decode extraction paths.
- Unit validation implemented with `rstest`.
- Behavioural validation implemented with `rstest-bdd` v0.5.0.
- Design decisions recorded in `docs/adr-004-pluggable-protocol-codecs.md`.
- `docs/users-guide.md` updated if any library-consumer public interface
  changes.
- Roadmap item `9.6.1` and its child bullets marked done only after all
  quality gates pass.

## Constraints

- Keep all existing public APIs source-compatible unless a benchmark support
  API must be exposed. If any public interface changes, document it in
  `docs/users-guide.md`.
- Use `rstest` for new unit tests and `rstest-bdd` v0.5.0 for new behavioural
  tests.
- Prefer existing custom codecs already in-tree; use
  `wireframe::codec::examples::HotlineFrameCodec` as the custom codec target
  for this item.
- Benchmark both small and large frames for encode and decode paths.
- Include fragmentation overhead measurement against an unfragmented baseline
  for equivalent payload classes.
- Record allocation baselines for payload wrapping and decode extraction.
- Record benchmark design decisions in
  `docs/adr-004-pluggable-protocol-codecs.md`. If fragmentation benchmark
  thresholds or interpretation rules are introduced, update
  `docs/generic-message-fragmentation-and-re-assembly-design.md`.
- Keep benchmark execution deterministic enough for local comparison by fixing
  sample sizes and avoiding wall-clock assertions in tests.
- Add or update Makefile targets so benchmark commands are discoverable via
  `make help`.
- Mark roadmap item `9.6.1` and all nested bullets as done in
  `docs/roadmap.md` only after implementation, docs, and validations succeed.
- Run all applicable quality gates with `set -o pipefail` and `tee` logging.

## Tolerances (exception triggers)

- Scope: if implementation requires more than 16 files or 1,000 net changed
  lines, stop and re-scope.
- Dependencies: if adding benchmark support requires crates beyond `criterion`
  and one allocation-counter helper crate, stop and escalate.
- Runtime: if a single benchmark target exceeds 6 minutes on a normal local
  run, stop and revise benchmark sample sizes.
- Stability: if benchmark variance (coefficient of variation) remains above
  20% after warm-up/sample tuning, stop and document mitigation options.
- Interface: if benchmark helpers force unavoidable public API exposure, stop
  and present API alternatives before merging.
- Iterations: if the same failing root cause persists after 3 fixes, stop and
  document options in `Decision log`.

## Risks

- Risk: microbenchmark results can be noisy across machines and produce false
  alarms. Severity: high. Likelihood: medium. Mitigation: compare relative
  ratios in docs, pin benchmark config, and avoid test assertions on raw timing
  values.

- Risk: allocation counting can perturb runtime measurements. Severity: medium.
  Likelihood: medium. Mitigation: keep allocation baselines in dedicated runs
  and separate them from primary throughput timing loops.

- Risk: behavioural tests can become flaky if they execute full benchmark
  loops. Severity: medium. Likelihood: medium. Mitigation: run reduced,
  deterministic sample loops in BDD fixtures and validate structure, not
  absolute timing.

- Risk: fragmentation overhead benchmark may measure helper overhead rather
  than end-to-end pipeline cost. Severity: medium. Likelihood: medium.
  Mitigation: benchmark the same payload through equivalent outbound prep paths
  and clearly document what is and is not included.

## Progress

- [x] (2026-02-23 00:00Z) Drafted ExecPlan for roadmap item `9.6.1`, including
      staged benchmark, testing, documentation, and validation scope.
- [x] (2026-02-23 01:30Z) Stage A complete: benchmark workload matrix and
      shared benchmark support helpers implemented in
      `tests/common/codec_benchmark_support.rs`.
- [x] (2026-02-23 02:10Z) Stage B complete: criterion benchmarks implemented in
      `benches/codec_performance.rs` and `benches/codec_performance_alloc.rs`,
      with `make bench-codec` wired in `Makefile`.
- [x] (2026-02-23 02:25Z) Stage C complete: rstest unit coverage added in
      `tests/codec_performance_benchmark_helpers.rs`.
- [x] (2026-02-23 02:40Z) Stage D complete: rstest-bdd feature, fixture, step,
      and scenario coverage added for benchmark behaviour validation.
- [x] (2026-02-23 02:55Z) Stage E complete: updated
      `docs/adr-004-pluggable-protocol-codecs.md` and marked roadmap item
      `9.6.1` done in `docs/roadmap.md`.
- [x] (2026-02-23 03:50Z) Stage F complete: all required gates passed
      (`make fmt`, `make check-fmt`, `make markdownlint`, `make nixie`,
      `make lint`, `make test-bdd`, `make test`, `make bench-codec`).

## Surprises and discoveries

- Observation: no `PLANS.md` exists. Evidence: repository root check on
  2026-02-23. Impact: this document is the sole execution plan source.

- Observation: there is currently no `benches/` directory and no benchmark
  target in `Makefile`. Evidence: repository file listing and Makefile review.
  Impact: this item must introduce benchmark wiring from scratch.

- Observation: `criterion` is referenced in design documents but not present in
  `Cargo.toml` today. Evidence: `Cargo.toml` search and docs references.
  Impact: benchmark dependency and target wiring must be added explicitly.

- Observation: custom codecs already exist in `src/codec/examples.rs`.
  Evidence: `HotlineFrameCodec` and `MysqlFrameCodec` implementations. Impact:
  no new protocol codec implementation is needed for benchmark coverage.

- Observation: Qdrant memory MCP tools (`qdrant-find`, `qdrant-store`) are not
  exposed in this environment. Evidence: MCP resource/template discovery
  returned empty results. Impact: no project-memory retrieval could be done for
  this draft.

- Observation: strict Clippy policy applies to bench targets as part of
  `make lint`, including missing docs and anti-truncation/division rules.
  Evidence: initial `make lint` failures in `benches/codec_performance.rs` and
  shared support code. Impact: benchmarks needed helper refactors (e.g.
  `Duration::checked_div`, `Duration::div_duration_f64`) and explicit `main()`
  entrypoints instead of criterion macro-generated undocumented functions.

## Decision log

- Decision: use `HotlineFrameCodec` as the custom codec benchmark target for
  9.6.1. Rationale: it is already maintained in-tree and exercises non-trivial
  header parsing/wrapping without introducing new test fixtures. Date/Author:
  2026-02-23 / Codex.

- Decision: add a dedicated benchmark entrypoint and Makefile target
  (`make bench-codec`) instead of ad hoc `cargo bench` commands. Rationale:
  keeps benchmark execution aligned with repository command conventions and
  discoverable for maintainers. Date/Author: 2026-02-23 / Codex.

- Decision: keep behavioural and unit tests focused on benchmark harness
  correctness and coverage matrix construction, not absolute timing thresholds.
  Rationale: timing thresholds are hardware-dependent and unsuitable for
  deterministic CI assertions. Date/Author: 2026-02-23 / Codex.

- Decision: implement criterion entrypoints using explicit documented `main()`
  functions rather than `criterion_group!`/`criterion_main!` macros to satisfy
  strict `missing_docs` linting on bench targets. Rationale: macro output
  functions were flagged by lint gates and macro-level doc attributes are
  ignored by rustc. Date/Author: 2026-02-23 / Codex.

## Outcomes and retrospective

Roadmap item `9.6.1` is implemented and validated.

Delivered outcomes:

- Added benchmark workload support helpers in
  `tests/common/codec_benchmark_support.rs`.
- Added throughput/latency and fragmentation-overhead benchmarks in
  `benches/codec_performance.rs`.
- Added allocation-baseline benchmark reporting in
  `benches/codec_performance_alloc.rs`.
- Added benchmark command wiring in `Makefile` via `bench-codec`.
- Added rstest unit validation in
  `tests/codec_performance_benchmark_helpers.rs`.
- Added rstest-bdd behavioural validation:
  `tests/features/codec_performance_benchmarks.feature`,
  `tests/fixtures/codec_performance_benchmarks.rs`,
  `tests/steps/codec_performance_benchmarks_steps.rs`,
  `tests/scenarios/codec_performance_benchmarks_scenarios.rs`, plus module
  wiring updates.
- Updated design decision record in
  `docs/adr-004-pluggable-protocol-codecs.md`.
- Marked roadmap item `9.6.1` complete in `docs/roadmap.md`.

Additional quality fix completed during validation:

- Resolved pre-existing `unused Result` failures in interleaved push queue
  tests by handling `push_expect!` results correctly in
  `tests/interleaved_push_queues.rs` and
  `tests/fixtures/interleaved_push_queues.rs`.

Public API impact:

- No library-consumer public runtime API changes were introduced, so
  `docs/users-guide.md` required no interface update for this item.

## Context and orientation

Primary implementation files and expected touch points:

- `Cargo.toml` for benchmark dependencies and bench target registration.
- `Makefile` for a benchmark target (`bench-codec`) with consistent flags.
- `benches/codec_performance.rs` for throughput and latency benchmarks.
- `benches/codec_performance_alloc.rs` or helper module for allocation
  baseline measurements.
- `src/codec/examples.rs` for custom codec under test (`HotlineFrameCodec`).
- `src/codec.rs` and `src/fragment/payload.rs` for benchmarked operations.
- `tests/` modules for rstest and rstest-bdd verification of benchmark harness
  behaviour.
- `docs/adr-004-pluggable-protocol-codecs.md` for benchmark design decisions.
- `docs/generic-message-fragmentation-and-re-assembly-design.md` for
  fragmentation overhead interpretation if thresholds are introduced.
- `docs/users-guide.md` for any public-interface updates required by the
  implementation.
- `docs/roadmap.md` to mark `9.6.1` complete.

Reference documents to keep aligned while implementing:

- `docs/generic-message-fragmentation-and-re-assembly-design.md`
- `docs/multi-packet-and-streaming-responses-design.md`
- `docs/the-road-to-wireframe-1-0-feature-set-philosophy-and-capability-maturity.md`
- `docs/hardening-wireframe-a-guide-to-production-resilience.md`
- `docs/rust-testing-with-rstest-fixtures.md`
- `docs/reliable-testing-in-rust-via-dependency-injection.md`
- `docs/rstest-bdd-users-guide.md`
- `docs/rust-doctest-dry-guide.md`

## Plan of work

### Stage A: define benchmark contract and workload matrix

Define benchmark cases and helper interfaces before implementing benches.
Specify payload classes and operation matrix:

- Payload sizes: small (32 bytes) and large (64 KiB) frames.
- Codecs: `LengthDelimitedFrameCodec` and `HotlineFrameCodec`.
- Operations: encode, decode, fragmentation-prep, and allocation baselines for
  wrap and decode extraction.

Planned edits:

- Add benchmark config constants and helper functions in benchmark modules.
- Add comments documenting what each benchmark includes and excludes.

Stage A acceptance:

- Benchmark matrix is fully defined in code and mirrored in doc comments.
- No public API change required at this stage.

Go/no-go:

Proceed only if the matrix satisfies every roadmap bullet under 9.6.1.

### Stage B: implement benchmark harness and Makefile target

Create criterion-driven benchmark targets and a single Makefile entrypoint.

Planned edits:

- Add `criterion` (and, if needed, one allocation helper crate) to
  `[dev-dependencies]` in `Cargo.toml`.
- Register benchmark targets in `Cargo.toml` (`[[bench]]`, `harness = false`).
- Add benchmark source files under `benches/`.
- Add `bench-codec` target in `Makefile` to run codec benchmarks.

Benchmark groups to implement:

- Encode throughput and latency for small and large payloads across default and
  custom codecs.
- Decode throughput and latency for the same matrix.
- Fragmentation overhead benchmark comparing fragmented path vs direct
  unfragmented wrapping for equivalent payloads.
- Allocation baselines for payload wrapping and payload extraction during
  decode.

Stage B acceptance:

- `make bench-codec` executes and emits grouped benchmark output.
- Output includes all roadmap-required benchmark categories.

Go/no-go:

Proceed only if benchmark runtime is within tolerance and output labels are
clear enough for future baseline comparison.

### Stage C: add rstest unit validation for benchmark support

Add unit tests that validate benchmark harness correctness independent of
machine speed.

Planned edits:

- Add `rstest` test modules for benchmark helper logic and workload matrix
  generation.
- Validate small/large payload classification, codec selection coverage,
  fragmentation path selection, and allocation counter reset semantics.

Stage C acceptance:

- Unit tests prove the benchmark harness executes every required case.
- No assertions depend on wall-clock timing.

### Stage D: add rstest-bdd behavioural validation

Add behavioural tests that exercise the benchmark harness as an observable
workflow.

Planned edits:

- `tests/features/codec_performance_benchmarks.feature`
- `tests/fixtures/codec_performance_benchmarks.rs`
- `tests/steps/codec_performance_benchmarks_steps.rs`
- `tests/scenarios/codec_performance_benchmarks_scenarios.rs`
- Module wiring updates in `tests/fixtures/mod.rs`, `tests/steps/mod.rs`, and
  `tests/scenarios/mod.rs`

Behaviour scenarios:

- Running the benchmark world produces encode/decode measurements for small and
  large frames across both codecs.
- Fragmentation overhead ratio is computed and reported.
- Allocation baseline report includes wrap and decode counters.

Stage D acceptance:

- `make test-bdd` passes with the new scenarios.
- Scenarios validate observable benchmark outputs, not raw performance numbers.

### Stage E: documentation and roadmap completion

Record design decisions and update roadmap/documentation after implementation.

Planned edits:

- Update `docs/adr-004-pluggable-protocol-codecs.md` with benchmark design
  decisions, measurement method, and baseline interpretation notes.
- Update `docs/generic-message-fragmentation-and-re-assembly-design.md` if
  fragmentation-overhead criteria are refined.
- Update `docs/users-guide.md` if any library-consumer public interface changes
  occurred; otherwise note explicitly in this ExecPlan that no public API
  change was introduced.
- Mark `9.6.1` and all nested checklist items done in `docs/roadmap.md`.

Stage E acceptance:

- Relevant design docs reflect the chosen benchmark policy.
- Roadmap is updated only after implementation and validation are complete.

### Stage F: full validation, evidence capture, and completion

Run quality gates and benchmark commands with logging as required by project
policy.

Required commands:

<!-- markdownlint-disable MD046 -->
```shell
    set -o pipefail; make fmt 2>&1 | tee /tmp/9-6-1-fmt.log
    set -o pipefail; make check-fmt 2>&1 | tee /tmp/9-6-1-check-fmt.log
    set -o pipefail; make markdownlint 2>&1 | tee /tmp/9-6-1-markdownlint.log
    set -o pipefail; make nixie 2>&1 | tee /tmp/9-6-1-nixie.log
    set -o pipefail; make lint 2>&1 | tee /tmp/9-6-1-lint.log
    set -o pipefail; make test-bdd 2>&1 | tee /tmp/9-6-1-test-bdd.log
    set -o pipefail; make test 2>&1 | tee /tmp/9-6-1-test.log
    set -o pipefail; make bench-codec 2>&1 | tee /tmp/9-6-1-bench-codec.log
```
<!-- markdownlint-enable MD046 -->

Completion criteria:

- All commands above exit successfully.
- Benchmark logs include encode/decode, fragmentation overhead, and allocation
  baseline sections.
- Documentation and roadmap updates are present and consistent.
- `Progress`, `Decision log`, and `Outcomes and retrospective` sections are
  updated with final evidence and dates.
