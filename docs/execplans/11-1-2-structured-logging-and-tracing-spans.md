# 11.1.2 Structured logging and tracing spans

This ExecPlan (execution plan) is a living document. The sections
`Constraints`, `Tolerances`, `Risks`, `Progress`, `Surprises & Discoveries`,
`Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work
proceeds.

Status: COMPLETE

## Purpose / big picture

Roadmap item 11.1.2 requires structured logging and tracing spans around client
connect, send, receive, and stream lifecycle events, plus configuration for
per-command timing. After this change, every client operation emits a `tracing`
span with structured fields (frame size, correlation ID, operation result, peer
address, stream frame count). Users optionally enable per-command elapsed-time
events via a `TracingConfig` builder method. When no `tracing` subscriber is
installed, all instrumentation is zero-cost.

Observable success: a user configures `TracingConfig` on the builder, performs
connect/send/receive/call/streaming/close, and each operation produces a
tracing span at the configured level with structured fields. Timing events
appear when enabled. All existing tests pass unchanged. New unit tests using
rstest with `tracing-test`, and BDD tests using rstest-bdd v0.5.0, validate the
instrumentation.

## Constraints

Hard invariants. Violation requires escalation, not workarounds.

- No single source file may exceed 400 lines.
- Public APIs must be documented with rustdoc (`///`) including examples.
- Every module must begin with a `//!` module-level comment.
- All code must pass `make check-fmt`, `make lint` (clippy `-D warnings`),
  and `make test`.
- Comments and documentation must use en-GB-oxendict spelling.
- Tests must use `rstest` fixtures; BDD tests use `rstest-bdd` v0.5.0.
- Existing public API signatures must not change (backwards compatibility).
- Clients configured without tracing must behave identically to today.
- The `builder_field_update!` macro in `src/client/builder/mod.rs` must be
  updated for every arm when a new field is added to the builder struct.
- No new external crate dependencies (`tracing`, `tracing-subscriber`, and
  `tracing-test` are already in `Cargo.toml`).
- Design decisions must be recorded in `docs/wireframe-client-design.md`.
- `docs/users-guide.md` must be updated with the new public API surface.
- `docs/roadmap.md` item 11.1.2 must be marked done only after all gates pass.

## Tolerances (exception triggers)

- Scope: if implementation requires changes to more than 30 files (net new +
  modified), stop and escalate.
- Interface: if any existing public API signature must change (not just adding
  new methods/types), stop and escalate.
- Dependencies: if a new external crate is required, stop and escalate.
- Iterations: if tests still fail after 3 attempts at a given milestone, stop
  and escalate.
- Time: if any single stage exceeds 4 hours elapsed, stop and escalate.
- Line budget: if any source file is projected to exceed 390 lines, extract
  code to a submodule before continuing.

## Risks

- Risk: `Span::enter()` guard held across `.await` in async methods.
  Severity: medium. Likelihood: low. Mitigation: client methods take
  `&mut self` (exclusive single-task access, linear async). Guards are created
  and dropped within the same method body, covering `.await` points
  sequentially. This pattern is explicitly documented as safe by the `tracing`
  crate. No spawning or concurrent access occurs within the span's lifetime.

- Risk: `runtime.rs` exceeds 400 lines after adding instrumentation.
  Severity: high. Likelihood: low. Mitigation: current is 354 lines, estimated
  addition is ~26 lines = ~380. If it approaches 400, extract `close()`
  instrumentation to a dedicated `src/client/close.rs` file.

- Risk: `dynamic_span!` macro causes binary bloat from 5 instantiations per
  call site. Severity: low. Likelihood: medium. Mitigation: each branch is a
  single `tracing::*_span!` call. LTO eliminates unused branches. Total call
  sites: 8 — negligible impact.

- Risk: BDD `tracing_subscriber` setup conflicts with other parallel tests.
  Severity: medium. Likelihood: medium. Mitigation: use
  `tracing::subscriber::set_default()` (thread-local, not global). The
  `DefaultGuard` is stored in the world fixture and dropped when the world is
  dropped, restoring the previous subscriber. Each test has its own subscriber.

