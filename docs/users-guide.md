# Wireframe library guide

Wireframe is a Rust library for building asynchronous binary protocol servers
with pluggable routing, middleware, and connection utilities.[^1] The guide
below walks through the components that exist today and explains how they work
together when assembling an application.

## Quick start: building an application

A `WireframeApp` collects route handlers and middleware. Each handler is stored
as an `Arc` pointing to an async function that receives a packet reference and
returns `()`. The builder caches these registrations until `handle_connection`
constructs the middleware chain for an accepted stream.[^2]

```no_run
use std::sync::Arc;
use wireframe::app::{Envelope, Handler, WireframeApp};

async fn ping(_env: &Envelope) {}

fn build_app() -> wireframe::Result<WireframeApp> {
    let handler: Handler<Envelope> = Arc::new(|env: &Envelope| {
        let _ = env; // inspect payload here
        Box::pin(ping(env))
    });

    WireframeApp::new()?
        .route(1, handler)?
        .wrap(wireframe::middleware::from_fn(|req, next| async move {
            let mut response = next.call(req).await?;
            response.frame_mut().extend_from_slice(b" pong");
            Ok(response)
        }))
}
```

The snippet below wires the builder into a Tokio runtime, decodes inbound
payloads, and emits a serialised response. It showcases the typical `main`
function for a microservice that listens on localhost and responds to a `Ping`
message with a `Pong` payload.[^2][^10][^15]

```rust,no_run
use std::{net::SocketAddr, sync::Arc};

use wireframe::{
    app::{Envelope, Handler, WireframeApp},
    middleware,
    message::Message,
    server::{ServerError, WireframeServer},
};

#[derive(bincode::Encode, bincode::BorrowDecode, Debug)]
struct Ping {
    body: String,
}

#[derive(bincode::Encode, bincode::BorrowDecode, Debug, PartialEq)]
struct Pong {
    body: String,
}

async fn ping(env: &Envelope) {
    log::info!("received correlation id: {:?}", env.clone().into_parts().correlation_id());
}

fn build_app() -> wireframe::app::Result<WireframeApp> {
    let handler: Handler<Envelope> = Arc::new(|env: &Envelope| Box::pin(ping(env)));

    WireframeApp::new()?
        .route(1, handler)?
        .wrap(middleware::from_fn(|req, next| async move {
            let ping = Ping::from_bytes(req.frame()).map(|(msg, _)| msg).ok();
            let mut response = next.call(req).await?;

            if let Some(ping) = ping {
                let payload = Pong {
                    body: format!("pong {}", ping.body),
                }
                .to_bytes()
                .expect("encode Pong message");
                response.frame_mut().clear();
                response.frame_mut().extend_from_slice(&payload);
            }

            Ok(response)
        }))
}

fn app_factory() -> WireframeApp {
    build_app().expect("configure Wireframe application")
}

#[tokio::main]
async fn main() -> Result<(), ServerError> {
    let addr: SocketAddr = "127.0.0.1:4000".parse().expect("valid socket address");
    let server = WireframeServer::new(app_factory).bind(addr)?;
    server.run().await
}
```

Route identifiers must be unique; the builder returns
`WireframeError::DuplicateRoute` when a handler is registered twice, keeping
the dispatch table unambiguous.[^2][^5] New applications default to the bundled
bincode serializer, a length-delimited codec capped at 1024 bytes per frame,
and a 100 ms read timeout. Clamp the length-delimited limit with
`buffer_capacity` (length-delimited only), swap codecs with `with_codec`, and
override the serializer with `with_serializer` when a different encoding
strategy is required.[^3][^4] Custom protocols implement `FrameCodec` to
describe their framing rules.

Once a stream is accepted—either from a manual accept loop or via
`WireframeServer`—`handle_connection(stream)` builds (or reuses) the middleware
chain, wraps the transport in the configured frame codec (length-delimited by
default), enforces per-frame read timeouts, and writes responses. Serialization
helpers `send_response` and `send_response_framed` (or
`send_response_framed_with_codec` for custom codecs) return typed `SendError`
variants when encoding or I/O fails, and the connection closes after ten
consecutive deserialization errors.[^6][^7]

### Custom frame codecs

Custom protocols supply a `FrameCodec` implementation to describe their framing
rules. The codec owns the Tokio `Decoder` and `Encoder` types, while Wireframe
uses the trait surface to map frames to payload bytes and correlation data.

A codec implementation must:

- Define a `Frame` type and paired decoder/encoder implementations that return
  `std::io::Error` on failure.
- Return only the logical payload bytes from `frame_payload` so metadata parsing
  and deserialisation run against the right buffer.
- Wrap outbound payloads with `wrap_payload(&self, Bytes)`, adding any protocol
  headers or metadata required by the wire format.
- Provide `correlation_id` when the protocol stores it outside the payload;
  Wireframe only uses this hook when the deserialized envelope is missing a
  correlation identifier.
- Report `max_frame_length`, which clamps inbound frames and seeds default
  fragmentation limits.

Install a custom codec with `with_codec`. The builder resets fragmentation to
the codec-derived defaults, so override fragmentation afterwards if the
protocol uses a different budget. Wireframe clones the codec per connection, so
stateful codecs should ensure `Clone` produces an independent state (for
example, reset sequence counters) when per-connection isolation is required.
When a framed stream is already available, use
`send_response_framed_with_codec`, so responses pass through
`FrameCodec::wrap_payload`.

Assume `MyCodec` implements `FrameCodec`:

```rust,no_run
use std::sync::Arc;

use wireframe::app::{Envelope, Handler, WireframeApp};

struct MyCodec;

let handler: Handler<Envelope> = Arc::new(|_: &Envelope| Box::pin(async {}));

let app = WireframeApp::new()?
    .with_codec(MyCodec)
    .route(1, handler)?;
```

See `examples/hotline_codec.rs` and `examples/mysql_codec.rs` for complete
implementations.

#### Zero-copy payload extraction

For performance-critical codecs, use `Bytes` instead of `Vec<u8>` for payload
storage and override `frame_payload_bytes` to avoid allocation:

```rust
use bytes::Bytes;
use wireframe::codec::FrameCodec;

pub struct MyFrame {
    pub metadata: u32,
    pub payload: Bytes,  // Use Bytes, not Vec<u8>
}

impl FrameCodec for MyCodec {
    type Frame = MyFrame;
    // ... other associated types ...

    fn frame_payload(frame: &Self::Frame) -> &[u8] {
        &frame.payload
    }

    fn frame_payload_bytes(frame: &Self::Frame) -> Bytes {
        frame.payload.clone()  // Cheap: atomic reference count increment
    }

    fn wrap_payload(&self, payload: Bytes) -> Self::Frame {
        MyFrame {
            metadata: 0,
            payload,  // Store directly, no copy
        }
    }

    // ... other methods ...
}
```

In the decoder, use `BytesMut::freeze()` instead of `.to_vec()`:

```rust
use bytes::BytesMut;
use tokio_util::codec::Decoder;

fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
    // ... parse header ...
    let payload = src.split_to(payload_len).freeze();  // Zero-copy
    Ok(Some(MyFrame { metadata, payload }))
}
```

The default `frame_payload_bytes` implementation copies from the
`frame_payload()` slice, ensuring backward compatibility for codecs that use
`Vec<u8>` payloads.

### Codec error handling

The codec layer provides a structured error taxonomy via `CodecError` that
enables recovery policies, structured logging, and proper EOF handling.

**Error categories:**

- `FramingError` - Wire-level frame boundary issues (oversized frames, invalid
  length encoding, incomplete headers, checksum mismatches, empty frames).
- `ProtocolError` - Semantic violations after frame extraction (unknown message
  types, invalid versions).
- `io::Error` - Transport layer failures (connection resets, timeouts).
- `EofError` - End-of-stream conditions with clean/mid-frame/mid-header
  variants.

**Recovery policies:**

Each error type has a default recovery policy:

| Error Type                     | Default Policy |
| ------------------------------ | -------------- |
| `FramingError::OversizedFrame` | Drop           |
| `FramingError::EmptyFrame`     | Drop           |
| Other `FramingError`           | Disconnect     |
| All `ProtocolError`            | Drop           |
| All `io::Error`                | Disconnect     |
| `EofError::CleanClose`         | Disconnect     |
| Other `EofError`               | Disconnect     |

Override recovery policies with a custom `RecoveryPolicyHook`:

```rust
use wireframe::codec::{
    CodecError, CodecErrorContext, RecoveryPolicy, RecoveryPolicyHook,
};

struct StrictRecovery;

impl RecoveryPolicyHook for StrictRecovery {
    fn recovery_policy(&self, _error: &CodecError, _ctx: &CodecErrorContext) -> RecoveryPolicy {
        // Disconnect on any codec error
        RecoveryPolicy::Disconnect
    }
}
```

`CodecErrorContext` provides connection metadata for policy decisions:

```rust
use wireframe::codec::CodecErrorContext;

let ctx = CodecErrorContext::new()
    .with_connection_id(42)
    .with_correlation_id(123)
    .with_codec_state("seq=5");
```

**Protocol hooks for EOF:**

The `WireframeProtocol` trait includes an `on_eof` hook for handling EOF
conditions during frame decoding:

```rust,ignore
use wireframe::{ConnectionContext, EofError, WireframeProtocol};

impl WireframeProtocol for MyProtocol {
    type Frame = Vec<u8>;
    type ProtocolError = String;

    fn on_eof(&self, error: &EofError, partial_data: &[u8], _ctx: &mut ConnectionContext) {
        match error {
            EofError::CleanClose => tracing::info!("connection closed cleanly"),
            EofError::MidFrame { bytes_received, expected } => {
                tracing::warn!(
                    received = bytes_received,
                    expected = expected,
                    partial_len = partial_data.len(),
                    "connection closed mid-frame"
                );
            }
            EofError::MidHeader { bytes_received, header_size } => {
                tracing::warn!(
                    received = bytes_received,
                    header_size = header_size,
                    "connection closed mid-header"
                );
            }
        }
    }
}
```

**Metrics:**

When the `metrics` feature is enabled, codec errors increment the
`wireframe_codec_errors_total` counter with `error_type` and `recovery_policy`
labels:

```text
wireframe_codec_errors_total{error_type="framing",recovery_policy="drop"} 5
wireframe_codec_errors_total{error_type="eof",recovery_policy="disconnect"} 2
```

## Packets, payloads, and serialization

Packets drive routing. Implement the `Packet` trait (or use the bundled
`Envelope`) to expose a message identifier, optional correlation id, and raw
payload. `PacketParts` rebuilds packets after middleware has finished editing
frames, and `inherit_correlation` patches mismatched response identifiers while
logging the discrepancy.[^8] Serializers plug into the `Serializer` trait;
`WireframeApp` defaults to `BincodeSerializer` but can run any implementation
that meets the trait bounds.[^4]

When `FrameMetadata::parse` succeeds, the framework extracts identifiers from
metadata without deserializing the payload. If parsing fails, it falls back to
full deserialization.[^9][^6] Application messages implement the `Message`
trait, gaining `to_bytes` and `from_bytes` helpers that use bincode with the
standard configuration.[^10]

### Fragmentation metadata

`wireframe::fragment` exposes small, copy-friendly structs to describe the
transport-level fragmentation state.[^41]

- `MessageId` uniquely identifies the logical message a fragment belongs to.
- `FragmentIndex` records the fragment's zero-based order.
- `FragmentHeader` bundles the identifier, index, and a boolean that signals
  whether the fragment is the last one in the sequence.
- `FragmentSeries` tracks the next expected fragment index and reports
  mismatches via structured errors.

Protocol implementers can emit a `FragmentHeader` for every physical frame and
feed the header back into `FragmentSeries` to guarantee ordering before a fully
reassembled message is surfaced to handlers. Behavioural tests can reuse the
same types to assert that new codecs obey the transport invariants without
spinning up a full server.[^42][^43]

The standalone `Fragmenter` helper now slices oversized payloads into capped
fragments while stamping the shared `MessageId` and sequential `FragmentIndex`.
Each call returns a `FragmentBatch` that reports whether the message required
fragmentation and yields individual `FragmentFrame` values for serialization or
logging. This keeps transport experiments lightweight while the full adaptor
layer evolves. The helper is fallible—`FragmentationError` surfaces encoding
failures or index overflows—so production code should bubble the error up or
log it rather than unwrapping.

