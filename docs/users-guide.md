# Wireframe library guide

Wireframe is a Rust library for building asynchronous binary protocol servers
with pluggable routing, middleware, and connection utilities.[^1] The following
sections describe the major components available today and explain how they fit
together when assembling an application.

## Quick start: building an application

A `WireframeApp` collects routes and middleware, then drives a framed transport
when `handle_connection` is called on an I/O stream.[^2][^3] Routes are keyed
by numeric identifiers and map to handler functions. Each handler is wrapped in
an `Arc` and returns a future that resolves to `()` once handler execution
completes.[^4]

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

Register each route with a unique identifier; the builder returns
`WireframeError::DuplicateRoute` if a handler is already registered for an id,
helping maintain an unambiguous table.[^5][^6] By default the builder uses the
bundled bincode serializer, accepts frames up to 1024 bytes, and enforces a 100
ms read timeout on each request. These limits are adjustable via
`buffer_capacity` and `read_timeout_ms`; values are clamped between 64 bytes
and 16 MiB, and between 1 ms and 24 h respectively.[^7] Store connection-wide
state for extractors using `app_data`, which registers values keyed by `TypeId`
so they can be retrieved from request contexts.[^8][^9]

Once a stream is accepted—either from a manual accept loop or via
`WireframeServer`—`handle_connection(stream)` deserializes frames, builds or
reuses the middleware chain, executes handlers, and writes responses back using
length-delimited framing. Deserialization failures are logged, counted, and the
connection is closed after ten consecutive errors.[^10] Immediate responses can
be emitted via `send_response` or `send_response_framed`, which serialize a
`Message` value with the configured `Serializer` and write it to an async
stream; errors surface as `SendError` variants that distinguish serialization
failures from I/O problems.[^11][^12]

## Understanding packets and serialization

Wireframe works with user-defined messages that implement the `Message` trait,
which provides helpers for bincode encoding and decoding.[^13] Serializers plug
into the `Serializer` trait; the default `BincodeSerializer` uses bincode’s
standard configuration and also implements `FrameMetadata` so the framework can
extract envelope metadata without decoding the full payload.[^14] Swap
serializers with `WireframeApp::with_serializer` during construction or the
`serializer` method later in the builder chain when a different encoding
strategy is required.[^15] Messages are wrapped in packets implementing the
`Packet` trait. The bundled `Envelope` stores the route identifier, an optional
correlation identifier, and raw payload bytes. Utilities on `PacketParts`
permit correlation identifiers to be inherited or overwritten before a response
is sent.[^16] When processing frames the application first tries to parse
metadata via `FrameMetadata::parse`, falling back to full deserialization if
the shortcut fails.[^17]

## Accessing request data with extractors

`MessageRequest` records connection metadata and shared state populated by
`WireframeApp::app_data`, making it available to extractors without global
variables.[^18] Custom extractors implement `FromMessageRequest`, receiving the
request context and a `Payload` view over the remaining bytes; helper methods
advance the payload safely or report the remaining length.[^19] Built-in
extractors cover common needs: `SharedState<T>` retrieves registered state,
`Message<T>` decodes the payload into a concrete type that implements
`Message`, and `ConnectionInfo` exposes the peer socket address.[^20] Failures
return `ExtractError`, distinguishing missing shared state from payload decode
issues so that responses can be shaped appropriately.[^21]

## Handling requests with middleware

Middleware is responsible for decoding incoming frames, applying cross-cutting
behaviour, and shaping the response payload. Each request flows through a chain
of `Service` implementations that accept a `ServiceRequest` and return a
`ServiceResponse`. Both wrappers expose methods to inspect or mutate the frame
buffer and correlation identifier when building a reply.[^22] The `Next`
continuation gives middleware access to the remainder of the chain, while the
`Transform` trait wraps services to build new pipeline stages.[^23] For simple
cases, use `middleware::from_fn` to adapt an async function into a middleware
component. The function receives the request and a `Next` reference, enabling
payload decoding, invocation of the inner service, and response editing before
it is serialized.[^24] When the pipeline reaches a registered route,
`HandlerService` reconstructs the packet, invokes the handler, then converts
the handler’s packet back into a `ServiceResponse` for the framework to
serialize.[^25]

## Connection lifecycle

