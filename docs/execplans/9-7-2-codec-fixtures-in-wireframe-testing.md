# 9.7.2 Add codec fixtures in wireframe\_testing

This ExecPlan (execution plan) is a living document. The sections
`Constraints`, `Tolerances`, `Risks`, `Progress`, `Surprises & Discoveries`,
`Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work
proceeds.

Status: COMPLETE

## Purpose / big picture

The `wireframe_testing` crate already provides codec-aware driver functions
(added in 9.7.1) that encode payloads, transport them over an in-memory duplex
stream, and decode the response. However, test authors who need to exercise
error paths — malformed frames, oversized payloads, truncated headers, or
frames carrying specific correlation metadata — must hand-craft raw byte
sequences each time. This is tedious, error-prone, and leads to duplicated
setup across test suites.

After this change, `wireframe_testing` exports a set of builder helpers that
produce ready-to-use byte sequences and typed frames for common codec test
scenarios. A test author can write:

```rust
use wireframe_testing::{
    valid_hotline_wire, oversized_hotline_wire,
    truncated_hotline_header, correlated_hotline_wire,
};

// Valid frame bytes ready for decode_frames_with_codec
let wire = valid_hotline_wire(b"hello", 7);

// Oversized payload that should be rejected by the decoder
let too_big = oversized_hotline_wire(4096);

// Frame with only a partial header — decoder should return Ok(None)
let partial = truncated_hotline_header();

// Sequence of frames sharing a correlation (transaction) ID
let correlated = correlated_hotline_wire(42, &[b"a", b"b", b"c"]);
```

Observable success: `make test` passes, including new unit tests in
`tests/codec_fixtures.rs` and new rstest-bdd behavioural scenarios in
`tests/scenarios/codec_fixtures_scenarios.rs` that exercise the fixture
functions with `HotlineFrameCodec`. The existing codec test harness tests
remain green.

## Constraints

- All existing `wireframe_testing` public APIs must remain source-compatible.
  The new functions are purely additive.
- No file may exceed 400 lines.
- Use `rstest` for new unit tests and `rstest-bdd` v0.5.0 for new behavioural
  tests.
- Use existing in-tree codecs (`HotlineFrameCodec` from
  `wireframe::codec::examples`) rather than defining new codec types.
- en-GB-oxendict spelling in all comments and documentation.
- Strict Clippy with `-D warnings` on all targets including tests.
- Every module must begin with a `//!` doc comment.
- Public functions must have `///` doc comments with examples.
- No `assert!`/`panic!` in functions returning `Result` (clippy
  `panic_in_result_fn`).
- No array indexing in tests — use `.get(n)` / `.first()` instead.
- `#[expect]` attributes must include `reason = "..."`.
- Quality gates (`make check-fmt`, `make lint`, `make test`,
  `make markdownlint`) must pass before completion.
- Tests for `wireframe_testing` functionality must live in the main crate's
  `tests/` directory as integration tests, because `wireframe_testing` is a
  dev-dependency of the main crate, not a workspace member. Internal
  `#[cfg(test)]` modules within `wireframe_testing` never execute via
  `cargo test`.

## Tolerances (exception triggers)

- Scope: if implementation requires more than 12 new/modified files or 1,000
  net changed lines, stop and re-scope.
- Dependencies: if any new external crate is required beyond what is already in
  `wireframe_testing/Cargo.toml`, stop and escalate.
- Interface: if a public API signature on the main `wireframe` crate must
  change beyond an additive accessor, stop and escalate.
- Iterations: if the same failing root cause persists after 3 fix attempts,
  stop and document options in the Decision Log.

## Risks

- Risk: the Hotline decoder returns `Ok(None)` for truncated data (insufficient
  bytes for a header or payload) rather than `Err(...)`. This means
  `decode_frames_with_codec` will treat truncated frames as "no more frames"
  rather than an error, unless the trailing-bytes check catches it. Fixtures
  producing truncated data must account for this: they will trigger the
  trailing-bytes error path in `decode_frames_with_codec`, not a decoder error.
  Severity: medium. Likelihood: certain. Mitigation: document this in the
  fixture API and ensure BDD tests assert on trailing-bytes errors specifically.

