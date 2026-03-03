# 8.5.1 Add utilities for feeding partial frames or fragments into an in-process app

This ExecPlan (execution plan) is a living document. The sections
`Constraints`, `Tolerances`, `Risks`, `Progress`, `Surprises & Discoveries`,
`Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work
proceeds.

Status: DONE

## Purpose / big picture

The `wireframe_testing` crate already provides driver functions that feed
**complete** frames into an in-process `WireframeApp` via a `tokio::io::duplex`
stream. Every existing driver writes each frame atomically with `write_all()`,
so the codec decoder never has to buffer a partial read. Real networks do not
behave this way: a single codec frame can arrive across multiple TCP reads, and
a logical message may span multiple fragment frames.

After this work, library consumers gain two new families of test helpers:

1. **Chunked-write drivers** — encode payloads via a codec, then drip-feed the
   resulting wire bytes in configurable chunk sizes (including one byte at a
   time). This exercises the codec decoder's buffering logic under realistic
   conditions.
2. **Fragment-feeding drivers** — accept a raw payload, fragment it with a
   `Fragmenter`, encode each fragment into a codec frame via
   `encode_fragment_payload`, and feed the resulting frames through the app.
   This lets test authors verify that fragmented messages survive the full app
   pipeline without manually constructing fragment wire bytes.

Success is observable by running `make test` and seeing the new unit and
behaviour-driven development (BDD) tests pass (they will fail before the
implementation and pass after).

## Constraints

- `wireframe_testing` is **not** a workspace member. Integration tests must
  live in the main crate's `tests/` directory, not inside `wireframe_testing`.
- No single source file may exceed 400 lines.
- All code must pass `make check-fmt`, `make lint` (clippy with `-D warnings`
  and all strict lints), and `make test`.
- Clippy strictness includes: no `[]` indexing (use `.get()`), no `assert!` /
  `panic!` in Result-returning functions (return `Err`), no
  `#[allow]`/`#[expect]` without `reason = "..."`, `doc_markdown` backticking,
  `cast_possible_truncation` via `try_from`.
- en-GB-oxendict spelling in comments and documentation ("-ize" / "-yse" /
  "-our").
- Public functions require `///` doc comments with usage examples.
- BDD fixtures must use `rstest-bdd` v0.5.0, and step function parameter names
  must match the fixture name exactly (e.g., `partial_frame_feeding_world`).
- Existing public APIs must not change signature.

## Tolerances (exception triggers)

- Scope: if the implementation requires changes to more than 18 files or 1200
  net lines, stop and escalate.
- Interface: if any existing public API signature must change, stop and
  escalate.
- Dependencies: if a new external dependency is required in `Cargo.toml` beyond
  what is already present, stop and escalate.
- Iterations: if tests still fail after 5 attempts at fixing, stop and
  escalate.
- Ambiguity: if multiple valid interpretations exist and the choice materially
  affects the outcome, stop and present options with trade-offs.

## Risks

- Risk: Fragment payload encoding depends on `bincode` serialization of
  `FragmentHeader` which is an internal wire format. If the fragment adapter
  layer intercepts before the driver sees the data, the test utility may need
  to bypass the adapter. Severity: medium Likelihood: low Mitigation: Use the
  existing public `encode_fragment_payload()` and `decode_fragment_payload()`
  from `src/fragment/payload.rs` which are the canonical encode/decode path. Do
  not bypass the adapter.

- Risk: The 400-line file limit may be tight for the BDD world fixture if it
  contains many helper methods. Severity: low Likelihood: medium Mitigation:
  Split the world fixture into submodules (following the
  `tests/fixtures/fragment/` pattern which already uses `mod reassembly`).

- Risk: The chunked-write driver may cause deadlocks if the duplex buffer is
  smaller than a single chunk and the server hasn't read yet. Severity: medium
  Likelihood: low Mitigation: Chunk writes are interleaved with the server
  running concurrently via `tokio::try_join!`. The duplex buffer provides
  backpressure naturally. Document the minimum capacity requirement.

## Progress

