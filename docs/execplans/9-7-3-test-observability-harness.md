# 9.7.3 Introduce a test observability harness in wireframe\_testing

This ExecPlan (execution plan) is a living document. The sections
`Constraints`, `Tolerances`, `Risks`, `Progress`, `Surprises & Discoveries`,
`Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work
proceeds.

Status: COMPLETE

## Purpose / big picture

Wireframe relies on logs and metrics for runtime observability. The codec
hardening work (phase 9) added structured errors (`CodecError` taxonomy) and
recovery policies (`RecoveryPolicy::Drop`, `Quarantine`, `Disconnect`) that
must be verified alongside behaviour. Today, tests can capture logs via
`wireframe_testing::logger`, but metrics require ad hoc `DebuggingRecorder`
setup, and there is no shared helper for asserting instrumentation. This makes
telemetry assertions inconsistent, brittle, and hard to reuse.

After this change, `wireframe_testing` exports an `ObservabilityHandle` that
combines log capture with metrics recording in a single fixture. A test author
can write:

```rust
use wireframe_testing::observability;

let mut obs = observability();
obs.clear();

metrics::with_local_recorder(obs.recorder(), || {
    wireframe::metrics::inc_codec_error("framing", "drop");
});

assert_eq!(
    obs.counter(
        wireframe::metrics::CODEC_ERRORS,
        &[("error_type", "framing"), ("recovery_policy", "drop")],
    ),
    1
);
```

Observable success: `make test` passes, including new integration tests in
`tests/test_observability_harness.rs` and new rstest-bdd behavioural scenarios
in `tests/scenarios/test_observability_scenarios.rs` that exercise the
observability handle. ADR-006 status is updated to "Accepted" and roadmap item
9.7.3 is ticked.

This is roadmap item 9.7.3, implementing ADR-006
(`docs/adr-006-test-observability.md`), and is a prerequisite for item 9.7.4
(codec error regression tests that will consume the harness).

## Constraints

- All existing `wireframe_testing` public APIs must remain source-compatible.
  The new module is purely additive.
- No file may exceed 400 lines.
- Use `rstest` for new unit tests and `rstest-bdd` v0.5.0 for new behavioural
  tests.
- en-GB-oxendict spelling in all comments and documentation.
- Strict Clippy with `-D warnings` on all targets including tests.
- Every module must begin with a `//!` doc comment.
- Public functions must have `///` doc comments with examples.
- No `assert!`/`panic!` in functions returning `Result`
  (`clippy::panic_in_result_fn`).
- No array indexing in tests — use `.get(n)` / `.first()` instead.
- `#[expect]` attributes must include `reason = "..."`.
- Quality gates (`make check-fmt`, `make lint`, `make test`,
  `make markdownlint`) must pass before completion.
- Tests for `wireframe_testing` functionality must live in the main crate's
  `tests/` directory as integration tests, because `wireframe_testing` is a
  dev-dependency, not a workspace member.
- The `wireframe_testing` crate does not currently depend on the `metrics`
  crate (only `metrics-util`). Adding `metrics = "0.24.3"` is the only new
  dependency permitted by the ADR.

## Tolerances (exception triggers)

- Scope: if implementation requires more than 18 new/modified files or 1200
  net changed lines, stop and re-scope.
- Dependencies: if any new external crate beyond `metrics = "0.24.3"` is
  required, stop and escalate.
- Interface: if a public API signature on the main `wireframe` crate or
  existing `wireframe_testing` APIs must change, stop and escalate.
- Iterations: if the same failing root cause persists after 3 fix attempts,
  stop and document options in the Decision Log.
- Line budget: if any file under `wireframe_testing/src/observability/`
  approaches 350 lines, split further before proceeding.

## Risks

- Risk: `metrics::with_local_recorder` is thread-local and will not capture
  metrics emitted on spawned Tokio tasks. Severity: medium. Likelihood: medium.
  Mitigation: Document this limitation. The existing `tests/metrics.rs` pattern
  uses the same approach successfully. For async tests, ensure the
  metric-emitting code runs on the test thread (use `block_on` or
  `#[tokio::test(flavor = "current_thread")]`). Note this in the module docs
  and user guide.