```no_run
use std::num::NonZeroUsize;
use wireframe::fragment::Fragmenter;

let fragmenter = Fragmenter::new(NonZeroUsize::new(512).unwrap());
let payload = [0_u8; 1400];
let batch = fragmenter.fragment_bytes(&payload).expect("fragment");
assert_eq!(batch.len(), 3);

for fragment in batch.fragments() {
    tracing::info!(
        msg_id = fragment.header().message_id().get(),
        index = fragment.header().fragment_index().get(),
        final = fragment.header().is_last_fragment(),
        payload_len = fragment.payload().len(),
    );
}
```

A companion `Reassembler` mirrors the helper on the inbound path. It buffers
fragments per `MessageId`, rejects out-of-order fragments, and enforces a
maximum assembled size while exposing `purge_expired` to clear stale partial
messages after a configurable timeout. When the final fragment arrives, it
returns a `ReassembledMessage` that can be decoded into the original type.

```rust
use std::{num::NonZeroUsize, time::Duration};
use wireframe::fragment::{
    FragmentHeader,
    FragmentIndex,
    MessageId,
    Reassembler,
};

let mut reassembler =
    Reassembler::new(NonZeroUsize::new(512).expect("non-zero capacity"), Duration::from_secs(30));

let header = FragmentHeader::new(MessageId::new(9), FragmentIndex::zero(), true);
let complete = reassembler
    .push(header, [0_u8; 12])
    .expect("fragments accepted")
    .expect("single fragment completes the message");

// Decode when ready:
// let message: MyType = complete.decode().expect("decode");
```

### Request parts and streaming bodies

`wireframe::request` exposes `RequestParts` to separate routing metadata from
streaming request payloads.[^45]

- `RequestParts::id()` returns the message identifier for routing.
- `RequestParts::correlation_id()` returns the optional correlation identifier.
- `RequestParts::metadata()` returns protocol-defined header bytes.

This type pairs with `RequestBodyStream` for incremental consumption of large
request payloads. Handlers can choose between buffered (existing) and streaming
consumption; the streaming path is opt-in.

```rust
use wireframe::request::RequestParts;

let parts = RequestParts::new(42, Some(123), vec![0x01, 0x02]);
assert_eq!(parts.id(), 42);
assert_eq!(parts.correlation_id(), Some(123));
assert_eq!(parts.metadata(), &[0x01, 0x02]);
```

Unlike `PacketParts` (which carries the raw payload for envelope
reconstruction), `RequestParts` carries only protocol-defined metadata required
to interpret the streaming body. The body itself is consumed through a separate
stream, enabling back-pressure and incremental processing.[^46]

### Message assembler hook

Wireframe exposes a protocol-facing `MessageAssembler` hook that parses
per-frame headers into `FrameHeader::First` and `FrameHeader::Continuation`
values. It returns a `ParsedFrameHeader` that includes the header length so the
remaining bytes can be treated as the body chunk.

Register an assembler with `WireframeApp::with_message_assembler`:

```rust,no_run
use wireframe::{
    app::WireframeApp,
    message_assembler::{
        FrameHeader,
        FirstFrameHeader,
        MessageAssembler,
        MessageKey,
        ParsedFrameHeader,
    },
};

struct DemoAssembler;

impl MessageAssembler for DemoAssembler {
    fn parse_frame_header(
        &self,
        _payload: &[u8],
    ) -> Result<ParsedFrameHeader, std::io::Error> {
        Ok(ParsedFrameHeader::new(
            FrameHeader::First(FirstFrameHeader {
                message_key: MessageKey(1),
                metadata_len: 0,
                body_len: 0,
                total_body_len: None,
                is_last: true,
            }),
            0,
        ))
    }
}

let _app = WireframeApp::new()
    .expect("builder")
    .with_message_assembler(DemoAssembler);
```

When configured, this hook now runs on the inbound connection path after
transport fragmentation reassembly and before handler dispatch. Incomplete
assemblies remain buffered per message key until completion or timeout eviction.

Message-assembly parsing and continuity failures are treated as inbound
deserialization failures and follow the existing failure threshold policy.

`WireframeApp::message_assembler` returns the configured hook as an
`Option<&Arc<dyn MessageAssembler>>` if direct access is required.

#### Message key multiplexing (8.2.3)

The `MessageAssemblyState` type manages multiple concurrent message assemblies
keyed by `MessageKey`. This enables interleaved frame streams where frames from
different logical messages arrive on the same connection:

```rust
use std::{num::NonZeroUsize, time::Duration};
use wireframe::message_assembler::{
    ContinuationFrameHeader,
    EnvelopeRouting,
    FirstFrameHeader,
    FirstFrameInput,
    FrameSequence,
    MessageAssemblyState,
    MessageKey,
};

let mut state = MessageAssemblyState::new(
    NonZeroUsize::new(1_048_576).expect("non-zero size"), // 1 mebibyte (MiB) max message
    Duration::from_secs(30),                               // 30s timeout for partial assemblies
);

// First frame for message key=1
let first1 = FirstFrameHeader {
    message_key: MessageKey(1),
    metadata_len: 0,
    body_len: 5,
    total_body_len: Some(15),
    is_last: false,
};
let routing1 = EnvelopeRouting { envelope_id: 1, correlation_id: None };
let input1 = FirstFrameInput::new(&first1, routing1, vec![], b"hello")
    .expect("header lengths match");
state.accept_first_frame(input1)?;

// First frame for message key=2 (interleaved)
let first2 = FirstFrameHeader {
    message_key: MessageKey(2),
    metadata_len: 0,
    body_len: 5,
    total_body_len: None,
    is_last: false,
};
let routing2 = EnvelopeRouting { envelope_id: 2, correlation_id: None };
let input2 = FirstFrameInput::new(&first2, routing2, vec![], b"world")
    .expect("header lengths match");
state.accept_first_frame(input2)?;

// Continuation for key=1 completes its message
let cont1 = ContinuationFrameHeader {
    message_key: MessageKey(1),
    sequence: Some(FrameSequence(1)),
    body_len: 10,
    is_last: true,
};
let msg1 = state.accept_continuation_frame(&cont1, b" completed")?
    .expect("message 1 should complete");
assert_eq!(msg1.body(), b"hello completed");
```

