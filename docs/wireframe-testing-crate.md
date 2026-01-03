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
observability harness, so tests can assert on logs and metrics
deterministically without external exporters.

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

Length-delimited helpers match the current `wireframe_testing` API and accept
raw frame bytes. Use `drive_with_frames` for pre-framed input (including
malformed frames) and `drive_with_payloads` to wrap payloads with the default
length-delimited framing.

```rust,no_run
use std::io;
use wireframe::app::{Packet, WireframeApp};

pub async fn drive_with_frames<S, C, E>(
    app: WireframeApp<S, C, E>,
    frames: Vec<Vec<u8>>,
) -> io::Result<Vec<u8>>
where
    S: TestSerializer,
    C: Send + 'static,
    E: Packet;

pub async fn drive_with_payloads<S, C, E>(
    app: WireframeApp<S, C, E>,
    payloads: Vec<Vec<u8>>,
) -> io::Result<Vec<u8>>
where
    S: TestSerializer,
    C: Send + 'static,
    E: Packet;

pub async fn drive_with_frames_mut<S, C, E>(
    app: &mut WireframeApp<S, C, E>,
    frames: Vec<Vec<u8>>,
) -> io::Result<Vec<u8>>
where
    S: TestSerializer,
    C: Send + 'static,
    E: Packet;

pub async fn drive_with_payloads_mut<S, C, E>(
    app: &mut WireframeApp<S, C, E>,
    payloads: Vec<Vec<u8>>,
) -> io::Result<Vec<u8>>
where
    S: TestSerializer,
    C: Send + 'static,
    E: Packet;
```

Codec-aware helpers should be added as non-breaking extensions so tests can
pass `FrameCodec` values and inspect protocol-specific frame metadata. Prefer
distinct names (for example, `drive_with_codec_frames`) so existing tests that
use raw byte frames continue to compile unchanged.

Behavioural details:

- `drive_with_frames` writes the provided bytes verbatim, making it the
  preferred helper for malformed frame and recovery tests.
- `drive_with_payloads` length-prefixes payload bytes before delegating to
  `drive_with_frames`.
- `drive_with_bincode` encodes a message with bincode and then length-prefixes
  the output before driving the app.
- Mutable variants (`drive_with_frames_mut` and `drive_with_payloads_mut`)
  accept `&mut WireframeApp` so tests can reuse a configured instance.
- I/O failures, framing errors, and server task panics are all returned as
  `io::Error` values, so tests can assert on error handling.

### Buffer capacity and limits

The duplex stream buffer defaults to `TEST_MAX_FRAME`, matching the shared
length-delimited framing guardrail. Use `run_app` or the `*_with_capacity`
helpers to override this value; they reject a `capacity` of zero or above the
maximum ceiling with `io::ErrorKind::InvalidInput`.

Codec-aware helpers should instead read `FrameCodec::max_frame_length()` to
align buffer sizing with protocol framing rules.

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
`bincode::config::standard()` and drives the app:

```rust,no_run
use std::io;
use wireframe::app::{Packet, WireframeApp};

pub async fn drive_with_bincode<M, S, C, E>(
    app: WireframeApp<S, C, E>,
    msg: M,
) -> io::Result<Vec<u8>>
where
    M: bincode::Encode,
    S: TestSerializer,
    C: Send + 'static,
    E: Packet;
```

```rust,no_run
use wireframe_testing::{decode_frames, drive_with_bincode};

#[derive(bincode::Encode)]
struct Ping(u8);

let bytes = drive_with_bincode(app, Ping(1)).await?;
let frames = decode_frames(bytes);
assert_eq!(frames[0], vec![1]);
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
- Access is serialized with a global lock, so concurrent tests do not
  interfere, but the harness will reduce parallelism for the affected suite.
- Observability-heavy suites should run in a single-threaded test runner (for
  example, pass `--test-threads=1` for the affected test binary), or share a
  single `ObservabilityHandle` via a per-suite fixture to amortise setup costs.
- When partial parallelism is needed, group observability assertions into a
  dedicated test binary that runs serially, and keep the remaining test suite
  in the default parallel runner.
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
serializes access, so favour a shared fixture or a dedicated serial test binary
for observability assertions.

## Helper macros

Keep `push_expect!` and `recv_expect!` for concise async assertions, with error
messages that include call-site information in debug builds.

## Example usage

```rust,no_run
use std::sync::Arc;

use wireframe::app::{Envelope, WireframeApp};
use wireframe_testing::{decode_frames, drive_with_bincode, observability};

#[tokio::test]
async fn round_trips_with_codec() -> std::io::Result<()> {
    let app = WireframeApp::new()?
        .route(1, Arc::new(|_: &Envelope| Box::pin(async {})))?;
    let env = Envelope::new(1, Some(5), vec![1, 2, 3]);
    let out = drive_with_bincode(app, env).await?;
    let frames = decode_frames(out);
    assert_eq!(frames.len(), 1);
    Ok(())
}

#[tokio::test]
async fn captures_metrics() -> std::io::Result<()> {
    use wireframe::metrics::{Direction, FRAMES_PROCESSED, inc_frames};

    let mut obs = observability();
    obs.clear();

    inc_frames(Direction::Inbound);
    assert_eq!(
        obs.counter(FRAMES_PROCESSED, &[("direction", "inbound")]),
        1
    );
    Ok(())
}
```

## Implementation notes

- Add codec-aware helpers (for example, `drive_with_codec_frames`) that accept
  `F: FrameCodec` and return `F::Frame` values for tests that need protocol
  metadata.
- Provide codec-aware `encode_frames` and `decode_frames` helpers that return
  `io::Result` on failures instead of panicking.
- Keep the length-delimited helpers so existing tests that use raw frame bytes
  remain compatible.
- Add fixture helpers for default and example codecs, including invalid frames
  used by codec error tests.
- Implement the observability harness and expose it via an `rstest` fixture in
  `wireframe_testing::observability`.
