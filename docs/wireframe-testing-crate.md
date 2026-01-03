# `wireframe_testing`: testing helpers for Wireframe

`wireframe_testing` is the companion crate for exercising Wireframe
applications and codecs in unit, integration, and behavioural tests. It
provides in-memory drivers for `WireframeApp` instances, codec-aware framing
helpers, and a scoped observability harness for log and metrics assertions.

## Motivation

Wireframe now supports pluggable protocol codecs (ADR 004). The test harness
must encode and decode frames using the selected `FrameCodec`, preserving
protocol metadata and correlation identifiers. It must also allow malformed
wire bytes for negative codec tests. ADR 006 proposes a unified test
observability harness so tests can assert on logs and metrics deterministically
without external exporters.

## Design goals

- Work with any `FrameCodec`, including codecs that carry metadata and
  correlation identifiers.
- Preserve frame metadata when driving handlers or asserting responses.
- Allow raw byte injection for malformed frame and recovery tests.
- Provide per-test log and metrics capture without leaking global state.
- Keep helpers fast by using in-memory duplex streams instead of sockets.

## Crate layout

- `src/lib.rs` re-exports the public API.
- `src/helpers.rs` provides in-memory drivers and codec-aware encode/decode
  helpers.
- `src/observability.rs` implements the observability harness from ADR 006.
- `src/logging.rs` retains the standalone `LoggerHandle` fixture.
- `src/fixtures/` (or `src/codec_fixtures.rs`) stores reusable frame fixtures
  for the default codec and example codecs.
- `src/multi_packet.rs` keeps the `collect_multi_packet` helper.

## Dependencies

```toml
[dependencies]
tokio = { version = "1", features = ["macros", "rt", "io-util"] }
wireframe = { version = "0.1.0", path = ".." }
bincode = "2.0"
bytes = "1.0"
futures = "0.3"
tokio-util = { version = "0.7", features = ["codec"] }
log = "0.4"
logtest = "2"
metrics = "0.24.2"
metrics-util = "0.20.0"
rstest = "0.18.2"
```

## Codec-aware drivers

The helpers remain centred on a single in-memory driver that runs
`WireframeApp::handle_connection` against a `tokio::io::duplex` stream. The
driver is responsible for framing inbound and outbound data using the selected
`FrameCodec` and for surfacing server panics as `io::Error` values prefixed
with `server task failed`.

`wireframe_testing` retains the `TestSerializer` trait alias to keep bounds
readable:

```rust,no_run
use wireframe::app::Envelope;
use wireframe::frame::FrameMetadata;
use wireframe::serializer::Serializer;

pub trait TestSerializer:
    Serializer + FrameMetadata<Frame = Envelope> + Send + Sync + 'static
{
}
```

### Driver entry points

The primary driver APIs accept both the app and the codec used to configure it.
Tests should pass the same codec instance used in `WireframeApp::with_codec` to
ensure framing configuration (such as maximum frame length) matches.

```rust,no_run
use std::io;
use wireframe::app::{Packet, WireframeApp};
use wireframe::codec::FrameCodec;

pub async fn drive_with_frames<S, C, E, F>(
    app: WireframeApp<S, C, E, F>,
    codec: &F,
    frames: Vec<F::Frame>,
) -> io::Result<Vec<F::Frame>>
where
    S: TestSerializer,
    C: Send + 'static,
    E: Packet,
    F: FrameCodec;

pub async fn drive_with_payloads<S, C, E, F>(
    app: WireframeApp<S, C, E, F>,
    codec: &F,
    payloads: Vec<Vec<u8>>,
) -> io::Result<Vec<F::Frame>>
where
    S: TestSerializer,
    C: Send + 'static,
    E: Packet,
    F: FrameCodec;

pub async fn drive_with_raw_bytes<S, C, E, F>(
    app: WireframeApp<S, C, E, F>,
    wire_bytes: Vec<Vec<u8>>,
    capacity: Option<usize>,
) -> io::Result<Vec<u8>>
where
    S: TestSerializer,
    C: Send + 'static,
    E: Packet,
    F: FrameCodec;
```

Behavioural details:

- `drive_with_frames` encodes each `F::Frame` with `codec.encoder()`, writes the
  resulting bytes to the duplex stream, reads the server response bytes, and
  decodes them with `codec.decoder()`.
- `drive_with_payloads` wraps payload bytes using `F::wrap_payload` before
  delegating to `drive_with_frames`.
- `drive_with_raw_bytes` writes the provided bytes verbatim without encoding or
  decoding. This is the entry point for malformed frame tests and recovery
  policy validation.
- Mutable variants (`drive_with_frames_mut` and `drive_with_payloads_mut`)
  accept `&mut WireframeApp` so tests can reuse a configured instance.
- I/O failures, codec encode/decode failures, and server task panics are all
  returned as `io::Error` values so tests can assert on error handling.

### Buffer capacity and limits

The duplex stream buffer should default to the codec maximum frame length,
clamped to a shared safety ceiling that matches Wireframe's guardrail. The
driver should reject a `capacity` of zero or above this ceiling with
`io::ErrorKind::InvalidInput`.

