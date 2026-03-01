# Implement `send_streaming(frame_header, body_reader)` transport helper (8.4.1)

This execution plan (ExecPlan) is a living document. The sections
`Constraints`, `Tolerances`, `Risks`, `Progress`, `Surprises & Discoveries`,
`Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work
proceeds.

Status: COMPLETE

No `PLANS.md` exists in this repository as of 2026-02-26.

## Purpose / big picture

Roadmap item 8.4.1 adds outbound streaming support to the wireframe client.
After this change, protocol implementations can send large request bodies as
multiple frames by providing an `AsyncRead` source and protocol-defined frame
headers. The framework handles chunking, framing, timeouts, and error hook
integration — eliminating duplicate "read N bytes, stamp header, write frame"
loops across protocol implementations while keeping header semantics
protocol-defined.

This is the outbound counterpart to the existing inbound `RequestBodyReader` /
`RequestBodyStream` in `src/request/mod.rs`.

Success is observable when:

- A caller can invoke `send_streaming` on a `WireframeClient` with an
  `AsyncRead` body and protocol-provided header bytes, and the helper emits
  correctly framed chunks over the transport.
- Timeout expiry returns `std::io::ErrorKind::TimedOut` without emitting a
  partial frame.
- Error hooks fire on failure, consistent with other client send methods.
- Unit tests (`rstest`) validate core chunking, timeout, and error behaviour.
- Behavioural (Behaviour-Driven Development, BDD) tests (`rstest-bdd` v0.5.0)
  validate end-to-end streaming send scenarios.
- Documentation is updated in ADR 0002, `docs/users-guide.md`, and
  `docs/roadmap.md`.
- All quality gates pass: `make check-fmt`, `make lint`, `make test`,
  `make markdownlint`.

## Constraints

- Scope is strictly roadmap items 8.4.1–8.4.5 (core helper, chunk
  configuration, timeout handling, hook integration, and tests).
- Do not modify public API signatures of existing methods (`send`,
  `send_envelope`, `call_streaming`, etc.).
- Do not add new external dependencies.
- Keep all modified or new source files at or below the 400-line repository
  guideline.
- `send_streaming` MUST NOT emit a partial frame on timeout or perform
  automatic retries. Timeouts MUST surface as `std::io::ErrorKind::TimedOut`.
- Callers MUST be able to assume the operation may have been partially
  successful (some frames sent before the error).
- Follow en-GB-oxendict spelling in comments and documentation.
- Validate with `rstest` unit tests and `rstest-bdd` behavioural tests.
- Follow testing guidance from `docs/rust-testing-with-rstest-fixtures.md`,
  `docs/reliable-testing-in-rust-via-dependency-injection.md`, and
  `docs/rstest-bdd-users-guide.md`.
- Record implementation decisions in ADR 0002.
- Update `docs/users-guide.md` for any consumer-visible behavioural change.
- Mark roadmap items 8.4.1–8.4.5 done only after all gates pass.

## Tolerances (exception triggers)

- Scope: if implementation requires changes to more than 18 files or more than
  900 net lines of code, stop and escalate.
- Interface: if implementing 8.4.1 requires changing existing public method
  signatures, stop and escalate.
- Dependencies: if any new external crate is required, stop and escalate.
- Iterations: if the same failing gate persists after 3 focused attempts, stop
  and escalate.
- Time: if any single stage exceeds 4 hours elapsed effort, stop and escalate.
- File size: if any source file would exceed 400 lines, extract into
  submodules before continuing.
- Ambiguity: if the `frame_header` semantics require protocol-specific
  interpretation not documented in ADR 0002, stop and present options.

## Risks

- Risk: `src/client/runtime.rs` is currently 351 lines. Adding
  `send_streaming` as a method there would push it over 400 lines. Severity:
  high Likelihood: high Mitigation: place `send_streaming` in a new file
  `src/client/send_streaming.rs` (symmetric with `streaming.rs` for inbound),
  keeping `runtime.rs` unchanged.

- Risk: the chunk size calculation must account for the `frame_header` length
  so that header + chunk fits within `max_frame_length`. Getting this wrong
  produces frames that exceed the codec limit and cause transport errors.
  Severity: high Likelihood: medium Mitigation: derive `max_chunk_size` as
  `max_frame_length - header_len`, validate at call time, and fail fast with a
  clear error if `header_len >= max_frame_length`.

- Risk: a timeout that fires during `framed.send()` could leave the `Framed`
  transport in an inconsistent state. Severity: medium Likelihood: low
  Mitigation: `LengthDelimitedCodec` writes the length prefix and payload
  atomically via the Encoder trait. A timeout during `send()` will cancel the
  future, but the underlying TCP socket may have partial bytes. Per ADR 0002,
  this is a transport-level write failure and the connection actor SHOULD
  terminate the connection. Document this explicitly.

- Risk: unit tests for timeout behaviour can be flaky with real time.
  Severity: medium Likelihood: medium Mitigation: use an mpsc-backed
  `StreamReader` (`blocking_reader()` in `send_streaming_infra.rs`) as the body
  source; the reader genuinely blocks, so `tokio::time::timeout` fires reliably
  with a short wall-clock duration (50 ms). See *Surprises & Discoveries* for
  why `tokio::time::pause()` was rejected.

- Risk: BDD step text could conflict with existing `client_streaming` steps.
  Severity: low Likelihood: medium Mitigation: use a `send-streaming` step
  prefix for all step definitions.

## Progress

- [x] (2026-02-26 00:00Z) Drafted ExecPlan for roadmap item 8.4.1.
- [x] (2026-02-26 00:05Z) Stage A: implement `SendStreamingConfig`,
  `SendStreamingOutcome`, and core `send_streaming` function.
- [x] (2026-02-26 00:10Z) Stage B: add unit tests (`rstest`). 12/12 pass.
- [x] (2026-02-26 00:15Z) Stage C: add behavioural tests (`rstest-bdd`
  v0.5.0). 4/4 pass.
- [x] (2026-02-26 00:20Z) Stage D: update documentation (ADR 0002,
  users-guide, roadmap).
- [x] (2026-02-26 00:25Z) Stage E: run all quality gates and finalize. All
  gates green.

## Surprises & discoveries

- Observation: `tokio::time::pause()` + `advance()` does not work when the
  blocked `AsyncRead` is a `tokio::io::duplex` reader in the same task.
  Evidence: initial timeout tests failed because the duplex reader blocked
  indefinitely and the timer never fired even after `advance()`. Impact:
  timeout tests use `mpsc`-backed `StreamReader` instead, allowing the
  `tokio::time::timeout` to fire naturally with real wall-clock time (50ms).
  This is reliable because the reader genuinely blocks.
- Observation: BDD fixture worlds that share data between a spawned server
  task and verification methods via `Arc<tokio::sync::Mutex<...>>` cause "no
  reactor running" panics when `rt.block_on()` is called from outside an async
  context on a `current_thread` runtime. Fix: redesign the server task to
  return collected data via `JoinHandle<Vec<...>>`, then await the handle to
  move data into a plain `Vec` field before verification. This matches the
  pattern used in `client_streaming` fixtures.
- Observation: this project enforces strict clippy lints including
  `indexing_slicing`, `panic_in_result_fn`, `integer_division_remainder_used`,
  `cast_possible_truncation`, and `allow_attributes_without_reason`. Test code
  must use `.get()` instead of `[]`, return `Err(...)` instead of
  `assert!`/`panic!`, and provide reasons on `#[expect]` attributes.

