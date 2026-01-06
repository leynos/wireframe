# Introduce a CodecError taxonomy

This execution plan (ExecPlan) is a living document. The sections `Progress`,
`Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must
be kept up to date as work proceeds.

## Purpose / Big Picture

The codec layer currently uses generic `io::Error` throughout, with no
taxonomic distinction between framing errors (wire-level frame boundary
issues), protocol errors (semantic violations after frame extraction), and
transport I/O failures. This makes it difficult to implement recovery policies,
structured logging, or protocol-specific error handling. This change introduces
a `CodecError` taxonomy that classifies errors into these three categories,
extends `WireframeError` to surface codec errors, defines recovery policy hooks
for malformed frames, and properly handles EOF mid-frame scenarios. Success is
visible when codec errors are surfaced with structured context, recovery
policies are applied consistently, and all error paths are covered by unit,
integration, and behavioural tests.

## Progress

- [x] (2026-01-06) Draft ExecPlan for 9.1.2.
- [ ] Create `src/codec/error.rs` with `CodecError` taxonomy.
- [ ] Create `src/codec/recovery.rs` with recovery policy types and traits.
- [ ] Extend `WireframeError` in `src/response.rs` with `Codec` variant.
- [ ] Extend `WireframeError` in `src/app/error.rs` with `Codec` variant.
- [ ] Update `LengthDelimitedDecoder` to produce structured EOF errors.
- [ ] Integrate `CodecErrorContext` population in connection processing.
- [ ] Add recovery policy evaluation in `src/app/connection.rs`.
- [ ] Add structured logging with codec error fields.
- [ ] Add `on_eof` hook to `ProtocolHooks`.
- [ ] Add codec error metrics.
- [ ] Add unit tests for `CodecError` taxonomy.
- [ ] Add integration tests for error propagation and recovery.
- [ ] Add Cucumber behavioural tests for codec error scenarios.
- [ ] Update `docs/users-guide.md` with error handling documentation.
- [ ] Update `docs/adr-004-pluggable-protocol-codecs.md` with design decisions.
- [ ] Mark roadmap entry 9.1.2 as done.
- [ ] Run formatting, lint, and test gates.

## Surprises & Discoveries

(To be filled during implementation)

## Decision Log

- Decision: Use a three-tier error taxonomy (Framing, Protocol, IO) plus EOF.
  Rationale: Matches the roadmap specification and provides clear recovery
  semantics for each category. Date/Author: 2026-01-06 / Codex.

- Decision: Recovery policies are Drop, Quarantine, or Disconnect.
  Rationale: Matches the hardening guide and provides graduated responses to
  error severity. Drop allows connection continuation for recoverable errors.
  Date/Author: 2026-01-06 / Codex.

- Decision: EOF handling distinguishes CleanClose, MidFrame, and MidHeader.
  Rationale: Enables proper resource cleanup and accurate error reporting for
  the three distinct EOF scenarios. Date/Author: 2026-01-06 / Codex.

- Decision: Add `Codec` variant to existing `WireframeError` types rather than
  replacing them. Rationale: Maintains backward compatibility while extending
  error surface. Date/Author: 2026-01-06 / Codex.

## Outcomes & Retrospective

(To be filled on completion)

## Context and Orientation

### Current Error Handling

The codec abstraction lives in `src/codec.rs` with the `FrameCodec` trait and
`LengthDelimitedFrameCodec` default. Currently:

- `FrameCodec::Decoder` and `FrameCodec::Encoder` return `io::Error`
- `LengthDelimitedEncoder.encode()` uses `io::ErrorKind::InvalidInput` for
  oversized frames
- No structured error taxonomy exists for distinguishing error categories

### Existing Error Types

- `WireframeError<E>` in `src/response.rs:216-221`: `Io` and `Protocol` variants
  for streaming responses
- `WireframeError` in `src/app/error.rs:10-14`: Only `DuplicateRoute` for setup
- `SendError` in `src/app/error.rs:17-26`: `Serialize` and `Io` variants
- `FragmentError` in `src/fragment/error.rs`: Fragmentation-specific errors
- `ClientError` in `src/client/error.rs`: Client operation errors

### Key Files

Codec implementation:

- `src/codec.rs` - `FrameCodec` trait and `LengthDelimitedFrameCodec`
- `src/codec/examples.rs` - Example codec implementations
- `src/app/combined_codec.rs` - Adapter for Tokio `Framed`

Error handling paths:

- `src/app/connection.rs` - Connection processing and error handling
- `src/app/frame_handling.rs` - Frame deserialization and response forwarding
- `src/hooks.rs` - Protocol hooks including error callbacks

Testing infrastructure:

- `tests/frame_codec.rs` - Codec integration tests
- `tests/features/` - Cucumber feature files
- `tests/steps/` and `tests/worlds/` - Step definitions and world types

Documentation to update:

- `docs/roadmap.md` - Mark 9.1.2 done
- `docs/users-guide.md` - Public interface documentation
- `docs/adr-004-pluggable-protocol-codecs.md` - Design decisions

## Plan of Work

### Phase 1: Error Taxonomy Types

Create the `CodecError` enum with variants for each error category. The
taxonomy follows a three-tier classification:

1. **Framing errors** - Wire-level frame boundary issues (e.g., oversized
   frame, invalid length encoding, incomplete header, checksum mismatch)
2. **Protocol errors** - Semantic violations after frame extraction (e.g.,
   missing header, unsupported version, unknown message type, sequence
   violation)
3. **IO errors** - Transport layer failures (wrapped `io::Error`)
4. **EOF errors** - End-of-stream handling with three sub-variants:
   - `CleanClose` - EOF at frame boundary (normal)
   - `MidFrame` - EOF during payload read (premature)
   - `MidHeader` - EOF during header read (premature)

Each error type provides a `default_recovery_policy()` method returning one of:

- `Drop` - Discard the malformed frame and continue (recoverable errors)
- `Quarantine` - Pause the connection temporarily (rate limiting)
- `Disconnect` - Terminate the connection (fatal errors)

### Phase 2: Recovery Policy Infrastructure

Create recovery policy types and the `RecoveryPolicyHook` trait that allows
customisation of error handling behaviour:

- `RecoveryPolicy` enum with `Drop`, `Quarantine`, `Disconnect` variants
- `CodecErrorContext` struct with `connection_id`, `peer_address`,
  `correlation_id`, and `codec_state` fields for structured logging
- `RecoveryPolicyHook` trait with methods:
  - `recovery_policy(&self, error, ctx) -> RecoveryPolicy`
  - `quarantine_duration(&self, error, ctx) -> Duration`
  - `on_error(&self, error, ctx, policy)` for logging/metrics
- `DefaultRecoveryPolicy` implementation using built-in defaults
- `RecoveryConfig` for builder configuration (max consecutive drops, quarantine
  duration, logging preferences)

### Phase 3: WireframeError Extension

Extend the existing `WireframeError` types to surface codec errors:

1. Add `Codec(CodecError)` variant to `WireframeError<E>` in `src/response.rs`
2. Add `Codec(CodecError)` variant to `WireframeError` in `src/app/error.rs`
3. Implement `From<CodecError>` for both types
4. Implement `From<CodecError> for io::Error` for backward compatibility

### Phase 4: Decoder Updates

Update `LengthDelimitedDecoder` to produce structured EOF errors:

1. Modify `decode_eof()` to detect clean vs. premature EOF
2. Return `CodecError::Eof(EofError::CleanClose)` for EOF at frame boundary
3. Return `CodecError::Eof(EofError::MidFrame{..})` for EOF during payload
4. Return `CodecError::Eof(EofError::MidHeader{..})` for EOF during header
5. Maintain `io::Error` return type via `From<CodecError> for io::Error`

### Phase 5: Connection Processing Integration

Integrate the error taxonomy into connection processing:

1. Populate `CodecErrorContext` from connection state in
   `src/app/connection.rs`
2. Evaluate recovery policies when codec errors occur
3. Apply recovery actions (drop frame, quarantine, disconnect)
4. Track consecutive drop count and escalate to disconnect if exceeded
5. Add structured logging with all context fields

### Phase 6: Protocol Hooks Extension

Extend `ProtocolHooks` in `src/hooks.rs`:

1. Add `on_eof` hook receiving `EofError`, partial data, and connection context
2. Allow protocol implementations to handle EOF scenarios (e.g., send error
   frame before disconnect)

### Phase 7: Metrics and Observability

Add codec error metrics in `src/metrics.rs`:

1. `wireframe_codec_errors_total` counter with `error_type` and
   `recovery_policy` labels
2. Helper function `inc_codec_error(error_type, recovery_policy)`
3. Integrate metric emission in error handling paths

### Phase 8: Testing

Add comprehensive tests:

1. **Unit tests** in `src/codec/error.rs`:
   - Recovery policy defaults for each error type
   - `is_clean_close()` and `should_disconnect()` helpers
   - `From<CodecError> for io::Error` conversion

2. **Integration tests** in `tests/codec_error.rs`:
   - Oversized frame triggers drop and connection continues
   - Max consecutive drops triggers disconnect
   - Mid-frame EOF surfaces to handler
   - Custom recovery policy hook is invoked

3. **Cucumber behavioural tests** in `tests/features/codec_error.feature`:
   - Scenario: Oversized frame triggers drop recovery
   - Scenario: Consecutive drop limit triggers disconnect
   - Scenario: Clean EOF at frame boundary
   - Scenario: Premature EOF mid-frame
   - Scenario: Custom recovery policy hook

4. Add step definitions in `tests/steps/codec_error_steps.rs`
5. Add world type in `tests/worlds/codec_error.rs`

### Phase 9: Documentation

Update documentation:

1. `docs/users-guide.md`:
   - Add section on codec error handling
   - Document `CodecError` variants and recovery policies
   - Document `RecoveryPolicyHook` for custom error handling
   - Update custom codec section with error handling guidance

2. `docs/adr-004-pluggable-protocol-codecs.md`:
   - Document the `CodecError` taxonomy design decision
   - Document recovery policy rationale
   - Document EOF handling design

3. `docs/roadmap.md`:
   - Mark all 9.1.2 sub-items as done

## Concrete Steps

1. Create `src/codec/error.rs` with the error taxonomy:
   - `FramingError` enum with `OversizedFrame`, `InvalidLengthEncoding`,
     `IncompleteHeader`, `ChecksumMismatch`, `EmptyFrame`
   - `ProtocolError` enum with `MissingHeader`, `UnsupportedVersion`,
     `UnknownMessageType`, `SequenceViolation`, `InvalidStateTransition`
   - `EofError` enum with `CleanClose`, `MidFrame`, `MidHeader`
   - `CodecError` enum wrapping all categories
   - `CodecError::default_recovery_policy()` implementation
   - `From<CodecError> for io::Error` implementation

2. Create `src/codec/recovery.rs` with recovery infrastructure:
   - `RecoveryPolicy` enum
   - `CodecErrorContext` struct
   - `RecoveryPolicyHook` trait
   - `DefaultRecoveryPolicy` implementation
   - `RecoveryConfig` builder config

3. Update `src/codec.rs`:
   - Add `pub mod error;` and `pub mod recovery;`
   - Re-export public types

4. Update `src/response.rs`:
   - Add `Codec(CodecError)` variant to `WireframeError<E>`
   - Add `from_codec()` constructor
   - Implement `From<CodecError>`
   - Update `Display` and `Error` implementations

5. Update `src/app/error.rs`:
   - Add `Codec(CodecError)` variant to `WireframeError`
   - Implement `From<CodecError>`

6. Update `src/codec.rs` `LengthDelimitedDecoder::decode_eof()`:
   - Detect EOF at frame boundary vs. mid-frame
   - Return appropriate `CodecError::Eof` variant
   - Convert to `io::Error` for trait compatibility

7. Update `src/app/connection.rs`:
   - Add `CodecErrorContext` population from connection state
   - Add recovery policy evaluation on decode errors
   - Track consecutive drop count
   - Add structured logging with context fields
   - Escalate to disconnect when max drops exceeded

8. Update `src/hooks.rs`:
   - Add `on_eof` field to `ProtocolHooks`
   - Add default no-op implementation

9. Update `src/metrics.rs`:
   - Add `inc_codec_error()` function
   - Add counter with error type and policy labels

10. Update `src/lib.rs`:
    - Re-export `CodecError`, `RecoveryPolicy`, `CodecErrorContext` from
      `codec` module

11. Create `tests/codec_error.rs`:
    - Add integration tests for error propagation
    - Add tests for recovery policy application

12. Create `tests/features/codec_error.feature`:
    - Add behavioural scenarios for codec errors

13. Create `tests/steps/codec_error_steps.rs` and
    `tests/worlds/codec_error.rs`:
    - Add step definitions and world type

14. Update `tests/cucumber.rs`:
    - Register new world type

15. Update `docs/users-guide.md`:
    - Add codec error handling section

16. Update `docs/adr-004-pluggable-protocol-codecs.md`:
    - Document error taxonomy design

17. Update `docs/roadmap.md`:
    - Mark 9.1.2 items as done

18. Run validation gates:

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

## Validation and Acceptance

Acceptance is based on observable behaviour:

- `CodecError` taxonomy distinguishes framing, protocol, IO, and EOF errors
- Each `CodecError` variant returns an appropriate default recovery policy
- `WireframeError` types surface `CodecError` via the `Codec` variant
- EOF mid-frame is surfaced as `EofError::MidFrame` with byte counts
- EOF at frame boundary is surfaced as `EofError::CleanClose`
- Recovery policies are applied consistently (drop, quarantine, disconnect)
- Structured logging includes connection_id, peer_address, correlation_id
- Codec error metrics are emitted with error_type and recovery_policy labels
- `on_eof` hook is invoked for protocol-specific EOF handling
- All unit tests pass for error taxonomy and recovery policies
- All integration tests pass for error propagation
- All Cucumber tests pass for behavioural scenarios
- `make check-fmt`, `make lint`, and `make test` succeed

## Idempotence and Recovery

All changes are safe to reapply. If a refactor breaks compilation, revert the
individual file changes and reapply the steps one by one. Behavioural tests can
be re-run without side effects because they bind to ephemeral TCP ports and
clean up after each scenario.

## Artifacts and Notes

Record key evidence here once available:

- `make fmt` log: `/tmp/wireframe-fmt.log`
- `make markdownlint` log: `/tmp/wireframe-markdownlint.log`
- `make check-fmt` log: `/tmp/wireframe-check-fmt.log`
- `make lint` log: `/tmp/wireframe-lint.log`
- `make test` log: `/tmp/wireframe-test.log`

## Interfaces and Dependencies

### CodecError Enum

    // src/codec/error.rs

    #[derive(Debug, Error)]
    pub enum CodecError {
        #[error("framing error: {0}")]
        Framing(#[from] FramingError),

        #[error("protocol error: {0}")]
        Protocol(#[from] ProtocolError),

        #[error("I/O error: {0}")]
        Io(#[from] io::Error),

        #[error("EOF: {0}")]
        Eof(#[from] EofError),
    }

### RecoveryPolicy Enum

    // src/codec/recovery.rs

    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub enum RecoveryPolicy {
        #[default]
        Drop,
        Quarantine,
        Disconnect,
    }

### Extended WireframeError

    // src/response.rs

    #[derive(Debug)]
    pub enum WireframeError<E = ()> {
        Io(std::io::Error),
        Protocol(E),
        Codec(CodecError),  // New variant
    }

## Files to Modify

| File                                        | Change                           |
| ------------------------------------------- | -------------------------------- |
| `src/codec/error.rs`                        | New: CodecError taxonomy         |
| `src/codec/recovery.rs`                     | New: Recovery policy types       |
| `src/codec.rs`                              | Modify: Re-export error types    |
| `src/app/error.rs`                          | Modify: Add Codec variant        |
| `src/response.rs`                           | Modify: Add Codec variant        |
| `src/app/connection.rs`                     | Modify: Integrate error handling |
| `src/hooks.rs`                              | Modify: Add on_eof hook          |
| `src/metrics.rs`                            | Modify: Add codec error metrics  |
| `src/lib.rs`                                | Modify: Re-export types          |
| `tests/codec_error.rs`                      | New: Integration tests           |
| `tests/features/codec_error.feature`        | New: Cucumber tests              |
| `tests/steps/codec_error_steps.rs`          | New: Step definitions            |
| `tests/worlds/codec_error.rs`               | New: World type                  |
| `tests/cucumber.rs`                         | Modify: Register world           |
| `docs/users-guide.md`                       | Modify: Error handling docs      |
| `docs/adr-004-pluggable-protocol-codecs.md` | Modify: Design decisions         |
| `docs/roadmap.md`                           | Modify: Mark 9.1.2 done          |

## Revision note (required when editing an ExecPlan)

Initial draft of ExecPlan for roadmap item 9.1.2.