#### Continuity validation (8.2.4)

The `MessageSeries` type validates frame ordering when protocols supply
sequence numbers via `ContinuationFrameHeader::sequence`. It detects:

- **Out-of-order frames**: sequence gaps produce
  `MessageSeriesError::SequenceMismatch`
- **Duplicate frames**: already-processed sequences produce
  `MessageSeriesError::DuplicateFrame`
- **Frames after completion**: produce `MessageSeriesError::SeriesComplete`

For protocols that do not supply sequence numbers, the series accepts frames in
any order (ordering validation is skipped).

```rust
use wireframe::message_assembler::{
    ContinuationFrameHeader,
    FirstFrameHeader,
    FrameSequence,
    MessageKey,
    MessageSeries,
    MessageSeriesError,
    MessageSeriesStatus,
};

let first = FirstFrameHeader {
    message_key: MessageKey(1),
    metadata_len: 0,
    body_len: 10,
    total_body_len: None,
    is_last: false,
};
let mut series = MessageSeries::from_first_frame(&first);

// Accept continuation with sequence 1
let cont1 = ContinuationFrameHeader {
    message_key: MessageKey(1),
    sequence: Some(FrameSequence(1)),
    body_len: 5,
    is_last: false,
};
assert_eq!(series.accept_continuation(&cont1), Ok(MessageSeriesStatus::Incomplete));

// Attempting sequence 3 (gap) fails
let cont3 = ContinuationFrameHeader {
    message_key: MessageKey(1),
    sequence: Some(FrameSequence(3)), // Expected 2
    body_len: 5,
    is_last: false,
};
assert!(matches!(
    series.accept_continuation(&cont3),
    Err(MessageSeriesError::SequenceMismatch { .. })
));

// Accept sequence 2, then try sequence 1 again (duplicate)
let cont2 = ContinuationFrameHeader {
    message_key: MessageKey(1),
    sequence: Some(FrameSequence(2)),
    body_len: 5,
    is_last: false,
};
assert_eq!(series.accept_continuation(&cont2), Ok(MessageSeriesStatus::Incomplete));

let dup = ContinuationFrameHeader {
    message_key: MessageKey(1),
    sequence: Some(FrameSequence(1)), // Already seen
    body_len: 5,
    is_last: false,
};
assert!(matches!(
    series.accept_continuation(&dup),
    Err(MessageSeriesError::DuplicateFrame { .. })
));

// Complete the series, then try to add more (SeriesComplete)
let final_cont = ContinuationFrameHeader {
    message_key: MessageKey(1),
    sequence: Some(FrameSequence(3)),
    body_len: 5,
    is_last: true,
};
assert_eq!(series.accept_continuation(&final_cont), Ok(MessageSeriesStatus::Complete));

let extra = ContinuationFrameHeader {
    message_key: MessageKey(1),
    sequence: Some(FrameSequence(4)),
    body_len: 5,
    is_last: false,
};
assert!(matches!(
    series.accept_continuation(&extra),
    Err(MessageSeriesError::SeriesComplete)
));
```

### Streaming request body consumption

Handlers can opt into streaming request bodies using the `StreamingBody`
extractor or by accepting a `RequestBodyStream` directly. The framework creates
a bounded channel and forwards body chunks as they arrive; back-pressure
propagates automatically when the handler consumes slower than the network
delivers.

```rust,no_run
use tokio::io::AsyncReadExt;
use wireframe::request::{RequestBodyReader, RequestBodyStream, RequestParts};

async fn handle_upload(parts: RequestParts, body: RequestBodyStream) {
    let mut reader = RequestBodyReader::new(body);
    let mut buf = Vec::new();
    reader.read_to_end(&mut buf).await.expect("read body");

    log::info!(
        "received {} bytes for request {}",
        buf.len(),
        parts.id()
    );
}
```

The `RequestBodyReader` adaptor implements `AsyncRead`, allowing protocol
crates to reuse existing parsers. For raw stream access, use the
`RequestBodyStream` directly with `StreamExt` methods:

```rust,no_run
use bytes::Bytes;
use futures::StreamExt;
use wireframe::request::{RequestBodyStream, RequestParts};

async fn handle_stream(parts: RequestParts, mut body: RequestBodyStream) {
    while let Some(result) = body.next().await {
        match result {
            Ok(chunk) => log::debug!("received {} bytes", chunk.len()),
            Err(e) => log::error!("stream error: {e}"),
        }
    }
}
```

The `StreamingBody` extractor wraps the stream with convenience methods:

```rust,no_run
use wireframe::{
    extractor::StreamingBody,
    request::RequestParts,
};

async fn with_extractor(parts: RequestParts, body: StreamingBody) {
    // Convert to AsyncRead
    let reader = body.into_reader();

    // Or access the raw stream
    // let stream = body.into_stream();
}
```

Back-pressure is enforced via bounded channels: when the internal buffer fills,
the framework pauses reading from the socket until the handler drains pending
chunks. This prevents memory exhaustion under slow consumer conditions. The
`body_channel` helper creates channels with configurable capacity:

```rust
use wireframe::request::body_channel;

// Create a channel with capacity for 8 chunks
let (tx, rx) = body_channel(8);
// tx: connection sends chunks
// rx: handler consumes via RequestBodyStream
```

See [ADR 0002][adr-0002-ref] for the complete design rationale.

## Working with requests and middleware

Every inbound frame becomes a `ServiceRequest`. Middleware can inspect or
mutate the byte buffer through `frame` and `frame_mut`, adjust correlation
identifiers, or wrap the remainder of the pipeline using `Transform`. The
`Next` continuation calls the inner service, returning a `ServiceResponse`
containing the frame that will be serialized back to the client.[^11]

The `middleware::from_fn` helper adapts async functions into middleware
components, enabling payload decoding, invocation of the inner service, and
response editing before the response is serialized. `HandlerService` rebuilds a
packet from the request bytes, invokes the registered handler, and by default
returns the original payload; replies are crafted in middleware (or custom
packet types with interior mutability). Decode typed payloads via the `Message`
helpers, then write the encoded response into
`ServiceResponse::frame_mut()`.[^10][^12]

