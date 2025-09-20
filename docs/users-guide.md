# Wireframe user guide

Wireframe is a Rust library for building asynchronous binary protocol servers
with pluggable routing, middleware, and connection utilities.[^1] This guide
walks through the major components available today and explains how they fit
together when you assemble an application.

## Quick start: building an application

A `WireframeApp` collects routes and middleware, then drives a framed transport
when you call `handle_connection` on an I/O stream.[^2][^3] Routes are keyed by
numeric identifiers and map to handler functions. Each handler is wrapped in an
`Arc` and returns a future that resolves to `()` once the handler has finished
its work.[^4]

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
helping you keep the table unambiguous.[^5][^6] By default the builder uses the
bundled bincode serializer, accepts frames up to 1024 bytes, and enforces a 100
ms read timeout on each request. You can adjust these limits with
`buffer_capacity` and `read_timeout_ms`; values are clamped between 64 bytes
and 16 MiB, and between 1 ms and 24 h respectively.[^7] Store connection-wide
state for extractors using `app_data`, which registers values keyed by `TypeId`
so they can be retrieved from request contexts.[^8][^9]

Once a stream is accepted—either from a manual accept loop or via
`WireframeServer`—call `handle_connection(stream)` to deserialise frames, build
or reuse the middleware chain, execute handlers, and write responses back using
length-delimited framing. Deserialisation failures are logged, counted, and the
connection is closed after ten consecutive errors. [^10] If you need to emit a
response immediately, use `send_response` or `send_response_framed` to
serialise a `Message` value with the configured `Serializer` and write it to an
async stream; errors surface as `SendError` variants that distinguish
serialisation failures from I/O problems.[^11][^12]

## Understanding packets and serialisation

Wireframe works with user-defined messages that implement the `Message` trait,
which provides helpers for bincode encoding and decoding.[^13] Serialisers plug
into the `Serializer` trait; the default `BincodeSerializer` uses bincode’s
standard configuration and also implements `FrameMetadata` so the framework can
extract envelope metadata without decoding the full payload.[^14] Swap
serialisers with `WireframeApp::with_serializer` during construction or the
`serializer` method later in the builder chain when you need a different
encoding strategy.[^15] Messages are wrapped in packets implementing the
`Packet` trait. The bundled `Envelope` stores the route identifier, an optional
correlation identifier, and raw payload bytes. Utilities on `PacketParts` let
you inherit or overwrite correlation identifiers before a response is
sent.[^16] When processing frames the application first tries to parse metadata
via `FrameMetadata::parse`, falling back to full deserialisation if the
shortcut fails.[^17]

## Accessing request data with extractors

`MessageRequest` records connection metadata and shared state populated by
`WireframeApp::app_data`, making it available to extractors without global
variables.[^18] Custom extractors implement `FromMessageRequest`, receiving the
request context and a `Payload` view over the remaining bytes; helper methods
let you advance the payload safely or query how much data is left.[^19]
Built-in extractors cover common needs: `SharedState<T>` retrieves registered
state, `Message<T>` decodes the payload into a concrete type that implements
`Message`, and `ConnectionInfo` exposes the peer socket address.[^20] Failures
return `ExtractError`, distinguishing missing shared state from payload decode
issues so you can respond appropriately.[^21]

## Handling requests with middleware

Middleware is responsible for decoding incoming frames, applying cross-cutting
behaviour, and shaping the response payload. Each request flows through a chain
of `Service` implementations that accept a `ServiceRequest` and return a
`ServiceResponse`. Both wrappers expose methods to inspect or mutate the frame
buffer and correlation identifier when building a reply.[^22] The `Next`
continuation gives middleware access to the remainder of the chain, while the
`Transform` trait wraps services to build new pipeline stages.[^23] For simple
cases, use `middleware::from_fn` to adapt an async function into a middleware
component. The function receives the request and a `Next` reference, allowing
you to decode the payload, call the inner service, and edit the response before
it is serialised.[^24] When the pipeline reaches a registered route,
`HandlerService` reconstructs the packet, invokes the handler, then converts
the handler’s packet back into a `ServiceResponse` for the framework to
serialise.[^25]

## Connection lifecycle

During `handle_connection`, the builder optionally runs per-connection setup
and teardown callbacks. Setup is invoked once per connection and may return
state of any type; the returned value is held until teardown runs after the
stream finishes processing.[^26] The connection driver runs the teardown
callback once the stream completes processing.[^27] Framed I/O uses a
`LengthDelimitedCodec` configured with the application’s buffer capacity, and
every read is wrapped in a timeout window derived from `read_timeout_ms`.[^28]
Inbound frames increment the inbound metrics counter, while serialisation and
handler failures bump error counters before warnings are logged.[^29][^30]
Responses inherit correlation identifiers via
`PacketParts::inherit_correlation` so that mis-matched values are corrected and
logged rather than silently propagated.[^31]