- Risk: BDD fixture parameter naming mismatch.
  Severity: high. Likelihood: medium. Mitigation: known gotcha — `rstest-bdd`
  resolves step function parameters by name, not type. All step functions must
  use `client_tracing_world` (matching the fixture function name exactly).

## Progress

- [x] Stage A: scaffolding — `TracingConfig` type, helpers, builder plumbing
- [x] Stage B: runtime integration — instrument all client methods with spans
- [x] Stage C: unit tests (rstest + `tracing-test`)
- [x] Stage D: BDD tests (rstest-bdd v0.5.0)
- [x] Stage E: documentation — users-guide, client-design, roadmap

## Surprises & discoveries

- `tracing-test`'s `FmtSubscriber` only outputs log lines when events are
  emitted within spans. Span names appear as context prefixed to event lines.
  Unit tests must enable per-command timing (or rely on other events) to
  produce observable output within spans for assertion.

- `clippy::cognitive_complexity` fires on every function using the
  `dynamic_span!` macro because the 5-branch match expands to complexity 17.
  Each span factory function requires `#[expect(clippy::cognitive_complexity)]`.

- `ResponseStream::poll_next` gained enough complexity from the added
  tracing events to trigger `clippy::cognitive_complexity` (10/9). Added an
  `#[expect]` attribute.

- `tracing::Level` is `Copy`, not just `Clone`. The plan initially used
  `.clone()` in `with_all_levels()`, but clippy flagged `clone_on_copy`.

- Timing events should fire on error paths too, not just success. The initial
  implementation only emitted timing on the success path in
  `receive_internal()`, causing the error-path tracing test to fail. Fixed by
  adding `emit_timing_event` calls to all error branches.

## Decision log

- Decision: use `tracing` spans with a `dynamic_span!` macro for dynamic level
  selection, not compile-time-only levels. Rationale: the `tracing` crate
  requires compile-time level constants in `span!` macros. To support
  user-configurable levels per operation, a `macro_rules!` macro matches on the
  five `Level` variants, delegating to the corresponding
  `tracing::<level>_span!` macro per branch. Each branch has static metadata
  while branch selection is dynamic.

- Decision: timing events always emit at `DEBUG` level, regardless of span
  level. Rationale: timing is opt-in diagnostic data. Emitting at the span's
  configured level could produce unexpected `INFO`-level timing events in
  production when a user only intended `INFO` for connection lifecycle. `DEBUG`
  is the correct diagnostic level.

- Decision: `ResponseStream` emits per-frame events (not spans) at `DEBUG`
  level. Rationale: creating spans inside `poll_next` is problematic because it
  is synchronous and called many times. Events are lightweight and appropriate
  for per-frame diagnostics.

- Decision: add a `frame_count` field and `frame_count()` accessor to
  `ResponseStream`. Rationale: enables structured `stream.frames_received` and
  `stream.frames_total` fields in tracing events. Also useful as a public API
  for users who want to track stream progress without tracing.

- Decision: place span factory functions in a dedicated `tracing_helpers.rs`
  module, not inline in each method. Rationale: centralises span creation,
  keeps instrumentation logic out of the hot-path methods, and enables
  consistent span naming and field conventions.

## Outcomes & retrospective

All five stages completed. Final validation passed all gates:

- `make check-fmt`: exit 0
- `make lint` (clippy `-D warnings` + rustdoc `-D warnings`): exit 0
- `make markdownlint`: 0 errors
- `make test`: 0 failures (387 unit tests + 146 integration tests +
  6 BDD scenarios)

Files created: 8. Files modified: 15. Total: 23 (within the 30-file tolerance).
All source files remain under 400 lines.

**What worked well:**

- The `dynamic_span!` macro strategy cleanly solved the compile-time level
  requirement while keeping per-call-site metadata static.
- Centralising span factories in `tracing_helpers.rs` kept the runtime
  methods clean and consistent.
- The `builder_field_update!` macro pattern made builder plumbing
  straightforward once all three arms were identified.

**What required adaptation:**

- `tracing-test` captures formatted event output, not raw span creation.
  Tests had to enable per-command timing for the operation under test so that
  timing events emitted within spans made span names visible in captured output.
