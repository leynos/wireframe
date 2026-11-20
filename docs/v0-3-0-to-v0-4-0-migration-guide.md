# v0.3.0 to v0.4.0 migration guide

This guide covers the prepared-application transition for applications that
drive accepted streams directly. It explains how to move route and middleware
setup out of connection handling while retaining the existing server factory
workflow.

## Removed: extractors and `app_data`

[ADR 017](adrs/adr-0017-remove-route-extractors-and-app-data.md) removes the
extractor module and the application-data store before 1.0. The module was
public, but no dispatch path invoked it: route handlers received only their
envelope, so `FromMessageRequest` implementations never ran and values stored by
`WireframeApp::app_data` were never readable through `SharedState<T>`.

Route handlers keep the `Handler<E>` shape:

<!-- tested-example: v0-3-0-to-v0-4-0-migration-guide-019 -->
```rust
use std::sync::Arc;

use wireframe::app::{Envelope, Handler};

fn make_handler() -> Handler<Envelope> {
    Arc::new(|envelope: &Envelope| {
        // Decode or inspect the envelope here.
        let _ = envelope.payload_bytes();
        Box::pin(async {})
    })
}
```

Replace each removed item as follows:

| Removed symbol                             | Replacement                                          |
| ------------------------------------------ | ---------------------------------------------------- |
| `wireframe::extractor::FromMessageRequest` | Decode in the handler or a helper function           |
| `wireframe::extractor::MessageRequest`     | Use the envelope fields the handler actually needs   |
| `wireframe::extractor::Payload`            | Borrow `Envelope::payload_bytes`                     |
| `wireframe::extractor::Message<T>`         | `T::from_bytes` on a bincode payload                 |
| `wireframe::extractor::SharedState<T>`     | Capture an `Arc<T>` in the handler closure           |
| `wireframe::extractor::ConnectionInfo`     | No peer address reaches route handlers               |
| `wireframe::extractor::StreamingBody`      | `RequestBodyReader::new(stream)` on a stream you own |
| `wireframe::extractor::ExtractError`       | Define a protocol- or application-specific error     |
| `wireframe::app_data_store::AppDataStore`  | Capture an `Arc` in the handler closure              |
| `WireframeApp::app_data`                   | Capture an `Arc` in the handler closure              |

### Shared state

Capture shared state in the handler closure instead of registering it on the
application:

<!-- tested-example: v0-3-0-to-v0-4-0-migration-guide-020 -->
```rust
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use wireframe::app::{Envelope, Handler, WireframeApp};

fn build_app(counter: Arc<AtomicU64>) -> wireframe::Result<WireframeApp> {
    let handler: Handler<Envelope> = Arc::new(move |_: &Envelope| {
        let counter = Arc::clone(&counter);
        Box::pin(async move {
            counter.fetch_add(1, Ordering::Relaxed);
        })
    });

    // Annotate the binding so the remaining type parameters take their defaults.
    let app: WireframeApp = WireframeApp::new()?.route(1, handler)?;
    Ok(app)
}
```

### Request decoding

`message::Message::from_bytes` decodes a bincode payload only. A protocol with
another payload format should call its serializer's decoder instead, binding
the target type through `DecodeWith`.

### Inbound streaming

`RequestParts`, `RequestBodyStream`, and `RequestBodyReader` remain available,
but the connection runtime does not create a body channel. Code that wants an
incremental body owns both ends of `body_channel` and wraps the receiving end
with `RequestBodyReader::new`.

## Fragment reassembly limits

`FragmentationConfig` now requires `max_concurrent_partials` and
`max_buffered_bytes` when constructed with a struct literal. Set either field to
`None` to disable that limit, or use a non-zero value to cap partial-message
count or aggregate buffered payload bytes.
`FragmentationConfig::for_frame_budget` derives both limits from the frame
budget: at most `min(frame_budget, 64)` concurrent partial messages and the
larger of `frame_budget` multiplied by 64 and `max_message_size` buffered
payload bytes. The multiplication saturates on overflow. These defaults also
apply when `WireframeApp` enables fragmentation without a custom configuration.
Connections that exceed either aggregate cap now close immediately; set the
corresponding field to `None` to disable that cap.

For example, add both fields to an existing literal:

<!-- tested-example: v0-3-0-to-v0-4-0-migration-guide-001 -->
```rust
use std::{num::NonZeroUsize, time::Duration};
use wireframe::fragment::FragmentationConfig;

let config = FragmentationConfig {
    fragment_payload_cap: NonZeroUsize::new(1024).expect("non-zero cap"),
    max_message_size: NonZeroUsize::new(16 * 1024).expect("non-zero cap"),
    max_concurrent_partials: NonZeroUsize::new(64),
    max_buffered_bytes: NonZeroUsize::new(64 * 1024),
    reassembly_timeout: Duration::from_secs(30),
};
```

`Reassembler::new` also accepts the two optional limits after its existing
message-size and timeout arguments. Pass `None, None` to preserve unbounded
aggregate buffering, or configure caps for standalone reassemblers:

<!-- tested-example: v0-3-0-to-v0-4-0-migration-guide-002 -->
```rust
use std::{num::NonZeroUsize, time::Duration};
use wireframe::fragment::Reassembler;

let message_limit = NonZeroUsize::new(16 * 1024).expect("non-zero cap");
let reassembler = Reassembler::new(
    message_limit,
    Duration::from_secs(30),
    NonZeroUsize::new(64),
    NonZeroUsize::new(64 * 1024),
);
```

If either configured aggregate cap is exceeded, the connection closes
immediately. Per-message size violations retain their existing counted
deserialization-failure behaviour.

## Client error matching

`ClientError` is now marked `#[non_exhaustive]`, and pooled acquisitions can
return `ClientError::PoolSaturated { limit }` when the configured waiter bound
is full. Add a wildcard arm to exhaustive matches and handle saturation at the
caller boundary. For example:

<!-- tested-example: v0-3-0-to-v0-4-0-migration-guide-003 -->
```rust
use wireframe::client::ClientError;

fn saturation_limit(error: ClientError) -> Option<usize> {
    match error {
        ClientError::PoolSaturated { limit } => Some(limit),
        _ => None,
    }
}
```

The pool rejects the acquisition before assigning a socket, so this error does
not indicate a broken connection. Applications can report temporary
unavailability or retry after capacity returns.

## Client pool lifecycle and scheduler failures

A client pool now owns one persistent scheduler task for its whole lifetime
instead of spawning a servicing task as capacity is released. The observable
contract changes in three ways.

First, `WireframeClientPool::close` is asynchronous and must be awaited. It
returns once every queued and buffered acquisition has been resolved, so an
existing `pool.close();` statement must become `pool.close().await;`.

Second, acquisitions on a closed pool return `ClientError::PoolClosed`, and
acquisitions on a pool whose scheduler task terminated abnormally return
`ClientError::SchedulerFailed`. Queued callers observe the same errors rather
than waiting forever or reporting a transport failure. Neither error means the
socket is dirty: `should_recycle_connection` returns `false` for both, so
discard the pool and reconnect instead of returning a lease.

Third, `ClientError::PoolSaturated { limit }` keeps its existing meaning. The
pool rejects the acquisition before assigning a slot, so callers can still
treat it as temporary unavailability and retry once capacity returns.

<!-- tested-example: v0-3-0-to-v0-4-0-migration-guide-004 -->
```rust
use wireframe::client::{ClientError, ClientPoolConfig, WireframeClient};

# async fn demo(addr: std::net::SocketAddr) -> Result<(), Box<dyn std::error::Error>> {
let pool = WireframeClient::builder()
    .connect_pool(addr, ClientPoolConfig::default())
    .await?;
let mut handle = pool.handle();
pool.close().await;

let error = match handle.acquire().await {
    Ok(_lease) => panic!("closed pool must refuse"),
    Err(error) => error,
};
assert!(matches!(error, ClientError::PoolClosed));
# Ok(())
# }
```

## Bounded peer decoding

The bundled bincode decoders now cap accounted decode work for peer-provided
container lengths. `Message::from_bytes`, the Bincode serializer, reassembled
messages, and the optional Serde bridge use the fixed `MAX_FRAME_LENGTH` budget
of 16 MiB. Preamble decoding uses the separate `MAX_PREAMBLE_LEN` budget of 1
KiB. Native bincode decoding rejects a declared container above its applicable
budget before reading or allocating its body. The Serde bridge accounts bytes
as elements are read and rejects decoding once those reads exceed the budget.
These accounting limits do not replace validation for type-specific resource
needs or custom decoders.

