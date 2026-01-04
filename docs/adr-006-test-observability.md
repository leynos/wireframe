# Architectural decision record (ADR) 006: test observability harness

## Status

Proposed.

## Date

2026-01-02.

## Context and Problem Statement

Wireframe relies on logs, metrics, and protocol hooks for runtime
observability. Upcoming codec hardening work adds structured errors and
recovery policies that must be verified alongside behaviour. Today, tests can
capture logs via `wireframe_testing::logger`, but metrics require ad hoc global
exporters, and there is no shared helper for asserting instrumentation. This
makes telemetry assertions inconsistent, brittle, and hard to reuse across
downstream crates.

A dedicated test observability harness is needed, so tests can assert on
logging and metrics deterministically without depending on external exporters
or global state leaks.

## Decision Drivers

- Provide deterministic, per-test observability assertions.
- Avoid external services or network dependencies in tests.
- Keep the harness reusable for downstream crates and example codecs.
- Guard global registries to prevent cross-test interference.

## Requirements

### Functional requirements

- Capture logs and metrics for a single test run.
- Provide helper APIs for inspecting log records and metric samples.
- Support async tests and background tasks without losing telemetry.

### Technical requirements

- Avoid environment variable mutations in tests.
- Use in-process recorders and restore prior global state on drop.
- Keep dependencies minimal and aligned with existing crates.

## Options Considered

### Option A: keep ad hoc log capture (status quo)

Continue using `logtest` directly in tests and rely on manual, ad hoc setup for
metrics.

### Option B: add a unified test observability harness (preferred)

Provide a `wireframe_testing::observability` module that installs a scoped log
capture and metrics recorder, returning a guard with inspection helpers.

### Option C: rely on external exporters in integration tests

Use Prometheus exporters or external telemetry services and parse their output
in tests.

## Decision Outcome / Proposed Direction

Adopt Option B and add a scoped test observability harness to
`wireframe_testing`. The harness should compose with the existing logger guard
and provide a metrics recorder suitable for assertions in unit, integration,
and behavioural tests.

## Approach

- Add an `ObservabilityHandle` that installs log capture and a metrics recorder
  when constructed and restores previous globals on drop.
- Reuse `logtest` for logs and introduce a metrics recorder based on
  `metrics-util` so tests can query counters and gauges without external
  exporters.
- Provide helper methods to clear state between assertions and to filter by
  labels when verifying counters.
- Serialize access with a global lock to avoid cross-test interference, and
  document that tests using the handle should not run concurrently.
- For parallel test runners, run observability-heavy test binaries with
  `--test-threads=1` or gate observability tests behind a shared fixture to
  prevent interleaving.

## Consequences

- Tests can assert on instrumentation for codec errors and recovery policies
  without bespoke setup.
- Global recorder access requires serialization, which may reduce parallelism
  for observability-heavy suites, such as forcing `--test-threads=1` for the
  affected test binary. See the mitigation guidance in the approach.
- The testing crate gains additional dependency surface for metrics capture.

## Roadmap

### Phase 1: Observability capture primitives

- Step: Introduce the harness in `wireframe_testing`.
  - Task: Add `ObservabilityHandle` and integrate with `LoggerHandle`.
  - Task: Provide metric snapshot helpers for counters and gauges.
  - Task: Document usage and thread-safety constraints.

### Phase 2: Codec and recovery assertions

- Step: Wire codec tests to the harness.
  - Task: Add helpers for asserting codec error counters and log fields.
  - Task: Update codec regression tests to use the new helpers.

## Mitigation and rollback

- Keep the harness behind a feature flag if global recorder conflicts with
  downstream test environments.
- If metrics capture proves flaky, fall back to log-based assertions while
  refining recorder isolation.