- Timing events needed to fire on error paths too. The initial
  implementation only emitted `elapsed_us` on the success path in
  `receive_internal()`. Three error branches were missing `emit_timing_event()`
  calls.
- The `poll_next` method in `response_stream.rs` exceeded clippy's
  cognitive complexity threshold (10 vs limit 9) after adding per-frame tracing
  events. An `#[expect]` annotation was added with a reason.

**What would be done differently:**

- Begin with a spike test for `tracing-test` capture semantics before
  writing all 15 unit tests. Understanding the event-not-span capture model
  earlier would have avoided a full rewrite of the test assertions.

## Context and orientation

The wireframe crate (`/home/user/project`) is a Rust async networking framework
using Tokio. The client subsystem lives in `src/client/`:

```plaintext
src/client/
  mod.rs              — module root, public re-exports (82 lines)
  runtime.rs          — WireframeClient struct, send/receive/call/close (354 lines)
  messaging.rs        — send_envelope, receive_envelope, call_correlated (284 lines)
  streaming.rs        — call_streaming, receive_streaming (150 lines)
  response_stream.rs  — ResponseStream impl Stream, poll_next (168 lines)
  hooks.rs            — LifecycleHooks, RequestHooks, type aliases (180 lines)
  error.rs            — ClientError enum (130 lines)
  tracing_config.rs   — NEW: TracingConfig struct
  tracing_helpers.rs  — NEW: span factories and timing helpers
  builder/
    mod.rs            — builder_field_update! macro (65 lines)
    core.rs           — WireframeClientBuilder struct (63 lines)
    connect.rs        — connect() method (101 lines)
    lifecycle.rs      — on_connection_setup/teardown/error (131 lines)
    request_hooks.rs  — before_send/after_receive builder methods (79 lines)
    tracing.rs        — NEW: tracing_config() builder method
    codec.rs          — codec config builder methods (58 lines)
    serializer.rs     — serializer() builder method
    preamble.rs       — preamble builder methods
  tests/
    mod.rs            — test module root (175 lines)
    helpers.rs        — test helpers (169 lines)
    tracing.rs        — NEW: tracing unit tests
```

BDD tests follow this four-file structure:

```plaintext
tests/
  features/           — Gherkin .feature files
  fixtures/           — World structs with #[fixture] fns and step methods
  steps/              — #[given], #[when], #[then] step definitions
  scenarios/          — #[scenario] bindings linking features to fixtures
```

Key dependencies already available: `tracing` (0.1.41 with `log-always`
feature), `tracing-subscriber` (0.3.18), `tracing-test` (0.2.5 dev-dep).

Data flow for send:
`message → serializer.serialize() → Vec<u8> → invoke_before_send_hooks() → framed.send()`

Data flow for receive:
`framed.next() → BytesMut → invoke_after_receive_hooks()`
`→ serializer.deserialize() → message`

## Plan of work

### Stage A: scaffolding — types, helpers, and builder plumbing

**A1. Create `src/client/tracing_config.rs`** (~115 lines)

Define `TracingConfig` with per-operation level and timing fields:

```rust
#[derive(Clone, Debug)]
pub struct TracingConfig {
    pub(crate) connect_level: Level,
    pub(crate) send_level: Level,
    pub(crate) receive_level: Level,
    pub(crate) call_level: Level,
    pub(crate) streaming_level: Level,
    pub(crate) close_level: Level,
    pub(crate) connect_timing: bool,
    pub(crate) send_timing: bool,
    pub(crate) receive_timing: bool,
    pub(crate) call_timing: bool,
    pub(crate) streaming_timing: bool,
    pub(crate) close_timing: bool,
}
```

Default: `INFO` for connect/close (lifecycle), `DEBUG` for send/receive/call/
streaming (data operations), timing disabled for all.

Provide 14 `#[must_use] pub fn with_*` setter methods (one level + one timing
per operation) and two convenience methods: `with_all_levels(Level)` and
`with_all_timing(bool)`. All documented with rustdoc examples.

**A2. Create `src/client/tracing_helpers.rs`** (~130 lines)