- [x] Stage A: Scaffolding — create new module files and register them.
- [x] Stage B: Implement `partial_frame.rs` (chunked-write driver).
- [x] Stage C: Implement `fragment_drive.rs` (fragment-feeding driver).
- [x] Stage D: Wire up re-exports in `helpers.rs` and `lib.rs`.
- [x] Stage E: Write unit tests in `tests/partial_frame_feeding.rs`.
- [x] Stage F: Write BDD infrastructure (feature, fixture, steps, scenarios).
- [x] Stage G: Update `docs/users-guide.md` with new public API.
- [x] Stage H: Mark roadmap item 8.5.1 as done.
- [x] Stage I: Run full validation (`make check-fmt && make lint && make test`).

## Surprises & discoveries

- Fragment payloads are `FRAG`-prefixed raw bytes produced by
  `encode_fragment_payload`. These must be wrapped inside a serialized
  `Envelope` before codec framing so the application's deserializer
  accepts them. Without wrapping, accumulating deserialization failures
  would close the connection after 10 frames (see
  `MAX_DESER_FAILURES` in `src/app/inbound_handler.rs`).
  `fragment_and_encode` now creates an `Envelope` with route ID 1 for
  each fragment and serializes it with `BincodeSerializer`.

## Decision log

- Decision: Place chunked-write logic in a new `partial_frame.rs` module
  alongside the existing `drive.rs`, rather than extending `drive_internal`.
  Rationale: `drive_internal` is `pub(super)` and used by all existing drivers.
  Adding chunked-write semantics to it would change the behaviour of every
  driver. A parallel internal function keeps the existing code untouched.
  Date/Author: 2026-03-01

- Decision: Fragment-feeding goes in `fragment_drive.rs`, composing
  `encode_fragment_payload` + `encode_payloads_with_codec` + `drive_internal`.
  Rationale: Follows the layering pattern of `codec_drive.rs` which composes
  `encode_payloads_with_codec` + `drive_internal` + `decode_frames_with_codec`.
  Fragment feeding is one layer above codec encoding. Date/Author: 2026-03-01

- Decision: BDD test suite named `partial_frame_feeding` (not
  `partial_frames` or `chunked_driver`) to match the roadmap wording "feeding
  partial frames". Rationale: Consistency with roadmap item title. Date/Author:
  2026-03-01

## Outcomes & retrospective

All acceptance criteria met:

- 9 unit tests pass in `tests/partial_frame_feeding.rs`.
- 4 BDD scenarios pass in
  `tests/scenarios/partial_frame_feeding_scenarios.rs`.
- `make check-fmt`, `make lint`, and `make test` all exit 0.
- `docs/users-guide.md` contains a new "Feeding partial frames and
  fragments" section documenting all 9 public functions.
- `docs/roadmap.md` item 8.5.1 is marked `[x]`.
- 14 files touched (7 created, 5 modified for registration/re-exports,
  2 documentation files updated), within the 18-file tolerance.

## Context and orientation

The wireframe project is a Rust async networking framework. The
`wireframe_testing` companion crate (at `wireframe_testing/`) provides
in-memory test drivers for `WireframeApp`. It is a `dev-dependency` of the main
crate, not a workspace member.

Key directories and files:

- `wireframe_testing/src/helpers/drive.rs` — base `drive_internal()` function
  that creates a `tokio::io::duplex`, writes frames to the client half, and
  collects response bytes. All higher-level drivers delegate here.
- `wireframe_testing/src/helpers/codec_drive.rs` — codec-aware drivers
  (`drive_with_codec_payloads`, `drive_with_codec_frames`) that encode payloads
  via a `FrameCodec`, transport via `drive_internal`, and decode responses.
- `wireframe_testing/src/helpers/codec_ext.rs` — `encode_payloads_with_codec`
  and `decode_frames_with_codec` utility functions.
- `wireframe_testing/src/helpers.rs` — module root, re-exports all public
  helpers, defines `TestSerializer` trait, constants `DEFAULT_CAPACITY` (4096),
  `MAX_CAPACITY` (10 MB), `TEST_MAX_FRAME` (4096).