The lower-level prefix contracts remain available: `Message::from_bytes`
returns the number of bytes consumed, and `read_preamble` returns bytes read
ahead of the decoded value as leftovers. At the inbound server boundary, an
`Envelope` frame must contain exactly one value, so trailing bytes now cause a
decoding error. If a producer previously appended data after an envelope,
encode that data within the envelope or send it in another frame. Deployments
that inspect rejection rates can use the optional
`wireframe_decode_rejections_total` counter, labelled by the bounded decode
path and rejection reason.

## Declared total body length is authoritative

`FirstFrameHeader::total_body_len` is now a protocol commitment. Message
assembly rejects a first frame that already contradicts its declaration, stops
a continuation from overshooting it, and requires the completed body to match
it exactly. See
[ADR 015](adrs/adr-0015-declared-total-body-length-is-authoritative.md) for the
decision.

Applications that declare a total must supply the exact body length. A protocol
that declares an approximate size will now see its messages rejected because
the length actually assembled can no longer differ from the length promised.
Leaving `total_body_len` as `None` opts out of the equality requirement and
keeps the previous behaviour, so that is the migration path for a protocol
whose totals were only ever advisory.

`MessageAssemblyError` gains three variants: `DeclaredTotalTooSmall`,
`DeclaredTotalExceeded`, and `DeclaredTotalMismatch`. The enum is not
`#[non_exhaustive]`, so an exhaustive match written against the previous
variants fails to compile. Add the new arms:

<!-- tested-example: v0-3-0-to-v0-4-0-migration-guide-005 -->
```rust
use wireframe::message_assembler::MessageAssemblyError;

fn is_retryable(error: MessageAssemblyError) -> bool {
    match error {
        MessageAssemblyError::DeclaredTotalTooSmall { .. }
        | MessageAssemblyError::DeclaredTotalExceeded { .. }
        | MessageAssemblyError::DeclaredTotalMismatch { .. } => false,
        _ => true,
    }
}
```

The inbound adapter maps all three to `io::ErrorKind::InvalidData` and counts a
deserialization failure, so a rejected message never reaches a handler.

## Preamble deadline

`WireframeServer::new` and `WireframeServer::from_app` now bound the preamble
read with `DEFAULT_PREAMBLE_TIMEOUT`, a 10-second deadline. Previously the read
was unbounded unless `preamble_timeout` was set, so a peer that connected and
never sent its preamble kept a connection task alive indefinitely. The deadline
covers only the read; success and failure callbacks and application message
handling run outside it.

Most clients send their preamble immediately and need no change. Adjust the
configuration when:

- A legitimate handshake exchanges several messages and can exceed 10 seconds.
  Call `preamble_timeout(Duration)` with a larger, explicit bound, or
  `disable_preamble_timeout()` when the wait must be unbounded.
- A client waits for a server greeting before sending anything (server-first
  protocols). Disabling the deadline does not by itself make the server speak
  first: a non-empty preamble is still read before the server writes, so the
  greeting must come from an outer layer that runs before the preamble read. A
  preamble that encodes to no bytes, such as a unit `()`, reads nothing and
  leaves the server free to greet from application code.

The two setters override each other and the last call wins. `Duration::ZERO`
still clamps to 1 ms, so use `disable_preamble_timeout()` rather than a zero
duration to remove the deadline. Disabling it restores the previous behaviour,
including the availability risk it carried.

<!-- tested-example: v0-3-0-to-v0-4-0-migration-guide-014 -->
```rust,no_run
use std::time::Duration;

use wireframe::{app::WireframeApp, server::WireframeServer};

let slower_handshake =
    WireframeServer::new(|| -> WireframeApp { WireframeApp::default() })
        .preamble_timeout(Duration::from_secs(30));

let server_first =
    WireframeServer::new(|| -> WireframeApp { WireframeApp::default() })
        .disable_preamble_timeout();
```

## Prepared application transition

`WireframeApp` remains the mutable builder. Register routes, middleware,
protocol hooks, and connection configuration on the builder, then consume it
with `prepare().await`:

<!-- tested-example: v0-3-0-to-v0-4-0-migration-guide-006 -->
```rust,no_run
use std::sync::Arc;

use wireframe::{
    app::{Envelope, Handler, WireframeApp},
    serializer::BincodeSerializer,
};

# async fn example() -> Result<(), Box<dyn std::error::Error>> {
let handler: Handler<Envelope> = Arc::new(|_envelope| Box::pin(async {}));
let app = WireframeApp::<BincodeSerializer>::new()?.route(1, handler)?;
let prepared = app.prepare().await?;
# let _ = prepared;
# Ok(())
# }
```

Preparation consumes the builder. It transforms every registered route's
middleware chain once and returns an immutable `PreparedApp` containing those
services and the runtime configuration. The builder's route-registration
methods are therefore unavailable after the transition; register all routes and
middleware before calling `prepare`.

`prepare` returns `Result<PreparedApp, PrepareError>`. Preparation can fail:
`PrepareError::MiddlewareTransform` reports a route middleware transform that
failed, and `PrepareError::InvalidPushConfig` reports a stored push-queue
template whose capacity or rate is unusable, which is rejected before any
connection exists rather than once per accepted connection. A failed
preparation does not expose a partially prepared application.

## Reuse the prepared application

Use the prepared connection methods for every accepted stream. Borrowing the
same `PreparedApp` lets multiple connections share the already-built route
services:

<!-- tested-example: v0-3-0-to-v0-4-0-migration-guide-007 -->
```rust,no_run
use tokio::io::duplex;
use wireframe::app::{PreparedApp, WireframeApp};

# async fn example() -> Result<(), Box<dyn std::error::Error>> {
let prepared: PreparedApp = WireframeApp::new()?.prepare().await?;

let (client_one, server_one) = duplex(64);
drop(client_one);
prepared.handle_connection_result(server_one).await?;

let (client_two, server_two) = duplex(64);
drop(client_two);
prepared.handle_connection_result(server_two).await?;
# Ok(())
# }
```

`PreparedApp::handle_connection_result` returns stream-processing and handler
I/O errors. `PreparedApp::handle_connection` is the logging convenience wrapper
when the caller does not need to inspect that result. The prepared application
is immutable and has no route-registration surface.

## Connection lifecycle finalization

When a setup hook produces state and a teardown hook is registered, teardown
now runs exactly once for every terminal outcome that resumes inside the
connection task: clean end of stream, decode and protocol errors, transport
read and write errors, and a handler panic. Previously a stream-processing
error skipped teardown.

A handler panic is finalized before it is re-raised to the connection-task
supervisor, so supervision still observes and logs it. The re-raise happens
only after teardown completes successfully: a teardown panic replaces the
handler panic observed by supervision, and supervision does not observe the
handler panic at all if teardown never completes. This recovery requires
unwinding; builds compiled with `panic = "abort"` cannot run teardown after a
handler panic.

A teardown callback that panics is invoked once, may not complete, and is not
retried. Teardown is not invoked when setup was absent, failed, or never
completed, nor when the connection future is dropped or aborted before
processing stops. Dropping the connection future after processing stops but
while teardown is awaiting interrupts the callback, so it may not complete.

## Update test drivers

Tests that use the `wireframe_testing` companion crate can prepare a builder
and drive one connection with `prepare_and_drive_with_frames`, or prepare once
and reuse the result with `drive_prepared_with_frames`:

<!-- tested-example: v0-3-0-to-v0-4-0-migration-guide-008 -->
```rust,ignore
use wireframe::app::WireframeApp;
use wireframe_testing::{drive_prepared_with_frames, prepare_and_drive_with_frames};

# async fn example() -> std::io::Result<()> {
let app = WireframeApp::new().map_err(std::io::Error::other)?;
let _response = prepare_and_drive_with_frames(app, Vec::new()).await?;

let app = WireframeApp::new().map_err(std::io::Error::other)?;
let prepared = app
    .prepare()
    .await
    .map_err(|error| std::io::Error::other(error.to_string()))?;
let _response = drive_prepared_with_frames(&prepared, Vec::new()).await?;
# Ok(())
# }
```

Both prepared helpers preserve custom `FrameCodec` types. Existing builder or
mutable drivers remain available as deprecated compatibility paths; migrate
tests to the prepared helpers when they need to prove one-time middleware
transformation or reuse prepared route services.