A private `dynamic_span!` macro that matches on the five `tracing::Level`
variants, delegating to the corresponding `tracing::<level>_span!` macro:

```rust
macro_rules! dynamic_span {
    ($level:expr, $name:expr $(, $($field:tt)*)?) => {
        match $level {
            Level::ERROR => tracing::error_span!($name $(, $($field)*)?),
            Level::WARN  => tracing::warn_span!($name $(, $($field)*)?),
            Level::INFO  => tracing::info_span!($name $(, $($field)*)?),
            Level::DEBUG => tracing::debug_span!($name $(, $($field)*)?),
            Level::TRACE => tracing::trace_span!($name $(, $($field)*)?),
        }
    };
}
```

Eight `pub(crate)` span factory functions:

| Function               | Span name                | Structured fields                      |
| ---------------------- | ------------------------ | -------------------------------------- |
| `connect_span`         | `client.connect`         | `peer.addr`                            |
| `send_span`            | `client.send`            | `frame.bytes`                          |
| `receive_span`         | `client.receive`         | `frame.bytes=Empty`, `result=Empty`    |
| `send_envelope_span`   | `client.send_envelope`   | `correlation_id`, `frame.bytes`        |
| `call_span`            | `client.call`            | `result=Empty`                         |
| `call_correlated_span` | `client.call_correlated` | `correlation_id=Empty`, `result=Empty` |
| `streaming_span`       | `client.call_streaming`  | `correlation_id`, `frame.bytes=Empty`  |
| `close_span`           | `client.close`           | (none)                                 |

One `pub(crate) fn emit_timing_event(start: Option<Instant>)` that emits a
`tracing::debug!` event with `elapsed_us` when `start` is `Some`.

**A3. Create `src/client/builder/tracing.rs`** (~55 lines)

One builder method `tracing_config(self, config: TracingConfig) -> Self` with
rustdoc and example.

**A4. Modify `src/client/builder/core.rs`** (63 → ~67 lines)

Add `pub(crate) tracing_config: TracingConfig` field to
`WireframeClientBuilder`. Initialise as `TracingConfig::default()` in `new()`.

**A5. Modify `src/client/builder/mod.rs`** (65 → ~75 lines)

Add `tracing_config: $self.tracing_config,` to all three arms of the
`builder_field_update!` macro (serializer, preamble_config, lifecycle_hooks).
Add `mod tracing;` to the submodule list.

**A6. Modify `src/client/mod.rs`** (82 → ~86 lines)

Add `mod tracing_config;` and `mod tracing_helpers;`. Add
`pub use tracing_config::TracingConfig;` to the public re-exports.

**A7. Validate stage A.**

```bash
set -o pipefail
make check-fmt 2>&1 | tee /tmp/wireframe-check-fmt.log; echo "check-fmt exit: $?"
make lint 2>&1 | tee /tmp/wireframe-lint.log; echo "lint exit: $?"
make test 2>&1 | tee /tmp/wireframe-test.log; echo "test exit: $?"
```

Expected: all three exit code 0. Existing tests pass unchanged.

### Stage B: runtime integration — instrument all client methods

**B1. Modify `src/client/builder/connect.rs`** (101 → ~120 lines)

At the start of `connect()`, create a connect span and optional timing start.
Thread `tracing_config` through the `WireframeClient` constructor.

```rust
let span = connect_span(&self.tracing_config, &addr.to_string());
let _guard = span.enter();
let start = self.tracing_config.connect_timing.then(Instant::now);
// ... existing connect logic ...
emit_timing_event(start);
Ok(WireframeClient {
    // ... existing fields ...
    tracing_config: self.tracing_config,   // NEW
    // ...
})
```

**B2. Modify `src/client/runtime.rs`** (354 → ~380 lines)

Add `pub(crate) tracing_config: TracingConfig` field to `WireframeClient`.

Instrument `send()`: after serialisation produces `bytes`, create `send_span`
with `frame.bytes`, emit timing on success.

Instrument `call()`: create `call_span` wrapping the send+receive pair, record
`result` on completion.

Instrument `close()`: create `close_span`, emit timing.