- `wireframe_testing/src/lib.rs` — crate root, re-exports from `helpers`.
- `src/fragment/fragmenter.rs` — `Fragmenter`, `FragmentBatch`,
  `FragmentFrame` types for splitting payloads into fragments.
- `src/fragment/payload.rs` — `encode_fragment_payload(header, payload)` and
  `decode_fragment_payload(payload)` for fragment wire encoding.
- `src/fragment/header.rs` — `FragmentHeader` with `message_id`,
  `fragment_index`, `is_last_fragment`.
- `tests/fixtures/mod.rs` — fixture module index.
- `tests/steps/mod.rs` — step definition module index.
- `tests/scenarios/mod.rs` — scenario binding module index (includes
  `#[path = "../steps/mod.rs"] pub(crate) mod steps;`).

Established BDD pattern (exemplified by `codec_test_harness`):

1. Feature file: `tests/features/<name>.feature` (Gherkin).
2. World fixture: `tests/fixtures/<name>.rs` (struct + `#[fixture]` +
   helper methods; manual `Debug` impl if it holds `WireframeApp`).
3. Step definitions: `tests/steps/<name>_steps.rs` (uses `#[given]`,
   `#[when]`, `#[then]` from `rstest_bdd_macros`; async steps use
   `tokio::runtime::Runtime::new()?.block_on()`).
4. Scenario bindings: `tests/scenarios/<name>_scenarios.rs` (uses
   `#[scenario(path = "...", name = "...")]`;
   `#[expect(unused_variables, reason = "rstest-bdd wires steps
   via parameters without using them directly")]`).
5. Module registration: add `mod <name>;` to `tests/fixtures/mod.rs`,
   `mod <name>_steps;` to `tests/steps/mod.rs`, `mod <name>_scenarios;` to
   `tests/scenarios/mod.rs`.

## Plan of work

### Stage A: Scaffolding

Create empty module files and register them in their respective `mod.rs` files.

New files to create:

1. `wireframe_testing/src/helpers/partial_frame.rs` — chunked-write driver.
2. `wireframe_testing/src/helpers/fragment_drive.rs` — fragment-feeding
   driver.
3. `tests/features/partial_frame_feeding.feature` — Gherkin scenarios.
4. `tests/fixtures/partial_frame_feeding.rs` — BDD world fixture.
5. `tests/steps/partial_frame_feeding_steps.rs` — BDD step definitions.
6. `tests/scenarios/partial_frame_feeding_scenarios.rs` — scenario bindings.
7. `tests/partial_frame_feeding.rs` — rstest unit tests (integration test
   file in main crate).

Files to modify for registration:

1. `wireframe_testing/src/helpers.rs` — add `mod partial_frame; mod
   fragment_drive;` and re-export public symbols.
2. `wireframe_testing/src/lib.rs` — re-export new public symbols.
3. `tests/fixtures/mod.rs` — add `pub mod partial_frame_feeding;`.
4. `tests/steps/mod.rs` — add `mod partial_frame_feeding_steps;`.
5. `tests/scenarios/mod.rs` — add `mod partial_frame_feeding_scenarios;`.

### Stage B: Implement chunked-write driver (`partial_frame.rs`)

This module provides a `drive_chunked_internal` function (analogous to
`drive_internal` in `drive.rs`) and public driver functions that compose codec
encoding with chunked writing.

#### `drive_chunked_internal` (private to `helpers`)

```rust
pub(super) async fn drive_chunked_internal<F, Fut>(
    server_fn: F,
    wire_bytes: Vec<u8>,
    chunk_size: NonZeroUsize,
    capacity: usize,
) -> io::Result<Vec<u8>>
where
    F: FnOnce(DuplexStream) -> Fut,
    Fut: std::future::Future<Output = ()> + Send,
```

Instead of writing each frame via `write_all`, this function concatenates all
wire bytes and writes them `chunk_size` bytes at a time, calling
`write_all(&chunk)` for each slice. This forces the codec decoder on the server
side to buffer partial frames across reads.