- Risk: the `codec_fixtures.rs` module might grow beyond 400 lines if too many
  fixture variants are added. Severity: low. Likelihood: low. Mitigation: start
  with the minimum required fixtures (valid, oversized, truncated, correlated);
  split into submodules only if the 400-line cap is approached.

- Risk: Hotline encoder rejects oversized payloads at encode time, so
  "oversized" fixtures must produce raw bytes directly rather than going
  through the encoder. Severity: low. Likelihood: certain. Mitigation:
  oversized fixture hand-crafts the 20-byte Hotline header + oversized payload
  bytes.

## Progress

- [x] (2026-02-28) Drafted ExecPlan.
- [x] (2026-02-28) Stage A: implement codec fixture module in
      `wireframe_testing`.
- [x] (2026-02-28) Stage B: add rstest unit tests for the fixtures (8 tests,
      all green).
- [x] (2026-02-28) Stage C: add rstest-bdd behavioural tests (4 scenarios, all
      green).
- [x] (2026-02-28) Stage D: documentation, design doc, and roadmap updates.
- [x] (2026-02-28) Stage E: full validation and evidence capture.

## Surprises & discoveries

- The Hotline decoder does not override `decode_eof`, so the default
  tokio-util implementation is used. When `decode()` returns `Ok(None)` but the
  buffer is non-empty, `decode_eof` returns `Err("bytes remaining on stream")`
  rather than `Ok(None)`. This means truncated fixtures trigger an error from
  `decode_eof` (wrapped as
  `"codec decode_eof failed: bytes remaining on stream"`) rather than from the
  trailing-bytes check in `decode_frames_with_codec`. The docstrings and BDD
  tests were updated to assert on `"bytes remaining"` instead of `"trailing"`.

## Decision log

- Decision: produce raw `Vec<u8>` wire bytes as the primary fixture output
  format rather than typed `Frame` values. Rationale: invalid and malformed
  frames cannot exist as valid typed values (e.g., `HotlineFrame` always has a
  well-formed payload). Wire bytes are what decoders and test drivers consume
  directly. For valid-frame fixtures, provide both raw bytes and a convenience
  function returning the typed frame for metadata inspection. Date/Author:
  2026-02-28 / Plan phase.

- Decision: provide Hotline-specific fixture functions rather than
  `FrameCodec`-generic fixtures. Rationale: generating invalid frames requires
  knowledge of the specific wire format (header layout, field positions, size
  constraints). A generic approach would need a `MalformedFrameGenerator` trait
  that every codec implements, which is over-engineering for a test utility.
  `HotlineFrameCodec` is the project's canonical example codec used across all
  codec tests. If additional codecs need fixtures in the future, they can be
  added as separate fixture functions following the same pattern. Date/Author:
  2026-02-28 / Plan phase.

- Decision: place fixtures in a new `codec_fixtures` submodule under
  `wireframe_testing/src/helpers/` rather than a top-level module. Rationale:
  this follows the existing pattern where all codec-related helpers live under
  `helpers/` (codec.rs, codec\_ext.rs, codec\_drive.rs). It keeps the module
  hierarchy consistent and the fixture functions close to the encode/decode
  helpers they build upon. Date/Author: 2026-02-28 / Plan phase.

## Outcomes & retrospective

Completed 2026-02-28. All quality gates pass:

- `make fmt` — clean (no formatting changes)
- `make check-fmt` — clean
- `make markdownlint` — clean (0 errors)
- `make lint` — clean (cargo doc + clippy with `-D warnings`)
- `make test` — all green (0 failures)

New test coverage:

- 8 integration tests in `tests/codec_fixtures.rs` (valid wire decode, valid
  frame metadata, oversized rejection, mismatched total\_size rejection,
  truncated header error, truncated payload error, correlated transaction IDs,
  sequential transaction IDs).
- 4 BDD scenarios in `tests/scenarios/codec_fixtures_scenarios.rs` (valid
  decode, oversized rejection, truncated error, correlated IDs).

Files created: 6 (`codec_fixtures.rs` in wireframe\_testing helpers,
`codec_fixtures.rs` integration tests, `.feature` file, BDD fixture/steps/
scenarios).
Files modified: 8 (helpers.rs, lib.rs, three BDD mod.rs files, users-guide.md,
roadmap.md, adr-004.md).

Key discovery: truncated frame fixtures trigger the `decode_eof` "bytes
remaining on stream" error path rather than the trailing-bytes check in
`decode_frames_with_codec`. This is because the default tokio-util
`decode_eof` implementation detects leftover bytes before the explicit trailing
check runs. Future fixture work should account for this when designing error
assertions.

## Context and orientation

### Repository layout (relevant subset)

```plaintext
src/
  codec.rs                          # FrameCodec trait (line 63-103)
  codec/examples.rs                 # HotlineFrameCodec (line 21-131), MysqlFrameCodec

wireframe_testing/
  Cargo.toml                        # dev-dependency on wireframe (path = "..")
  src/lib.rs                        # public re-exports (68 lines)
  src/helpers.rs                    # TestSerializer trait, module root, constants (71 lines)
  src/helpers/codec.rs              # new_test_codec, decode_frames, encode_frame (77 lines)
  src/helpers/codec_ext.rs          # encode_payloads_with_codec, decode_frames_with_codec,
                                    #   extract_payloads (144 lines)
  src/helpers/codec_drive.rs        # drive_with_codec_payloads/frames (279 lines)
  src/helpers/drive.rs              # drive_internal, drive_with_frame[s] (297 lines)
  src/helpers/payloads.rs           # drive_with_payloads, drive_with_bincode (152 lines)
  src/helpers/runtime.rs            # run_app, run_with_duplex_server (89 lines)
  src/integration_helpers.rs        # TestApp, TestError, TestResult, factory (294 lines)

tests/
  codec_test_harness.rs             # Integration tests for codec drivers (162 lines)
  fixtures/mod.rs                   # BDD world fixtures (34 modules)
  fixtures/codec_test_harness.rs    # CodecTestHarnessWorld (132 lines)
  steps/mod.rs                      # BDD step definitions (35 modules)
  steps/codec_test_harness_steps.rs # Codec harness steps (41 lines)
  scenarios/mod.rs                  # BDD scenario registrations (37 modules)
  scenarios/codec_test_harness_scenarios.rs # 2 scenario tests (26 lines)
  features/codec_test_harness.feature      # 2 Gherkin scenarios (15 lines)

docs/
  roadmap.md                        # Item 9.7.2 at line 435-436
  users-guide.md                    # Existing wireframe_testing section at line 224
  generic-message-fragmentation-and-re-assembly-design.md
  hardening-wireframe-a-guide-to-production-resilience.md
```

### Key types

`FrameCodec` (defined in `src/codec.rs:63-103`) is a trait requiring
`Send + Sync + Clone + 'static` with associated types `Frame`, `Decoder`,
`Encoder` and methods `decoder()`, `encoder()`, `frame_payload()`,
`wrap_payload()`, `correlation_id()`, and `max_frame_length()`.

`HotlineFrameCodec` (defined in `src/codec/examples.rs:21-131`) uses a 20-byte
header: `data_size: u32`, `total_size: u32`, `transaction_id: u32`,
`reserved: [u8; 8]`, followed by payload bytes. The decoder returns `Ok(None)`
when fewer than 20 bytes are available or when the full frame has not arrived.
It returns `Err(InvalidData, "payload too large")` when
`data_size > max_frame_length` and `Err(InvalidData, "invalid total size")`
when `total_size != data_size + 20`.