```rust
use std::convert::Infallible;

use wireframe::{
    app::Envelope,
    message::Message,
    middleware::{from_fn, HandlerService, Next, ServiceRequest, ServiceResponse},
};

#[derive(bincode::Encode, bincode::BorrowDecode)]
struct Ping(u8);

#[derive(bincode::Encode, bincode::BorrowDecode)]
struct Pong(u8);

async fn decode_and_respond(
    mut req: ServiceRequest,
    next: Next<'_, HandlerService<Envelope>>,
) -> Result<ServiceResponse, Infallible> {
    let ping = Ping::from_bytes(req.frame()).map(|(msg, _)| msg).ok();
    let mut response = next.call(req).await?;

    if let Some(Ping(value)) = ping {
        let bytes = Pong(value + 1)
            .to_bytes()
            .expect("encode Pong");
        response.frame_mut().clear();
        response.frame_mut().extend_from_slice(&bytes);
    }

    Ok(response)
}

let middleware = from_fn(decode_and_respond);
```

Advanced integrations can adopt the `wireframe::extractor` module, which
defines `MessageRequest`, `Payload`, and `FromMessageRequest` for building
Actix-style extractors in custom middleware or services. These types expose
shared state, peer addresses, and payload cursors for frameworks that want to
layer additional ergonomics on top of the core primitives.[^13]

## Connection lifecycle

`WireframeApp` supports optional setup and teardown callbacks that run once per
connection. Setup can return arbitrary state retained until teardown executes
after the stream finishes processing.[^2] During `handle_connection` the
framework caches middleware chains, enforces read timeouts, and records metrics
for inbound frames, serialization failures, and handler errors before logging
warnings.[^6][^7] `PacketParts::inherit_correlation` ensures response packets
carry the correct correlation identifier even when middleware omits it.[^8]

Immediate responses are available through `send_response` and
`send_response_framed`, both of which report serialization or I/O problems via
`SendError`.[^6][^5]

```rust,no_run
use tokio::io::{self, AsyncReadExt, AsyncWriteExt};

use wireframe::{
    app::{SendError, WireframeApp},
    message::Message,
};

#[derive(bincode::Encode, bincode::BorrowDecode, Debug, PartialEq)]
struct Ready(&'static str);

#[tokio::main]
async fn main() -> Result<(), SendError> {
    let app = WireframeApp::default();
    let (mut client, mut server) = io::duplex(64);

    let ready = Ready("ready");
    app.send_response(&mut server, &ready).await?;
    server.shutdown().await?;

    let mut buffer = Vec::new();
    client.read_to_end(&mut buffer).await?;
    let (decoded, _) = Ready::from_bytes(&buffer).expect("decode Ready frame");
    assert_eq!(decoded, ready);

    Ok(())
}
```

## Message fragmentation

`WireframeApp` now fragments oversized payloads automatically. The builder
derives a `FragmentationConfig` from the active frame codec's maximum frame
length (the default length-delimited codec uses `buffer_capacity`): any payload
that will not fit into a single frame is split into fragments carrying a
`FragmentHeader` (`message_id`, `fragment_index`, `is_last_fragment`) wrapped
with the `FRAG` marker. The connection reassembles fragments before invoking
handlers, so handlers continue to work with complete `Envelope` values.[^6]

Fragmented messages enforce two guards: `max_message_size` caps the total
reassembled payload, and `reassembly_timeout` evicts stale partial messages.
Customize or disable fragmentation via the builder:

```rust
use std::{num::NonZeroUsize, time::Duration};
use wireframe::{
    app::WireframeApp,
    fragment::FragmentationConfig,
};

// Assume `handler` is defined elsewhere; any Handler compatible with WireframeApp works.
let cfg = FragmentationConfig::for_frame_budget(
    1024,
    NonZeroUsize::new(16 * 1024).unwrap(),
    Duration::from_secs(30),
).expect("frame budget too small for fragments");

let app = WireframeApp::new()?
    .fragmentation(Some(cfg))
    .route(42, handler)?;
```

Set `fragmentation(None)` when the transport already supports large frames, or
when fragmentation should be deferred to an upstream gateway. The
`ConnectionActor` mirrors the same behaviour for push traffic and streaming
responses through `enable_fragmentation`, ensuring client-visible frames follow
the same format.

## Protocol hooks

Install a custom protocol with `with_protocol`. `protocol_hooks()` converts the
stored implementation into `ProtocolHooks`, which the connection actor consumes
when draining responses and push queues.[^2][^7] `WireframeProtocol` exposes
callbacks for connection setup, per-frame mutation, command completion,
protocol errors, and optional end-of-stream frames. The connection actor
invokes these hooks around every outbound frame and when response streams end
or emit errors.[^14]

## Running servers

`WireframeServer::new` clones the application factory per worker, defaults the
worker count to the host CPU total (never below one), supports a readiness
signal, and normalizes accept-loop backoff settings through
`accept_backoff`.[^15][^16] Servers start in an unbound state; call `bind` or
`bind_existing_listener` to transition into the `Bound` typestate, inspect the
bound address, or rebind later.[^17]

`run` awaits Ctrl+C, while `run_with_shutdown` cancels all worker tasks when
the supplied future resolves.[^18] Each worker runs `accept_loop`, which clones
the factory, rewinds leftover preamble bytes, and hands the stream to the
application. Transient accept failures trigger exponential backoff capped by
the configured maximum delay.[^18][^19] Preamble hooks support asynchronous
success handlers and asynchronous failure callbacks that receive the stream,
enabling replies or decode-error logging before the application runs. An
optional `preamble_timeout` caps how long `read_preamble` waits; timeouts use
the failure callback path.[^20]

`spawn_connection_task` wraps each accepted stream in `read_preamble` and
`RewindStream`, records connection panics, and logs failures without crashing
worker tasks.[^20][^37][^38] `ServerError` surfaces bind and accept failures as
typed errors so callers can react appropriately.[^21]

## Client runtime

`WireframeClient` provides a first-class client runtime that mirrors the
server's framing and serialization layers, with a builder that configures the
serializer, codec settings, and socket options before connecting.[^44] Use
`ClientCodecConfig` to align `max_frame_length` with the server's
`buffer_capacity`, and apply `SocketOptions` when TCP tuning is required, such
as `TCP_NODELAY` or buffer size adjustments.