The server-side panic handling mirrors `drive_internal` exactly (using
`catch_unwind` + `wireframe::panic::format_panic`). The client-side loop
replaces the per-frame `write_all` with a chunked iteration:

```rust
let client_fut = async {
    let total = wire_bytes.len();
    let step = chunk_size.get();
    let mut offset = 0;
    while offset < total {
        let end = (offset + step).min(total);
        let chunk = wire_bytes.get(offset..end)
            .ok_or_else(|| io::Error::other("chunk slice out of bounds"))?;
        client.write_all(chunk).await?;
        offset = end;
    }
    client.shutdown().await?;

    let mut buf = Vec::new();
    client.read_to_end(&mut buf).await?;
    io::Result::Ok(buf)
};
```

#### Public API: chunked codec drivers

Following the naming pattern of `drive_with_codec_payloads`, the new functions
are:

```rust
/// Drive `app` with payloads encoded by `codec`, writing wire bytes in
/// chunks of `chunk_size` to exercise partial-frame buffering.
pub async fn drive_with_partial_frames<S, C, E, F>(
    app: WireframeApp<S, C, E, F>,
    codec: &F,
    payloads: Vec<Vec<u8>>,
    chunk_size: NonZeroUsize,
) -> io::Result<Vec<Vec<u8>>>

/// Variant returning full decoded codec frames.
pub async fn drive_with_partial_codec_frames<S, C, E, F>(
    app: WireframeApp<S, C, E, F>,
    codec: &F,
    payloads: Vec<Vec<u8>>,
    chunk_size: NonZeroUsize,
) -> io::Result<Vec<F::Frame>>

/// Mutable-app variant returning decoded payloads.
pub async fn drive_with_partial_frames_mut<S, C, E, F>(
    app: &mut WireframeApp<S, C, E, F>,
    codec: &F,
    payloads: Vec<Vec<u8>>,
    chunk_size: NonZeroUsize,
) -> io::Result<Vec<Vec<u8>>>

/// Variant with explicit duplex buffer capacity.
pub async fn drive_with_partial_frames_with_capacity<S, C, E, F>(
    app: WireframeApp<S, C, E, F>,
    codec: &F,
    payloads: Vec<Vec<u8>>,
    chunk_size: NonZeroUsize,
    capacity: usize,
) -> io::Result<Vec<Vec<u8>>>
```

Each function composes: `encode_payloads_with_codec` → flatten to single
`Vec<u8>` → `drive_chunked_internal` → `decode_frames_with_codec` → optionally
`extract_payloads`.

### Stage C: Implement fragment-feeding driver (`fragment_drive.rs`)

This module provides functions that fragment a payload and feed the resulting
fragment frames through the app.

#### Public API

```rust
/// Fragment `payload` using `fragmenter`, encode each fragment with
/// `encode_fragment_payload`, wrap in codec frames, and drive through `app`.
/// Returns decoded response payloads.
pub async fn drive_with_fragments<S, C, E, F>(
    app: WireframeApp<S, C, E, F>,
    codec: &F,
    fragmenter: &Fragmenter,
    payload: Vec<u8>,
) -> io::Result<Vec<Vec<u8>>>

/// Variant returning full decoded codec frames.
pub async fn drive_with_fragment_frames<S, C, E, F>(
    app: WireframeApp<S, C, E, F>,
    codec: &F,
    fragmenter: &Fragmenter,
    payload: Vec<u8>,
) -> io::Result<Vec<F::Frame>>

/// Mutable-app variant.
pub async fn drive_with_fragments_mut<S, C, E, F>(
    app: &mut WireframeApp<S, C, E, F>,
    codec: &F,
    fragmenter: &Fragmenter,
    payload: Vec<u8>,
) -> io::Result<Vec<Vec<u8>>>

/// Fragment and feed with configurable duplex capacity.
pub async fn drive_with_fragments_with_capacity<S, C, E, F>(
    app: WireframeApp<S, C, E, F>,
    codec: &F,
    fragmenter: &Fragmenter,
    payload: Vec<u8>,
    capacity: usize,
) -> io::Result<Vec<Vec<u8>>>
```

