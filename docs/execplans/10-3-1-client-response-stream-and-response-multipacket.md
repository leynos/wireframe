# Support Response::Stream and Response::MultiPacket on the client

This ExecPlan is a living document. The sections `Constraints`, `Tolerances`,
`Risks`, `Progress`, `Surprises & Discoveries`, `Decision Log`, and
`Outcomes & Retrospective` must be kept up to date as work proceeds.

Status: COMPLETE

This document must be maintained in accordance with `AGENTS.md` at the
repository root, including all quality gates, commit conventions, and code
style requirements defined therein.

## Purpose / big picture

The wireframe server already supports `Response::Stream` and
`Response::MultiPacket` — a single request can produce a sequence of correlated
frames terminated by a protocol-defined end-of-stream marker. The client
library currently lacks the ability to consume these multi-frame responses.
After this work, a library consumer can send a request and receive a typed
async stream of response frames, with correlation validation, terminator
detection, and natural TCP back-pressure — completing the symmetry demanded by
design goal G5 in the multi-packet design document.

Observable outcome: a user calls `client.call_streaming(request)` and receives
a `ResponseStream<P>` that yields each data frame as `Result<P, ClientError>`,
terminating cleanly when the server's end-of-stream marker arrives. Running
`make test` passes, including new unit and BDD behavioural tests that exercise
single-frame, multi-frame, empty-stream, terminator validation, correlation
mismatch, and back-pressure scenarios.

## Constraints

- The `Packet` trait (`src/app/envelope.rs:64`) is a public API. Any
  addition must be backward-compatible (default method implementations).
- The `Envelope` struct (`src/app/envelope.rs:88`) is a public type;
  its existing fields and methods must not change.
- `WireframeClient` (`src/client/runtime.rs`) existing public methods
  (`send`, `receive`, `call`, `send_envelope`, `receive_envelope`,
  `call_correlated`, `close`, `codec_config`, `stream`) must remain unchanged
  in signature and behaviour.
- No new external crate dependencies may be added. All required
  functionality (`futures::Stream`, `tokio::sync::mpsc`,
  `tokio_util::codec::Framed`, `pin_project_lite`) is already available in the
  dependency tree.
- All code must pass `make check-fmt`, `make lint`, and `make test`.
- Documentation must use en-GB-oxendict spelling per `AGENTS.md`.
- No single source file may exceed 400 lines per `AGENTS.md`.
- BDD tests must use `rstest-bdd` 0.5.0 (not the removed Cucumber
  framework).

## Tolerances (exception triggers)

- Scope: if implementation requires changes to more than 20 files or
  500 lines of code (net), stop and escalate.
- Interface: if an existing public API signature must change (beyond
  adding defaulted trait methods), stop and escalate.
- Dependencies: if a new external crate dependency is required, stop
  and escalate.
- Iterations: if tests still fail after 5 attempts at a fix, stop and
  escalate.
- Ambiguity: if the terminator detection design proves incompatible
  with an existing protocol implementation, stop and present options.

## Risks

- Risk: Adding `is_stream_terminator` to `Packet` may surprise
  downstream protocol implementors who derive `Packet` on custom types.
  Severity: low. Likelihood: low. Mitigation: provide a default `false`
  implementation so existing code compiles without changes. Document the new
  method clearly.

- Risk: Splitting the `Framed` transport for streaming may introduce
  borrow-checker complexity. Severity: medium. Likelihood: medium. Mitigation:
  the streaming API borrows `&mut self` exclusively — the caller cannot
  interleave other operations while the stream is live. This is the same
  pattern used by `receive()` today. The `ResponseStream` struct will hold a
  mutable reference to the client's internals, not split ownership.

- Risk: BDD step definitions may conflict with existing step names.
  Severity: low. Likelihood: low. Mitigation: use scenario-specific prefixes in
  step wording and review existing steps before authoring new ones.

## Progress