**B3. Modify `src/client/messaging.rs`** (284 → ~330 lines)

Instrument `send_envelope()`: after correlation ID is resolved and
serialisation succeeds, create `send_envelope_span` with `correlation_id` and
`frame.bytes`.

Instrument `receive_internal()`: create `receive_span`, record `frame.bytes`
after frame arrives, record `result` on success/failure.

Instrument `call_correlated()`: create `call_correlated_span` with
`correlation_id=Empty`, record the actual ID after `send_envelope` returns,
record `result` on completion.

**B4. Modify `src/client/streaming.rs`** (150 → ~178 lines)

Instrument `call_streaming()`: after correlation ID is resolved, create
`streaming_span` with `correlation_id`, record `frame.bytes` after
serialisation.

**B5. Modify `src/client/response_stream.rs`** (168 → ~195 lines)

Add `frame_count: usize` field to `ResponseStream`, initialised to 0. Add
`pub fn frame_count(&self) -> usize` accessor.

In `poll_next()`, on `Ok(bytes)`: increment frame count, emit `tracing::debug!`
event with `frame.bytes`, `stream.frames_received`, `correlation_id`.

In `process_frame()`, when terminator detected: emit `tracing::debug!` event
with `stream.frames_total` and `correlation_id`.

**B6. Validate stage B.**

Same validation commands as A7. All existing tests must still pass.

### Stage C: unit tests (rstest + `tracing-test`)

**C1. Create `src/client/tests/tracing.rs`** (~280 lines)

Uses `#[traced_test]` from `tracing-test` combined with `#[rstest]` and
`#[tokio::test]`. The `tracing` crate has `features = ["log", "log-always"]` so
all events bridge to `log` and are captured by `tracing-test`.

Test cases (15):

1. `connect_emits_span_with_peer_address` — `logs_assert` finds
   `"client.connect"` and the peer address.
2. `send_emits_span_with_frame_bytes` — finds `"client.send"` and
   `"frame.bytes"`.
3. `receive_emits_span_with_frame_bytes_and_result` — finds
   `"client.receive"`, `"frame.bytes"`, `"result"`.
4. `call_emits_wrapping_span` — finds `"client.call"` and `"result"`.
5. `call_correlated_emits_span_with_correlation_id` — finds
   `"client.call_correlated"` and `"correlation_id"`.
6. `send_envelope_emits_span_with_correlation_id_and_frame_bytes` — finds
   `"client.send_envelope"`, `"correlation_id"`, `"frame.bytes"`.
7. `call_streaming_emits_span_with_correlation_id` — finds
   `"client.call_streaming"` and `"correlation_id"`.
8. `close_emits_span` — finds `"client.close"`.
9. `receive_error_records_result_err` — server drops connection, finds
   `"result"` indicating error.
10. `timing_disabled_by_default` — no line contains `"elapsed_us"`.
11. `timing_enabled_emits_elapsed_us_for_send` — with
    `with_send_timing(true)`, finds `"elapsed_us"`.
12. `timing_enabled_for_connect` — with `with_connect_timing(true)`, finds
    `"elapsed_us"`.
13. `all_timing_convenience_enables_all_operations` — with
    `with_all_timing(true)`, multiple `"elapsed_us"` lines.
14. `response_stream_emits_frame_events` — streaming server sends 3 frames +
    terminator; finds 3 `"stream frame received"` and 1
    `"stream terminated"` with `"stream.frames_total=3"`.
15. `default_config_is_backwards_compatible` — no builder tracing call, echo
    works, no panic.

**C2. Modify `src/client/tests/mod.rs`** (175 → ~176 lines)

Add `mod tracing;`.

**C3. Validate stage C.**

Same validation commands. All 15 new tests pass alongside existing tests.

### Stage D: BDD tests (rstest-bdd v0.5.0)

**D1. Create `tests/features/client_tracing.feature`** (~50 lines)

Six scenarios covering: connect span with peer address, send span with frame
size, receive span with result, per-command timing emission, timing not emitted
when disabled, close span.

