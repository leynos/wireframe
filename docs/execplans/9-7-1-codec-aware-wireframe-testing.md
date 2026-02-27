# 9.7.1 Extend wireframe\_testing with codec-aware test drivers

This ExecPlan (execution plan) is a living document. The sections
`Constraints`, `Tolerances`, `Risks`, `Progress`, `Surprises & Discoveries`,
`Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work
proceeds.

Status: COMPLETE

## Purpose / big picture

The `wireframe_testing` crate provides in-memory test drivers that spin up a
`WireframeApp` on a `tokio::io::duplex` stream and return the bytes written by
the app. Today every driver is hard-wired to the default
`LengthDelimitedFrameCodec`. A test author wanting to exercise a custom
`FrameCodec` (e.g. `HotlineFrameCodec`, `MysqlFrameCodec`, or any user-defined
codec) must manually create a duplex, spawn the app, encode frames with the
codec's encoder, collect raw output, and decode with the codec's decoder. This
is roughly 30 lines of boilerplate per test.

After this change a test author can write:

```rust
use wireframe::codec::examples::HotlineFrameCodec;
use wireframe_testing::drive_with_codec_payloads;

let codec = HotlineFrameCodec::new(4096);
let app = WireframeApp::new()?.with_codec(codec.clone());
let response_payloads = drive_with_codec_payloads(
    app, &codec, vec![payload_bytes],
).await?;
```

The codec-aware drivers handle frame encoding, transport, and decoding
internally, returning decoded payload bytes (or, for the frame-level variant,
decoded `F::Frame` values for metadata inspection).

Observable success: running `make test` passes, including new unit tests in
`wireframe_testing` and new rstest-bdd behavioural scenarios that exercise the
codec-aware drivers with `HotlineFrameCodec`.

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
- Quality gates (`make check-fmt`, `make lint`, `make test`) must pass before
  completion.

## Tolerances (exception triggers)

- Scope: if implementation requires more than 15 new/modified files or 1,200
  net changed lines, stop and re-scope.
- Dependencies: if any new external crate is required beyond what is already in
  `wireframe_testing/Cargo.toml`, stop and escalate.
- Interface: if a public API signature on the main `wireframe` crate must
  change beyond an additive accessor, stop and escalate.
- Iterations: if the same failing root cause persists after 3 fix attempts,
  stop and document options in the Decision Log.

## Risks

- Risk: `drive_internal` is `pub(super)`, so a new sibling module
  `codec_drive.rs` under `helpers/` can call it directly. If the module
  hierarchy changes, this access path breaks. Severity: low. Likelihood: low.
  Mitigation: the new module lives alongside `drive.rs` under `helpers/`,
  matching the existing pattern.

- Risk: the `FrameCodec` trait's `Encoder`/`Decoder` associated types are
  opaque; codec-aware decode may fail on partial frames if the duplex buffer is
  too small. Severity: medium. Likelihood: medium. Mitigation: default capacity
  is 4096 bytes (matching existing helpers); document that callers should
  increase capacity for large-frame codecs.

- Risk: adding a `pub fn codec(&self) -> &F` accessor to `WireframeApp` is a
  minor public API addition to the main crate. Severity: low. Likelihood:
  certain. Mitigation: this is a non-breaking, additive accessor. It enables
  convenient codec reuse in tests without requiring the caller to hold a
  separate clone. Document in `docs/users-guide.md`.

## Progress

- [x] (2026-02-26) Drafted ExecPlan.
- [x] (2026-02-26) Stage A: add `codec()` accessor to `WireframeApp`.
- [x] (2026-02-26) Stage B: implement codec-aware encode/decode helpers in
      `wireframe_testing`.
- [x] (2026-02-26) Stage C: implement codec-aware driver functions.
- [x] (2026-02-26) Stage D: add rstest unit tests for the new drivers.
- [x] (2026-02-26) Stage E: add rstest-bdd behavioural tests.
- [x] (2026-02-26) Stage F: documentation and roadmap updates.
- [x] (2026-02-26) Stage G: full validation and evidence capture.

## Surprises & discoveries

- `WireframeApp` does not implement `Debug`. The behaviour-driven
  development (BDD) world struct `CodecTestHarnessWorld` stores an
  `Option<WireframeApp<...>>`, so `#[derive(Debug)]` cannot be used. Resolved
  with a manual `Debug` implementation that redacts the app field.