`decode_frames_with_codec` (in `wireframe_testing/src/helpers/codec_ext.rs:80`)
calls `decoder.decode()` in a loop, then `decoder.decode_eof()`, then checks
for trailing bytes. Trailing bytes produce
`Err(InvalidData, "trailing N byte(s) after decoding")`.

Constants: `MIN_FRAME_LENGTH = 64`, `MAX_FRAME_LENGTH = 16 MiB`,
`LENGTH_HEADER_SIZE = 4`, `TEST_MAX_FRAME = 4096`.

### BDD test pattern

The project uses rstest-bdd v0.5.0 with a 4-file pattern per test domain:

1. `tests/features/<domain>.feature` — Gherkin scenarios.
2. `tests/fixtures/<domain>.rs` — world struct with `#[fixture]` function.
3. `tests/steps/<domain>_steps.rs` — `#[given]`/`#[when]`/`#[then]` functions.
4. `tests/scenarios/<domain>_scenarios.rs` — `#[scenario]` functions.

Steps are synchronous; async calls use
`tokio::runtime::Runtime::new()?.block_on(...)`. The fixture parameter name in
scenario functions must match the fixture function name exactly. All step
functions return `TestResult` (`Result<(), Box<dyn Error>>`).

Module wiring: `tests/scenarios/mod.rs` loads `tests/steps/mod.rs` via
`#[path = "../steps/mod.rs"] pub(crate) mod steps;`, then declares each
scenario submodule.

## Plan of work

### Stage A: implement codec fixture module in wireframe\_testing

Create `wireframe_testing/src/helpers/codec_fixtures.rs` containing functions
that produce raw wire bytes for Hotline-framed test scenarios. The module
provides four categories of fixtures:

**1. Valid frame fixtures** — properly encoded Hotline frames.

`valid_hotline_wire(payload: &[u8], transaction_id: u32) -> Vec<u8>` — builds a
single valid Hotline frame as raw wire bytes by writing the 20-byte header
(data\_size, total\_size, transaction\_id, 8 reserved zero bytes) followed by
the payload. This bypasses the tokio-util encoder so fixtures are independent
of the encoder implementation.

`valid_hotline_frame(payload: &[u8], transaction_id: u32) -> HotlineFrame` —
returns a typed `HotlineFrame` for tests needing metadata inspection. Delegates
to `HotlineFrameCodec::wrap_payload` but overrides the `transaction_id`.

**2. Invalid frame fixtures** — wire bytes that should cause decode errors.

`oversized_hotline_wire(max_frame_length: usize) -> Vec<u8>` — crafts a Hotline
header where `data_size` exceeds `max_frame_length` by 1 byte, followed by that
many payload bytes. The decoder should reject this with
`InvalidData("payload too large")`.

`mismatched_total_size_wire(payload: &[u8]) -> Vec<u8>` — crafts a Hotline
header where `total_size` does not equal `data_size + 20`. The decoder should
reject this with `InvalidData("invalid total size")`.

**3. Incomplete frame fixtures** — wire bytes that are valid prefixes but
insufficient for a complete frame.

`truncated_hotline_header() -> Vec<u8>` — returns fewer than 20 bytes (e.g., 10
bytes of zeroes). The decoder returns `Ok(None)`; when passed to
`decode_frames_with_codec` the trailing-bytes check produces an error.

`truncated_hotline_payload(payload_len: usize) -> Vec<u8>` — writes a valid
20-byte header claiming `data_size = payload_len` but provides only half the
payload bytes. The decoder returns `Ok(None)` because the buffer is too short;
`decode_frames_with_codec` reports trailing bytes.

**4. Correlation metadata fixtures** — frames with specific transaction IDs for
correlation testing.

