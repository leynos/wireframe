# Client Message API with Correlation Identifiers

This Execution Plan (ExecPlan) is a living document. The sections `Progress`,
`Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must
be kept up to date as work proceeds.

## Purpose / Big Picture

Wireframe's client library needs async `send`, `receive`, and `call` APIs that
encode `Message` implementers, forward correlation identifiers, and deserialize
typed responses using the configured serializer. This work extends the existing
client runtime to support envelope-aware messaging with automatic correlation
ID generation and validation.

Success is observable when:

- The client provides `send_envelope`, `receive_envelope`, and `call_correlated`
  methods that work with the `Packet` trait.
- Correlation IDs are auto-generated when not present on outbound envelopes.
- Response correlation IDs are validated against request IDs in
  `call_correlated`.
- A `CorrelationMismatch` error variant exists for mismatched correlation IDs.
- Unit tests cover correlation ID generation, stamping, and mismatch detection.
- Cucumber behavioural tests validate the messaging flow.
- `docs/users-guide.md` documents the new client APIs.
- `docs/roadmap.md` marks 10.2.1 as done.

## Progress

- [x] (2026-01-09) Draft ExecPlan for 10.2.1.
- [x] (2026-01-09) Add `correlation_counter: AtomicU64` field to
      `WireframeClient`.
- [x] (2026-01-09) Add `CorrelationMismatch` error variant to `ClientError`.
- [x] (2026-01-09) Implement `next_correlation_id()` method.
- [x] (2026-01-09) Implement `send_envelope<P: Packet>()` with auto-correlation.
- [x] (2026-01-09) Implement `receive_envelope<P: Packet>()` method.
- [x] (2026-01-09) Implement `call_correlated<P: Packet>()` with validation.
- [x] (2026-01-09) Add unit tests in `src/client/tests/messaging.rs`.
- [x] (2026-01-09) Add Cucumber feature and steps for client messaging.
- [x] (2026-01-09) Update `docs/users-guide.md` with client messaging API
      documentation.
- [x] (2026-01-09) Mark roadmap 10.2.1 as done.
- [ ] Run full validation (fmt, lint, test).

## Surprises & Discoveries

- The existing `send`, `receive`, and `call` methods work with raw `Message`
  types and do not use correlation IDs. The new envelope-aware APIs are
  additive and do not modify existing behaviour, preserving backwards
  compatibility.

- The `Packet` trait already provides `correlation_id()`, `into_parts()`, and
  `from_parts()` methods that enable generic envelope manipulation without
  knowing the concrete type.

- The `Envelope` struct's `payload()` method consumes `self`, requiring use of
  `into_parts()` to extract the payload while preserving other fields.

## Decision Log

- Decision: Add envelope-aware APIs (`send_envelope`, `receive_envelope`,
  `call_correlated`) alongside existing raw message APIs rather than modifying
  them. Rationale: Preserves backwards compatibility for existing client code
  that doesn't need correlation. Date/Author: 2026-01-09 (Codex).

- Decision: Use per-client `AtomicU64` counter starting from 1 for correlation
  ID generation. Rationale: Avoids global contention and ensures uniqueness
  within a connection. Counter starts at 1 so that 0 can be distinguished from
  auto-generated IDs if needed. Date/Author: 2026-01-09 (Codex).

- Decision: Validate correlation ID match in `call_correlated` and return
  `CorrelationMismatch` error on failure. Rationale: Mismatched correlation IDs
  indicate protocol violations or interleaved responses. Explicit error variant
  enables callers to handle the case appropriately. Date/Author: 2026-01-09
  (Codex).

- Decision: `send_envelope` returns the correlation ID that was used (auto-
  generated or explicit). Rationale: Allows callers to track correlation
  without inspecting the envelope after mutation. Date/Author: 2026-01-09
  (Codex).

## Outcomes & Retrospective

Not started yet.

## Context and Orientation

The wireframe client runtime (`src/client/runtime.rs`) already provides basic
`send`, `receive`, and `call` methods that work with raw `Message` types. These
methods use the configured `Serializer` for encoding/decoding but do not handle
correlation identifiers.

The server-side uses the `Envelope` struct (`src/app/envelope.rs`) and `Packet`
trait for routing and correlation. The `Envelope` contains:

- `id: u32` - Message type ID for routing
- `correlation_id: Option<u64>` - Optional correlation ID for request/response
  matching
- `payload: Vec<u8>` - Serialized message payload

The `CorrelatableFrame` trait (`src/correlation.rs`) provides generic access to
correlation IDs on frames.

Key references:

- `src/client/runtime.rs` - Client runtime implementation
- `src/client/error.rs` - Client error types
- `src/app/envelope.rs` - Envelope and Packet trait definitions
- `src/correlation.rs` - CorrelatableFrame trait
- `docs/multi-packet-and-streaming-responses-design.md` - Correlation ID
  requirements
- Testing guidance: `docs/rust-testing-with-rstest-fixtures.md`,
  `docs/behavioural-testing-in-rust-with-cucumber.md`

## Plan of Work

Start by adding the correlation ID counter field to `WireframeClient` and
initializing it in the builder. Add the `CorrelationMismatch` error variant to
`ClientError`. Then implement the three new methods: `send_envelope` (with
auto-correlation), `receive_envelope`, and `call_correlated` (with validation).
Add unit tests covering the new functionality. Add Cucumber behavioural tests.
Finally, update documentation and mark the roadmap item as done.

## Concrete Steps

1. Add `correlation_counter: AtomicU64` field to `WireframeClient` struct in
   `src/client/runtime.rs`.

2. Initialize the counter to 1 in the builder's `connect` method in
   `src/client/builder.rs`.

3. Add `CorrelationMismatch { expected: Option<u64>, received: Option<u64> }`
   variant to `ClientError` in `src/client/error.rs`.

4. Implement `next_correlation_id(&self) -> u64` method that atomically
   increments and returns the counter.

5. Implement `send_envelope<P: Packet>(&mut self, envelope: P) -> Result<u64,
   ClientError>`:
   - Auto-generate correlation ID if not present
   - Serialize and send the envelope
   - Return the correlation ID used

6. Implement `receive_envelope<P: Packet>(&mut self) -> Result<P, ClientError>`:
   - Receive and deserialize the frame as the packet type
   - Invoke error hook on failures

7. Implement `call_correlated<P: Packet>(&mut self, request: P) -> Result<P,
   ClientError>`:
   - Send request with auto-correlation
   - Receive response
   - Validate correlation ID matches
   - Return `CorrelationMismatch` error on mismatch

8. Add unit tests in `src/client/tests/messaging.rs`:
   - Test correlation ID generation (sequential, unique)
   - Test auto-stamping on outbound envelopes
   - Test explicit correlation ID preservation
   - Test correlation mismatch detection
   - Test error hook invocation on mismatch
   - Test round-trip with various payload sizes

9. Add behavioural tests using existing test infrastructure:
   - Create `tests/features/client_messaging.feature`
   - Create `tests/worlds/client_messaging.rs`
   - Create `tests/steps/client_messaging_steps.rs`
   - Register world in `tests/cucumber.rs` and `tests/world.rs`
   - Note: Uses current Cucumber infrastructure pending rstest-bdd migration
     per ADR-003.

10. Update `docs/users-guide.md` with client messaging API documentation:
    - Add section on correlation ID support
    - Document `send_envelope`, `receive_envelope`, `call_correlated` methods
    - Provide usage examples

11. Update `docs/roadmap.md` to mark 10.2.1 as done.

## Validation and Acceptance

Acceptance requires all of the following:

- New public methods compile and are documented with examples.
- Unit tests cover correlation ID generation, stamping, and validation.
- Behavioural tests for client messaging pass.
- Documentation updated in users guide.
- Roadmap item 10.2.1 marked as done.

Run validation from the repository root (use `tee` to capture full output):

    set -o pipefail
    timeout 300 make fmt 2>&1 | tee /tmp/wireframe-fmt.log
    echo "fmt exit: $?"

    set -o pipefail
    timeout 300 make markdownlint 2>&1 | tee /tmp/wireframe-markdownlint.log
    echo "markdownlint exit: $?"

    set -o pipefail
    timeout 300 make check-fmt 2>&1 | tee /tmp/wireframe-check-fmt.log
    echo "check-fmt exit: $?"

    set -o pipefail
    timeout 300 make lint 2>&1 | tee /tmp/wireframe-lint.log
    echo "lint exit: $?"

    set -o pipefail
    timeout 300 make test 2>&1 | tee /tmp/wireframe-test.log
    echo "test exit: $?"

## Idempotence and Recovery

All steps are additive and can be re-run safely. If a step fails, fix the
underlying issue and re-run only the affected command(s). Use the `tee` outputs
to locate the failure before retrying. Avoid destructive commands; if a local
change needs to be backed out, revert only the specific files edited for this
feature.

## Artifacts and Notes

Expected artifacts after completion:

- Modified `src/client/runtime.rs` with new methods and correlation counter.
- Modified `src/client/error.rs` with `CorrelationMismatch` variant.
- Modified `src/client/builder.rs` to initialize correlation counter.
- New `src/client/tests/messaging.rs` with unit tests.
- New `tests/features/client_messaging.feature` with behaviour-driven
  development
  (BDD) scenarios.
- New `tests/worlds/client_messaging.rs` with test world.
- New `tests/steps/client_messaging_steps.rs` with step definitions.
- Updated `docs/users-guide.md` with API documentation.
- Updated `docs/roadmap.md` with completion status.

## Interfaces and Dependencies

At the end of this work, the following public interfaces exist:

- `WireframeClient::next_correlation_id(&self) -> u64` - Generate unique ID
- `WireframeClient::send_envelope<P: Packet>(&mut self, envelope: P) ->
  Result<u64, ClientError>` - Send with auto-correlation
- `WireframeClient::receive_envelope<P: Packet>(&mut self) -> Result<P,
  ClientError>` - Receive typed envelope
- `WireframeClient::call_correlated<P: Packet>(&mut self, request: P) ->
  Result<P, ClientError>` - Request-response with validation
- `ClientError::CorrelationMismatch { expected: Option<u64>, received:
  Option<u64> }` - Error variant for mismatched IDs

The methods depend on the `Packet` trait which is implemented by `Envelope`.