## Decision log

- Decision: place `send_streaming` in a new file `src/client/send_streaming.rs`
  rather than extending `runtime.rs`. Rationale: `runtime.rs` is at 351 lines.
  The `streaming.rs` / `send_streaming.rs` naming creates a symmetric pair
  (inbound/outbound). This follows the same decomposition pattern used
  elsewhere (`connection/frame.rs`, `connection/response.rs`, etc.).
  Date/Author: 2026-02-26 / ExecPlan draft.

- Decision: use a configuration struct (`SendStreamingConfig`) rather than
  individual parameters. Rationale: ADR 0002 specifies chunk size and timeout
  as configurable. A config struct avoids parameter bloat and is extensible.
  This follows the project's documented preference for grouping related
  parameters in meaningfully named structs (AGENTS.md). Date/Author: 2026-02-26
  / ExecPlan draft.

- Decision: `send_streaming` operates on raw bytes (`frame_header: &[u8]`,
  `body_reader: R`) rather than typed envelopes. Rationale: ADR 0002 says
  "keeping header semantics protocol-defined". The helper is a transport-level
  utility. It prepends protocol-provided header bytes to each chunk and sends
  the concatenation as a frame. The caller controls serialization of headers.
  Date/Author: 2026-02-26 / ExecPlan draft.

