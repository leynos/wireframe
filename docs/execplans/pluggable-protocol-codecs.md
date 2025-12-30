# Pluggable protocol codecs for Wireframe

This ExecPlan is a living document. The sections `Progress`,
`Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must
be kept up to date as work proceeds.

No `PLANS.md` exists in this repository as of 2025-12-30.

<!-- ═══════════════════ WHAT (Acceptance Criteria) ═══════════════════ -->

## Purpose / Big Picture

Wireframe must support multiple wire protocol framing schemes (Hotline, MySQL,
and Redis RESP) without rewriting routing, middleware, or serialisation. A new
pluggable codec layer allows protocol-specific framing while preserving default
length-delimited behaviour for existing users. Success is visible when a custom
codec can be plugged into `WireframeApp` and the default behaviour remains
unchanged.

## Validation and Acceptance

Acceptance is based on observable behaviour:

- Existing wireframe users can still call `WireframeApp::new()` and use the
  default length-delimited framing without code changes.
- A new `FrameCodec` trait and `LengthDelimitedFrameCodec` implementation exist
  and are exported from the crate.
- `WireframeApp` and `WireframeServer` are generic over a codec type parameter
  with a default, and `.with_codec()` swaps codecs.
- New unit tests for the default codec pass.
- Optional examples (`examples/hotline_codec.rs`, `examples/mysql_codec.rs`)
  compile if included.

Validation commands (run from the repository root):

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

If Mermaid diagrams are added or edited, also run:

    set -o pipefail
    timeout 300 make nixie 2>&1 | tee /tmp/wireframe-nixie.log
    echo "nixie exit: $?"

This section defines what success looks like, independently of how it is
achieved.

<!-- ═══════════════════ HOW (Implementation Approach) ═══════════════════ -->

## Context and Orientation

The existing framing is hardcoded to `tokio_util::codec::LengthDelimitedCodec`.
Key files:

- `src/app/builder.rs` defines `WireframeApp`, its builder methods, and the
  `buffer_capacity` configuration currently tied to length-delimited framing.
- `src/app/connection.rs` builds `Framed` streams with
  `LengthDelimitedCodec` and reserves buffer capacity for reads.
- `src/app/frame_handling.rs` writes responses through
  `Framed<_, LengthDelimitedCodec>`.
- `src/server/mod.rs` and `src/server/runtime.rs` propagate `WireframeApp` type
  parameters through the server factory types.
- `src/hooks.rs` defines `WireframeProtocol`, which is currently constrained to
  `Frame = Vec<u8>`.

This work introduces `src/codec.rs` to define a `FrameCodec` trait and a
`LengthDelimitedFrameCodec` default. The new codec type parameter will flow
through `WireframeApp` and `WireframeServer`. Connection handling will use
codec-provided `Decoder` and `Encoder` instances, and buffer sizing will use
`FrameCodec::max_frame_length()`.

## Plan of Work

Start by introducing the codec abstraction and default implementation in a new
module, including tests. Next, add a new generic parameter to `WireframeApp`
with a default codec and thread the codec through builder methods. Then update
connection handling and response forwarding to use `FrameCodec` for decoding,
encoding, payload extraction, and correlation lookup. Finally, propagate the
codec type parameter through server factory types and update examples as
needed. Update documentation where new decisions are captured.

## Concrete Steps

1. Inspect existing framing usage to confirm the integration points.
   Review `src/app/connection.rs`, `src/app/frame_handling.rs`, and
   `src/app/builder.rs`.

2. Add `src/codec.rs` with:
   - A module-level `//!` comment describing the purpose.
   - `FrameCodec` trait with `Frame`, `decoder`, `encoder`, `frame_payload`,
     `wrap_payload`, optional `correlation_id`, and `max_frame_length`.
   - `LengthDelimitedFrameCodec` default implementation using
     `tokio_util::codec::LengthDelimitedCodec`.
   - Unit tests for default behaviour (decode/encode round-trip and max length
     enforcement).

3. Export the new codec module from `src/lib.rs`.

4. Update `WireframeApp` in `src/app/builder.rs`:
   - Add generic parameter `F: FrameCodec = LengthDelimitedFrameCodec`.
   - Add `codec: F` to the struct and initialise it in `Default`.
   - Add builder method `.with_codec()` and wire it through type transitions.
   - Decide how to handle `buffer_capacity()` (deprecate or re-map to codec).

5. Update connection handling in `src/app/connection.rs` and
   `src/app/frame_handling.rs`:
   - Replace `LengthDelimitedCodec` with `FrameCodec` decoder/encoder.
   - Parameterise `FrameHandlingContext` and `ResponseContext` over the codec
     and frame type.
   - Use `FrameCodec::frame_payload()` to access bytes for deserialisation.
   - Use `FrameCodec::wrap_payload()` to send responses.
   - Base buffer reservation and fragmentation defaults on
     `FrameCodec::max_frame_length()`.