`correlated_hotline_wire(transaction_id: u32, payloads: &[&[u8]]) -> Vec<u8>` —
encodes multiple Hotline frames sharing the same `transaction_id`, suitable for
verifying correlation ID propagation across frame sequences.

`sequential_hotline_wire(base_transaction_id: u32, payloads: &[&[u8]]) -> Vec<u8>`
— encodes multiple frames with incrementing transaction IDs (`base`,
`base + 1`, …), suitable for verifying frame ordering.

Register the module in `wireframe_testing/src/helpers.rs` as
`mod codec_fixtures;` and add `pub use codec_fixtures::*;` exports. Add the
public re-exports to `wireframe_testing/src/lib.rs`.

Stage A acceptance: `make check-fmt && make lint` pass. The new functions
compile and are accessible from the crate root via
`wireframe_testing::valid_hotline_wire` etc.

### Stage B: add rstest unit tests for the fixtures

Create `tests/codec_fixtures.rs` as an integration test file exercising every
fixture function. Tests live outside `wireframe_testing` because the crate is
not a workspace member.

Tests to implement:

1. `valid_hotline_wire_decodes_successfully` — encode a payload with
   `valid_hotline_wire`, decode with `decode_frames_with_codec`, verify the
   payload and transaction ID match.

2. `valid_hotline_frame_has_correct_metadata` — call `valid_hotline_frame`,
   verify `transaction_id` and payload bytes.

3. `oversized_hotline_wire_rejected_by_decoder` — encode with
   `oversized_hotline_wire(4096)`, attempt `decode_frames_with_codec` with a
   codec capped at 4096, verify `Err` with `InvalidData`.

4. `mismatched_total_size_rejected_by_decoder` — call
   `mismatched_total_size_wire`, attempt decode, verify `Err` with
   `InvalidData`.

5. `truncated_header_produces_trailing_bytes_error` — call
   `truncated_hotline_header`, attempt decode, verify `Err` mentioning
   "trailing".

6. `truncated_payload_produces_trailing_bytes_error` — call
   `truncated_hotline_payload(100)`, attempt decode, verify `Err` mentioning
   "trailing".

7. `correlated_frames_share_transaction_id` — call `correlated_hotline_wire`
   with `transaction_id = 42` and 3 payloads, decode, verify all 3 frames have
   `transaction_id = 42`.

8. `sequential_frames_have_incrementing_ids` — call `sequential_hotline_wire`
   with `base = 10` and 3 payloads, decode, verify transaction IDs are 10, 11,
   12.

Stage B acceptance: `make test` passes with all 8 new tests green.

### Stage C: add rstest-bdd behavioural tests

Create four files for the `codec_fixtures` BDD domain:

**`tests/features/codec_fixtures.feature`**:

```gherkin
Feature: Codec test fixtures
  The wireframe_testing crate provides codec fixture functions for
  generating valid and invalid Hotline-framed wire bytes for testing.

  Scenario: Valid fixture decodes to expected payload
    Given a Hotline codec allowing frames up to 4096 bytes
    When a valid fixture frame is decoded
    Then the decoded payload matches the fixture input

  Scenario: Oversized fixture is rejected by decoder
    Given a Hotline codec allowing frames up to 4096 bytes
    When an oversized fixture frame is decoded
    Then the decoder reports an invalid data error

  Scenario: Truncated fixture produces a trailing bytes error
    Given a Hotline codec allowing frames up to 4096 bytes
    When a truncated fixture frame is decoded
    Then the decoder reports trailing bytes

  Scenario: Correlated fixtures share the same transaction identifier
    Given a Hotline codec allowing frames up to 4096 bytes
    When correlated fixture frames are decoded
    Then all frames have the expected transaction identifier
```

**`tests/fixtures/codec_fixtures.rs`**: A `CodecFixturesWorld` struct holding
the codec, wire bytes, decoded frames, and any decode error. Methods:
`configure_codec(max_frame_length)`, `decode_valid_fixture()`,
`decode_oversized_fixture()`, `decode_truncated_fixture()`,
`decode_correlated_fixture()`, `verify_payload_matches()`,
`verify_invalid_data_error()`, `verify_trailing_bytes_error()`,
`verify_correlated_transaction_ids()`.