```rust
use std::{net::SocketAddr, time::Duration};

use wireframe::{
    client::{ClientCodecConfig, SocketOptions},
    WireframeClient,
};

#[derive(bincode::Encode, bincode::BorrowDecode)]
struct Login {
    username: String,
}

#[derive(bincode::Encode, bincode::BorrowDecode, Debug, PartialEq)]
struct LoginAck {
    ok: bool,
}

let addr: SocketAddr = "127.0.0.1:7878".parse().expect("valid socket address");
let codec = ClientCodecConfig::default().max_frame_length(2048);
let socket = SocketOptions::default()
    .nodelay(true)
    .keepalive(Some(Duration::from_secs(30)));

let mut client = WireframeClient::builder()
    .codec_config(codec)
    .socket_options(socket)
    .connect(addr)
    .await?;

let login = Login {
    username: "guest".to_string(),
};
let ack: LoginAck = client.call(&login).await?;
assert!(ack.ok);
```

### Client preamble exchange

The client builder supports an optional preamble exchange before framing
begins. Use `with_preamble` to send a preamble immediately after TCP connect,
and register callbacks for success or failure scenarios.[^47]

```rust
use std::{net::SocketAddr, time::Duration};

use futures::FutureExt;
use wireframe::{
    preamble::read_preamble,
    WireframeClient,
};

#[derive(bincode::Encode, bincode::BorrowDecode)]
struct ClientHello {
    version: u16,
}

#[derive(bincode::BorrowDecode)]
struct ServerAck {
    accepted: bool,
}

let addr: SocketAddr = "127.0.0.1:7878".parse().expect("valid socket address");

let client = WireframeClient::builder()
    .with_preamble(ClientHello { version: 1 })
    .preamble_timeout(Duration::from_secs(5))
    .on_preamble_success(|_preamble, stream| {
        async move {
            // Read server acknowledgement
            let (ack, leftover) = read_preamble::<_, ServerAck>(stream)
                .await
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
            if !ack.accepted {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::ConnectionRefused,
                    "server rejected preamble",
                ));
            }
            Ok(leftover) // Return any leftover bytes for framing layer
        }
        .boxed()
    })
    .on_preamble_failure(|err, _stream| {
        async move {
            eprintln!("Preamble exchange failed: {err}");
            Ok(())
        }
        .boxed()
    })
    .connect(addr)
    .await?;
```

The success callback receives the sent preamble and a mutable reference to the
TCP stream, enabling bidirectional preamble negotiation. Any bytes read beyond
the server's response must be returned as "leftover" bytes so they can be
replayed before the framing layer begins. The failure callback runs when the
preamble exchange fails (timeout, I/O error, or encode error) and can log
diagnostics or send an error response before the connection closes.

### Client lifecycle hooks

The client builder supports lifecycle hooks that mirror the server's hook
system, enabling consistent instrumentation across both client and server.[^48]

- **Setup hook** (`on_connection_setup`): Invoked once after the TCP connection
  is established and preamble exchange (if configured) succeeds. Returns
  connection-specific state stored for the connection's lifetime.
- **Teardown hook** (`on_connection_teardown`): Invoked when `close()` is
  called. Receives the state produced by the setup hook for cleanup.
- **Error hook** (`on_error`): Invoked when errors occur during send, receive,
  or call operations. Receives a reference to the error for logging or metrics.

```rust
use std::{net::SocketAddr, sync::Arc};
use std::sync::atomic::{AtomicUsize, Ordering};

use wireframe::WireframeClient;

struct SessionState {
    request_count: AtomicUsize,
}

let addr: SocketAddr = "127.0.0.1:7878".parse().expect("valid socket address");

let client = WireframeClient::builder()
    .on_connection_setup(|| async {
        SessionState {
            request_count: AtomicUsize::new(0),
        }
    })
    .on_connection_teardown(|state: SessionState| async move {
        println!(
            "Session ended after {} requests",
            state.request_count.load(Ordering::SeqCst)
        );
    })
    .on_error(|err| async move {
        eprintln!("Client error: {err}");
    })
    .connect(addr)
    .await?;

// Use the client...

client.close().await; // Teardown hook invoked here
```

The setup hook runs after connection establishment (and preamble exchange if
configured). The teardown hook only runs if a setup hook was configured and
successfully produced state. The error hook is independent and can be
configured without a setup hook.

### Client message API with correlation identifiers

The client provides envelope-aware messaging APIs that work with the `Packet`
trait, supporting automatic correlation ID generation and validation. These
methods complement the basic `send`, `receive`, and `call` methods that operate
on raw `Message` types.

**Correlation ID generation**: Each client maintains an atomic counter for
generating unique correlation IDs. The `next_correlation_id` method returns the
next ID, which is useful when managing correlation manually.

**Envelope-aware methods**:

- `send_envelope<P: Packet>(envelope: P)` — Sends an envelope, auto-generating
  a correlation ID if not present. Returns the correlation ID used.
- `receive_envelope<P: Packet>()` — Receives and deserializes the next frame as
  the specified packet type.
- `call_correlated<P: Packet>(request: P)` — Sends a request with auto-
  generated correlation ID, receives the response, and validates that the
  response's correlation ID matches the request. Returns
  `ClientError::CorrelationMismatch` if the IDs differ.

```rust,no_run
use std::net::SocketAddr;

use wireframe::{
    app::{Envelope, Packet},
    WireframeClient,
};

# async fn example() -> Result<(), wireframe::ClientError> {
let addr: SocketAddr = "127.0.0.1:7878".parse().expect("valid socket address");
let mut client = WireframeClient::builder()
    .connect(addr)
    .await?;

// Create an envelope without a correlation ID.
let request = Envelope::new(1, None, vec![1, 2, 3]);

// call_correlated auto-generates a correlation ID, sends the request,
// receives the response, and validates the correlation ID matches.
let response: Envelope = client.call_correlated(request).await?;

// The response's correlation ID matches the auto-generated request ID.
assert!(response.correlation_id().is_some());
# Ok(())
# }
```

For more control over correlation, use `send_envelope` and `receive_envelope`
separately:

```rust,no_run
use wireframe::app::{Envelope, Packet};
# use wireframe::WireframeClient;
# async fn example(client: &mut WireframeClient) -> Result<(), wireframe::ClientError> {

// Auto-generate correlation ID when sending.
let envelope = Envelope::new(1, None, b"payload".to_vec());
let correlation_id = client.send_envelope(envelope).await?;

// Receive the response.
let response: Envelope = client.receive_envelope().await?;

// Manually verify correlation if needed.
assert_eq!(response.correlation_id(), Some(correlation_id));
# Ok(())
# }
```

The `CorrelationMismatch` error provides diagnostic information when validation
fails:

```rust,ignore
use wireframe::client::ClientError;

match client.call_correlated(request).await {
    Ok(response) => {
        // Handle successful response.
    }
    Err(ClientError::CorrelationMismatch { expected, received }) => {
        eprintln!(
            "Correlation mismatch: expected {:?}, received {:?}",
            expected, received
        );
    }
    Err(e) => {
        // Handle other errors.
    }
}
```

## Push queues and connection actors

Background work interacts with connections through `PushQueues`. The fluent
builder configures high- and low-priority capacities, optional rate limits, and
an optional dead-letter queue with tunable logging cadence for dropped
frames.[^23] Queue construction validates capacities and rate limits, clamping
rates to the supported range.[^24] `PushHandle` exposes async
`push_high_priority` and `push_low_priority` helpers that honour the rate
limiter before awaiting channel capacity, while `try_push` implements
policy-controlled drops with optional warnings and dead-letter forwarding.[^26]
Cloneable handles downgrade to `Weak` references for registration in a session
registry.[^25]

`PushQueues::recv` prefers high-priority frames but eventually drains the
low-priority queue; `close` lets tests release resources when no actor is
running.[^27] The connection actor consumes these queues, an optional streaming
response, and a cancellation token.

`FairnessConfig` and `FairnessTracker` limit consecutive high-priority frames
and optionally enforce a time slice. Resetting the tracker after low-priority
work keeps fairness predictable.[^28][^29] `ConnectionActor::run` polls the
shutdown token, push queues, and response stream using a biased `select!` loop,
invokes protocol hooks, records metrics for outbound frames, and returns a
`WireframeError` when the response stream hits an I/O problem.[^30][^33] Active
connection counts are tracked with a guard that increments on creation and
decrements on drop; the `active_connection_count()` helper exposes the current
gauge.[^31]

## Session management

`ConnectionId` wraps a `u64` identifier with helpers for construction,
formatting, and retrieval. `SessionRegistry` stores weak references to
`PushHandle` instances keyed by `ConnectionId`, automatically pruning dead
entries during lookups. Helpers insert and remove handles, bulk-prune stale
entries, or return live handle/identifier pairs for background tasks that need
to enumerate active sessions.[^40]

## Streaming responses

The `Response` enum models several reply styles: a single frame, a vector of
frames, a streamed response, a channel-backed multi-packet response, or an
empty reply. `into_stream` converts any variant into a boxed `FrameStream`,
ready to install on a connection actor with `set_response` so streaming output
can be interleaved with push traffic. `WireframeError` distinguishes transport
failures from protocol-level errors emitted by streaming
responses.[^34][^35][^31]

When constructing imperative streams, the `async-stream` crate integrates
smoothly. The example below yields five frames and converts them into a
`Response::Stream` value:[^36]

```rust
use async_stream::try_stream;
use wireframe::response::Response;

#[derive(bincode::Encode, bincode::BorrowDecode, Debug, PartialEq)]
struct Frame(u32);

fn stream_response() -> Response<Frame> {
    let frames = try_stream! {
        for value in 0..5 {
            yield Frame(value);
        }
    };
    Response::Stream(Box::pin(frames))
}
```

`Response::MultiPacket` exposes a different surface: callers hand ownership of
the receiving half of a `tokio::sync::mpsc` channel to the connection actor,
retain the sender, and push frames whenever back-pressure allows. The
`Response::with_channel` helper constructs the pair and returns the sender
alongside a `Response::MultiPacket`, making the ergonomic tuple pattern
documented in ADR 0001 trivial to adopt. The library awaits channel capacity
before accepting each frame, so producers can rely on the `send().await` future
to coordinate flow control with the peer.

```rust
use tokio::spawn;
use wireframe::response::{Frame, Response};

async fn multi_packet() -> (tokio::sync::mpsc::Sender<Frame>, Response<Frame>) {
    let (sender, response) = Response::with_channel(16);

    spawn({
        let mut producer = sender.clone();
        async move {
            // Back-pressure: `send` awaits whenever the channel is full.
            for chunk in [b"alpha".as_slice(), b"beta".as_slice()] {
                let frame = Frame::from(chunk);
                if producer.send(frame).await.is_err() {
                    return; // connection dropped; stop producing
                }
            }

            // Dropping the last sender releases the channel and triggers the
            // terminator. Explicitly drop when the stream should end.
            drop(producer);
        }
    });

    (sender, response)
}
```

Multi-packet responders rely on the protocol hook `stream_end_frame` to emit a
terminator when the producer side of the channel closes naturally. The
connection actor records why the channel ended (`drained`, `disconnected`, or
`shutdown`), stamps the stored `correlation_id` on the terminator frame, and
routes it through the standard `before_send` instrumentation so telemetry and
higher-level lifecycle hooks observe a consistent end-of-stream signal.
Dropping all senders closes the channel; the actor logs the termination reason
and forwards the terminator through the same hooks used for regular frames so
existing observability continues to work.

## Versioning and graceful deprecation

Phase out older message versions without breaking clients:

- Accept versions N and N-1 on ingress; rewrite legacy payloads in middleware so
  downstream handlers see the current schema.[^10][^12]
- Emit version N on egress so clients observe a single schema.
- Publish metrics and logs describing legacy usage to support operator
  dashboards.[^33][^8]
- Remove adaptors once the sunset window ends.