The mutable capacity-aware drivers `drive_with_frame_with_capacity_mut` and
`drive_with_frames_with_capacity_mut` are re-exported from the
`wireframe_testing` crate root, matching every other `_mut` driver:

<!-- tested-example: v0-3-0-to-v0-4-0-migration-guide-018 -->
```rust,no_run
use wireframe_testing::{
    drive_with_frame_with_capacity_mut,
    drive_with_frames_with_capacity_mut,
};
```

Both borrow the application, so one builder instance can be reused across calls
while the duplex buffer size is set explicitly.

## Benchmark allocation counting

`wireframe_testing::benchmark_alloc_support::CountingAllocator` has been
removed. The counting mechanism now comes from `stats_alloc`, which
`wireframe_testing::benchmark_alloc_support` re-exports as
`INSTRUMENTED_SYSTEM` and `StatsAlloc`, together with `std::alloc::System`.

Replace the old allocator declaration:

<!-- tested-example: v0-3-0-to-v0-4-0-migration-guide-015 -->
```rust,ignore
// Before
use wireframe_testing::benchmark_alloc_support::CountingAllocator;

#[global_allocator]
static GLOBAL_ALLOCATOR: CountingAllocator = CountingAllocator;
```

with the instrumenting allocator:

<!-- tested-example: v0-3-0-to-v0-4-0-migration-guide-016 -->
```rust
// After
use std::alloc::System;
use wireframe_testing::benchmark_alloc_support::{INSTRUMENTED_SYSTEM, StatsAlloc};

#[global_allocator]
static GLOBAL_ALLOCATOR: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;
```

The measurement helpers keep their names: `count_allocations`,
`allocation_count`, `reset_allocation_count`, and
`is_allocation_counting_enabled`. `count_allocations_or_panic` is new, and
closes the window before reporting a failed measurement for benchmark binaries
that treat one as fatal.

Counting semantics are unchanged: both fresh allocations and reallocations are
counted, exactly as the removed allocator counted them, so an existing baseline
stays comparable across the upgrade.

## Install your own tracing subscriber

`tracing-subscriber` is no longer a runtime dependency of Wireframe; it is a
development dependency used by the examples, benchmarks and tests. The
`tracing` facade Wireframe emits through stays a runtime dependency, but it has
no subscriber of its own, so a released application that relied on Wireframe
pulling in a subscriber must now declare one and install it during start-up.

Add the crate to the application's own manifest:

<!-- tested-example: v0-3-0-to-v0-4-0-migration-guide-009 -->
```toml
[dependencies]
tracing-subscriber = { version = "0.3.18", features = ["env-filter"] }
```

Then initialize it before any Wireframe work begins:

<!-- tested-example: v0-3-0-to-v0-4-0-migration-guide-010 -->
```rust
fn main() {
    tracing_subscriber::fmt().init();
    // Start the application here.
}
```

Applications that never configured a subscriber see no behaviour change: they
were discarding Wireframe's events before this release too. Applications that
did configure one keep working, because installing an application-owned
subscriber is now the supported arrangement rather than a duplicate.

## Migrate byte-handling APIs

The prepared-application transition is independent of the zero-copy byte
migration. Codecs already receive payloads as `bytes::Bytes`; what is still
planned is a `PayloadBytes` wrapper for read-only hand-offs and an explicit
edit-on-demand operation for mutation. Neither planned API is available in
v0.4. Middleware and hook editor APIs are not yet finalized either; their
migration is deferred to roadmap items 12.1.2 and 12.2.1. The compatibility
helper names described below are defined by
[ADR 009](adrs/adr-0009-rollout-strategy-for-the-vec-u8-to-zero-copy-migration.md).

### Middleware

The public edit-on-demand API for middleware requests and responses is not yet
finalized. Continue using the current `frame_mut()` and `into_inner()`
compatibility methods while this migration is tracked by roadmap item 12.1.2.
Do not assume a response-editor method or introduce an editor method until that
API is implemented and documented. Read-only middleware should avoid editing
the frame altogether.

### Protocol and client hooks

The hook editor API is also deferred to roadmap item 12.2.1. Keep existing
`Vec<u8>` hook implementations until that API is finalized; client preamble
leftovers intentionally remain `Vec<u8>` in this release. The compatibility
policy is defined in
[ADR 009](adrs/adr-0009-rollout-strategy-for-the-vec-u8-to-zero-copy-migration.md).