- [x] Stage A: Trait extension — add `is_stream_terminator` to `Packet`.
- [x] Stage B: New client error variants — add `StreamTerminated` and
      `StreamCorrelationMismatch`.
- [x] Stage C: `ResponseStream` type — implement the core streaming
      receiver.
- [x] Stage D: Client methods — add `call_streaming` and
      `receive_streaming` to `WireframeClient`.
- [x] Stage E: Unit tests — rstest fixtures and parameterized cases.
      8 unit tests passing.
- [x] Stage F: BDD behavioural tests — feature file, fixtures, steps,
      scenarios. 4 BDD scenarios passing.
- [x] Stage G: Documentation — update design doc, users guide, roadmap.
- [x] Stage H: Validation — full quality gate pass.
      `make check-fmt`, `make lint`, `make test` all exit 0.

## Surprises & discoveries

- The initial `ResponseStream` implementation attempted to use
  `poll_unpin` on a `futures::stream::Next` future. This failed because `Next`
  is not `Unpin` by default. The fix was to poll
  `Pin::new(&mut this.client.framed).poll_next(cx)` directly on the `Framed`
  stream, which is `Unpin` because `T: ClientStream` requires `Unpin`.
- `Framed<T, LengthDelimitedCodec>` yields `BytesMut`, not `Bytes` or
  `[u8]`. Pattern matching needed adjustment to bind the result correctly and
  pass `&bytes` to the deserializer.
- No new crate dependencies were needed; `pin_project_lite` was
  initially considered but proved unnecessary since the `Framed` stream is
  already `Unpin`.

## Decision log

- Decision: Use `is_stream_terminator` on the `Packet` trait rather than
  a closure parameter or a separate `ClientProtocol` trait. Rationale: the
  `Packet` trait is already the central protocol abstraction shared by server
  and client. Adding a defaulted detection method mirrors the server's
  `stream_end_frame` production method symmetrically. A closure parameter would
  require passing a predicate at every call site, adding noise. A separate
  trait would add an abstraction layer with no clear benefit since `Packet`
  already encapsulates protocol-specific frame semantics. Date: 2026-02-15.

- Decision: Return a `ResponseStream<'_, P, S, C>` struct implementing
  `futures::Stream` rather than returning a boxed `FrameStream`. Rationale: a
  concrete type avoids dynamic dispatch overhead, allows the borrow checker to
  enforce exclusive access to the transport, and keeps the API consistent with
  the zero-cost abstractions philosophy of the crate. Date: 2026-02-15.

- Decision: The `ResponseStream` holds `&mut WireframeClient` (by
  borrowing the internal `framed` field and `serializer` through the client
  reference) rather than splitting the `Framed` transport. Rationale: splitting
  `Framed` into read/write halves requires `AsyncRead + AsyncWrite` bounds that
  may not compose cleanly with the existing `RewindStream` wrapper. Borrowing
  `&mut self` is simpler and the caller already cannot perform concurrent
  operations on a `&mut self` client. Date: 2026-02-15.

## Outcomes & retrospective

All stages completed successfully. Quality gates pass: `make check-fmt`,
`make lint`, `make test`, `make markdownlint` exit 0.

- 8 unit tests and 4 BDD scenarios cover the streaming client API.
- No new crate dependencies were required.
- The `Packet` trait extension is fully backward-compatible.
- `ResponseStream` borrows `&mut WireframeClient` exclusively, which
  proved simpler than splitting the `Framed` transport.
- The main surprises were around `poll_unpin` / `Unpin` semantics for
  `Framed` and `BytesMut` versus `Bytes` in pattern matching — both resolved
  without architectural changes.

## Context and orientation

### Repository structure (relevant files)

