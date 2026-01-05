# Instance-aware payload wrapping for FrameCodec

This execution plan (ExecPlan) is a living document. The sections `Progress`,
`Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must
be kept up to date as work proceeds.

No `PLANS.md` exists in this repository as of 2026-01-04.

## Purpose / Big Picture

Stateful codecs (for example, protocols that stamp sequence counters or
transaction IDs in headers) need access to per-connection state when wrapping
outbound payloads. Previously `FrameCodec::wrap_payload` was a static function
that only saw a `Vec<u8>`, which forced copies and prevented deterministic
state advancement. This change makes `wrap_payload` instance-aware and accept
`Bytes`, then threads a per-connection codec instance through the send path so
state advances deterministically per connection. Success is visible when a
custom codec can increment a sequence counter on each response without
cross-connection interference, and the default length-delimited behaviour
remains unchanged.

## Progress

- [x] (2026-01-04 00:00Z) Drafted ExecPlan for 9.1.1.
- [x] (2026-01-04 01:05Z) Reviewed codec usage and send paths for wrap sites.
- [x] (2026-01-04 01:20Z) Updated `FrameCodec::wrap_payload` to accept `&self`
  and `Bytes`, then adjusted implementations and call sites.
- [x] (2026-01-04 01:30Z) Reused a per-connection codec instance and encoder in
  the connection pipeline.
- [x] (2026-01-04 01:50Z) Added unit, integration, and behavioural tests for
  instance-aware wrapping.
- [x] (2026-01-04 02:05Z) Updated documentation and marked roadmap entry 9.1.1
  as done.
- [x] (2026-01-04 02:30Z) Ran formatting, lint, and test gates.

## Surprises & Discoveries

- Observation: `make markdownlint` required overriding `MDLINT` to point at
  `/root/.bun/bin/markdownlint-cli2`. Evidence:
  `/tmp/wireframe-markdownlint.log`.

## Decision Log

- Decision: Require `FrameCodec` to be `Clone` and clone it per connection.
  Rationale: `wrap_payload(&self, Bytes)` needs a per-connection state holder,
  and cloning allows deterministic counters without cross-connection leakage.
  Date/Author: 2026-01-04 / Codex.

## Outcomes & Retrospective

Instance-aware payload wrapping is implemented, with per-connection codec
clones and a `Bytes`-based payload surface. Unit, integration, and behavioural
tests validate the new stateful wrapping behaviour, and the ADR, user's guide,
and roadmap entries are updated to reflect the new API. Remaining work is to
run the formatting, lint, and test gates.

## Context and Orientation

The codec abstraction lives in `src/codec.rs` with the `FrameCodec` trait and
`LengthDelimitedFrameCodec` default. The outbound send path uses
`FrameCodec::wrap_payload` in two places:

- `src/app/connection.rs` in `WireframeApp::send_response` and
  `WireframeApp::send_response_framed_with_codec`.
- `src/app/frame_handling.rs` in `send_response_payload`, which feeds the
  `Framed` stream used by `handle_connection_result`.

Inbound decoding uses `FrameCodec::frame_payload` in
`WireframeApp::decode_envelope` (`src/app/connection.rs`). Connection framing
uses `CombinedCodec` in `src/app/combined_codec.rs` to pair a codec decoder and
encoder into a `Framed` stream.

Tests and examples that implement `FrameCodec` live in:

- `tests/frame_codec.rs` and `tests/example_codecs.rs`.
- `src/codec/examples.rs` plus `examples/hotline_codec.rs` and
  `examples/mysql_codec.rs`.
- `src/app/frame_handling.rs` test module.

Relevant documentation to update or cross-check:

- `docs/roadmap.md` (mark 9.1.1 done when complete).
- `docs/adr-004-pluggable-protocol-codecs.md` (record the signature change and
  per-connection state decision).
- `docs/users-guide.md` (public interface description of `FrameCodec`).
- `docs/multi-packet-and-streaming-responses-design.md` (references current
  `wrap_payload` limits; update if the description changes).
- `docs/generic-message-fragmentation-and-re-assembly-design.md`.
- docs/the-road-to-wireframe-1-0-feature-set-philosophy-and-capability-
  maturity.md.
- docs/hardening-wireframe-a-guide-to-production-resilience.md for alignment
  notes.

Behavioural tests run via `tests/cucumber.rs` and feature files in
`tests/features/*.feature` with step implementations under `tests/steps/` and
worlds under `tests/worlds/`.

## Plan of Work

First, adjust the `FrameCodec` trait to accept `&self` and `Bytes` in
`wrap_payload`, then update all implementations to match. Next, ensure the
outbound send path uses a per-connection codec instance so stateful wrappers
advance deterministically per connection. That requires threading the codec
instance through response forwarding and revisiting `send_response` helpers
that build frames outside a `Framed` stream.

After the API change, update all tests and examples to use `Bytes` payloads and
instance-aware wrapping. Add new tests that confirm per-connection sequence
state advances correctly, including a behavioural scenario using the existing
Cucumber harness. Finally, update documentation (ADR and user's guide) and mark
roadmap item 9.1.1 as done, then run the required Makefile gates.

## Concrete Steps

1. Inspect the existing trait and send paths to catalogue `wrap_payload` usage:
   `src/codec.rs`, `src/app/connection.rs`, `src/app/frame_handling.rs`,
   `tests/frame_codec.rs`, `tests/example_codecs.rs`, and the codec examples.

2. Update the trait signature in `src/codec.rs`:
   - Change `fn wrap_payload(payload: Vec<u8>) -> Self::Frame` to
     `fn wrap_payload(&self, payload: Bytes) -> Self::Frame`.
   - If per-connection state requires cloning, add `Clone` to the `FrameCodec`
     trait bounds and derive or implement `Clone` for codec implementations.
   - Update `LengthDelimitedFrameCodec` to return the passed `Bytes` directly
     (no extra copy).

3. Thread the codec instance through the outbound send path:
   - Update `send_response_payload` in `src/app/frame_handling.rs` to accept a
     `&F` codec reference and `Bytes` payload, then call
     `codec.wrap_payload(payload)`.
   - Extend `ResponseContext` (and any related context structs) to carry a
     `&F` so `send_response_payload` can use the per-connection codec
     instance.
   - In `WireframeApp::process_stream` (`src/app/connection.rs`), create a
     per-connection codec value (for example
     `let mut codec = self.codec.clone();`)
     and use it both to build the `CombinedCodec` and to populate the response
     context passed to `frame_handling::forward_response`.
   - Update `WireframeApp::send_response` and
     `WireframeApp::send_response_framed_with_codec` to wrap payloads via
     `self.codec.wrap_payload(Bytes::from(bytes))`.

4. Update all codec implementations and tests:
   - Adjust `src/codec/examples.rs`, `examples/hotline_codec.rs`,
     `examples/mysql_codec.rs`, `tests/frame_codec.rs`,
     `tests/example_codecs.rs`, and the codec test module in
     `src/app/frame_handling.rs` to use `Bytes` payloads and `&self` wrapping.
   - Ensure any stateful codec used in tests increments a counter in
     `wrap_payload` and stores it in the frame.

5. Add tests for instance-aware wrapping:
   - Unit tests in `src/codec.rs` to confirm `LengthDelimitedFrameCodec` accepts
     a `Bytes` payload without copying and still enforces size limits.
   - Integration test in `tests/frame_codec.rs` that confirms a stateful codec
     increments sequence numbers across multiple responses in a single
     connection.
   - Behavioural test using the Cucumber harness:
     - Add a feature file such as `tests/features/codec_stateful.feature` that
       opens two connections, sends two requests on each, and asserts sequence
       numbers reset per connection.
     - Add a new world and step definitions (for example
       `tests/worlds/codec_stateful.rs` and
       `tests/steps/codec_stateful_steps.rs`)
       that spin up a `WireframeApp` with the stateful codec and decode frames
       using the same codec to observe the sequence counters.

6. Update documentation:
   - `docs/adr-004-pluggable-protocol-codecs.md`: record the new signature,
     the rationale for `Bytes`, and the per-connection state decision.
   - `docs/users-guide.md`: update the `FrameCodec` requirements section and
     any examples to show `wrap_payload(&self, Bytes)`.
   - Review the design documents listed in the prompt for references to
     `wrap_payload` limitations and update if the guidance has changed.
   - Mark roadmap entry 9.1.1 as done in `docs/roadmap.md`.

7. Run validation gates (from repository root), using tee for full logs:

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

   If Mermaid diagrams are touched, also run:

       set -o pipefail
       timeout 300 make nixie 2>&1 | tee /tmp/wireframe-nixie.log
       echo "nixie exit: $?"

## Validation and Acceptance

Acceptance is based on observable behaviour:

- A custom stateful codec can increment a sequence counter in
  `wrap_payload(&self, Bytes)` and the counter advances deterministically for
  each response on a single connection.
- Opening a second connection resets the counter (per-connection state), which
  is verified by a behavioural test scenario.
- The default `LengthDelimitedFrameCodec` still round-trips payloads and
  enforces maximum frame length, and existing clients remain unaffected.
- The new unit test fails before the API change (due to missing instance-aware
  wrapping) and passes after.
- `make check-fmt`, `make lint`, and `make test` succeed.

## Idempotence and Recovery

All changes are safe to reapply. If a refactor breaks compilation, revert the
individual file changes and reapply the steps one by one. Behavioural tests can
be re-run without side effects because they bind to ephemeral Transmission
Control Protocol (TCP) ports and clean up after each scenario.

## Artifacts and Notes

Record key evidence here once available, for example:

    - `make fmt` log: `/tmp/wireframe-fmt.log`.
    - `make markdownlint` log: `/tmp/wireframe-markdownlint.log`.
    - `make check-fmt` log: `/tmp/wireframe-check-fmt.log`.
    - `make lint` log: `/tmp/wireframe-lint.log`.
    - `make test` log: `/tmp/wireframe-test.log`.

## Interfaces and Dependencies

At completion, the codec interface must look like this (in `src/codec.rs`):

    pub trait FrameCodec: Send + Sync + Clone + 'static {
        type Frame: Send + Sync + 'static;
        type Decoder: Decoder<Item = Self::Frame, Error = io::Error> + Send;
        type Encoder: Encoder<Self::Frame, Error = io::Error> + Send;

        fn decoder(&self) -> Self::Decoder;
        fn encoder(&self) -> Self::Encoder;
        fn frame_payload(frame: &Self::Frame) -> &[u8];
        fn wrap_payload(&self, payload: Bytes) -> Self::Frame;
        fn correlation_id(_frame: &Self::Frame) -> Option<u64> { None }
        fn max_frame_length(&self) -> usize;
    }

Wireframe clones the codec per connection, so ensure `Clone` produces the
desired per-connection state when counters or dictionaries are involved.

## Revision note (required when editing an ExecPlan)

Updated progress, decisions, and outcomes to reflect the implemented changes,
including the `FrameCodec` signature update, per-connection codec reuse, tests,
and documentation edits.