- Risk: Global `logtest::Logger` mutex and thread-local metrics recorder
  have different scoping semantics. Severity: low. Likelihood: low. Mitigation:
  Document in `ObservabilityHandle` that logs are global (serialized by mutex)
  while metrics are thread-local. The `clear()` method drains both.

- Risk: The `observability.rs` module might approach the 400-line limit if
  too many assertion helpers are added. Severity: low. Likelihood: low.
  Mitigation: Start with the minimum required API from
  `docs/wireframe-testing-crate.md`. Split into submodules only if approaching
  350 lines.

## Progress

- [x] Stage A: dependency and scaffolding.
- [x] Stage B: core `ObservabilityHandle` implementation.
- [x] Stage C: assertion helper methods.
- [x] Stage D: integration tests (rstest). 13 tests green.
- [x] Stage E: BDD tests (rstest-bdd, 4-file pattern). 4 scenarios green.
- [x] Stage F: documentation updates.
- [x] Stage G: validation and evidence capture. All gates pass.

## Surprises & discoveries

- `metrics_util::debugging::Snapshotter::snapshot()` is destructive: it
  uses `c.swap(0, Ordering::SeqCst)` to atomically drain counter values to zero
  on each call. This means calling `counter()` multiple times after a single
  metric recording returns 0 on all calls after the first if each call takes a
  fresh snapshot. Fix: store the snapshot result in a
  `captured: Vec<SnapshotEntry>` field and query it for subsequent counter
  lookups. The explicit `snapshot()` method drains once; counter methods read
  the stored vector. Date: 2026-03-02.

- `metrics::Unit` and `metrics::SharedString` live in the `metrics`
  crate, not `metrics_util`. The snapshot tuple type is
  `(CompositeKey, Option<Unit>, Option<SharedString>, DebugValue)` where `Unit`
  and `SharedString` are re-exported from `metrics`. Date: 2026-03-02.

- `cargo check -p wireframe_testing` panics with a feature resolver
  error because `wireframe_testing` is not a workspace member. Building via the
  workspace root (`cargo check` or `cargo test --test ...`) works fine. Date:
  2026-03-02.

## Decision log

- Decision: Use `metrics::with_local_recorder` (thread-local) rather than
  installing a global recorder. Rationale: Matches the existing pattern in
  `tests/metrics.rs`; avoids cross-test interference without requiring
  `--test-threads=1`. The ADR mentions serialized access with a global lock,
  which is already satisfied by the `LoggerHandle` mutex for logs. Metrics are
  inherently isolated per-thread with `with_local_recorder`. Date: 2026-03-02.

- Decision: Compose `LoggerHandle` by value inside `ObservabilityHandle`.
  Rationale: `LoggerHandle` acquires its global mutex on construction. By
  owning it, `ObservabilityHandle` keeps the mutex held for its lifetime,
  providing ADR-006's serialization guarantee. On drop, the mutex releases.
  Date: 2026-03-02.

- Decision: Do not feature-gate the observability module.
  Rationale: Per ADR-006 and `docs/wireframe-testing-crate.md`, the handle
  should still capture logs when the `metrics` feature is disabled. The
  `metrics` crate is unconditional in the test utility crate. The feature gate
  matters only in the main crate's production code. Date: 2026-03-02.

- Decision: Name BDD world `TestObservabilityWorld` with fixture
  `test_observability_world`, following the established naming convention (cf.
  `CodecFixturesWorld` / `codec_fixtures_world`). Date: 2026-03-02.

## Outcomes & retrospective

Completed 2026-03-02.

Files created: 7 new, 8 modified = 15 total (within 18-file tolerance).

All quality gates pass:

- `make check-fmt` — exit 0.
- `make markdownlint` — exit 0.
- `make lint` — exit 0.
- `make test` — exit 0. 13 new integration tests and 4 new BDD
  scenarios all green.

Key design insight: the `DebuggingRecorder`'s snapshot is destructive (atomic
swap to zero), so the `ObservabilityHandle` caches the snapshot result in a
`Vec<SnapshotEntry>` field. Users call `snapshot()` once after recording
metrics, then query the cached vector via `counter()` and related methods. This
was not anticipated in the original design doc but is essential for correct
multi-query usage.