During `handle_connection`, the builder optionally runs per-connection setup
and teardown callbacks. Setup is invoked once per connection and may return
state of any type; the returned value is held until teardown runs after the
stream finishes processing.[^26] The connection driver runs the teardown
callback once the stream completes processing.[^27] Framed I/O uses a
`LengthDelimitedCodec` configured with the application’s buffer capacity, and
every read is wrapped in a timeout window derived from `read_timeout_ms`.[^28]
Inbound frames increment the inbound metrics counter, while serialization and
handler failures bump error counters before warnings are logged.[^29][^30]
Responses inherit correlation identifiers via
`PacketParts::inherit_correlation` so that mis-matched values are corrected and
logged rather than silently propagated.[^31]

## Protocol hooks

Implement `WireframeProtocol` when custom logic around outbound frames is
required. The trait exposes callbacks for connection setup, pre-send mutation,
command completion, protocol error handling, and optional end-of-stream frame
generation.[^32] Attach a protocol to the builder with `with_protocol`. Use
`protocol_hooks()` to convert the stored implementation into `ProtocolHooks`
when constructing a `ConnectionActor`.[^33] `ConnectionActor::with_hooks`
installs these callbacks so that pushes and streamed responses invoke them
consistently alongside any connection metrics or fairness logic.[^34][^35]

## Running servers

`WireframeServer::new` clones an application factory per worker and starts in
the unbound state; bind a socket with `bind` or `bind_existing_listener` before
calling any run method. A convenience `worker_count` accessor reports the
current worker total, which defaults to the host CPU count but never drops
below one.[^36] A one-shot readiness notifier is installed with `ready_signal`,
and `accept_backoff` customizes accept-loop backoff delays by normalizing
values via `BackoffConfig` to keep them within sensible bounds.[^37] The
runtime enforces those normalized limits inside the accept loop to prevent
runaway retries.[^38] Binding transitions the server into the `Bound`
typestate, exposes `local_addr`, and supports rebinding to a new listener when
required; failures surface as `ServerError::Bind` or `ServerError::Accept`
variants.[^39][^40]

`run` awaits Ctrl+C for shutdown, while `run_with_shutdown` accepts any future
and cancels all worker tasks once it resolves.[^41] Each worker runs an accept
loop that clones the factory, applies optional preamble handlers, rewinds
leftover bytes, and hands the stream to the application. Transient accept
failures trigger exponential backoff capped by the configured maximum
delay.[^42] Accepted streams are processed by `spawn_connection_task`, which
rewinds leftover bytes, runs optional handlers, and then invokes the
application.[^43] Custom preambles are enabled via `with_preamble` alongside
success and failure callbacks; both hooks run before the stream reaches the
application, enabling handshakes or structured logging of validation
errors.[^44] The preamble module also exposes `read_preamble`, which
incrementally decodes up to 1 KiB using bincode, and `RewindStream`, which
replays leftover bytes before continuing with the underlying
transport.[^45][^46]

## Push queues and connection actors

Push queues buffer outbound frames that originate from background tasks.[^47]
The fluent `PushQueuesBuilder` configures separate capacities for high- and
low-priority queues, an optional global rate limit (default 100 pushes per
second), and an optional dead-letter queue with adjustable logging cadence for
dropped frames.[^48] Builder helpers such as `unlimited`, `dlq`, and
`dlq_log_every_n` simplify throughput and observability tuning.[^48] Validation
ensures capacities are non-zero and that rate limits fall within
`1..=MAX_PUSH_RATE` (10 000 messages per second).[^49] Frames are drained with
`recv`, which prioritizes the high-priority queue but eventually yields
low-priority frames, and `close` allows tests to release resources when no
actor is draining the queues.[^50] `PushHandle` instances are cloneable and
expose async methods for pushing high- or low-priority frames subject to the
configured rate limiter. The synchronous `try_push` supports policies that
either return an error, drop the frame, or drop and log a warning; when a
dead-letter queue is present, dropped frames are forwarded there with throttled
logging to avoid noise.[^51] Push operations report back using the `PushError`
enum, while configuration errors use `PushConfigError` to signal invalid
capacities or rates.[^52] All frame types must implement the marker trait
`FrameLike`, which simply requires `Send + 'static`.[^53]

`ConnectionActor` consumes the push queues, optional streaming responses, and a
shutdown token. It keeps per-connection metrics via a Resource Acquisition Is
Initialization (RAII) guard, exposes `set_response` for attaching a
`FrameStream`, and offers `set_fairness` to tune how often low-priority frames
are drained. The main `run` loop honours cancellation, preserves strict
ordering for high-priority frames, and emits protocol hooks around every
outbound frame.[^54] Fairness is governed by `FairnessConfig`, which limits
consecutive high-priority frames and can enforce a time slice;
`FairnessTracker` records usage and determines when to yield to low-priority
work.[^55][^56] When a streaming response yields `WireframeError::Protocol`,
the actor invokes `handle_error` on the installed protocol and continues
draining; transport errors abort the run with the error so callers can log or
reconnect.[^57] Use `active_connection_count()` to inspect the current
connection gauge, or query the shutdown token when cancellation from another
thread is required.[^58]