The wireframe crate lives at the repository root. Key paths:

    src/client/mod.rs          — Client module root; re-exports public types
    src/client/runtime.rs      — WireframeClient struct and core methods
    src/client/messaging.rs    — Envelope-based messaging (send_envelope,
                                 receive_envelope, call_correlated)
    src/client/error.rs        — ClientError, ClientProtocolError,
                                 ClientWireframeError
    src/client/builder/        — WireframeClientBuilder
    src/client/hooks.rs        — Client lifecycle hook types
    src/client/tests/          — Client unit tests
    src/response.rs            — Response enum, WireframeError, FrameStream
    src/app/envelope.rs        — Packet trait, Envelope, PacketParts
    src/correlation.rs         — CorrelatableFrame trait
    src/hooks.rs               — WireframeProtocol trait (server-side),
                                 ProtocolHooks, stream_end_frame
    src/connection/            — Server-side ConnectionActor (reference)
    src/connection/output.rs   — ActiveOutput enum (reference)
    src/connection/shutdown.rs — Server terminator emission (reference)

    wireframe_testing/src/     — Shared test utilities crate
    tests/features/            — BDD .feature files
    tests/fixtures/            — BDD fixture modules
    tests/steps/               — BDD step definitions
    tests/scenarios/           — BDD scenario wiring

    docs/multi-packet-and-streaming-responses-design.md
    docs/users-guide.md
    docs/roadmap.md

### Key types

`Packet` (trait, `src/app/envelope.rs:64`): protocol-level frame abstraction
with `id()`, `correlation_id()`, `into_parts()`, `from_parts()`. Extends
`CorrelatableFrame + Message + Send + Sync`.

`Envelope` (struct, `src/app/envelope.rs:88`): concrete `Packet` impl with
fields `id: u32`, `correlation_id: Option<u64>`, `payload: Vec<u8>`.

`WireframeClient<S, T, C>` (struct, `src/client/runtime.rs:57`): the client
runtime. Holds `framed: Framed<T, LengthDelimitedCodec>`, `serializer: S`,
`codec_config`, `connection_state`, hook handlers,
`correlation_counter: AtomicU64`.

`ClientError` (enum, `src/client/error.rs:27`): client error type with variants
`Wireframe`, `Serialize`, `PreambleEncode`, `PreambleWrite`, `PreambleRead`,
`PreambleTimeout`, `CorrelationMismatch`.

`receive_internal<R: Message>(&mut self)` (method,
`src/client/messaging.rs:213`): reads one frame from
`self.framed.next().await`, deserializes it, invokes error hook on failure.

### How the server emits streaming responses

The server's `ConnectionActor` polls `Response::Stream` or
`Response::MultiPacket` in its `select!` loop. Each frame is stamped with the
request's `correlation_id` via `MultiPacketContext`. When the stream/channel
closes, the actor calls `hooks.stream_end_frame()` which returns `Option<F>` —
a protocol-defined terminator frame. If `Some`, the terminator is stamped with
the same `correlation_id` and sent. The actor then calls `on_command_end`.

On the wire, the client therefore sees: N data frames (all carrying the same
`correlation_id`), followed by one terminator frame (same `correlation_id`,
protocol-specific content that identifies it as end-of-stream). If the
protocol's `stream_end_frame` returns `None`, the stream ends silently
(connection closes or next request begins).

### Back-pressure

Back-pressure is natural via TCP flow control. If the client reads slowly from
the socket, the TCP receive buffer fills, the server's TCP send buffer fills,
the server's `write` suspends, and the server stops polling the response
stream/channel. No explicit flow-control messages are needed.

## Plan of work

### Stage A: Extend the Packet trait with terminator detection

Add a defaulted method `is_stream_terminator(&self) -> bool` to the `Packet`
trait in `src/app/envelope.rs`. The default returns `false`. `Envelope`'s impl
inherits the default. Protocol crates override this method to match their
terminator format (e.g., checking a specific message ID or payload pattern).

File: `src/app/envelope.rs` Location: inside the `Packet` trait definition
(after `from_parts`) Change: add one method with doc comment and default
implementation.

### Stage B: Add new ClientError variants