The implemented API extends the design doc's proposed API with additional
convenience methods for codec error assertions and log assertions, as these are
the primary use cases for the harness in upcoming codec regression tests
(9.7.4).

## Context and orientation

### Repository layout (relevant subset)

```plaintext
wireframe_testing/
  Cargo.toml                        # dependencies (metrics 0.24, metrics-util 0.20)
  src/lib.rs                        # public re-exports
  src/logging.rs                    # LoggerHandle wrapping logtest::Logger
  src/observability/mod.rs          # ObservabilityHandle, queries, fixture
  src/observability/labels.rs       # Labels type and From conversions
  src/observability/assertions.rs   # assertion helpers and find_counter
  src/helpers.rs                    # TestSerializer trait, constants
  src/helpers/codec_fixtures.rs     # Hotline codec fixtures
  src/helpers/codec_ext.rs          # encode/decode with FrameCodec (145 lines)

src/
  metrics.rs                        # inc_*, CODEC_ERRORS, Direction (153 lines)
  codec/error.rs                    # CodecError, FramingError, etc. (312 lines)
  codec/recovery/policy.rs          # RecoveryPolicy enum (76 lines)
  codec/recovery/hook.rs            # RecoveryPolicyHook trait (79 lines)
  codec/recovery/context.rs         # CodecErrorContext struct (75 lines)
  codec/recovery/config.rs          # RecoveryConfig (77 lines)

tests/
  metrics.rs                        # Existing metrics tests using DebuggingRecorder (96 lines)
  fixtures/mod.rs                   # BDD world fixtures (36 modules)
  steps/mod.rs                      # BDD step definitions (37 modules)
  scenarios/mod.rs                  # BDD scenario registrations (39 modules)
  features/                         # Gherkin feature files

docs/
  adr-006-test-observability.md     # ADR (status: Proposed)
  wireframe-testing-crate.md        # Design doc with proposed ObservabilityHandle API
  roadmap.md                        # line 437: - [ ] 9.7.3.
  users-guide.md                    # Testing section at line 224
```

### Key types and patterns

`LoggerHandle` (in `wireframe_testing/src/logging.rs`) wraps a
`MutexGuard<'static, Logger>` from the `logtest` crate. It implements
`Deref`/`DerefMut` to `Logger`, providing `pop() -> Option<Record>` for
draining log entries and `clear()` to drain all. It is a rstest `#[fixture]`.

`DebuggingRecorder` (from `metrics_util::debugging`) creates a `Snapshotter`
via `.snapshotter()`. After recording metrics within
`metrics::with_local_recorder(&recorder, || { ... })`, the snapshotter provides
`snapshot().into_vec()` returning
`Vec<(CompositeKey, Option<Unit>, Option<SharedString>, DebugValue)>`.
`CompositeKey` has `.key()` -> `Key` with `.name()` -> `&str` and `.labels()`
-> iterator of `Label` with `.key()` and `.value()`. `DebugValue::Counter(u64)`
holds counter values. This pattern is already established in `tests/metrics.rs`.

The BDD test pattern uses four files per domain: `.feature` (Gherkin),
`fixtures/<name>.rs` (world struct + `#[fixture]`), `steps/<name>_steps.rs`
(step functions), and `scenarios/<name>_scenarios.rs` (`#[scenario]` wiring).
Step parameters must use the full fixture name. World fixtures use
`#[rustfmt::skip]` before `#[fixture]`.

`inc_codec_error(error_type, recovery_policy)` in `src/metrics.rs` records the
`wireframe_codec_errors_total` counter with `error_type` and `recovery_policy`
labels. Both parameters are `&'static str`.

### Original proposed API from design doc

`docs/wireframe-testing-crate.md` originally specified this minimal API:

```rust
pub struct ObservabilityHandle { /* fields omitted */ }

impl ObservabilityHandle {
    pub fn new() -> Self;
    pub fn logs(&mut self) -> &mut LoggerHandle;
    pub fn snapshot(&self) -> Snapshot;
    pub fn clear(&mut self);
    pub fn counter(&self, name: &str, labels: &[(&str, &str)]) -> u64;
}

pub fn observability() -> ObservabilityHandle;
```