```rust
use std::sync::Arc;
use wireframe::{
    app::{Envelope, Handler, WireframeApp},
    middleware,
    message::Message,
    Result,
};

#[derive(bincode::Encode, bincode::BorrowDecode)]
struct MsgV1 {
    id: u64,
    name: String,
}

#[derive(bincode::Encode, bincode::BorrowDecode)]
struct MsgV2 {
    id: u64,
    display_name: String,
}

impl From<MsgV1> for MsgV2 {
    fn from(v1: MsgV1) -> Self {
        Self {
            id: v1.id,
            display_name: v1.name,
        }
    }
}

async fn handle_v2(_env: &Envelope) {}

fn build_app() -> Result<WireframeApp> {
    let handler: Handler<Envelope> = Arc::new(|env: &Envelope| Box::pin(handle_v2(env)));

    WireframeApp::new()?
        .route(42, handler)?
        .wrap(middleware::from_fn(|mut req, next| async move {
            if let Ok((legacy, _)) = MsgV1::from_bytes(req.frame()) {
                let upgraded = MsgV2::from(legacy);
                let bytes = upgraded.to_bytes().expect("encode MsgV2");
                req.frame_mut().clear();
                req.frame_mut().extend_from_slice(&bytes);
            }

            let mut response = next.call(req).await?;
            if let Ok((legacy, _)) = MsgV1::from_bytes(response.frame()) {
                let upgraded = MsgV2::from(legacy);
                let bytes = upgraded.to_bytes().expect("encode MsgV2");
                response.frame_mut().clear();
                response.frame_mut().extend_from_slice(&bytes);
            }

            Ok(response)
        }))
}
```

## Metrics and observability

When the optional `metrics` feature is enabled, Wireframe updates the
`wireframe_connections_active` gauge, frame counters tagged by direction, error
counters tagged by kind, and a counter for panicking connection tasks. All
helpers become no-ops when the feature is disabled so instrumentation can stay
in place.[^33] `handle_connection`, the connection actor, and the panic wrapper
call these helpers to maintain consistent telemetry.[^6][^7][^31][^20]

## Additional utilities

- `read_preamble` decodes up to 1 KiB using bincode, returning the decoded
  value plus any leftover bytes that must be replayed before normal frame
  processing.[^37]
- `RewindStream` replays leftover bytes before delegating reads and writes to
  the underlying stream, keeping the framing layer transparent.[^38]
- `panic::format_panic` renders panic payloads for consistent logging across
  `log` and `tracing` consumers.[^39]

[^1]: Implemented in `src/lib.rs` (lines 2-33).
[^2]: Implemented in `src/app/builder.rs` (lines 66-209).
[^3]: Implemented in `src/app/builder.rs` (lines 100-121, 347-360).
[^4]: Implemented in `src/app/builder.rs` (lines 326-344).
[^5]: Implemented in `src/app/error.rs` (lines 7-26).
[^6]: Implemented in `src/app/connection.rs` (lines 41-205).
[^7]: Implemented in `src/app/connection.rs` (lines 207-289).
[^8]: Implemented in `src/app/envelope.rs` (lines 11-172).
[^9]: Implemented in `src/frame/metadata.rs` (lines 1-40).
[^10]: Implemented in `src/message.rs` (lines 4-41).
[^11]: Implemented in `src/middleware.rs` (lines 13-206).
[^12]: Implemented in `src/middleware.rs` (lines 252-347).
[^13]: Implemented in `src/extractor.rs` (lines 1-236).
[^14]: Implemented in `src/hooks.rs` (lines 18-175).
[^15]: Implemented in `src/server/config/mod.rs` (lines 47-152).
[^16]: Implemented in `src/server/runtime.rs` (lines 46-83).
[^17]: Implemented in `src/server/config/binding.rs` (lines 68-214).
[^18]: Implemented in `src/server/runtime.rs` (lines 90-233).
[^19]: Implemented in `src/server/runtime.rs` (lines 240-333).
[^20]: Implemented in `src/server/config/preamble.rs` (lines 14-135) and
    `src/server/connection.rs` (lines 1-222).
[^21]: Implemented in `src/server/error.rs` (lines 7-18).
[^23]: Implemented in `src/push/queues/mod.rs` (lines 41-190).
[^24]: Implemented in `src/push/queues/errors.rs` (lines 7-28).
[^25]: Implemented in `src/push/queues/handle.rs` (lines 84-225).
[^26]: Implemented in `src/push/queues/handle.rs` (lines 198-295).
[^27]: Implemented in `src/push/queues/mod.rs` (lines 255-318).
[^28]: Implemented in `src/connection.rs` (lines 69-205).
[^29]: Implemented in `src/fairness.rs` (lines 1-79).
[^30]: Implemented in `src/connection.rs` (lines 240-441).
[^31]: Implemented in `src/connection.rs` (lines 22-46).
[^33]: Implemented in `src/metrics.rs` (lines 21-111).
[^34]: Implemented in `src/response.rs` (lines 46-151).
[^35]: Implemented in `src/response.rs` (lines 156-209).
[^36]: Demonstrated in `examples/async_stream.rs` (lines 1-23).
[^37]: Implemented in `src/preamble.rs` (lines 1-128).
[^38]: Implemented in `src/rewind_stream.rs` (lines 14-76).
[^39]: Implemented in `src/panic.rs` (lines 1-18).
[^40]: Implemented in `src/session.rs` (lines 13-255).
[^41]: Implemented in `src/fragment/mod.rs` and supporting submodules.
[^42]: Exercised in `tests/features/fragment.feature`.
[^43]: Step definitions in `tests/steps/fragment_steps.rs`.
[^44]: Implemented in `src/client/runtime.rs`, `src/client/builder.rs`,
    `src/client/config.rs`, and `src/client/error.rs`.
[^45]: Implemented in `src/request.rs`.
[^46]: See ADR 0002 for design rationale:
    `docs/adr/0002-streaming-requests-and-shared-message-assembly.md`.
[^47]: Client preamble support implemented in `src/client/builder.rs`
    (lines 288-566) and `src/client/mod.rs` (callback types).
[^48]: Client lifecycle hooks implemented in `src/client/hooks.rs`,
    `src/client/builder.rs` (lifecycle methods), and `src/client/runtime.rs`
    (hook invocation). Behaviour-driven development (BDD) tests in
    `tests/features/client_lifecycle.feature`.

[adr-0002-ref]: adr/0002-streaming-requests-and-shared-message-assembly.md