Add `StreamTerminated` to `ClientError` in `src/client/error.rs`. This error is
returned if the caller attempts to read from a `ResponseStream` after the
terminator has been received, or if the connection closes unexpectedly
mid-stream.

Add `StreamCorrelationMismatch` variant to `ClientError` for when a frame
within a streaming response carries a different `correlation_id` than the
request. This is distinct from the existing `CorrelationMismatch` which covers
single-frame `call_correlated`.

File: `src/client/error.rs` Location: inside the `ClientError` enum Change: add
two new variants with doc comments and thiserror annotations.

### Stage C: Implement ResponseStream

Create a new file `src/client/response_stream.rs` containing the
`ResponseStream` struct. This is the core of the feature.

`ResponseStream` is a struct that implements `futures::Stream` with
`Item = Result<P, ClientError>`. It:

1. Holds a mutable reference to the client's `Framed` transport and
   serializer (via `&mut WireframeClient`).
2. Stores the expected `correlation_id` from the request.
3. Stores a `terminated: bool` flag.
4. On each `poll_next`:
   a. If `terminated`, returns `Poll::Ready(None)`. b. Polls
   `self.framed.next()` for the next raw frame. c. If the transport yields
   `None` (connection closed), returns an
      error (`ClientError::disconnected()`).
   d. Deserializes the raw bytes into `P` using the serializer. e. Validates
   `correlation_id` matches the expected value. If
      mismatch, returns `Err(ClientError::StreamCorrelationMismatch)`.
   f. Checks `P::is_stream_terminator()`. If true, sets
      `terminated = true` and returns `Poll::Ready(None)` — the
      terminator is NOT yielded to the consumer.
   g. Otherwise, yields `Poll::Ready(Some(Ok(frame)))`.

The struct uses `pin_project_lite::pin_project!` for safe pinning since it
wraps the `Framed` stream.

File: `src/client/response_stream.rs` (new file) Exports: `ResponseStream`

### Stage D: Add client methods

Add two new public methods to `WireframeClient`:

1. `receive_streaming<P: Packet>(&mut self, correlation_id: u64)`
   returns a `ResponseStream` that reads frames from the transport, validates
   correlation IDs, and terminates on the protocol's end-of-stream marker. This
   is the lower-level API for cases where the caller has already sent the
   request separately.

2. `call_streaming<P: Packet>(&mut self, request: P)` returns a
   `Result<ResponseStream, ClientError>`. It sends the request (stamping a
   correlation ID if needed), then returns a `ResponseStream` for consuming the
   response. This is the high-level convenience method.

File: `src/client/streaming.rs` (new file, to keep `runtime.rs` focused) The
new file contains an `impl` block on `WireframeClient` with the two methods.

File: `src/client/mod.rs` Change: add `mod response_stream; mod streaming;`
declarations and re-export `ResponseStream` from the public API.

### Stage E: Unit tests

Add unit tests in `src/client/tests/streaming.rs` (new file) using rstest
fixtures:

1. `response_stream_yields_data_frames_in_order` — server sends 3
   correlated frames + terminator; stream yields exactly the 3 data frames.
2. `response_stream_terminates_on_terminator` — stream returns `None`
   after terminator; subsequent polls also return `None`.
3. `response_stream_validates_correlation_id` — server sends a frame
   with wrong correlation ID; stream yields `StreamCorrelationMismatch` error.
4. `response_stream_handles_empty_stream` — server sends only
   terminator; stream yields no data frames.
5. `response_stream_handles_connection_close` — transport closes
   mid-stream; stream yields disconnected error.
6. `call_streaming_sends_request_and_returns_stream` — end-to-end test
   using a duplex in-memory transport.
7. `call_streaming_auto_generates_correlation_id` — verify the request
   gets a correlation ID stamped.

These tests use `tokio::io::duplex` for in-memory transport, matching the
pattern in `wireframe_testing/src/helpers.rs`.