The shipped API extends this with `Labels` (supporting `impl Into<Labels>` for
ergonomic label passing), convenience methods for codec error assertions and
log assertions, and a module split into `mod.rs`, `labels.rs`, and
`assertions.rs`. See the "Public API surface" section below for the current
signatures.

## Plan of work

### Stage A: dependency and scaffolding

Add `metrics = "0.24.3"` to `wireframe_testing/Cargo.toml` under
`[dependencies]`. This is required for `metrics::with_local_recorder`.

Create `wireframe_testing/src/observability/mod.rs` as an empty module with a
`//!` doc comment.

Edit `wireframe_testing/src/lib.rs` to add `pub mod observability;` after the
existing module declarations (after line 26, `pub mod multi_packet;`) and add
re-exports: `pub use observability::{ObservabilityHandle, observability};`.

Stage A acceptance: `cargo check` (workspace root) compiles.

### Stage B: core ObservabilityHandle implementation

Implement the `ObservabilityHandle` struct in
`wireframe_testing/src/observability/mod.rs`.

The struct owns three fields:

1. `logger: LoggerHandle` — acquired via `LoggerHandle::new()`, holds the
   global mutex for log serialization.
2. `recorder: DebuggingRecorder` — created fresh via
   `DebuggingRecorder::new()`.
3. `snapshotter: Snapshotter` — extracted from the recorder via
   `.snapshotter()`.

Implement:

- `ObservabilityHandle::new()` — constructs all three fields.
- `logs(&mut self) -> &mut LoggerHandle` — returns mutable access to the
  logger for `pop()` / `clear()`.
- `recorder(&self) -> &DebuggingRecorder` — returns a reference for use
  with `metrics::with_local_recorder`.
- `clear(&mut self)` — clears the logger (drains all records) and takes a
  snapshot (draining captured metrics so the next snapshot starts fresh).

Implement the rstest fixture:

```rust
#[rustfmt::skip]
#[fixture]
pub fn observability() -> ObservabilityHandle {
    ObservabilityHandle::new()
}
```

Stage B acceptance: `cargo check` (workspace root) compiles.

### Stage C: assertion helper methods

Add these methods to `ObservabilityHandle`:

Counter queries:

- `counter(&self, name: &str, labels: &[(&str, &str)]) -> u64` — find the
  counter matching `name` and all provided labels in the snapshot. Returns 0 if
  not found. Iterates the snapshot vector, matching on
  `key.key().name() == name` and all label pairs present.

- `counter_without_labels(&self, name: &str) -> u64` — convenience for
  `counter(name, &[])`.

- `codec_error_counter(&self, error_type: &str, recovery_policy: &str) -> u64`
  — convenience calling `self.counter` with `CODEC_ERRORS` and the `error_type`
  / `recovery_policy` label pair.

Counter assertions (return `Result<(), String>` for clippy compliance):

- `assert_counter(&self, name: &str, labels: &[(&str, &str)], expected: u64)
  -> Result<(), String>` — returns `Err` if the counter does not match.

- `assert_no_metric(&self, name: &str) -> Result<(), String>` — verifies
  no metric with the given name exists in the snapshot.

- `assert_codec_error_counter(&self, error_type: &str,
  recovery_policy: &str, expected: u64) -> Result<(),
  String>` — convenience for the common codec error assertion.

Log assertions (return `Result<(), String>`):

- `assert_log_contains(&mut self, substring: &str) -> Result<(), String>`
  — drains the log buffer, checks any record's `args()` string contains the
  substring.

- `assert_log_at_level(&mut self, level: log::Level, substring: &str)
  -> Result<(), String>` — same as above but also filters by level.

Stage C acceptance: `cargo check` (workspace root) compiles.

### Stage D: integration tests (rstest)

Create `tests/test_observability_harness.rs` with `#![cfg(not(loom))]` and
integration tests for every public method on `ObservabilityHandle`.

Tests (all returning `io::Result<()>` with explicit error checks, no `assert!`
/ `panic!`):

1. `counter_returns_zero_for_unrecorded_metric` — acquire handle, query
   a counter that was never incremented, verify returns 0.
2. `counter_captures_incremented_metric` — use `with_local_recorder` to
   increment a counter, verify `counter()` returns expected value.