```gherkin
Feature: Client structured logging and tracing spans
  The wireframe client emits tracing spans and structured events around
  connect, send, receive, call, streaming, and close operations.
  Per-command timing can be enabled via TracingConfig.

  Background:
    Given a running echo server for tracing tests

  Scenario: Connect emits a tracing span with the peer address
    Given a client with default tracing config
    When the client connects to the server
    Then the tracing output contains a "client.connect" span
    And the tracing span includes the peer address

  Scenario: Send emits a tracing span with frame size
    Given a connected tracing client with default config
    When the client sends an envelope via the tracing client
    Then the tracing output contains a "client.send" span
    And the tracing span includes "frame.bytes"

  Scenario: Receive emits a tracing span recording result
    Given a connected tracing client with default config
    When the client sends and receives via the tracing client
    Then the tracing output contains a "client.receive" span
    And the tracing span includes "result=ok"

  Scenario: Per-command timing emits elapsed microseconds
    Given a connected tracing client with send timing enabled
    When the client sends an envelope via the tracing client
    Then the tracing output contains "elapsed_us"

  Scenario: Timing is not emitted when disabled
    Given a connected tracing client with default config
    When the client sends an envelope via the tracing client
    Then the tracing output does not contain "elapsed_us"

  Scenario: Close emits a tracing span
    Given a connected tracing client with default config
    When the tracing client closes the connection
    Then the tracing output contains a "client.close" span
```

**D2. Create `tests/fixtures/client_tracing.rs`** (~220 lines)

`ClientTracingWorld` struct with server addr, server handle, client, tracing
config, captured lines (`Arc<Mutex<Vec<String>>>`), and subscriber guard.
Installs a thread-local `tracing` subscriber via
`tracing::subscriber::set_default()` with a `CaptureWriter` that appends
formatted output to the shared vec.

Methods: `start_echo_server()`, `connect_with_config(TracingConfig)`,
`connect_default()`, `connect_with_send_timing()`, `send_envelope()`,
`send_and_receive()`, `close_connection()`, `assert_output_contains(needle)`,
`assert_output_not_contains(needle)`.

**D3. Create `tests/steps/client_tracing_steps.rs`** (~110 lines)

Step definitions using `rstest_bdd_macros::{given, when, then}`. Each step
calls world async methods via `tokio::runtime::Runtime::new()?.block_on()`. All
world parameters named `client_tracing_world` (matching fixture name).

**D4. Create `tests/scenarios/client_tracing_scenarios.rs`** (~55 lines)

Six `#[scenario]` functions binding feature scenarios to the fixture.

**D5. Register BDD modules.**

- `tests/fixtures/mod.rs`: add `pub mod client_tracing;`
- `tests/steps/mod.rs`: add `mod client_tracing_steps;`
- `tests/scenarios/mod.rs`: add `mod client_tracing_scenarios;`

**D6. Validate stage D.**

Same validation commands. All 6 BDD scenarios pass alongside existing tests.

### Stage E: documentation and cleanup

**E1. Update `docs/wireframe-client-design.md`.**

Add "Tracing instrumentation" section after "Request hooks" documenting: span
names and levels per operation, structured fields table, `ResponseStream`
per-frame events, per-command timing mechanism, and design rationale for
`Span::enter()` in async context and `dynamic_span!` macro.

Add a row to the client configuration reference table:

```plaintext
| Tracing config | tracing_config(TracingConfig) |
| INFO connect/close, DEBUG data ops, timing off |
| Customise tracing span levels and per-command timing. |
```

**E2. Update `docs/users-guide.md`.**

Add "Client tracing" subsection with builder API examples, `TracingConfig`
usage, span output examples, and per-command timing example.

**E3. Mark roadmap item done.**

In `docs/roadmap.md`, change `- [ ] 11.1.2.` to `- [x] 11.1.2.`.

**E4. Final validation.**

```bash
set -o pipefail
make fmt 2>&1 | tee /tmp/wireframe-fmt.log; echo "fmt exit: $?"
make markdownlint MDLINT=/root/.bun/bin/markdownlint-cli2 2>&1 | tee /tmp/wireframe-mdlint.log; echo "mdlint exit: $?"
make check-fmt 2>&1 | tee /tmp/wireframe-check-fmt.log; echo "check-fmt exit: $?"
make lint 2>&1 | tee /tmp/wireframe-lint.log; echo "lint exit: $?"
make test 2>&1 | tee /tmp/wireframe-test.log; echo "test exit: $?"
```