### Stage F: BDD behavioural tests

Create a new feature file and supporting BDD infrastructure.

File: `tests/features/client_streaming.feature` (new)

Feature file content (draft):

    @client_streaming
    Feature: Client streaming response consumption
      Background:
        Given a streaming echo server

      Scenario: Client receives a multi-frame streaming response
        When the client sends a request expecting a streaming response
        Then all data frames are received in order
        And the stream terminates after the end-of-stream marker

      Scenario: Client receives an empty streaming response
        When the client sends a request expecting an empty stream
        Then no data frames are received
        And the stream terminates after the end-of-stream marker

      Scenario: Client detects correlation ID mismatch in stream
        Given a server that returns mismatched correlation IDs
        When the client sends a request expecting a streaming response
        Then a stream correlation mismatch error is returned

      Scenario: Client handles server disconnect during stream
        Given a server that disconnects after two frames
        When the client sends a request expecting a streaming response
        Then the first two data frames are received
        And a disconnection error is returned

File: `tests/fixtures/client_streaming.rs` (new) — world/fixture struct with
test server setup.

File: `tests/steps/client_streaming_steps.rs` (new) — step definitions using
`#[given]`, `#[when]`, `#[then]` macros.

File: `tests/scenarios/client_streaming_scenarios.rs` (new) — scenario wiring
with `#[scenario]` macros.

Update `tests/fixtures/mod.rs` to add `pub mod client_streaming;`. Update
`tests/steps/mod.rs` to add `mod client_streaming_steps;`. Update
`tests/scenarios/mod.rs` to add `mod client_streaming_scenarios;`.

### Stage G: Documentation

1. Update `docs/multi-packet-and-streaming-responses-design.md`:
   add a new section (Section 12 or similar) documenting the client-side
   streaming design decisions — the `is_stream_terminator` method on `Packet`,
   the `ResponseStream` type, and the `call_streaming` / `receive_streaming`
   methods.

2. Update `docs/users-guide.md`:
   add a subsection under the client section documenting:
   - How to implement `is_stream_terminator` on a custom `Packet` type.
   - How to use `call_streaming` to consume a streaming response.
   - How to use `receive_streaming` for lower-level control.
   - Back-pressure behaviour (natural via TCP).
   - Example code showing the streaming API end-to-end.

3. Update `docs/roadmap.md`:
   mark item 10.3.1 as done.

### Stage H: Validation

Run all quality gates:

    make check-fmt
    make lint
    make test

Ensure all new tests pass and no regressions are introduced.

## Concrete steps

All commands are run from the repository root (`/home/user/project`).

Stage A:

    # Edit src/app/envelope.rs — add is_stream_terminator to Packet
    # Verify compilation
    cargo check --all-targets --all-features

Stage B:

    # Edit src/client/error.rs — add StreamTerminated,
    #   StreamCorrelationMismatch variants
    cargo check --all-targets --all-features

Stage C:

    # Create src/client/response_stream.rs
    # Edit src/client/mod.rs — add module declaration and re-export
    cargo check --all-targets --all-features

Stage D:

    # Create src/client/streaming.rs
    # Edit src/client/mod.rs — add module declaration
    cargo check --all-targets --all-features

Stage E:

    # Create src/client/tests/streaming.rs
    # Edit src/client/tests/mod.rs — add module declaration
    set -o pipefail && make test 2>&1 | tee /tmp/test-stage-e.log

    Expected: all tests pass, including the 7+ new streaming tests.

Stage F:

    # Create tests/features/client_streaming.feature
    # Create tests/fixtures/client_streaming.rs
    # Create tests/steps/client_streaming_steps.rs
    # Create tests/scenarios/client_streaming_scenarios.rs
    # Edit tests/fixtures/mod.rs, tests/steps/mod.rs,
    #   tests/scenarios/mod.rs
    set -o pipefail && make test-bdd 2>&1 | tee /tmp/test-stage-f.log

    Expected: all BDD scenarios pass.