- Decision: return `SendStreamingOutcome` (with `frames_sent: u64`) on success
  rather than `()`. Rationale: since the operation may be partially successful,
  callers benefit from knowing how many frames were emitted. This is cheap to
  track and useful for logging/instrumentation. Date/Author: 2026-02-26 /
  ExecPlan draft.

- Decision: wrap the entire `send_streaming` operation in a single
  `tokio::time::timeout` rather than per-frame timeouts. Rationale: ADR 0002
  says "Timeouts MUST fail the current send operation". A per-operation timeout
  is simpler, prevents unbounded total duration, and matches the
  `preamble_exchange.rs` pattern. Per-frame timeouts would be a separate
  concern and could be added later if needed. Date/Author: 2026-02-26 /
  ExecPlan draft.

- Decision: timeout errors surface as
  `ClientError::Wireframe(WireframeError::Io(_))` with
  `io::ErrorKind::TimedOut`. Rationale: `ClientError` already has
  `From<io::Error>` which maps to `Wireframe(WireframeError::from_io(value))`.
  Constructing an `io::Error::new(io::ErrorKind::TimedOut, ...)` and converting
  is consistent with how other I/O errors are surfaced by the client. This
  matches ADR 0002's requirement to "return `std::io::ErrorKind::TimedOut`".
  Date/Author: 2026-02-26 / ExecPlan draft.

## Outcomes & retrospective

All acceptance criteria met. Summary of deliverables:

- `src/client/send_streaming.rs` (~170 lines): `SendStreamingConfig`,
  `SendStreamingOutcome`, `send_streaming` method, `effective_chunk_size`
  helper.
- `src/client/tests/send_streaming.rs` (~250 lines): 12 unit tests.
- `src/client/tests/send_streaming_infra.rs` (~165 lines): test
  infrastructure.
- `tests/features/client_send_streaming.feature`: 4 BDD scenarios.
- `tests/fixtures/client_send_streaming.rs`: BDD world struct.
- `tests/steps/client_send_streaming_steps.rs`: BDD step definitions.
- `tests/scenarios/client_send_streaming_scenarios.rs`: BDD scenario
  bindings.
- ADR 0002 updated with implementation decisions for Section 4.
- `docs/users-guide.md` updated with outbound streaming sends subsection.
- `docs/roadmap.md` items 8.4.1–8.4.5 marked done.

Quality gates: `make fmt`, `make check-fmt`, `make lint`, `make test`,
`make markdownlint` all pass.

## Context and orientation

The wireframe client (`src/client/`) provides a layered API for binary protocol
communication:

- Layer 1 (`runtime.rs`): raw `send`/`receive` of serialized messages via
  `Framed<T, LengthDelimitedCodec>`.
- Layer 2 (`messaging.rs`): envelope-aware APIs with correlation ID management.
- Layer 3 (`streaming.rs` + `response_stream.rs`): inbound streaming response
  consumption.

This plan adds a Layer 1.5 capability: outbound streaming sends that operate on
raw bytes below the serialization layer but above the transport.

Key types and their locations:

- `WireframeClient<S, T, C>` in `src/client/runtime.rs` — the main client
  struct. Fields: `framed` (`Framed<T, LengthDelimitedCodec>`), `serializer`,
  `codec_config`, `connection_state`, `on_disconnect`, `on_error`,
  `correlation_counter`.
