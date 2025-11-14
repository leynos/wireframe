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

```rust
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
`WireframeError::DuplicateRoute` when you try to register a handler twice,
keeping the dispatch table unambiguous.[^2][^5] New applications default to the
bundled bincode serializer, a 1024-byte frame buffer, and a 100 ms read
timeout. Clamp these limits with `buffer_capacity` and `read_timeout_ms`, or
swap the serializer with `with_serializer` when you need a different encoding
strategy.[^3][^4]

Once a stream is accepted—either from a manual accept loop or via
`WireframeServer`—`handle_connection(stream)` builds (or reuses) the middleware
chain, wraps the transport in a length-delimited codec, enforces per-frame read
timeouts, and writes responses. Serialization helpers `send_response` and
`send_response_framed` return typed `SendError` variants when encoding or I/O
fails, and the connection closes after ten consecutive deserialization
errors.[^6][^7]

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
transport-level fragmentation state:

- `MessageId` uniquely identifies the logical message a fragment belongs to.
- `FragmentIndex` records the fragment's zero-based order.
- `FragmentHeader` bundles the identifier, index, and a boolean that signals
  whether the fragment is the last one in the sequence.
- `FragmentSeries` tracks the next expected fragment index and reports
  mismatches via structured errors.

Protocol implementers can emit a `FragmentHeader` for every physical frame and
feed the header back into `FragmentSeries` to guarantee ordering before a fully
re-assembled message is surfaced to handlers. Behavioural tests can reuse the
same types to assert that new codecs obey the transport invariants without
spinning up a full server.

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

Length-delimited framing absorbs partial reads before invoking a handler, so a
single logical message can arrive as many transport-level fragments without the
application noticing.[^6] The example below constrains the application buffer
to 64 bytes, writes a 512-byte payload in 16-byte chunks, and shows the handler
receiving the fully reassembled message.

```rust
use std::sync::Arc;

use bytes::BytesMut;
use tokio::{
    io::{self, AsyncWriteExt},
    sync::mpsc,
};
use tokio_util::codec::Encoder;
use wireframe::{
    app::{Envelope, Handler, WireframeApp},
    message::Message,
    serializer::BincodeSerializer,
};

#[derive(bincode::Encode, bincode::BorrowDecode, Debug, PartialEq, Eq)]
struct Large(Vec<u8>);

#[tokio::main]
async fn main() -> wireframe::app::Result<()> {
    let (tx, mut rx) = mpsc::unbounded_channel::<Vec<u8>>();
    let handler_tx = tx.clone();

    let handler: Handler<Envelope> = Arc::new(move |env: &Envelope| {
        let handler_tx = handler_tx.clone();
        let envelope = env.clone();

        Box::pin(async move {
            let parts = envelope.into_parts();
            let payload = parts.payload();
            let (Large(bytes), _) =
                Large::from_bytes(&payload).expect("decode fragmented payload");

            handler_tx
                .send(bytes)
                .expect("receiver dropped before handler completed");
        })
    });

    let app = WireframeApp::new()?
        .buffer_capacity(64)
        .read_timeout_ms(500)
        .route(42, handler)?;

    let mut codec = app.length_codec();
    let (mut client, server) = io::duplex(16);
    let server_task = tokio::spawn(async move { app.handle_connection(server).await });

    let expected = Large(vec![b'Z'; 512]);
    let serializer = BincodeSerializer;
    let payload = expected.to_bytes().expect("encode Large payload");
    let envelope = Envelope::new(42, Some(9001), payload);
    let bytes = serializer
        .serialize(&envelope)
        .expect("serialize envelope");
    let mut framed = BytesMut::with_capacity(bytes.len() + 4);
    codec
        .encode(bytes.into(), &mut framed)
        .expect("frame encoding");
    let frame = framed.to_vec();

    for chunk in frame.chunks(16) {
        client.write_all(chunk).await.expect("chunk write");
        client.flush().await.expect("flush chunk");
    }
    client.shutdown().await.expect("finish writes");

    let received = rx.recv().await.expect("message delivered");
    assert_eq!(received.len(), expected.0.len());
    assert!(received.iter().all(|&b| b == b'Z'));

    server_task.await.expect("connection finished");
    Ok(())
}
```

Increase `buffer_capacity` when the length-delimited codec should accept larger
frames; any payload that exceeds the cap is rejected before the handler runs.
Slow links that fragment heavily may require raising `read_timeout_ms` so the
codec has time to aggregate every chunk before the timeout elapses.[^3][^6]

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
success handlers and synchronous failure callbacks, letting you reject
connections or log decode errors before the application runs.[^20]

`spawn_connection_task` wraps each accepted stream in `read_preamble` and
`RewindStream`, records connection panics, and logs failures without crashing
worker tasks.[^20][^37][^38] `ServerError` surfaces bind and accept failures as
typed errors so callers can react appropriately.[^21]

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
- Remove adapters once the sunset window ends.

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
[^20]: Implemented in `src/server/config/preamble.rs` (lines 14-100) and
    `src/server/connection.rs` (lines 17-84).
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