3. `counter_with_labels_filters_correctly` — increment two label
   combinations, verify each returns the correct count.
4. `codec_error_counter_convenience_method` — use `inc_codec_error`,
   verify `codec_error_counter()` returns expected.
5. `assert_counter_passes_on_match` — verify `assert_counter` returns
   `Ok`.
6. `assert_counter_fails_on_mismatch` — verify `assert_counter` returns
   `Err`.
7. `assert_no_metric_passes_when_absent` — verify returns `Ok` for
   unrecorded metric.
8. `assert_no_metric_fails_when_present` — increment, verify returns
   `Err`.
9. `clear_resets_log_and_metric_state` — increment + log, call `clear()`,
   verify both empty.
10. `log_assertion_finds_matching_substring` — emit log, call
    `assert_log_contains`, verify `Ok`.
11. `log_assertion_fails_on_missing_substring` — verify
    `assert_log_contains("nonexistent")` returns `Err`.
12. `log_at_level_filters_correctly` — emit warn and info, verify
    `assert_log_at_level(Warn, ...)` finds warn but not info.
13. `assert_codec_error_counter_convenience` — exercise the combined
    assertion.

Stage D acceptance: `make check-fmt && make lint && make test` pass with all 13
tests green.

### Stage E: BDD tests (rstest-bdd, 4-file pattern)

Create the BDD suite following the established pattern:

`tests/features/test_observability.feature`:

```gherkin
Feature: Test observability harness
  The wireframe_testing crate provides an observability handle that
  captures logs and metrics for deterministic test assertions.

  Scenario: Metrics are captured via the observability handle
    Given an observability harness is acquired
    When a codec error metric is recorded
    Then the codec error counter equals 1

  Scenario: Logs are captured via the observability handle
    Given an observability harness is acquired
    When a warning log is emitted
    Then the log buffer contains the expected message

  Scenario: Clear resets captured state
    Given an observability harness is acquired
    When a codec error metric is recorded
    And the observability state is cleared
    Then the codec error counter equals 0

  Scenario: Absent metrics return zero
    Given an observability harness is acquired
    Then the codec error counter equals 0
```

`tests/fixtures/test_observability.rs`: A `TestObservabilityWorld` struct
holding an `Option<ObservabilityHandle>`. Methods: `acquire_harness()`,
`record_codec_error()`, `emit_warning_log()`, `clear_state()`,
`verify_codec_error_counter(expected)`, `verify_log_contains(substring)`. Uses
`metrics::with_local_recorder` internally. Manual `Debug` impl if needed (check
whether `ObservabilityHandle` derives `Debug`).

`tests/steps/test_observability_steps.rs`: Step functions matching Gherkin
steps, delegating to world methods. All parameters named
`test_observability_world`.

`tests/scenarios/test_observability_scenarios.rs`: Four `#[scenario]` functions
with `#[expect(unused_variables, reason = "...")]`.

Register in `tests/fixtures/mod.rs`, `tests/steps/mod.rs`, and
`tests/scenarios/mod.rs`.

Stage E acceptance: `make test` passes with all 4 BDD scenarios green.

### Stage F: documentation updates

1. Update `docs/adr-006-test-observability.md` — change `Status` from
   "Proposed." to "Accepted." (line 5).

2. Update `docs/roadmap.md` — change `- [ ] 9.7.3.` to `- [x] 9.7.3.`
   (line 437).

3. Update `docs/users-guide.md` — add a new subsection "Test
   observability" under the testing section (after the "Codec test fixtures"
   subsection, around line 300) documenting `ObservabilityHandle`, the
   `observability()` fixture, and the assertion helpers with a code example
   showing the `with_local_recorder` + `counter()` pattern. Use en-GB-oxendict
   spelling.

4. Verify `docs/wireframe-testing-crate.md` — check that the implemented
   API matches the proposed API in the design doc. Update if the implementation
   diverged (e.g., if additional methods were added).

Stage F acceptance: `make markdownlint MDLINT=/root/.bun/bin/markdownlint-cli2`
passes.

### Stage G: validation and evidence capture

Run all quality gates with logging:

<!-- markdownlint-disable MD046 -->
```shell
set -o pipefail; make fmt 2>&1 | tee /tmp/9-7-3-fmt.log
set -o pipefail; make check-fmt 2>&1 | tee /tmp/9-7-3-check-fmt.log
set -o pipefail; make markdownlint MDLINT=/root/.bun/bin/markdownlint-cli2 2>&1 | tee /tmp/9-7-3-markdownlint.log
set -o pipefail; make lint 2>&1 | tee /tmp/9-7-3-lint.log
set -o pipefail; make test 2>&1 | tee /tmp/9-7-3-test.log
```
<!-- markdownlint-enable MD046 -->

Update the `Progress` and `Outcomes & Retrospective` sections with final
evidence and timestamps.

Stage G acceptance: all commands exit 0.

## Validation and acceptance

Quality criteria (what "done" means):

- Tests: `make test` passes, including 13 new integration tests in
  `tests/test_observability_harness.rs` and 4 new BDD scenarios in
  `tests/scenarios/test_observability_scenarios.rs`.
- Lint: `make lint` passes (Clippy with `-D warnings` on all targets).
- Format: `make check-fmt` passes.
- Markdown: `make markdownlint` passes.
- Documentation: `docs/users-guide.md` documents the new observability
  API. `docs/roadmap.md` item 9.7.3 is marked done. ADR-006 status is
  "Accepted."
- Design doc: `docs/wireframe-testing-crate.md` reflects the implemented
  API.

Quality method:

```shell
set -o pipefail
make check-fmt 2>&1 | tee /tmp/9-7-3-check-fmt.log; echo "exit: $?"
make lint 2>&1 | tee /tmp/9-7-3-lint.log; echo "exit: $?"
make test 2>&1 | tee /tmp/9-7-3-test.log; echo "exit: $?"
make markdownlint MDLINT=/root/.bun/bin/markdownlint-cli2 2>&1 | tee /tmp/9-7-3-mdlint.log; echo "exit: $?"
```

Specific new tests that must pass:

- `test_observability_harness::counter_returns_zero_for_unrecorded_metric`
- `test_observability_harness::counter_captures_incremented_metric`
- `test_observability_harness::counter_with_labels_filters_correctly`
- `test_observability_harness::codec_error_counter_convenience_method`
- `test_observability_harness::assert_counter_passes_on_match`
- `test_observability_harness::assert_counter_fails_on_mismatch`
- `test_observability_harness::assert_no_metric_passes_when_absent`
- `test_observability_harness::assert_no_metric_fails_when_present`
- `test_observability_harness::clear_resets_log_and_metric_state`
- `test_observability_harness::log_assertion_finds_matching_substring`
- `test_observability_harness::log_assertion_fails_on_missing_substring`
- `test_observability_harness::log_at_level_filters_correctly`
- `test_observability_harness::assert_codec_error_counter_convenience`

BDD scenarios:

- "Metrics are captured via the observability handle"
- "Logs are captured via the observability handle"
- "Clear resets captured state"
- "Absent metrics return zero"

## Idempotence and recovery

All stages produce additive changes. If a stage fails partway through, the
incomplete changes can be reverted with `git checkout -- .` and the stage
retried. No destructive operations are involved.

## Interfaces and dependencies

### New dependency

In `wireframe_testing/Cargo.toml`:

```toml
metrics = "0.24.3"
```

### Public API surface

The observability module is split into three files under
`wireframe_testing/src/observability/`:

- `mod.rs` — `ObservabilityHandle` struct, core query methods, and the
  rstest fixture.
- `labels.rs` — `Labels` type and `From` conversions.
- `assertions.rs` — assertion methods and `find_counter` helper.