- `ClientError` in `src/client/error.rs` — the client error enum.
  `From<io::Error>` maps to `ClientError::Wireframe(WireframeError::Io(_))`.
- `ClientCodecConfig` in `src/client/codec_config.rs` — codec configuration
  with `max_frame_length_value()` returning the maximum frame payload size.
- `invoke_error_hook` in `src/client/messaging.rs` — calls the registered
  error handler. Declared as `pub(crate)` on `WireframeClient`.
- `ClientStream` trait in `src/client/runtime.rs` — `AsyncRead + AsyncWrite +
  Unpin` trait alias.

Inbound counterpart (for reference):

- `RequestBodyStream` in `src/request/mod.rs` —
  `Pin<Box<dyn Stream<Item = Result<Bytes, io::Error>> + Send + 'static>>`.
- `RequestBodyReader` in `src/request/mod.rs` — `AsyncRead` adapter over
  `RequestBodyStream` using `tokio_util::io::StreamReader`.

Existing send patterns:

- `runtime.rs` `send()`: serialize → `self.framed.send(Bytes::from(bytes))` →
  error hook on failure.
- `messaging.rs` `send_envelope()`: same + correlation ID auto-generation.
- `streaming.rs` `call_streaming()`: same + returns `ResponseStream`.

Client test infrastructure:

- `src/client/tests/mod.rs` — unit test module with submodules.
- `src/client/tests/streaming_infra.rs` — `TestServer`, `TestStreamEnvelope`,
  newtypes, `create_test_client()`, `spawn_test_server()`.
- `src/client/tests/streaming.rs` — inbound streaming unit tests.

BDD test infrastructure (4-file pattern):

- `tests/features/client_streaming.feature` — Gherkin feature file.
- `tests/fixtures/client_streaming.rs` — `ClientStreamingWorld` struct and
  `client_streaming_world()` fixture.
- `tests/steps/client_streaming_steps.rs` — step definitions using
  `rstest_bdd_macros`.
- `tests/scenarios/client_streaming_scenarios.rs` — `#[scenario]` test
  functions.
- Registration in `tests/fixtures/mod.rs`, `tests/steps/mod.rs`,
  `tests/scenarios/mod.rs`.

## Plan of work

### Stage A: implement `SendStreamingConfig`, `SendStreamingOutcome`, and core `send_streaming` function

Create `src/client/send_streaming.rs` containing the new types and the
`send_streaming` method on `WireframeClient`.

The file structure:

1. Module-level `//!` comment explaining outbound streaming sends.
2. `SendStreamingConfig` struct with:
   - `chunk_size: Option<usize>` — `None` means auto-derive from
     `max_frame_length - header_len`.
   - `timeout: Option<Duration>` — `None` means no timeout.
   - `#[derive(Clone, Copy, Debug, Default)]`.
   - Builder methods: `with_chunk_size(usize)`, `with_timeout(Duration)`.
3. `SendStreamingOutcome` struct with:
   - `frames_sent: u64`.
   - `#[derive(Clone, Copy, Debug, PartialEq, Eq)]`.
   - Accessor: `frames_sent(&self) -> u64`.
4. `impl<S, T, C> WireframeClient<S, T, C> where S: Serializer + Send + Sync,
   T: ClientStream` block containing `send_streaming`.

The `send_streaming` function:

```rust
pub async fn send_streaming<R: AsyncRead + Unpin>(
    &mut self,
    frame_header: &[u8],
    body_reader: R,
    config: SendStreamingConfig,
) -> Result<SendStreamingOutcome, ClientError>
```

Internal logic:

1. Compute `effective_chunk_size`:
   - `max_frame_length` from `self.codec_config.max_frame_length_value()`.
   - If `frame_header.len() >= max_frame_length`, return error immediately
     (`io::Error::new(InvalidInput, "frame header exceeds max frame length")`).
   - If `config.chunk_size` is `Some(size)`, use
     `size.min(max_frame_length - header_len)`.
   - Otherwise, derive as `max_frame_length - header_len`.
2. Delegate to an inner async function `send_streaming_inner` to keep timeout
   wrapping clean.