Stage G:

    # Edit docs/multi-packet-and-streaming-responses-design.md
    # Edit docs/users-guide.md
    # Edit docs/roadmap.md
    make markdownlint

Stage H:

    set -o pipefail && make check-fmt 2>&1 | tee /tmp/gate-fmt.log
    set -o pipefail && make lint 2>&1 | tee /tmp/gate-lint.log
    set -o pipefail && make test 2>&1 | tee /tmp/gate-test.log

    Expected: all three commands exit 0 with no warnings.

## Validation and acceptance

Quality criteria:

- Tests: `make test` passes. New tests include at least 7 unit tests
  (streaming.rs) and 4 BDD scenarios (client_streaming.feature). The new unit
  tests fail if `is_stream_terminator`, `ResponseStream`, or `call_streaming`
  are removed.
- Lint: `make lint` exits 0.
- Format: `make check-fmt` exits 0.
- Type safety: `cargo check --all-targets --all-features` exits 0.

Quality method:

- Run `make check-fmt && make lint && make test` and verify exit 0.
- Manually verify `docs/roadmap.md` shows 10.3.1 as done.
- Verify `docs/users-guide.md` contains streaming client documentation.

## Idempotence and recovery

All stages are idempotent — re-running any stage overwrites the same files and
re-runs the same checks. No database migrations or destructive operations are
involved.

If a stage fails mid-way, fix the issue and re-run the stage's validation
command. No rollback is needed beyond `git checkout` of affected files.

## Artifacts and notes

Expected `ResponseStream` type signature (sketch):

    pub struct ResponseStream<'a, P, S, T, C>
    where
        P: Packet,
        S: Serializer + Send + Sync,
        T: ClientStream,
    {
        client: &'a mut WireframeClient<S, T, C>,
        correlation_id: u64,
        terminated: bool,
        _phantom: PhantomData<P>,
    }

Expected `call_streaming` signature:

    pub async fn call_streaming<P: Packet>(
        &mut self,
        request: P,
    ) -> Result<ResponseStream<'_, P, S, T, C>, ClientError>

Expected `receive_streaming` signature:

    pub fn receive_streaming<P: Packet>(
        &mut self,
        correlation_id: u64,
    ) -> ResponseStream<'_, P, S, T, C>

Expected `is_stream_terminator` addition to `Packet`:

    /// Returns `true` if this frame represents an end-of-stream
    /// terminator.
    ///
    /// The default implementation returns `false`. Protocol
    /// implementations should override this to detect the
    /// protocol-specific terminator format emitted by the server's
    /// `stream_end_frame` hook.
    fn is_stream_terminator(&self) -> bool { false }

## Interfaces and dependencies

No new external dependencies. All types are built from existing crate
infrastructure.

New public types and methods after completion:

In `src/app/envelope.rs`, the `Packet` trait gains:

    fn is_stream_terminator(&self) -> bool { false }

In `src/client/error.rs`, `ClientError` gains:

    #[error("streaming response terminated")]
    StreamTerminated,

    #[error(
        "correlation ID mismatch in streaming response: \
         expected {expected:?}, received {received:?}"
    )]
    StreamCorrelationMismatch {
        expected: Option<u64>,
        received: Option<u64>,
    },

In `src/client/response_stream.rs`:

    pub struct ResponseStream<'a, P, S, T, C> { … }
    impl Stream for ResponseStream<…> {
        type Item = Result<P, ClientError>;
    }

In `src/client/streaming.rs`:

    impl WireframeClient<S, T, C> {
        pub async fn call_streaming<P: Packet>(…)
            -> Result<ResponseStream<…>, ClientError>;
        pub fn receive_streaming<P: Packet>(…)
            -> ResponseStream<…>;
    }

Re-exported from `src/client/mod.rs`:

    pub use response_stream::ResponseStream;