- `wireframe_testing` is a dev-dependency of the main crate, not a workspace
  member. Internal `#[cfg(test)]` modules within `wireframe_testing` do not run
  via `cargo test` from the workspace root. Resolved by placing the unit tests
  in `tests/codec_test_harness.rs` as an integration test file instead.

- `WireframeApp::new()` returns a `Result` whose `Ok` variant uses
  `LengthDelimitedFrameCodec` as the default codec. When chaining
  `.with_codec()` immediately, the compiler cannot infer the initial codec type
  parameter. Resolved by writing
  `WireframeApp::<BincodeSerializer, (), Envelope>::new()`.

- Clippy `panic_in_result_fn` lint fires on test functions that return
  `io::Result<()>` and use `assert_eq!`/`assert!`. Resolved by replacing
  assertions with explicit `if` checks returning `Err(io::Error::other(...))`.

## Decision log

- Decision: accept the codec as an explicit `&F` parameter rather than
  extracting it from the app. Rationale: `WireframeApp.codec` is
  `pub(in crate::app)` and not accessible from `wireframe_testing`. While we
  add a `codec()` accessor for convenience, the driver API takes the codec
  explicitly so it works regardless of whether the caller holds the app or a
  reference. This also mirrors the existing `payloads.rs` pattern where the
  codec is constructed independently. Date/Author: 2026-02-26 / Plan phase.

- Decision: provide two levels of codec-aware drivers — payload-level
  (returns `Vec<Vec<u8>>`) and frame-level (returns `Vec<F::Frame>`).
  Rationale: payload-level covers the common case (90% of tests care about
  payload bytes, not codec metadata). Frame-level covers advanced tests needing
  to inspect correlation IDs, sequence numbers, or other frame-level metadata.
  Date/Author: 2026-02-26 / Plan phase.

- Decision: do not introduce a new `TestCodecSerializer` trait.
  Rationale: the codec generic `F` is orthogonal to the serializer generic `S`.
  The existing `TestSerializer` trait already captures serializer bounds. The
  new drivers simply add `F: FrameCodec` as an additional generic parameter.
  Date/Author: 2026-02-26 / Plan phase.

## Outcomes & retrospective

Completed 2026-02-26. All quality gates pass:

- `make fmt` — clean (no formatting changes)
- `make check-fmt` — clean
- `make lint` — clean (cargo doc + clippy with `-D warnings`)
- `make test` — all green (0 failures)

New test coverage:

- 6 integration tests in `tests/codec_test_harness.rs` (encode round-trip,
  empty decode, extract payloads, payload driver round-trip, frame metadata
  preservation, mutable app reuse).
- 2 BDD scenarios in `tests/scenarios/codec_test_harness_scenarios.rs`
  (payload round-trip, frame-level metadata).

Files created: 7 (codec\_ext.rs, codec\_drive.rs, codec\_test\_harness.rs,
codec\_test\_harness.feature, codec\_test\_harness fixture/steps/scenarios).
Files modified: 10 (codec.rs accessor, helpers.rs, lib.rs re-exports, BDD mod
files, ADR, users-guide, roadmap).

The Stage D plan originally called for tests inside
`wireframe_testing/src/helpers/tests/`, but the crate is not a workspace member
so internal tests never execute. Moving them to `tests/` as an integration test
was the correct approach. Future ExecPlans targeting `wireframe_testing`
internals should account for this constraint.

## Context and orientation

### Repository layout (relevant subset)

```plaintext
src/
  codec.rs                          # FrameCodec trait (line 59-103)
  codec/examples.rs                 # HotlineFrameCodec, MysqlFrameCodec
  app/builder/core.rs               # WireframeApp<S,C,E,F> struct (line 29-50)
  app/builder/codec.rs              # with_codec(), serializer(), buffer_capacity()
  app/inbound_handler.rs            # handle_connection[_result]() for any F: FrameCodec

wireframe_testing/
  Cargo.toml                        # dev-dependency on wireframe (path = "..")
  src/lib.rs                        # public re-exports
  src/helpers.rs                    # TestSerializer trait, module root, constants
  src/helpers/drive.rs              # drive_internal (pub(super)), drive_with_frame[s][_mut]
  src/helpers/payloads.rs           # drive_with_payloads, drive_with_bincode
  src/helpers/codec.rs              # new_test_codec, decode_frames, encode_frame
  src/helpers/runtime.rs            # run_app, run_with_duplex_server
  src/helpers/tests/                # internal unit tests

tests/
  fixtures/mod.rs                   # BDD world fixtures
  steps/mod.rs                      # BDD step definitions
  scenarios/mod.rs                  # BDD scenario registrations
  features/                         # Gherkin .feature files
  fixtures/codec_stateful.rs        # Existing codec BDD world (reference pattern)
```