3. If `config.timeout` is `Some(duration)`, wrap the inner function in
   `tokio::time::timeout(duration, inner).await`. On `Elapsed`, construct
   `io::Error::new(ErrorKind::TimedOut, ...)`, invoke error hook, return the
   error as `ClientError`.
4. If no timeout, call inner directly.

The inner function:

1. Allocate a reusable `Vec<u8>` buffer of `effective_chunk_size` bytes
   (zeroed).
2. Loop:
   a. Read up to `effective_chunk_size` bytes from `body_reader` using
      `AsyncReadExt::read()`.
   b. If 0 bytes read (EOF), break. c. Construct frame: `Vec<u8>` of capacity
   `header_len + n`, extend with
      `frame_header`, extend with `buffer[..n]`.
   d. Call `self.framed.send(Bytes::from(frame_bytes)).await`. e. On send
   error, invoke error hook and return the error. f. Increment `frames_sent`
   counter.
3. Return `Ok(SendStreamingOutcome { frames_sent })`.

Register the new module in `src/client/mod.rs`:

- Add `mod send_streaming;` to the module declarations.
- Add `pub use send_streaming::{SendStreamingConfig, SendStreamingOutcome};`
  to the re-exports.

Go/no-go: `cargo check` must succeed before proceeding.

### Stage B: add unit tests (`rstest`)

Create `src/client/tests/send_streaming.rs` with unit tests. This file will
mirror the patterns in `src/client/tests/streaming.rs` and
`src/client/tests/streaming_infra.rs`.

Test infrastructure needed:

- A "receiving server" that accepts a connection, reads frames from the
  transport, and stores them for assertion. This is the inverse of the existing
  `spawn_test_server` which sends frames. It can reuse `TestServer` and
  `create_test_client` from `streaming_infra.rs`.
- `tokio::io::Cursor` as a simple in-memory `AsyncRead` source for body data.

If the infra exceeds 200 lines, split into `send_streaming.rs` and
`send_streaming_infra.rs` (matching the existing `streaming.rs` /
`streaming_infra.rs` pattern).

Test cases:

1. `send_streaming_emits_correct_frames` — 300-byte body, 4-byte header,
   100-byte chunk size. Expect 3 frames, each starting with the header followed
   by up to 100 bytes of body. Verify the server receives exactly 3 frames with
   correct content.

2. `send_streaming_handles_exact_chunk_boundary` — provide exactly
   `chunk_size` bytes. Expect exactly 1 frame (no trailing empty frame).

3. `send_streaming_handles_partial_final_chunk` — `chunk_size + 1` bytes.
   Expect 2 frames (one full, one with 1 byte).

4. `send_streaming_empty_body` — 0 bytes. Expect 0 frames sent.
   `SendStreamingOutcome::frames_sent() == 0`.

5. `send_streaming_rejects_oversized_header` — header whose length >=
   `max_frame_length`. Expect an immediate error.

6. `send_streaming_auto_derives_chunk_size` — do not set `chunk_size` in
   config. Verify that frames are emitted with payload size =
   `max_frame_length - header_len`.

7. `send_streaming_timeout_returns_timed_out` — provide an `AsyncRead` that
   blocks indefinitely using an `mpsc`-backed `StreamReader` (see Surprises &
   Discoveries for why `tokio::time::pause()` with a duplex reader does not
   work). Set a short timeout. Verify the error is `io::ErrorKind::TimedOut`.

8. `send_streaming_invokes_error_hook_on_failure` — register an error hook
   and verify it is called when a transport error occurs (drop server
   connection before client finishes sending).

9. `send_streaming_invokes_error_hook_on_timeout` — register an error hook
   and verify it is called when timeout fires.

10. `send_streaming_reports_frames_sent` — send 5 chunks, verify
    `outcome.frames_sent() == 5`.

Register in `src/client/tests/mod.rs`: add `mod send_streaming;`.

Go/no-go: `cargo test --lib send_streaming` must pass.

### Stage C: add behavioural tests (`rstest-bdd` v0.5.0)

Create the four BDD files following the established pattern:

1. `tests/features/client_send_streaming.feature`:

```gherkin
@client_send_streaming
Feature: Client outbound streaming sends
  The client can send large request bodies as multiple frames by reading
  from an AsyncRead source with protocol-provided frame headers.

  Background:
    Given a send-streaming receiving server

  Scenario: Client sends a multi-chunk body
    When the client streams 300 bytes with a 4 byte header and 100 byte chunks
    Then the server receives 3 frames
    And each received frame starts with the protocol header

  Scenario: Client sends an empty body
    When the client streams 0 bytes with a 4 byte header and 100 byte chunks
    Then the server receives 0 frames

  Scenario: Client send operation times out on a slow body reader
    Given a send-streaming body reader that blocks indefinitely
    When the client streams with a 100 ms timeout
    Then a TimedOut error is returned
    And the send-streaming error hook is invoked

  Scenario: Client handles transport failure during streaming send
    Given a send-streaming server that disconnects immediately
    When the client streams 300 bytes with a 4 byte header and 100 byte chunks
    Then a transport error is returned
    And the send-streaming error hook is invoked
```

1. `tests/fixtures/client_send_streaming.rs`:
   - `ClientSendStreamingWorld` struct holding: `runtime`, `runtime_error`,
     `addr`, `server` handle, `client`, `received_frames` (`Vec<Vec<u8>>`),
     `frames_sent` (outcome), `last_error`, `error_hook_invoked`, `protocol_header`.
   - `client_send_streaming_world()` rstest fixture function.
   - Helper methods: `start_receiving_server()`, `start_disconnecting_server()`,
     `connect_client()`, `do_send_streaming(body_size, header_size, chunk_size)`,
     `do_send_streaming_with_timeout(duration)`.
   - Uses the `block_on` pattern from `ClientStreamingWorld`.

2. `tests/steps/client_send_streaming_steps.rs`:
   - Step functions with `send-streaming`-prefixed step text to avoid
     collisions.

3. `tests/scenarios/client_send_streaming_scenarios.rs`:
   - `#[scenario]` functions for each scenario.

4. Register in:
   - `tests/fixtures/mod.rs`: `pub mod client_send_streaming;`
   - `tests/steps/mod.rs`: `mod client_send_streaming_steps;`
   - `tests/scenarios/mod.rs`: `mod client_send_streaming_scenarios;`

Go/no-go: `cargo test --test bdd --all-features client_send_streaming` must
pass.

### Stage D: documentation and roadmap updates

1. `docs/adr-002-streaming-requests-and-shared-message-assembly.md`:
   Add an "Implementation decisions (2026-02-26)" subsection under Section 4
   documenting:
   - `send_streaming` lives in `src/client/send_streaming.rs`.
   - `SendStreamingConfig` provides chunk size and timeout configuration.
   - Chunk size auto-derives from `max_frame_length - header_len` when not
     explicitly set.
   - Timeout wraps the entire operation (not per-frame).
   - Error hook integration is consistent with other client send methods.

2. `docs/users-guide.md`:
   Add a new subsection under the client documentation titled "Outbound
   streaming sends" explaining:
   - When to use `send_streaming` (large request bodies, protocol-defined
     chunking).
   - The `SendStreamingConfig` builder API.
   - Timeout semantics and partial-success assumption.
   - A code example showing typical usage.

3. `docs/roadmap.md`:
   Mark items 8.4.1–8.4.5 as done (`[x]`).

### Stage E: quality gates and final verification

Run all required gates and capture logs with `tee`:

```shell
set -o pipefail
make fmt 2>&1 | tee /tmp/wireframe-8-4-1-fmt.log
make markdownlint MDLINT=/root/.bun/bin/markdownlint-cli2 2>&1 \
  | tee /tmp/wireframe-8-4-1-markdownlint.log
make check-fmt 2>&1 | tee /tmp/wireframe-8-4-1-check-fmt.log
make lint 2>&1 | tee /tmp/wireframe-8-4-1-lint.log
make test 2>&1 | tee /tmp/wireframe-8-4-1-test.log
```

No roadmap checkbox update until every gate is green.