## Session management

`ConnectionId` wraps a `u64` identifier with convenience constructors and
formatting helpers.[^59] `SessionRegistry` stores weak references to
`PushHandle` instances keyed by `ConnectionId`, allowing other tasks to
discover active connections without preventing cleanup. Lookups attempt to
upgrade the stored pointer and prune dead entries automatically; explicit
`prune` calls remove stale handles in bulk. Dedicated methods insert or remove
handles, and helper accessors return either the live handle pairs or just their
identifiers.[^60]

## Streaming responses

The `Response` enum models several response styles: a single frame, a vector of
frames, a streamed response, a multi-packet channel backed by `tokio::mpsc`, or
an empty response. Call `into_stream` to convert any variant into a boxed
`FrameStream`, suitable for passing to `ConnectionActor::set_response` when
interleaving pushes and streamed handler output is required. Protocol or
transport errors are wrapped in `WireframeError`, enabling clients to
differentiate between logical failures and I/O issues.[^61][^62]

When constructing imperative streams within handlers, prefer the `async-stream`
crate’s `stream!` macro as the canonical approach. The macro produces
`FrameStream` values once items are wrapped in `Result` and mapped into
`WireframeError`:[^66]

```rust
use async_stream::stream;
use futures::StreamExt;
use wireframe::response::{Response, WireframeError};

fn stream_response() -> Response<u32> {
    let frames = stream! {
        for value in 0..5 {
            yield Ok::<u32, ()>(value);
        }
    }
    .map(|frame| frame.map_err(WireframeError::from));
    Response::Stream(Box::pin(frames))
}
```

## Versioning and graceful deprecation

When evolving message schemas, deprecate older versions in stages:

- Accept N and N-1 on ingress; emit N on egress using the `Message` extractor
  and canonical `Response` conversion.[^20][^61]
- Add adapters to up-convert N-1 payloads to N inside extractors, keeping
  routing logic focused on the newest data shape.[^19][^20]
- Emit deprecation metrics and logs through the `metrics` helpers with
  rate-limited warnings to avoid noise.[^63][^67]
- Announce removal dates; delete adapters after the sunset date.

Example up-conversion adapter:

```rust
#[derive(serde::Deserialize)]
struct MsgV1 { id: u64, name: String }

#[derive(serde::Deserialize, serde::Serialize)]
struct MsgV2 { id: u64, display_name: String }

impl From<MsgV1> for MsgV2 {
    fn from(v1: MsgV1) -> Self {
        MsgV2 { id: v1.id, display_name: v1.name }
    }
}
```

Expose both decoders temporarily, then standardize on `MsgV2` internally.

## Metrics and observability

When the optional `metrics` feature is enabled, Wireframe updates several named
counters and gauges: `wireframe_connections_active`,
`wireframe_frames_processed_total` tagged by direction,
`wireframe_errors_total` with error type labels, and
`wireframe_connection_panics_total` for panicking connection tasks.[^63][^64]
Each helper becomes a no-op when the feature is disabled, allowing
instrumentation without sprinkling conditional compilation throughout the
codebase. Outbound and inbound frames call these helpers from both the
connection actor and the request-processing pipeline.[^29][^58]

## Additional utilities

For manual integrations, call `read_preamble` directly when it is necessary to
validate an initial handshake. The helper reads up to 1 KiB, decoding
additional bytes as required, and returns any leftover data alongside the
decoded value. Wrap the stream in `RewindStream` so the leftover bytes are
replayed before normal processing begins.[^45][^46] If a connection task
panics, call `panic::format_panic` to render the payload for consistent logging
across `log` and `tracing` consumers.[^65]