6. Propagate the codec type parameter through server types in
   `src/server/mod.rs`, `src/server/runtime.rs`, and
   `src/server/connection.rs`. Ensure factory bounds remain ergonomic with
   default type parameters.

7. Add examples for Hotline and MySQL codecs if included in scope. Ensure
   they compile with `cargo build --example hotline_codec` and
   `cargo build --example mysql_codec`.

8. Run the validation commands in the Acceptance section. If any fail, fix the
   root cause and rerun the affected command.

## Interfaces and Dependencies

At the end of this work, the following interfaces must exist and be public:

- `crate::codec::FrameCodec` trait with the following signature:

    pub trait FrameCodec: Send + Sync + 'static {
        type Frame: Send + Sync + 'static;

        fn decoder(&self) -> impl tokio_util::codec::Decoder<Item = Self::Frame,
            Error = std::io::Error> + Send;
        fn encoder(&self) -> impl tokio_util::codec::Encoder<Self::Frame,
            Error = std::io::Error> + Send;
        fn frame_payload(frame: &Self::Frame) -> &[u8];
        fn wrap_payload(payload: Vec<u8>) -> Self::Frame;
        fn correlation_id(frame: &Self::Frame) -> Option<u64> { None }
        fn max_frame_length(&self) -> usize;
    }

- `crate::codec::LengthDelimitedFrameCodec` struct with `Default` and
  `new(max_frame_length: usize)`, using `Vec<u8>` as the default frame type.

- `WireframeApp` signature updated to include the codec type parameter with a
  default:

    pub struct WireframeApp<
        S: Serializer + Send + Sync = BincodeSerializer,
        C: Send + 'static = (),
        E: Packet = Envelope,
        F: FrameCodec = LengthDelimitedFrameCodec,
    > { … }

- Builder method:

    pub fn with_codec<F2: FrameCodec>(self, codec: F2) -> WireframeApp<S, C, E, F2>

Document any new helper methods for framed responses, such as
`send_response_framed_with_codec`, when added to support custom codecs.

Document any additional changes to `WireframeProtocol` or protocol hooks if
needed to align with the new frame type.

## Idempotence and Recovery

All steps are additive and can be re-run safely. If a step fails (for example,
Clippy or tests), revert only the changes introduced by that step, correct the
issue, and reapply the edit. Use the logs generated by `tee` in the validation
section to locate the failure before retrying.

<!-- ═══════════════════ TRACKING (Living Sections) ═══════════════════ -->

## Progress

- [x] (2025-12-30 16:20Z) Draft ExecPlan and ADR.
- [x] Implement `FrameCodec` and default codec module.
- [x] Thread codec through `WireframeApp` and connection handling.
- [x] Propagate codec type through server APIs.
- [x] Update documentation for codec defaults and fragmentation behaviour.
- [ ] Add protocol codec examples (if still in scope).
- [x] Run validation commands and record results.

## Surprises & Discoveries

- Observation: `tokio_util::codec::LengthDelimitedCodec` encodes `Bytes`, not
  `Vec<u8>`. Evidence: `tokio-util` source shows
  `impl Encoder<Bytes> for LengthDelimitedCodec` only.

## Decision Log

- Decision: Follow ADR 004 proposed direction for a `FrameCodec` abstraction
  with a default length-delimited implementation. Rationale: Meets protocol
  framing needs while preserving backward compatibility. Date/Author:
  2025-12-30 (Codex).
- Decision: Keep `LengthDelimitedFrameCodec` frames as `Vec<u8>` to preserve
  `WireframeProtocol` compatibility and existing framed helpers. Rationale:
  Prevents breaking protocol hooks and existing `send_response_framed` usage.
  Date/Author: 2025-12-30 (Codex).
- Decision: Add `send_response_framed_with_codec` for custom codecs and keep
  `send_response_framed` for the length-delimited default. Rationale: Maintains
  backward compatibility while enabling custom framing. Date/Author: 2025-12-30
  (Codex).
- Decision: Reset fragmentation defaults when swapping codecs via
  `WireframeApp::with_codec`. Rationale: Prevents fragmentation limits from
  exceeding the new codec's maximum frame length. Date/Author: 2025-12-30
  (Codex).
- Decision: Require `FrameCodec::Frame` to be `Send + Sync` for async send
  safety across spawned tasks. Rationale: Connection handling borrows frames
  across `await` boundaries. Date/Author: 2025-12-30 (Codex).

## Outcomes & Retrospective

- Completed framing abstraction and server propagation with full lint/test
  coverage. Protocol codec examples were not added in this pass.

## Artifacts and Notes

- None yet.