### Serializers

The following serializer migration is proposed and is not available in v0.4.
`Serializer::serialize` currently returns `Vec<u8>`. The example shows a
possible future transition to a stable byte wrapper: use
`PayloadBytes::from_vec` only at an existing compatibility boundary, and keep
the zero-copy value through the codec hand-off.

<!-- tested-example: v0-3-0-to-v0-4-0-migration-guide-011 -->
```text
# Before: serialization materializes a Vec<u8> for every outbound message.
let bytes: Vec<u8> = serializer.serialize(&message)?;
let frame = codec.wrap_payload(bytes::Bytes::from(bytes));

# Proposed: the serializer returns the stable shared byte representation.
let bytes: PayloadBytes = serializer.serialize(&message)?;
let frame = codec.wrap_payload(bytes.into_bytes());

# Proposed compatibility option, if implemented.
let bytes: Vec<u8> = serializer.serialize_to_vec(&message)?;
```

`PayloadBytes` is not implemented. Whether a `serialize_to_vec` compatibility
shim will ship remains undecided; `PayloadBytes::into_vec`, if it is added, is
an escape hatch rather than the normal transport path.

### Custom codecs

Codecs should store payloads as `Bytes` when possible and override
`frame_payload_bytes` to return a cheap clone. The `wrap_payload` argument is
already `Bytes`, so only the frame type and extraction methods need changing:

<!-- tested-example: v0-3-0-to-v0-4-0-migration-guide-012 -->
```rust,ignore
// Before: a custom frame owns a Vec<u8> payload.
struct MyEnvelope {
    payload: Vec<u8>,
}

// After: the frame shares its payload buffer with the codec driver.
use bytes::Bytes;

struct MyEnvelope {
    payload: Bytes,
}

impl FrameCodec for MyCodec {
    type Frame = MyEnvelope;

    fn frame_payload(frame: &MyEnvelope) -> &[u8] { &frame.payload }

    fn frame_payload_bytes(frame: &MyEnvelope) -> Bytes { frame.payload.clone() }

    fn wrap_payload(&self, payload: Bytes) -> MyEnvelope { MyEnvelope { payload } }
}
```

Keep `Vec<u8>` conversion at the edge of legacy callers; do not add per-codec
conversion constructors. The `PayloadBytes::from_vec` and
`PayloadBytes::into_vec` helpers named by
[ADR 009](adrs/adr-0009-rollout-strategy-for-the-vec-u8-to-zero-copy-migration.md)
are proposed and are not available in v0.4. See
[ADR 008](adrs/adr-0008-zero-copy-public-byte-container.md) for the read-only
and edit-on-demand design, and the
[zero-copy migration roadmap](zero-copy-frame-and-payload-migration-roadmap.md)
for the staged rollout.

## Inspect fragment metadata in const code

Fragment metadata constructors and read-only accessors are now `const`, so
const function bodies can call them. This does not make every result usable as
an evaluated const value: `FragmentFrame`, `FragmentParts`, and
`ReassembledMessage` own `Vec<u8>` values, and const items that call their
accessors fail with E0493. `FragmentBatch` is also produced by `Fragmenter` at
runtime, but its `fragments()`, `len()`, `is_empty()`, and `is_fragmented()`
accessors are const-callable from helper functions that receive a batch
reference.

An empty payload is represented by one final fragment with empty bytes, so its
batch is not empty. Use `is_empty()` to inspect a batch's stored frame
collection, and inspect the frame payload when the input payload's length is
what matters.

## Server factory compatibility

`WireframeServer::new` now evaluates its `AppFactory` once for each
`run_with_shutdown` invocation, prepares the resulting application once before
readiness, and shares one immutable prepared root across all workers and
connection tasks. Move connection-specific state to lifecycle setup state `C`
through `on_connection_setup` rather than constructing it in the application
factory.

Use `WireframeServer::from_app(app)` when the application is already built. The
server still prepares that application when it starts, so the same readiness
guarantee applies. The constructor only supplies the application to the server;
it does not initialize tracing or create a Tokio runtime. Configure server
settings through the usual builder methods.

If a factory previously created a fresh builder for every connection, update
the construction path as follows:

<!-- tested-example: v0-3-0-to-v0-4-0-migration-guide-013 -->
```rust,ignore
// One-time startup construction through the factory.
let server = WireframeServer::new(build_startup_app).bind(addr)?;

// Or construct the application before creating the server.
let app = build_startup_app()?;
let server = WireframeServer::from_app(app).bind(addr)?;
```

## Protocol hooks on application responses

`WireframeApp` handler responses now pass through the protocol's `before_send`
hook. Previously only actor-driven push and multi-packet frames did so, and
ordinary request/response traffic bypassed the hook.

The hook runs per connection, after the response is serialized and wrapped by
the `FrameCodec`, and immediately before the frame is written. It receives the
serialized, codec-wrapped frame before codec encoding, so a custom encoder may
still add its own header or trailer after the hook returns:

<!-- tested-example: v0-3-0-to-v0-4-0-migration-guide-017 -->
```text
serialize -> FrameCodec::wrap_payload -> before_send -> codec encode -> socket
```

Each connection constructs its own `ProtocolHooks` and `ConnectionContext`, so
per-connection context and callback instances are not shared. State held by a
shared `WireframeProtocol` implementation is still shared by every connection
that installs it. A hook that appends a marker now marks every handler
response, including every fragment of a fragmented response.

Hook output must fit the codec's `max_frame_length`. The pipeline does not
check this limit. The default `LengthDelimitedFrameCodec` rejects an oversized
frame, so the send fails and the frame is not written; a custom encoder defines
its own behaviour for an oversized frame, so it must enforce `max_frame_length`
itself.