## Concrete steps

Run from the repository root (`/home/user/project`).

1. Create `src/client/send_streaming.rs` with `SendStreamingConfig`,
   `SendStreamingOutcome`, and `send_streaming` method.

2. Update `src/client/mod.rs` to declare and re-export the new module.

3. Verify compilation:

```shell
set -o pipefail
cargo check 2>&1 | tee /tmp/wireframe-8-4-1-check.log
```

1. Create `src/client/tests/send_streaming.rs` (and
   `src/client/tests/send_streaming_infra.rs` if needed). Update
   `src/client/tests/mod.rs`.

2. Run focused unit tests:

```shell
set -o pipefail
cargo test --lib send_streaming 2>&1 | tee /tmp/wireframe-8-4-1-unit.log
```

1. Create BDD test files:
   `tests/features/client_send_streaming.feature`,
   `tests/fixtures/client_send_streaming.rs`,
   `tests/steps/client_send_streaming_steps.rs`,
   `tests/scenarios/client_send_streaming_scenarios.rs`. Register in
   `tests/fixtures/mod.rs`, `tests/steps/mod.rs`, `tests/scenarios/mod.rs`.

2. Run focused BDD tests:

```shell
set -o pipefail
cargo test --test bdd --all-features client_send_streaming 2>&1 \
  | tee /tmp/wireframe-8-4-1-bdd.log
```

1. Update documentation: ADR 0002, users-guide, roadmap.

2. Run full quality gates:

```shell
set -o pipefail
make fmt 2>&1 | tee /tmp/wireframe-8-4-1-fmt.log
make markdownlint MDLINT=/root/.bun/bin/markdownlint-cli2 2>&1 \
  | tee /tmp/wireframe-8-4-1-markdownlint.log
make check-fmt 2>&1 | tee /tmp/wireframe-8-4-1-check-fmt.log
make lint 2>&1 | tee /tmp/wireframe-8-4-1-lint.log
make test 2>&1 | tee /tmp/wireframe-8-4-1-test.log
```

1. If any gate fails, fix only the failing area and rerun the failing command
    until green, then rerun affected downstream gates.

## Validation and acceptance

Acceptance criteria:

- `send_streaming` correctly chunks an `AsyncRead` body into frames, each
  prepended with protocol-provided header bytes.
- Chunk size is auto-derived from `max_frame_length` when not explicitly
  configured.
- Timeout returns `io::ErrorKind::TimedOut` without emitting a partial frame.
- Error hook is invoked on all failure paths.
- `SendStreamingOutcome` reports the number of frames actually sent.
- Unit tests (`rstest`) cover: normal chunking, boundary conditions, empty
  body, oversized header rejection, timeout, error hook invocation.
- Behavioural tests (`rstest-bdd` 0.5.0) cover: multi-chunk send, empty send,
  timeout, server disconnect.
- ADR 0002 is updated with implementation decisions.
- Users-guide documents the new API.
- Roadmap marks 8.4.1–8.4.5 done.

Quality criteria:

- tests: `make test` passes.
- lint: `make lint` passes with no warnings.
- formatting: `make fmt` and `make check-fmt` pass.
- markdown: `make markdownlint` passes.

## Idempotence and recovery

All planned edits are additive and safe to rerun.

If a step fails:

- Preserve local changes.
- Inspect the relevant `/tmp/wireframe-8-4-1-*.log` file.
- Apply the minimal fix.
- Rerun only the failed command first, then downstream gates.

Avoid destructive git commands. If rollback is required, revert only files
changed for 8.4.1.

## Artefacts and notes

Expected artefacts after completion:

- New: `src/client/send_streaming.rs` (~150–200 lines).
- New: `src/client/tests/send_streaming.rs` (~250–350 lines).
- New: `src/client/tests/send_streaming_infra.rs` (~100–150 lines, if needed).
- Modified: `src/client/mod.rs` (add module declaration + re-exports).
- Modified: `src/client/tests/mod.rs` (add module declaration).
- New: `tests/features/client_send_streaming.feature` (~30 lines).
- New: `tests/fixtures/client_send_streaming.rs` (~200 lines).
- New: `tests/steps/client_send_streaming_steps.rs` (~100 lines).
- New: `tests/scenarios/client_send_streaming_scenarios.rs` (~30 lines).
- Modified: `tests/fixtures/mod.rs` (add module declaration).
- Modified: `tests/steps/mod.rs` (add module declaration).
- Modified: `tests/scenarios/mod.rs` (add module declaration).
- Modified: `docs/adr-002-streaming-requests-and-shared-message-assembly.md`.
- Modified: `docs/users-guide.md`.
- Modified: `docs/roadmap.md` (mark 8.4.1–8.4.5 done).
- New: `docs/execplans/8-4-1-send-streaming-helper.md` (this file).
- Gate logs: `/tmp/wireframe-8-4-1-*.log`.

Estimated total: ~15–17 files changed/added, ~800–1000 net lines.

## Interfaces and dependencies

No new external dependencies are required.

Internal interfaces expected at the end of this milestone:

In `src/client/send_streaming.rs`:

```rust
use std::time::Duration;

/// Configuration for outbound streaming sends.
///
/// Controls chunk sizing and timeout behaviour for
/// [`WireframeClient::send_streaming`].
#[derive(Clone, Copy, Debug, Default)]
pub struct SendStreamingConfig {
    chunk_size: Option<usize>,
    timeout: Option<Duration>,
}

impl SendStreamingConfig {
    /// Set the maximum number of body bytes per frame.
    ///
    /// When not set, the chunk size is derived as
    /// `max_frame_length - header_len`.
    #[must_use]
    pub fn with_chunk_size(mut self, size: usize) -> Self {
        self.chunk_size = Some(size);
        self
    }

    /// Set a timeout for the entire streaming send operation.
    ///
    /// If the timeout elapses, `send_streaming` returns
    /// `std::io::ErrorKind::TimedOut` and stops emitting frames.
    /// Any frames already sent remain sent.
    #[must_use]
    pub fn with_timeout(mut self, duration: Duration) -> Self {
        self.timeout = Some(duration);
        self
    }
}

/// Outcome of a successful streaming send operation.
///
/// Reports the number of frames emitted during the operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SendStreamingOutcome {
    frames_sent: u64,
}

impl SendStreamingOutcome {
    /// Return the number of frames emitted during the operation.
    #[must_use]
    pub const fn frames_sent(&self) -> u64 { self.frames_sent }
}
```

The `send_streaming` method signature on `WireframeClient`:

```rust
impl<S, T, C> WireframeClient<S, T, C>
where
    S: Serializer + Send + Sync,
    T: ClientStream,
{
    /// Send a large body as multiple frames, reading from an async source.
    ///
    /// Each emitted frame consists of `frame_header` followed by up to
    /// `chunk_size` bytes read from `body_reader`. The chunk size defaults
    /// to `max_frame_length - frame_header.len()` when not explicitly
    /// configured.
    ///
    /// # Timeout semantics
    ///
    /// When a timeout is configured, it wraps the entire operation. If the
    /// timeout elapses, no further frames are emitted and
    /// `std::io::ErrorKind::TimedOut` is returned. Any frames already sent
    /// remain sent — callers must assume the operation may have been
    /// partially successful. A `TimedOut` error is a transport-level write
    /// failure; the connection SHOULD be terminated and MUST NOT be reused
    /// (see ADR 0002, §4).
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] if:
    /// - The frame header is longer than `max_frame_length` (no room for
    ///   body data).
    /// - A transport I/O error occurs during a frame send.
    /// - The configured timeout elapses.
    ///
    /// The error hook is invoked before any error is returned.
    pub async fn send_streaming<R: AsyncRead + Unpin>(
        &mut self,
        frame_header: &[u8],
        body_reader: R,
        config: SendStreamingConfig,
    ) -> Result<SendStreamingOutcome, ClientError> {
        // Implementation as described in Stage A.
    }
}
```

In `src/client/mod.rs`, add to re-exports:

```rust
pub use send_streaming::{SendStreamingConfig, SendStreamingOutcome};
```