[^1]: Implemented in `src/lib.rs` (lines 2-33).
[^2]: Implemented in `src/app/builder.rs` (lines 66-209).
[^3]: Implemented in `src/app/connection.rs` (lines 121-289).
[^4]: Implemented in `src/app/builder.rs` (lines 66-179).
[^5]: Implemented in `src/app/builder.rs` (lines 166-179).
[^6]: Implemented in `src/app/error.rs` (lines 7-24).
[^7]: Implemented in `src/app/builder.rs` (lines 100-361).
[^8]: Implemented in `src/app/builder.rs` (lines 181-195).
[^9]: Implemented in `src/extractor.rs` (lines 31-84).
[^10]: Implemented in `src/app/connection.rs` (lines 26-289).
[^11]: Implemented in `src/app/connection.rs` (lines 41-97).
[^12]: Implemented in `src/app/error.rs` (lines 16-26).
[^13]: Implemented in `src/message.rs` (lines 15-58).
[^14]: Implemented in `src/serializer.rs` (lines 11-57).
[^15]: Implemented in `src/app/builder.rs` (lines 153-345).
[^16]: Implemented in `src/app/envelope.rs` (lines 11-172).
[^17]: Implemented in `src/app/connection.rs` (lines 99-120).
[^18]: Implemented in `src/extractor.rs` (lines 1-84).
[^19]: Implemented in `src/extractor.rs` (lines 87-172).
[^20]: Implemented in `src/extractor.rs` (lines 188-359).
[^21]: Implemented in `src/extractor.rs` (lines 200-236).
[^22]: Implemented in `src/middleware.rs` (lines 13-116).
[^23]: Implemented in `src/middleware.rs` (lines 119-189).
[^24]: Implemented in `src/middleware.rs` (lines 191-273).
[^25]: Implemented in `src/middleware.rs` (lines 275-347).
[^26]: Implemented in `src/app/builder.rs` (lines 211-264).
[^27]: Implemented in `src/app/connection.rs` (lines 130-152).
[^28]: Implemented in `src/app/connection.rs` (lines 41-193).
[^29]: Implemented in `src/app/connection.rs` (lines 200-280).
[^30]: Implemented in `src/metrics.rs` (lines 21-111).
[^31]: Implemented in `src/app/envelope.rs` (lines 104-149).
[^32]: Implemented in `src/hooks.rs` (lines 1-186).
[^33]: Implemented in `src/app/builder.rs` (lines 266-324).
[^34]: Implemented in `src/connection.rs` (lines 161-205).
[^35]: Implemented in `src/hooks.rs` (lines 80-186).
[^36]: Implemented in `src/server/config/mod.rs` (lines 47-152).
[^37]: Implemented in `src/server/config/mod.rs` (lines 89-152).
[^38]: Implemented in `src/server/runtime.rs` (lines 46-107).
[^39]: Implemented in `src/server/config/binding.rs` (lines 70-200).
[^40]: Implemented in `src/server/error.rs` (lines 1-13).
[^41]: Implemented in `src/server/runtime.rs` (lines 85-205).
[^42]: Implemented in `src/server/runtime.rs` (lines 200-333).
[^43]: Implemented in `src/server/connection.rs` (lines 17-85).
[^44]: Implemented in `src/server/config/preamble.rs` (lines 20-100).
[^45]: Implemented in `src/preamble.rs` (lines 5-128).
[^46]: Implemented in `src/rewind_stream.rs` (lines 1-66).
[^47]: Implemented in `src/push/queues/mod.rs` (lines 1-172).
[^48]: Implemented in `src/push/queues/builder.rs` (lines 16-166).
[^49]: Implemented in `src/push/queues/mod.rs` (lines 112-173).
[^50]: Implemented in `src/push/queues/mod.rs` (lines 255-318).
[^51]: Implemented in `src/push/queues/handle.rs` (lines 84-298).
[^52]: Implemented in `src/push/queues/errors.rs` (lines 7-19).
[^53]: Implemented in `src/push/queues/mod.rs` (lines 33-45).
[^54]: Implemented in `src/connection.rs` (lines 22-400).
[^55]: Implemented in `src/connection.rs` (lines 69-205).
[^56]: Implemented in `src/fairness.rs` (lines 1-86).
[^57]: Implemented in `src/connection.rs` (lines 400-434).
[^58]: Implemented in `src/connection.rs` (lines 22-205).
[^59]: Implemented in `src/session.rs` (lines 7-46).
[^60]: Implemented in `src/session.rs` (lines 57-216).
[^61]: Implemented in `src/response.rs` (lines 1-189).
[^62]: Implemented in `src/connection.rs` (lines 193-205).
[^63]: Implemented in `src/metrics.rs` (lines 1-89).
[^64]: Implemented in `src/server/connection.rs` (lines 41-47).
[^65]: Implemented in `src/panic.rs` (lines 1-13).
[^66]: Demonstrated in `examples/async_stream.rs` (lines 1-30).
[^67]: Illustrated in `docs/asynchronous-outbound-messaging-design.md`
    (lines 344-359).