Internal composition: `fragmenter.fragment_bytes(payload)` → for each
`FragmentFrame`: `encode_fragment_payload(header, payload)` → collect as
payloads → `encode_payloads_with_codec(codec, payloads)` →
`drive_internal(handler, encoded, capacity)` →
`decode_frames_with_codec(codec, raw)` → optionally `extract_payloads`.

A combined helper that fragments AND feeds in chunks is also provided:

```rust
/// Fragment `payload` and feed the resulting wire bytes in chunks of
/// `chunk_size`, exercising both fragmentation and partial-frame
/// buffering simultaneously.
pub async fn drive_with_partial_fragments<S, C, E, F>(
    app: WireframeApp<S, C, E, F>,
    codec: &F,
    fragmenter: &Fragmenter,
    payload: Vec<u8>,
    chunk_size: NonZeroUsize,
) -> io::Result<Vec<Vec<u8>>>
```

### Stage D: Wire up re-exports

In `wireframe_testing/src/helpers.rs`:

- Add `mod partial_frame;` and `mod fragment_drive;`.
- Add `pub use partial_frame::{...};` for all public functions.
- Add `pub use fragment_drive::{...};` for all public functions.

In `wireframe_testing/src/lib.rs`:

- Add re-exports for all new public symbols to the existing `pub use
  helpers::{…};` block.

### Stage E: Unit tests (`tests/partial_frame_feeding.rs`)

Integration test file in the main crate. Uses `rstest` for fixtures and
parameterised cases. Tests:

1. **Chunked single-byte write round-trips** — encode a payload via
   `HotlineFrameCodec`, feed one byte at a time via
   `drive_with_partial_frames`, verify decoded payloads match input.
2. **Chunked multi-byte write round-trips** — same with `chunk_size = 7`
   (deliberately misaligned with frame boundaries).
3. **Multiple payloads chunked** — encode two payloads, feed in 3-byte
   chunks, verify both decoded correctly.
4. **Fragment round-trip** — fragment a 100-byte payload with
   `max_fragment_size = 20`, feed via `drive_with_fragments`, verify response
   received (app processes the fragment frames).
5. **Partial fragment round-trip** — fragment a payload AND feed in chunks
   via `drive_with_partial_fragments`, verify response received.
6. **Mutable app reuse** — call `drive_with_partial_frames_mut` twice with
   the same app, verify both succeed.

### Stage F: BDD infrastructure

#### Feature file: `tests/features/partial_frame_feeding.feature`

```gherkin
@partial-frame-feeding
Feature: Partial frame and fragment feeding utilities

  Scenario: Single payload survives byte-at-a-time chunked delivery
    Given a wireframe app with a Hotline codec allowing 4096-byte frames
    When a test payload is fed in 1-byte chunks
    Then the decoded response payloads are non-empty

  Scenario: Multiple payloads survive misaligned chunked delivery
    Given a wireframe app with a Hotline codec allowing 4096-byte frames
    When 2 test payloads are fed in 7-byte chunks
    Then the decoded response contains 2 payloads

  Scenario: Fragmented payload is delivered as fragment frames
    Given a wireframe app with a Hotline codec allowing 4096-byte frames
    And a fragmenter capped at 20 bytes per fragment
    When a 100-byte payload is fragmented and fed through the app
    Then the app receives fragment frames

  Scenario: Fragmented payload survives chunked delivery
    Given a wireframe app with a Hotline codec allowing 4096-byte frames
    And a fragmenter capped at 20 bytes per fragment
    When a 100-byte payload is fragmented and fed in 3-byte chunks
    Then the app receives fragment frames
```

#### World fixture: `tests/fixtures/partial_frame_feeding.rs`

A `PartialFrameFeedingWorld` struct holding:

- `codec: Option<HotlineFrameCodec>`
- `app: Option<WireframeApp<BincodeSerializer, (), Envelope, HotlineFrameCodec>>`
- `fragmenter: Option<Fragmenter>`
- `response_payloads: Vec<Vec<u8>>`
- `response_frames: Vec<HotlineFrame>`

Manual `Debug` impl (because `WireframeApp` lacks `Debug`). Default impl.
`#[fixture] pub fn partial_frame_feeding_world()` fixture function.

Helper methods:

- `configure_app(max_frame_length)` — build app with `HotlineFrameCodec`
  and a no-op route handler.
- `configure_fragmenter(max_payload)` — create `Fragmenter`.
- `drive_chunked(chunk_size)` — call `drive_with_partial_frames`.
- `drive_chunked_multiple(count, chunk_size)` — multiple payloads.
- `drive_fragmented(payload_len)` — call `drive_with_fragments`.
- `drive_partial_fragmented(payload_len, chunk_size)` — call
  `drive_with_partial_fragments`.
- Assertion helpers: `assert_payloads_non_empty()`,
  `assert_payload_count(n)`, `assert_received_fragments()`.

#### Steps: `tests/steps/partial_frame_feeding_steps.rs`

Step functions delegating to world methods, following the
`codec_test_harness_steps.rs` pattern. Async operations wrapped in
`tokio::runtime::Runtime::new()?.block_on(...)`.

#### Scenarios: `tests/scenarios/partial_frame_feeding_scenarios.rs`

One `#[scenario]` function per feature scenario, each with
`#[expect(unused_variables, reason = "rstest-bdd wires steps
via parameters without using them directly")]`.

### Stage G: Update `docs/users-guide.md`

Add a new subsection after the existing "Testing custom codecs with
`wireframe_testing`" section (around line 257) titled "Feeding partial frames
and fragments". Document all new public functions with a short code example
showing `drive_with_partial_frames` and `drive_with_fragments`.

### Stage H: Mark roadmap done

In `docs/roadmap.md`, change the 8.5.1 line from `- [ ]` to `- [x]`.

### Stage I: Full validation

Run `make check-fmt && make lint && make test` and verify all pass.

## Concrete steps

All commands run from the repository root `/home/user/project`.

### Validation before changes

```bash
set -o pipefail
make check-fmt 2>&1 | tee /tmp/wireframe-check-fmt.log
make lint 2>&1 | tee /tmp/wireframe-lint.log
make test 2>&1 | tee /tmp/wireframe-test.log
```

Expected: all three exit 0.

### After each stage

```bash
set -o pipefail
make check-fmt 2>&1 | tee /tmp/wireframe-check-fmt.log
make lint 2>&1 | tee /tmp/wireframe-lint.log
make test 2>&1 | tee /tmp/wireframe-test.log
```

Expected: all three exit 0.

### Final validation

```bash
set -o pipefail
make check-fmt 2>&1 | tee /tmp/wireframe-check-fmt.log; echo "check-fmt exit: $?"
make lint 2>&1 | tee /tmp/wireframe-lint.log; echo "lint exit: $?"
make test 2>&1 | tee /tmp/wireframe-test.log; echo "test exit: $?"
```

Expected: all three report exit 0.

## Validation and acceptance

Quality criteria:

- Tests: `make test` passes. New tests in `tests/partial_frame_feeding.rs`
  and BDD scenarios in `tests/scenarios/partial_frame_feeding_scenarios.rs` all
  pass. BDD feature file has at least 4 scenarios.
- Lint/typecheck: `make check-fmt` and `make lint` exit 0 with no warnings.
- Documentation: `docs/users-guide.md` contains a new section documenting
  the partial-frame and fragment feeding utilities.
- Roadmap: `docs/roadmap.md` item 8.5.1 is marked `[x]`.

Quality method:

- `make check-fmt && make lint && make test` run from the repository root.
- Manual inspection that the new section exists in `docs/users-guide.md`.
- Manual inspection that `docs/roadmap.md` shows `[x]` for 8.5.1.

## Idempotence and recovery