To keep the guardrail aligned, expose a public constant (or helper) from
`wireframe::codec` so `wireframe_testing` does not duplicate the limit.

### Frame encoding and decoding helpers

Codec-aware helpers make it easy to build fixtures or inspect raw bytes:

```rust,no_run
use std::io;
use wireframe::codec::FrameCodec;

pub fn encode_frames<F>(codec: &F, frames: Vec<F::Frame>) -> io::Result<Vec<u8>>
where
    F: FrameCodec;

pub fn decode_frames<F>(codec: &F, bytes: Vec<u8>) -> io::Result<Vec<F::Frame>>
where
    F: FrameCodec;
```

`decode_frames` should return an error when trailing bytes remain in the buffer
after the last frame, so tests can detect partial or malformed streams.

### Bincode convenience wrapper

Most tests still send a single request encoded with bincode. Keep a small
wrapper that performs `bincode::encode_to_vec` with
`bincode::config::standard()`, wraps the payload via `F::wrap_payload`, and
drives the app:

```rust,no_run
#[derive(bincode::Encode)]
struct Ping(u8);

let frames = drive_with_bincode(app, &codec, Ping(1)).await?;
assert_eq!(F::frame_payload(&frames[0]), &[1]);
```

## Codec fixtures

Add reusable fixtures to avoid duplicated framing logic in tests:

- Default codec fixtures that yield `Bytes` frames, oversized payloads, and
  truncated prefixes for negative tests.
- Example codec fixtures that mirror `wireframe::codec::examples` (for example
  Hotline and MySQL), including helpers for correlation identifiers.
- Invalid frame builders (bad lengths, missing headers, truncated payloads) for
  codec error and recovery assertions.

Fixtures should return `F::Frame` where possible and provide explicit
`wire_bytes()` helpers for malformed cases.

## Test observability harness

Introduce `wireframe_testing::observability`, providing an
`ObservabilityHandle` that combines log capture with metrics recording.

Key behaviours:

- Acquisition installs log capture via `LoggerHandle` and a scoped metrics
  recorder using `metrics_util::debugging::DebuggingRecorder`.
- Access is serialized with a global lock so concurrent tests do not interfere.
- Metrics snapshots should consume the captured values (matching
  `DebuggingRecorder` semantics) so `clear()` can be implemented by draining a
  snapshot.
- The handle should restore the previous recorder on drop by swapping the
  active recorder back into a global delegating recorder. This keeps the global
  recorder stable while still providing per-test isolation.
- When the `metrics` feature is disabled, the handle should still capture logs
  and return empty metric snapshots.

Proposed public API:

```rust,no_run
use metrics_util::debugging::Snapshot;
use wireframe_testing::LoggerHandle;

pub struct ObservabilityHandle { /* fields omitted */ }

impl ObservabilityHandle {
    pub fn new() -> Self;
    pub fn logs(&mut self) -> &mut LoggerHandle;
    pub fn snapshot(&self) -> Snapshot;
    pub fn clear(&mut self);
    pub fn counter(&self, name: &str, labels: &[(&str, &str)]) -> u64;
}

pub fn observability() -> ObservabilityHandle;
```

Tests using `ObservabilityHandle` should not run concurrently; the global lock
will serialize access, and the documentation must call this out explicitly.

## Helper macros

Keep `push_expect!` and `recv_expect!` for concise async assertions, with error
messages that include call-site information in debug builds.

## Example usage

```rust,no_run
use wireframe::app::WireframeApp;
use wireframe::codec::LengthDelimitedFrameCodec;
use wireframe_testing::{drive_with_payloads, observability};

#[tokio::test]
async fn round_trips_with_codec() -> std::io::Result<()> {
    let codec = LengthDelimitedFrameCodec::new(1024);
    let app = WireframeApp::new()?.with_codec(codec.clone());
    let frames = drive_with_payloads(app, &codec, vec![b"ping".to_vec()]).await?;
    assert_eq!(
        LengthDelimitedFrameCodec::frame_payload(&frames[0]),
        b"ping"
    );
    Ok(())
}

#[tokio::test]
async fn captures_codec_metrics() -> std::io::Result<()> {
    let mut obs = observability();
    obs.clear();

    let codec = LengthDelimitedFrameCodec::new(16);
    let app = WireframeApp::new()?.with_codec(codec.clone());
    let _ = wireframe_testing::drive_with_raw_bytes(
        app,
        vec![vec![0, 0, 0, 8, 1]],
        None,
    )
    .await?;

    assert_eq!(obs.counter("wireframe.codec.errors", &[]), 1);
    Ok(())
}
```

## Implementation notes

- Update all driver helpers to accept `F: FrameCodec` and operate on `F::Frame`
  rather than raw length-delimited bytes.
- Add codec-aware `encode_frames` and `decode_frames` helpers that return
  `io::Result` on failures instead of panicking.
- Provide a raw byte driver for malformed input and recovery tests.
- Add fixture helpers for default and example codecs, including invalid frames
  used by codec error tests.
- Implement the observability harness and expose it via an `rstest` fixture in
  `wireframe_testing::observability`.