### Key types

`WireframeApp<S, C, E, F>` is the central application builder, generic over
serializer (`S`), connection context (`C`), envelope/packet type (`E`), and
frame codec (`F`). The codec defaults to `LengthDelimitedFrameCodec`.

`FrameCodec` (defined in `src/codec.rs:59-103`) requires
`Send + Sync + Clone + 'static` and has associated types `Frame`, `Decoder`,
`Encoder` with methods `decoder()`, `encoder()`, `frame_payload()`,
`wrap_payload()`, `correlation_id()`, and `max_frame_length()`.

`drive_internal` (in `wireframe_testing/src/helpers/drive.rs:31-72`) is the
low-level transport function: it takes a `server_fn: FnOnce(DuplexStream)`, raw
`Vec<Vec<u8>>` frames, and a capacity, writes the frames to the client half,
and returns the raw bytes produced by the server half. All existing drivers
delegate to it. It is `pub(super)`, accessible from sibling modules under
`helpers/`.

### BDD test pattern

The project uses rstest-bdd v0.5.0. A BDD test domain consists of four files:

1. `tests/features/<domain>.feature` — Gherkin scenarios.
2. `tests/fixtures/<domain>.rs` — a world struct with `#[fixture]` function.
3. `tests/steps/<domain>_steps.rs` — `#[given]`, `#[when]`, `#[then]`
   functions that mutate the world.
4. `tests/scenarios/<domain>_scenarios.rs` — `#[scenario]` functions binding
   feature file to steps.

Steps use `tokio::runtime::Runtime::new()?.block_on(...)` for async. Worlds are
plain structs implementing `Default`. Each must be wired into
`tests/fixtures/mod.rs`, `tests/steps/mod.rs`, and `tests/scenarios/mod.rs`.

## Plan of work

### Stage A: add `codec()` accessor to `WireframeApp`

Add a public method to `src/app/builder/codec.rs` within the existing
`impl<S, C, E, F> WireframeApp<S, C, E, F>` block (line 10-38):

```rust
/// Return a reference to the configured frame codec.
pub fn codec(&self) -> &F { &self.codec }
```

This is a one-line additive change. It enables test code (and library
consumers) to access the codec instance without cloning. The change goes in the
existing impl block that already has the correct bounds.

Stage A acceptance: `make check-fmt && make lint && make test` pass. The
accessor is callable from an external crate.

### Stage B: implement codec-aware encode/decode helpers

Create `wireframe_testing/src/helpers/codec_ext.rs` with generic functions that
encode payloads into raw bytes and decode raw bytes into frames or payloads
using any `FrameCodec`.

Functions to implement:

1. `encode_payloads_with_codec` — for each payload, call
   `codec.wrap_payload(Bytes::from(payload))` then encode the resulting frame
   using `codec.encoder()` into a `BytesMut`, returning the raw bytes for each
   encoded frame.

2. `decode_frames_with_codec` — feed the raw bytes into
   `codec.decoder()` and collect all decoded frames.

3. `extract_payloads` — call `F::frame_payload(frame).to_vec()`
   for each frame.

These are pure, testable functions with no async or side effects.

Register the module in `wireframe_testing/src/helpers.rs` as `mod codec_ext;`
and re-export the three functions via `pub use codec_ext::{...};`.

Stage B acceptance: `make check-fmt && make lint` pass. Functions compile and
are accessible from the crate root.

### Stage C: implement codec-aware driver functions

Create `wireframe_testing/src/helpers/codec_drive.rs` with the following public
async functions. All delegate to `drive_internal` from the sibling `drive`
module.