## Protocol hooks

Implement `WireframeProtocol` when you need to run custom logic around outbound
frames. The trait exposes callbacks for connection setup, pre-send mutation,
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
below one.[^36] You can install a one-shot readiness notifier with
`ready_signal` and customise accept-loop backoff delays using `accept_backoff`,
which normalises values via `BackoffConfig` to keep them within sensible
bounds.[^37] The runtime enforces those normalised limits inside the accept
loop to prevent runaway retries.[^38] Binding transitions the server into the
`Bound` typestate, exposes `local_addr`, and supports rebinding to a new
listener when required; failures surface as `ServerError::Bind` or
`ServerError::Accept` variants.[^39][^40]

`run` awaits Ctrl+C for shutdown, while `run_with_shutdown` accepts any future
and cancels all worker tasks once it resolves.[^41] Each worker runs an accept
loop that clones the factory, applies optional preamble handlers, rewinds
leftover bytes, and hands the stream to the application. Transient accept
failures trigger exponential backoff capped by the configured maximum
delay.[^42] Accepted streams are processed by `spawn_connection_task`, which
rewinds leftover bytes, runs optional handlers, and then invokes your
application.[^43] Custom preambles are enabled via `with_preamble` alongside
success and failure callbacks; both hooks run before the stream reaches your
application so you can perform handshakes or log validation errors.[^44] The
preamble module also exposes `read_preamble`, which incrementally decodes up to
1 KiB using bincode, and `RewindStream`, which replays leftover bytes before
continuing with the underlying transport.[^45][^46]

## Push queues and connection actors

Push queues buffer outbound frames that originate from background tasks.[^47]
The fluent `PushQueuesBuilder` configures separate capacities for high- and
low-priority queues, an optional global rate limit (default 100 pushes per
second), and an optional dead-letter queue with adjustable logging cadence for
dropped frames.[^48] Builder helpers such as `unlimited`, `dlq`, and
`dlq_log_every_n` make it easy to tune throughput and observability.[^48]
Validation ensures capacities are non-zero and that rate limits fall within
`1..=MAX_PUSH_RATE` (10 000 messages per second).[^49] Frames are drained with
`recv`, which prioritises the high-priority queue but eventually yields
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
shutdown token. It keeps per-connection metrics via an RAII guard, exposes
`set_response` for attaching a `FrameStream`, and offers `set_fairness` to tune
how often low-priority frames are drained. The main `run` loop honours
cancellation, preserves strict ordering for high-priority frames, and emits
protocol hooks around every outbound frame.[^54] Fairness is governed by
`FairnessConfig`, which limits consecutive high-priority frames and can enforce
a time slice; `FairnessTracker` records usage and determines when to yield to
low-priority work.[^55][^56] When a streaming response yields
`WireframeError::Protocol`, the actor invokes `handle_error` on the installed
protocol and continues draining; transport errors abort the run with the error
so callers can log or reconnect.[^57] Use `active_connection_count()` to
inspect the current connection gauge, or query the shutdown token if you need
to cancel the actor from another thread.[^58]

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
`FrameStream`, which you can pass to `ConnectionActor::set_response` when you
need to interleave pushes and streamed handler output. Protocol or transport
errors are wrapped in `WireframeError`, enabling clients to differentiate
between logical failures and I/O issues.[^61][^62]

## Metrics and observability

When the optional `metrics` feature is enabled, Wireframe updates several named
counters and gauges: `wireframe_connections_active`,
`wireframe_frames_processed_total` tagged by direction,
`wireframe_errors_total` with error type labels, and
`wireframe_connection_panics_total` for panicking connection tasks.[^63][^64]
Each helper becomes a no-op when the feature is disabled, so you can enable
instrumentation without sprinkling conditional compilation throughout your
code. Outbound and inbound frames call these helpers from both the connection
actor and the request-processing pipeline.[^29][^58]

## Additional utilities

For manual integrations you can call `read_preamble` directly when you need to
validate an initial handshake. The helper reads up to 1 KiB, decoding
additional bytes as required, and returns any leftover data alongside the
decoded value. Wrap the stream in `RewindStream` so the leftover bytes are
replayed before normal processing begins.[^45][^46] If a connection task
panics, use `panic::format_panic` to render the payload for consistent logging
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