Only the direct response helpers (`send_response`, `send_response_framed`, and
`send_response_framed_with_codec`) bypass the hook. Push traffic submitted
directly to a `ConnectionActor` runs `before_send` from the actor's own hooks.
On a prepared server connection, handler responses — including streamed and
multi-packet bodies — are written through the connection's `FramePipeline`, and
the actor built for that connection deliberately has no `before_send` hook of
its own, so the hook runs exactly once per frame. See the
[users' guide](users-guide.md) for the user-facing contract.

`on_command_end` now fires once for every completed handler response on a
prepared server connection, including an ordinary `route` reply that previously
only emitted bytes. A protocol that releases per-request state in that hook
therefore sees one call per completed reply, not only for actor-driven push and
multi-packet traffic. The hook is not a guaranteed cleanup point: a response
cancelled by the peer-close grace skips it, and a terminator write that fails
returns before it, so per-request state that must always be released needs a
separate fallback.

Prepared connections also require `S: 'static` on the serializer.
`PreparedApp::handle_connection_result` and the deprecated
`WireframeApp::handle_connection*` methods now carry this bound, so a
serializer that borrows connection-local data can no longer be used with them.

## Response-producing routes and multi-packet correlation

`WireframeApp::route_response` registers a handler that returns a whole
`Response` rather than raw bytes, so a route can stream or send several frames
instead of exactly one reply. `ResponseHandler<E>` is re-exported from
`wireframe::app`, and route identifiers stay unique across `route` and
`route_response`. `E` is the request type: the handler receives a borrowed
request of that type and may hold the borrow across an `await`, and it must
return `Response<Envelope, ()>` rather than bytes.

<!-- tested-example: v0-3-0-to-v0-4-0-migration-guide-021 -->
```rust,no_run
use std::sync::Arc;

use wireframe::app::{Envelope, ResponseHandler};
use wireframe::response::Response;

fn handler() -> ResponseHandler<Envelope> {
    Arc::new(|request: &Envelope| {
        let payload = request.payload_bytes().to_vec();
        Box::pin(async move { Response::Single(Envelope::new(1, None, payload)) })
    })
}
```

A route registered with `route` is unchanged: its bytes are wrapped in one
envelope and sent as a single reply.

Correlation behaviour for direct `ConnectionActor` multi-packet traffic has
also changed. When the request carries no correlation identifier, a frame that
supplies its own keeps it instead of having `None` written over it. A frame
that supplies a different identifier than the request is still overwritten and
logged as a warning. On a prepared server connection the protocol terminator is
written without decoding or re-wrapping, so the adapter does not replace the
identifier produced by `stream_end_frame` for either streamed or multi-packet
responses (`before_send` can still rewrite the frame); see the
[users' guide](users-guide.md#response-producing-routes) for the full contract.

## Decoder recovery on live connections

A decoder failure on a live connection no longer always ends the connection
without consultation. Install a `RecoveryPolicyHook` with
`WireframeApp::with_recovery_hook`, and tune the global recovery limits with
`WireframeApp::recovery_config`. The connection loop then asks the hook for a
policy on each failure, applies the documented fallbacks, and reports the
policy it actually applied.

With the `metrics` feature enabled — it is a default feature — every decoder
failure increments `wireframe_codec_errors_total` once, labelled with the error
type and the applied policy, so a dashboard can separate framing, protocol,
I/O, and EOF failures. With no hook installed, the applied policy is still
`Disconnect`: an application that installs no hook keeps the previous behaviour.

One failure is reported without consulting the hook. When a response ends the
connection, reading stops with a failure still buffered behind it, and no
selected policy can take effect on a closing connection. That failure is
reported as the `Disconnect` it actually is, so `on_error` and the counter
still observe it exactly once.

The built-in length-delimited codec now classifies an oversized frame as
`error_type="framing"` rather than `"io"`. A dashboard or alert that filtered on
`error_type="io"` must also match `framing` to keep counting those failures.

See [codec error handling](users-guide.md#codec-error-handling) in the users'
guide for the error taxonomy, the default policy per error type, and the full
decision order and fallbacks.

## Push queues on server connections

Server connections now have a push path, so code that drove a `ConnectionActor`
by hand only to push frames can configure the application instead.
`WireframeApp::with_push_queues` takes a `PushQueuesBuilder` template that is
cloned once per accepted connection, and
`WireframeProtocol::on_connection_setup` receives that connection's
`PushHandle` on the prepared server path. An invalid capacity or rate is
reported when the application is prepared, so check that result rather than
expecting a per-connection failure.

`ConnectionContext` is no longer a unit struct. It is `#[non_exhaustive]`,
built by the server, and exposes `connection_id()`. Replace `ConnectionContext`
literals with `ConnectionContext::default()`, and read the connection's
identifier from the context rather than allocating one.

`WireframeApp::with_push_dlq` is deprecated since 0.4.0. Its sink receives
serialized payloads, is not wired to connection queues, and only warns when
configured. Pass a `dlq` sender to `PushQueues::builder()` inside
`with_push_queues` instead, so dropped frames arrive as codec frames.

`SessionRegistry` now allocates connection identifiers from `1`, increasing
monotonically, and never reuses one. A server connection claims its identifier
and publishes its handle in one step with `allocate_and_publish`, before its
setup callback runs, and removes that entry when the connection task ends, so
an application that shares a registry no longer inserts or removes entries
itself. `allocate_connection_id` remains available for an application that
needs an identifier of its own.

An application that still needs entries of its own, as earlier releases
required, should keep them in a separate registry. A shared registry also keeps
the two sets apart: the claim skips an identifier a live entry occupies, so the
first server connection receives `1` only when nothing else holds it, and
`insert` refuses to replace a live entry, so a later manual insert cannot
displace a server connection's handle. Because the claim is atomic, a
connection is never published under an identifier that already holds another
live handle. An application can still take one over deliberately, by removing
the entry and inserting its own handle in its place: the connection then simply
stops being addressable under that identifier. Teardown removes an entry only
while it still holds that connection's own handle, so an identifier an
application took over after a `remove` survives the connection's end. A
producer that moves from a hand-picked identifier to the server registry must
address connections by the identifier the connection publishes — read it from
`ConnectionContext::connection_id` in the setup callback, or enumerate
`active_handles` — rather than reusing the identifier it chose itself.

A clean peer close attempts to drain frames still queued, high priority first,
though a write that fails or is abandoned drops the frames behind it. On server
shutdown a running handler keeps its push lanes open until it has delivered its
first push batch, so a push accepted before then is queued rather than refused.
Delivery is still only attempted: the run's write is raced against the stop
token, so a write that is blocked when the server stops is abandoned, and a
handler response that becomes ready first ends the run with the push still
queued. After that batch, shutdown discards the queued frames and closes the
lanes, and later pushes fail with `PushError::Closed`. See the
[users' guide](users-guide.md#server-connections-and-push-queues) and
[ADR 014](adrs/adr-0014-server-shutdown-ownership-and-terminal-observation.md).