**Payload-level drivers** (return `Vec<Vec<u8>>`):

1. `drive_with_codec_payloads`
2. `drive_with_codec_payloads_with_capacity`
3. `drive_with_codec_payloads_mut`
4. `drive_with_codec_payloads_with_capacity_mut`

**Frame-level drivers** (return `Vec<F::Frame>`):

1. `drive_with_codec_frames`
2. `drive_with_codec_frames_with_capacity`

Generic bounds on all functions:
`S: TestSerializer, C: Send + 'static, E: Packet, F: FrameCodec`.

Internal flow for payload-level drivers:

```plaintext
encode_payloads_with_codec(codec, payloads)
  -> Vec<Vec<u8>> (raw encoded frames)
    -> drive_internal(|s| app.handle_connection(s), frames, capacity)
      -> Vec<u8> (raw output bytes)
        -> decode_frames_with_codec(codec, raw_output)
          -> Vec<F::Frame>
            -> extract_payloads(frames)
              -> Vec<Vec<u8>>
```

Frame-level drivers follow the same flow but return after the decode step
without extracting payloads.

Add module declaration `mod codec_drive;` and re-exports in
`wireframe_testing/src/helpers.rs`. Add public re-exports in
`wireframe_testing/src/lib.rs`.

Stage C acceptance: `make check-fmt && make lint` pass. Functions compile and
are accessible from the crate root.

### Stage D: add integration tests

Create `tests/codec_test_harness.rs` as an integration test file exercising the
new codec-aware drivers. (The tests live outside `wireframe_testing` because
the crate is a dev-dependency, not a workspace member — see Surprises.)

Tests to implement:

1. `encode_payloads_with_codec_produces_decodable_frames` — round-trip test:
   encode payloads with `HotlineFrameCodec`, decode back, verify payloads match.

2. `decode_frames_with_codec_handles_empty_input` — empty byte vector
   produces an empty frame vector.

3. `extract_payloads_returns_payload_bytes` — construct `HotlineFrame`
   values manually, verify `extract_payloads` returns the correct byte slices.

4. `drive_with_codec_payloads_round_trips_through_echo_app` — build a
   `WireframeApp` with `HotlineFrameCodec` and an echo route, drive it, verify
   payload bytes round-trip.

5. `drive_with_codec_frames_preserves_codec_metadata` — build a
   `WireframeApp` with `HotlineFrameCodec`, drive it, verify decoded
   `HotlineFrame` values carry expected `transaction_id` values.

6. `drive_with_codec_payloads_mut_allows_app_reuse` — drive a mutable app
   reference twice, verify both calls succeed.

Stage D acceptance: `make test` passes with the new tests green.

### Stage E: add rstest-bdd behavioural tests

Create four files for the `codec_test_harness` BDD domain:

**`tests/features/codec_test_harness.feature`**:

```gherkin
Feature: Codec-aware test harness drivers
  The wireframe_testing crate provides codec-aware driver functions
  that handle frame encoding and decoding transparently for any
  FrameCodec implementation.

  Scenario: Payload round-trip through a custom codec driver
    Given a wireframe app configured with a Hotline codec
    When a test payload is driven through the codec-aware driver
    Then the response payloads are non-empty

  Scenario: Frame-level driver preserves codec metadata
    Given a wireframe app configured with a Hotline codec
    When a test payload is driven through the frame-level driver
    Then the response frames contain transaction identifiers
```

**`tests/fixtures/codec_test_harness.rs`**: A `CodecTestHarnessWorld` struct
holding an optional app, codec, response payloads, and response frames.
Provides async helper methods for building the app, driving it with the
payload-level driver, and driving it with the frame-level driver. Includes a
`#[fixture]` function `codec_test_harness_world`.

**`tests/steps/codec_test_harness_steps.rs`**: Step definitions using
`#[given]`, `#[when]`, `#[then]` from `rstest_bdd_macros`. Each step wraps
async calls with `tokio::runtime::Runtime::new()?.block_on(...)`.

**`tests/scenarios/codec_test_harness_scenarios.rs`**: Two `#[scenario]`
functions binding to the feature file.

Wire the new module into:

- `tests/fixtures/mod.rs` — add `pub mod codec_test_harness;`
- `tests/steps/mod.rs` — add `mod codec_test_harness_steps;`
- `tests/scenarios/mod.rs` — add `mod codec_test_harness_scenarios;`

Stage E acceptance: `make test` passes with the new BDD scenarios green.

### Stage F: documentation and roadmap updates

1. Update `docs/adr-004-pluggable-protocol-codecs.md` with a design decision
   recording the codec-aware test harness API and its rationale (explicit codec
   parameter, payload vs frame level drivers, reuse of `drive_internal`).

2. Update `docs/users-guide.md` with a section documenting:
   - The new `codec()` accessor on `WireframeApp`.
   - The new `drive_with_codec_payloads` and `drive_with_codec_frames`
     families of functions in `wireframe_testing`.
   - A short usage example.

3. Mark roadmap item `9.7.1` as done in `docs/roadmap.md`:
   change `- [ ] 9.7.1.` to `- [x] 9.7.1.`

Stage F acceptance: documentation is internally consistent and
`make markdownlint` passes.

### Stage G: full validation and evidence capture

Run all quality gates with logging:

<!-- markdownlint-disable MD046 -->
```shell
set -o pipefail; make fmt 2>&1 | tee /tmp/9-7-1-fmt.log
set -o pipefail; make check-fmt 2>&1 | tee /tmp/9-7-1-check-fmt.log
set -o pipefail; make markdownlint 2>&1 | tee /tmp/9-7-1-markdownlint.log
set -o pipefail; make lint 2>&1 | tee /tmp/9-7-1-lint.log
set -o pipefail; make test 2>&1 | tee /tmp/9-7-1-test.log
```
<!-- markdownlint-enable MD046 -->

Update the `Progress` and `Outcomes & Retrospective` sections with final
evidence and timestamps.

Stage G acceptance: all commands exit 0.

## Validation and acceptance

Quality criteria (what "done" means):

- Tests: `make test` passes, including the new integration tests in
  `tests/codec_test_harness.rs` and the new rstest-bdd scenarios in
  `tests/scenarios/codec_test_harness_scenarios.rs`.
- Lint: `make lint` passes (Clippy with `-D warnings` on all targets).
- Format: `make check-fmt` passes.
- Markdown: `make markdownlint` passes.
- Documentation: `docs/users-guide.md` documents the new public API.
  `docs/roadmap.md` item 9.7.1 is marked done.

Quality method:

- Run the shell commands in Stage G and verify all exit 0.
- Verify the new BDD scenarios pass: `cargo test codec_test_harness`.
- Verify the new integration tests pass: `cargo test codec_test_harness`.

## Idempotence and recovery

All stages produce additive changes. If a stage fails partway through, the
incomplete changes can be reverted with `git checkout -- .` and the stage
retried from the beginning. No destructive operations are involved.

## Interfaces and dependencies

### New public accessor (wireframe crate)

In `src/app/builder/codec.rs`, within the existing impl block:

```rust
/// Return a reference to the configured frame codec.
pub fn codec(&self) -> &F { &self.codec }
```

### New public functions (wireframe\_testing crate)

In `wireframe_testing/src/helpers/codec_ext.rs`:

```rust
pub fn encode_payloads_with_codec<F: FrameCodec>(
    codec: &F,
    payloads: Vec<Vec<u8>>,
) -> io::Result<Vec<Vec<u8>>>;

pub fn decode_frames_with_codec<F: FrameCodec>(
    codec: &F,
    bytes: Vec<u8>,
) -> io::Result<Vec<F::Frame>>;

pub fn extract_payloads<F: FrameCodec>(
    frames: &[F::Frame],
) -> Vec<Vec<u8>>;
```

In `wireframe_testing/src/helpers/codec_drive.rs`:

```rust
pub async fn drive_with_codec_payloads<S, C, E, F>(
    app: WireframeApp<S, C, E, F>,
    codec: &F,
    payloads: Vec<Vec<u8>>,
) -> io::Result<Vec<Vec<u8>>>
where
    S: TestSerializer, C: Send + 'static, E: Packet, F: FrameCodec;

pub async fn drive_with_codec_payloads_with_capacity<S, C, E, F>(
    app: WireframeApp<S, C, E, F>,
    codec: &F,
    payloads: Vec<Vec<u8>>,
    capacity: usize,
) -> io::Result<Vec<Vec<u8>>>
where
    S: TestSerializer, C: Send + 'static, E: Packet, F: FrameCodec;

pub async fn drive_with_codec_payloads_mut<S, C, E, F>(
    app: &mut WireframeApp<S, C, E, F>,
    codec: &F,
    payloads: Vec<Vec<u8>>,
) -> io::Result<Vec<Vec<u8>>>
where
    S: TestSerializer, C: Send + 'static, E: Packet, F: FrameCodec;

pub async fn drive_with_codec_payloads_with_capacity_mut<S, C, E, F>(
    app: &mut WireframeApp<S, C, E, F>,
    codec: &F,
    payloads: Vec<Vec<u8>>,
    capacity: usize,
) -> io::Result<Vec<Vec<u8>>>
where
    S: TestSerializer, C: Send + 'static, E: Packet, F: FrameCodec;

pub async fn drive_with_codec_frames<S, C, E, F>(
    app: WireframeApp<S, C, E, F>,
    codec: &F,
    payloads: Vec<Vec<u8>>,
) -> io::Result<Vec<F::Frame>>
where
    S: TestSerializer, C: Send + 'static, E: Packet, F: FrameCodec;

pub async fn drive_with_codec_frames_with_capacity<S, C, E, F>(
    app: WireframeApp<S, C, E, F>,
    codec: &F,
    payloads: Vec<Vec<u8>>,
    capacity: usize,
) -> io::Result<Vec<F::Frame>>
where
    S: TestSerializer, C: Send + 'static, E: Packet, F: FrameCodec;
```

### Files to create

| File                                              | Purpose                       | Est. lines |
| ------------------------------------------------- | ----------------------------- | ---------- |
| `wireframe_testing/src/helpers/codec_ext.rs`      | Generic encode/decode helpers | ~80        |
| `wireframe_testing/src/helpers/codec_drive.rs`    | Codec-aware driver functions  | ~200       |
| `tests/codec_test_harness.rs`                     | Integration tests             | ~160       |
| `tests/features/codec_test_harness.feature`       | BDD feature file              | ~15        |
| `tests/fixtures/codec_test_harness.rs`            | BDD world fixture             | ~120       |
| `tests/steps/codec_test_harness_steps.rs`         | BDD step definitions          | ~50        |
| `tests/scenarios/codec_test_harness_scenarios.rs` | BDD scenario registrations    | ~25        |

### Files to modify

| File                                                             | Change                                               |
| ---------------------------------------------------------------- | ---------------------------------------------------- |
| `src/app/builder/codec.rs`                                       | Add `pub fn codec(&self) -> &F` accessor             |
| `wireframe_testing/src/helpers.rs`                               | Add `mod codec_ext; mod codec_drive;` and re-exports |
| `wireframe_testing/src/lib.rs`                                   | Add re-exports for new public functions              |
| (none — tests placed in `tests/codec_test_harness.rs` instead)   | See Surprises                                        |
| `tests/fixtures/mod.rs`                                          | Add `pub mod codec_test_harness;`                    |
| `tests/steps/mod.rs`                                             | Add `mod codec_test_harness_steps;`                  |
| `tests/scenarios/mod.rs`                                         | Add `mod codec_test_harness_scenarios;`              |
| `docs/adr-004-pluggable-protocol-codecs.md`                      | Add test harness design decision                     |
| `docs/users-guide.md`                                            | Document new public API                              |
| `docs/roadmap.md`                                                | Mark 9.7.1 done                                      |

## Artifacts and notes

Reference pattern for BDD world: `tests/fixtures/codec_stateful.rs` — defines
`CodecStatefulWorld` with a `SeqFrameCodec`, server management, and frame
exchange helpers. The new `CodecTestHarnessWorld` follows the same pattern but
uses `wireframe_testing` codec-aware drivers instead of manual
`Framed`/`SinkExt` operations.

Reference pattern for step definitions: `tests/steps/codec_stateful_steps.rs` —
wraps async calls with `tokio::runtime::Runtime::new()?.block_on(...)`.

Reference pattern for scenarios: `tests/scenarios/codec_stateful_scenarios.rs`
— `#[scenario]` macro with `#[expect(unused_variables)]` attribute.