**`tests/steps/codec_fixtures_steps.rs`**: Step functions matching the Gherkin
steps, delegating to world methods.

**`tests/scenarios/codec_fixtures_scenarios.rs`**: Four `#[scenario]` functions.

Wire into `tests/fixtures/mod.rs`, `tests/steps/mod.rs`, and
`tests/scenarios/mod.rs`.

Stage C acceptance: `make test` passes with all 4 BDD scenarios green.

### Stage D: documentation, design doc, and roadmap updates

1. Update `docs/users-guide.md` — add a subsection under the existing "Testing
   custom codecs with `wireframe_testing`" section (around line 224)
   documenting the new codec fixture functions with a brief usage example
   showing how to use `valid_hotline_wire`, `oversized_hotline_wire`, and
   `correlated_hotline_wire`.

2. Update the relevant design document
   (`docs/hardening-wireframe-a-guide-to-production-resilience.md`) with a
   short note recording the design decision to provide Hotline-specific
   raw-byte fixtures rather than generic `FrameCodec`-parameterised generators,
   and the rationale (invalid frames require format-specific knowledge).

3. Mark roadmap item `9.7.2` as done in `docs/roadmap.md`: change
   `- [ ] 9.7.2.` to `- [x] 9.7.2.`

Stage D acceptance: `make markdownlint` passes. Documentation is internally
consistent.

### Stage E: full validation and evidence capture

Run all quality gates with logging:

<!-- markdownlint-disable MD046 -->
```shell
set -o pipefail; make fmt 2>&1 | tee /tmp/9-7-2-fmt.log
set -o pipefail; make check-fmt 2>&1 | tee /tmp/9-7-2-check-fmt.log
set -o pipefail; make markdownlint MDLINT=/root/.bun/bin/markdownlint-cli2 2>&1 | tee /tmp/9-7-2-markdownlint.log
set -o pipefail; make lint 2>&1 | tee /tmp/9-7-2-lint.log
set -o pipefail; make test 2>&1 | tee /tmp/9-7-2-test.log
```
<!-- markdownlint-enable MD046 -->

Update the `Progress` and `Outcomes & Retrospective` sections with final
evidence and timestamps.

Stage E acceptance: all commands exit 0.

## Validation and acceptance

Quality criteria (what "done" means):

- Tests: `make test` passes, including 8 new unit tests in
  `tests/codec_fixtures.rs` and 4 new BDD scenarios in
  `tests/scenarios/codec_fixtures_scenarios.rs`.
- Lint: `make lint` passes (Clippy with `-D warnings` on all targets).
- Format: `make check-fmt` passes.
- Markdown: `make markdownlint` passes.
- Documentation: `docs/users-guide.md` documents the new fixture API.
  `docs/roadmap.md` item 9.7.2 is marked done.

Quality method:

- Run the shell commands in Stage E and verify all exit 0.
- Verify the new BDD scenarios pass: `cargo test codec_fixtures`.
- Verify the new unit tests pass: `cargo test codec_fixtures`.

## Idempotence and recovery

All stages produce additive changes. If a stage fails partway through, the
incomplete changes can be reverted with `git checkout -- .` and the stage
retried from the beginning. No destructive operations are involved.

## Interfaces and dependencies

### New public functions (wireframe\_testing crate)

In `wireframe_testing/src/helpers/codec_fixtures.rs`:

```rust
/// Build a single valid Hotline frame as raw wire bytes.
///
/// Writes the 20-byte Hotline header (data_size, total_size,
/// transaction_id, 8 reserved zero bytes) followed by the payload.
pub fn valid_hotline_wire(payload: &[u8], transaction_id: u32) -> Vec<u8>;

/// Return a typed `HotlineFrame` with the given payload and transaction ID.
pub fn valid_hotline_frame(payload: &[u8], transaction_id: u32) -> HotlineFrame;

/// Build a Hotline frame whose data_size exceeds `max_frame_length` by one
/// byte. The decoder should reject this with `InvalidData`.
pub fn oversized_hotline_wire(max_frame_length: usize) -> Vec<u8>;

/// Build a Hotline frame with a mismatched total_size field.
/// The decoder should reject this with `InvalidData`.
pub fn mismatched_total_size_wire(payload: &[u8]) -> Vec<u8>;

/// Return fewer than 20 bytes — a truncated Hotline header.
pub fn truncated_hotline_header() -> Vec<u8>;

/// Return a valid Hotline header claiming `payload_len` bytes of payload,
/// but provide only half the payload bytes.
pub fn truncated_hotline_payload(payload_len: usize) -> Vec<u8>;

/// Encode multiple Hotline frames sharing the same `transaction_id`.
pub fn correlated_hotline_wire(
    transaction_id: u32,
    payloads: &[&[u8]],
) -> Vec<u8>;

/// Encode multiple Hotline frames with incrementing transaction IDs.
pub fn sequential_hotline_wire(
    base_transaction_id: u32,
    payloads: &[&[u8]],
) -> Vec<u8>;
```

### Files to create

| File                                              | Purpose                        | Est. lines |
| ------------------------------------------------- | ------------------------------ | ---------- |
| `wireframe_testing/src/helpers/codec_fixtures.rs` | Codec fixture functions        | ~180       |
| `tests/codec_fixtures.rs`                         | Integration tests for fixtures | ~180       |
| `tests/features/codec_fixtures.feature`           | BDD feature file               | ~25        |
| `tests/fixtures/codec_fixtures.rs`                | BDD world fixture              | ~130       |
| `tests/steps/codec_fixtures_steps.rs`             | BDD step definitions           | ~60        |
| `tests/scenarios/codec_fixtures_scenarios.rs`     | BDD scenario registrations     | ~40        |

### Files to modify

| File                                                           | Change                                   |
| -------------------------------------------------------------- | ---------------------------------------- |
| `wireframe_testing/src/helpers.rs`                             | Add `mod codec_fixtures;` and re-exports |
| `wireframe_testing/src/lib.rs`                                 | Add re-exports for new public functions  |
| `tests/fixtures/mod.rs`                                        | Add `pub mod codec_fixtures;`            |
| `tests/steps/mod.rs`                                           | Add `mod codec_fixtures_steps;`          |
| `tests/scenarios/mod.rs`                                       | Add `mod codec_fixtures_scenarios;`      |
| `docs/users-guide.md`                                          | Document new fixture API                 |
| `docs/roadmap.md`                                              | Mark 9.7.2 done                          |
| `docs/hardening-wireframe-a-guide-to-production-resilience.md` | Record design decision                   |

### Artifacts and notes

Reference pattern for raw Hotline header construction (from
`src/codec/examples.rs:84-103`, the encoder):

```rust
const HEADER_LEN: usize = 20;
// data_size: u32 (payload length)
// total_size: u32 (data_size + 20)
// transaction_id: u32
// reserved: 8 zero bytes
// payload bytes
```

Reference pattern for the BDD world: `tests/fixtures/codec_test_harness.rs`
(132 lines) — `CodecTestHarnessWorld` with manual `Debug` impl (because
`WireframeApp` does not implement `Debug`). The new `CodecFixturesWorld` does
not hold a `WireframeApp`, so `#[derive(Debug)]` can be used directly.

Reference for trailing-bytes behaviour: `decode_frames_with_codec` in
`wireframe_testing/src/helpers/codec_ext.rs:106-113` — if `buf` is non-empty
after all decode loops, returns
`Err(InvalidData, "trailing N byte(s) after decoding")`.