```rust
use log::Level;
use metrics_util::debugging::DebuggingRecorder;

use wireframe_testing::observability::Labels;

pub struct ObservabilityHandle { /* fields omitted */ }

impl ObservabilityHandle {
    pub fn new() -> Self;
    pub fn logs(&mut self) -> &mut LoggerHandle;
    pub fn recorder(&self) -> &DebuggingRecorder;
    pub fn snapshot(&mut self);
    pub fn clear(&mut self);
    pub fn counter(&self, name: &str, labels: impl Into<Labels>) -> u64;
    pub fn counter_without_labels(&self, name: &str) -> u64;
    pub fn codec_error_counter(
        &self, error_type: &str, recovery_policy: &str,
    ) -> u64;
    pub fn assert_counter(
        &self, name: &str, labels: impl Into<Labels>, expected: u64,
    ) -> Result<(), String>;
    pub fn assert_no_metric(&self, name: &str) -> Result<(), String>;
    pub fn assert_codec_error_counter(
        &self, error_type: &str, recovery_policy: &str, expected: u64,
    ) -> Result<(), String>;
    pub fn assert_log_contains(
        &mut self, substring: &str,
    ) -> Result<(), String>;
    pub fn assert_log_at_level(
        &mut self, level: Level, substring: &str,
    ) -> Result<(), String>;
}

#[fixture]
pub fn observability() -> ObservabilityHandle;
```

### Files to create

| File                                                | Purpose                   | Est. lines |
| --------------------------------------------------- | ------------------------- | ---------- |
| `wireframe_testing/src/observability/mod.rs`        | Handle, queries, fixture  | 250-300    |
| `wireframe_testing/src/observability/labels.rs`     | Labels type and From impl | 70-80      |
| `wireframe_testing/src/observability/assertions.rs` | Assertion helpers         | 200-250    |
| `tests/test_observability_harness.rs`               | Integration tests         | 200-300    |
| `tests/features/test_observability.feature`         | BDD feature file          | 25-30      |
| `tests/fixtures/test_observability.rs`              | BDD world fixture         | 100-150    |
| `tests/steps/test_observability_steps.rs`           | BDD step definitions      | 60-80      |
| `tests/scenarios/test_observability_scenarios.rs`   | BDD scenario wiring       | 30-40      |

### Files to modify

| File                                 | Change                                    |
| ------------------------------------ | ----------------------------------------- |
| `wireframe_testing/Cargo.toml`       | Add `metrics = "0.24.3"`                  |
| `wireframe_testing/src/lib.rs`       | Add `pub mod observability;` + re-exports |
| `tests/fixtures/mod.rs`              | Add `pub mod test_observability;`         |
| `tests/steps/mod.rs`                 | Add `mod test_observability_steps;`       |
| `tests/scenarios/mod.rs`             | Add `mod test_observability_scenarios;`   |
| `docs/adr-006-test-observability.md` | Status: Proposed -> Accepted              |
| `docs/roadmap.md`                    | Tick 9.7.3 checkbox                       |
| `docs/users-guide.md`                | Add test observability section            |

Total: 8 new + 8 modified = 16 files (within 18-file tolerance).

### Artifacts and notes

Reference pattern for counter query (from `tests/metrics.rs:87-95`):

```rust
fn assert_counter_eq(snapshotter: &Snapshotter, name: &str, expected: u64) {
    let metrics = snapshotter.snapshot().into_vec();
    assert!(
        metrics.iter().any(|(key, _, _, value)| {
            key.key().name() == name
                && matches!(value, DebugValue::Counter(c) if *c == expected)
        }),
        "expected {name} == {expected}, got {metrics:#?}"
    );
}
```

Reference pattern for BDD world with `with_local_recorder`:

```rust
impl TestObservabilityWorld {
    pub fn record_codec_error(&self) -> TestResult {
        let obs = self.obs.as_ref()
            .ok_or_else(|| "handle not acquired".to_string())?;
        metrics::with_local_recorder(obs.recorder(), || {
            wireframe::metrics::inc_codec_error("framing", "drop");
        });
        Ok(())
    }
}
```

Key gotcha: `inc_codec_error` takes `&'static str`. BDD step functions
receiving strings from feature files must map to static literals via match or
use hardcoded values in the world methods.

Key gotcha: `wireframe_testing` is not a workspace member — internal
`#[cfg(test)]` modules never run. All tests must go in the main crate's
`tests/` directory.

Key gotcha: `WireframeApp` does not implement `Debug`. If any world struct
holds one, it needs a manual `Debug` impl. `ObservabilityHandle` does not hold
a `WireframeApp`, so this is unlikely to apply here, but verify whether
`DebuggingRecorder` and `Snapshotter` implement `Debug`.