All stages are additive (new files and new `pub use` lines). No existing code
is modified except for adding `mod` and `pub use` declarations. Stages can be
re-run by overwriting the created files. If a stage fails validation, fix the
issue and re-run that stage's validation.

## Artifacts and notes

Files created (7):

- `wireframe_testing/src/helpers/partial_frame.rs`
- `wireframe_testing/src/helpers/fragment_drive.rs`
- `tests/features/partial_frame_feeding.feature`
- `tests/fixtures/partial_frame_feeding.rs`
- `tests/steps/partial_frame_feeding_steps.rs`
- `tests/scenarios/partial_frame_feeding_scenarios.rs`
- `tests/partial_frame_feeding.rs`

Files modified (5):

- `wireframe_testing/src/helpers.rs`
- `wireframe_testing/src/lib.rs`
- `tests/fixtures/mod.rs`
- `tests/steps/mod.rs`
- `tests/scenarios/mod.rs`

Documentation files modified (2):

- `docs/users-guide.md`
- `docs/roadmap.md`

Total: 14 files touched.

## Interfaces and dependencies

All dependencies are already present in `wireframe_testing/Cargo.toml`
(`tokio`, `wireframe`, `bytes`, `futures`, `tokio-util`) and the main crate's
`Cargo.toml` (`rstest`, `rstest-bdd`, `wireframe_testing`). No new dependencies
are required.

### New public types and functions in `wireframe_testing`

In `wireframe_testing/src/helpers/partial_frame.rs`:

```rust
use std::num::NonZeroUsize;

// Internal (pub(super)):
pub(super) async fn drive_chunked_internal<F, Fut>(
    server_fn: F,
    wire_bytes: Vec<u8>,
    chunk_size: NonZeroUsize,
    capacity: usize,
) -> io::Result<Vec<u8>>;

// Public:
pub async fn drive_with_partial_frames<S, C, E, F>(...) -> io::Result<Vec<Vec<u8>>>;
pub async fn drive_with_partial_frames_with_capacity<S, C, E, F>(...) -> io::Result<Vec<Vec<u8>>>;
pub async fn drive_with_partial_frames_mut<S, C, E, F>(...) -> io::Result<Vec<Vec<u8>>>;
pub async fn drive_with_partial_codec_frames<S, C, E, F>(...) -> io::Result<Vec<F::Frame>>;
```

In `wireframe_testing/src/helpers/fragment_drive.rs`:

```rust
pub async fn drive_with_fragments<S, C, E, F>(...) -> io::Result<Vec<Vec<u8>>>;
pub async fn drive_with_fragments_with_capacity<S, C, E, F>(...) -> io::Result<Vec<Vec<u8>>>;
pub async fn drive_with_fragments_mut<S, C, E, F>(...) -> io::Result<Vec<Vec<u8>>>;
pub async fn drive_with_fragment_frames<S, C, E, F>(...) -> io::Result<Vec<F::Frame>>;
pub async fn drive_with_partial_fragments<S, C, E, F>(...) -> io::Result<Vec<Vec<u8>>>;
```

### Reused existing functions

- `drive_internal` from `wireframe_testing/src/helpers/drive.rs` — used by
  `fragment_drive.rs` for non-chunked fragment feeding.
- `encode_payloads_with_codec` from
  `wireframe_testing/src/helpers/codec_ext.rs` — used by both new modules for
  codec encoding.
- `decode_frames_with_codec` from
  `wireframe_testing/src/helpers/codec_ext.rs` — used by both new modules for
  response decoding.
- `extract_payloads` from `wireframe_testing/src/helpers/codec_ext.rs` —
  used to convert frames to payload byte vectors.
- `encode_fragment_payload` from `src/fragment/payload.rs` — used by
  `fragment_drive.rs` to encode fragment headers and payloads.
- `Fragmenter`, `FragmentBatch`, `FragmentFrame` from
  `src/fragment/fragmenter.rs` — used by `fragment_drive.rs`.
- `DEFAULT_CAPACITY` from `wireframe_testing/src/helpers.rs`.