## Concrete steps

All commands run from `/home/user/project`.

After each stage, run validation:

```bash
set -o pipefail
make check-fmt 2>&1 | tee /tmp/wireframe-check-fmt.log; echo "check-fmt exit: $?"
make lint 2>&1 | tee /tmp/wireframe-lint.log; echo "lint exit: $?"
make test 2>&1 | tee /tmp/wireframe-test.log; echo "test exit: $?"
```

Expected: all three exit code 0.

## Validation and acceptance

Quality criteria (what "done" means):

- `make test` passes. New unit tests in `src/client/tests/tracing.rs` and
  BDD scenarios in `tests/scenarios/client_tracing_scenarios.rs` all pass.
- `make lint` passes with zero warnings.
- `make check-fmt` passes.
- `make markdownlint MDLINT=/root/.bun/bin/markdownlint-cli2` passes.
- All pre-existing client tests pass unchanged (backwards compatibility).
- The `TracingConfig` type and `tracing_config()` builder method are public.
- The `frame_count()` method on `ResponseStream` is public.
- `docs/users-guide.md` documents the new builder methods and `TracingConfig`.
- `docs/wireframe-client-design.md` records the design decisions.
- `docs/roadmap.md` marks 11.1.2 as done.

## Idempotence and recovery

All stages are additive. If any stage fails partway, `git stash` or
`git checkout .` reverts to the last known-good state. Re-run from the
beginning of the failed stage. No destructive operations are involved.

## Artifacts and notes

**Span hierarchy example — `call()`:**

```plaintext
client.call                           [DEBUG]
  client.send{frame.bytes=42}         [DEBUG]
  client.receive{frame.bytes=42}      [DEBUG, result=ok]
```

**Span hierarchy example — `call_correlated()`:**

```plaintext
client.call_correlated{correlation_id=1}                  [DEBUG]
  client.send_envelope{correlation_id=1, frame.bytes=42}  [DEBUG]
  client.receive{frame.bytes=42, result=ok}               [DEBUG]
```

**Streaming events example:**

```plaintext
client.call_streaming{correlation_id=1, frame.bytes=42}  [DEBUG]
  DEBUG stream frame received frame.bytes=38 stream.frames_received=1 correlation_id=1
  DEBUG stream frame received frame.bytes=38 stream.frames_received=2 correlation_id=1
  DEBUG stream terminated stream.frames_total=2 correlation_id=1
```

**Files to create (8 new):**

| File                                          | Purpose                                                | Est. lines |
| --------------------------------------------- | ------------------------------------------------------ | ---------- |
| `src/client/tracing_config.rs`                | `TracingConfig` struct with 14 setter methods          | ~115       |
| `src/client/tracing_helpers.rs`               | `dynamic_span!` macro, 8 span factories, timing helper | ~130       |
| `src/client/builder/tracing.rs`               | `tracing_config()` builder method                      | ~55        |
| `src/client/tests/tracing.rs`                 | 15 unit tests (rstest + `tracing-test`)                | ~280       |
| `tests/features/client_tracing.feature`       | BDD feature file (6 scenarios)                         | ~50        |
| `tests/fixtures/client_tracing.rs`            | BDD world/fixture struct + methods                     | ~220       |
| `tests/steps/client_tracing_steps.rs`         | BDD step definitions                                   | ~110       |
| `tests/scenarios/client_tracing_scenarios.rs` | BDD scenario bindings                                  | ~55        |

**Files to modify (15):**

| File                              | Current lines | Est. new lines | Change                                                          |
| --------------------------------- | ------------- | -------------- | --------------------------------------------------------------- |
| `src/client/runtime.rs`           | 354           | ~380           | `tracing_config` field, spans in `send`, `call`, `close`        |
| `src/client/messaging.rs`         | 284           | ~330           | Spans in `send_envelope`, `receive_internal`, `call_correlated` |
| `src/client/streaming.rs`         | 150           | ~178           | Span in `call_streaming`                                        |
| `src/client/response_stream.rs`   | 168           | ~195           | `frame_count` field, events in `poll_next`/`process_frame`      |
| `src/client/builder/core.rs`      | 63            | ~67            | `tracing_config` field                                          |
| `src/client/builder/mod.rs`       | 65            | ~75            | `tracing_config` in 3 macro arms + `mod tracing`                |
| `src/client/builder/connect.rs`   | 101           | ~120           | Connect span + timing + thread field                            |
| `src/client/mod.rs`               | 82            | ~86            | Module declarations + re-export                                 |
| `src/client/tests/mod.rs`         | 175           | ~176           | `mod tracing;`                                                  |
| `tests/fixtures/mod.rs`           | ~34           | ~35            | `pub mod client_tracing;`                                       |
| `tests/steps/mod.rs`              | ~33           | ~34            | `mod client_tracing_steps;`                                     |
| `tests/scenarios/mod.rs`          | ~36           | ~37            | `mod client_tracing_scenarios;`                                 |
| `docs/wireframe-client-design.md` | 369           | ~400           | "Tracing instrumentation" section                               |
| `docs/users-guide.md`             | 1926          | ~1975          | "Client tracing" subsection                                     |
| `docs/roadmap.md`                 | ~500          | ~500           | Mark 11.1.2 done                                                |

Total: 8 new + 15 modified = 23 files. All source files under 400 lines.

**Line-budget safety (tightest files):**

| File                 | Current | Added | New total | Headroom to 400 |
| -------------------- | ------- | ----- | --------- | --------------- |
| `runtime.rs`         | 354     | ~26   | ~380      | 20              |
| `messaging.rs`       | 284     | ~46   | ~330      | 70              |
| `response_stream.rs` | 168     | ~27   | ~195      | 205             |
| `builder/connect.rs` | 101     | ~19   | ~120      | 280             |

## Interfaces and dependencies

No new external crate dependencies.

### Types to define

In `src/client/tracing_config.rs`:

```rust
/// Controls tracing span levels and per-command timing for client operations.
#[derive(Clone, Debug)]
pub struct TracingConfig {
    pub(crate) connect_level: Level,
    pub(crate) send_level: Level,
    pub(crate) receive_level: Level,
    pub(crate) call_level: Level,
    pub(crate) streaming_level: Level,
    pub(crate) close_level: Level,
    pub(crate) connect_timing: bool,
    pub(crate) send_timing: bool,
    pub(crate) receive_timing: bool,
    pub(crate) call_timing: bool,
    pub(crate) streaming_timing: bool,
    pub(crate) close_timing: bool,
}
```

### Builder method to define

In `src/client/builder/tracing.rs`:

```rust
impl<S, P, C> WireframeClientBuilder<S, P, C>
where
    S: Serializer + Send + Sync,
{
    pub fn tracing_config(mut self, config: TracingConfig) -> Self;
}
```

### Helper functions to define

In `src/client/tracing_helpers.rs`:

```rust
pub(crate) fn connect_span(config: &TracingConfig, peer_addr: &str) -> Span;
pub(crate) fn send_span(config: &TracingConfig, frame_bytes: usize) -> Span;
pub(crate) fn receive_span(config: &TracingConfig) -> Span;
pub(crate) fn send_envelope_span(config: &TracingConfig, correlation_id: u64, frame_bytes: usize) -> Span;
pub(crate) fn call_span(config: &TracingConfig) -> Span;
pub(crate) fn call_correlated_span(config: &TracingConfig) -> Span;
pub(crate) fn streaming_span(config: &TracingConfig, correlation_id: u64) -> Span;
pub(crate) fn close_span(config: &TracingConfig) -> Span;
pub(crate) fn emit_timing_event(start: Option<Instant>);
```

### Public accessor to define

In `src/client/response_stream.rs`:

```rust
impl<P, S, T, C> ResponseStream<'_, P, S, T, C> {
    #[must_use]
    pub fn frame_count(&self) -> usize;
}
```
