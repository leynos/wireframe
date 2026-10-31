# v0.3.0 to v0.4.0 migration guide

This guide covers the prepared-application transition for applications that
drive accepted streams directly. It explains how to move route and middleware
setup out of connection handling while retaining the existing server factory
workflow.

## Client error matching

`ClientError` is now marked `#[non_exhaustive]`, and pooled acquisitions can
return `ClientError::PoolSaturated { limit }` when the configured waiter bound
is full. Add a wildcard arm to exhaustive matches and handle saturation at the
caller boundary. For example:

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

## Preamble deadline

`WireframeServer::new` and `WireframeServer::from_app` now bound the preamble
read with `DEFAULT_PREAMBLE_TIMEOUT`, a 10-second deadline. Previously the read
was unbounded unless `preamble_timeout` was set, so a peer that connected and
never sent its preamble kept a connection task alive indefinitely. The deadline
covers only the read; success and failure callbacks and application message
handling run outside it.

Most clients send their preamble immediately and need no change. Adjust the
configuration when:

- A client waits for a server greeting before sending anything (server-first
  protocols). Call `disable_preamble_timeout()` so the server does not close
  the connection first.
- A legitimate handshake exchanges several messages and can exceed 10 seconds.
  Call `preamble_timeout(Duration)` with a larger, explicit bound.

The two setters override each other and the last call wins. `Duration::ZERO`
still clamps to 1 ms, so use `disable_preamble_timeout()` rather than a zero
duration to remove the deadline. Disabling it restores the previous behaviour,
including the availability risk it carried.

```rust,no_run
use std::time::Duration;

use wireframe::{app::WireframeApp, server::WireframeServer};

let slower_handshake = WireframeServer::new(|| WireframeApp::default())
    .preamble_timeout(Duration::from_secs(30));

let server_first = WireframeServer::new(|| WireframeApp::default())
    .disable_preamble_timeout();
```

## Prepared application transition

`WireframeApp` remains the mutable builder. Register routes, middleware,
protocol hooks, and connection configuration on the builder, then consume it
with `prepare().await`:

```rust,no_run
use std::sync::Arc;

use wireframe::app::{Envelope, Handler, WireframeApp};

# async fn example() -> Result<(), Box<dyn std::error::Error>> {
let handler: Handler<Envelope> = Arc::new(|_envelope| Box::pin(async {}));
let app = WireframeApp::new()?.route(1, handler)?;
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

`prepare` returns `Result<PreparedApp, PrepareError>`. Preparation is currently
infallible, but the typed error provides a stable place for callers to handle
future fallible middleware or runtime preparation steps. A failed preparation
does not expose a partially prepared application.

## Reuse the prepared application

Use the prepared connection methods for every accepted stream. Borrowing the
same `PreparedApp` lets multiple connections share the already-built route
services:

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

## Update test drivers

Tests that use the `wireframe_testing` companion crate can prepare a builder
and drive one connection with `prepare_and_drive_with_frames`, or prepare once
and reuse the result with `drive_prepared_with_frames`:

```rust,no_run
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

## Install your own tracing subscriber

`tracing-subscriber` is no longer a runtime dependency of Wireframe; it is a
development dependency used by the examples, benchmarks and tests. The
`tracing` facade Wireframe emits through stays a runtime dependency, but it has
no subscriber of its own, so a released application that relied on Wireframe
pulling in a subscriber must now declare one and install it during start-up.

Add the crate to the application's own manifest:

```toml
[dependencies]
tracing-subscriber = { version = "0.3.18", features = ["env-filter"] }
```

Then initialize it before any Wireframe work begins:

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
migration. The v0.4 byte-facing APIs use `bytes::Bytes` (or the `PayloadBytes`
wrapper) for read-only hand-offs and an explicit edit-on-demand operation for
mutation. Middleware and hook editor APIs are not yet finalized; their
migration is deferred to roadmap items 12.1.2 and 12.2.1. The compatibility
helper names described below are defined by
[ADR 009](adr-009-vec-u8-migration-rollout.md).

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
policy is defined in [ADR 009](adr-009-vec-u8-migration-rollout.md).

### Serializers

Serializer output moves from an owned vector to the stable byte wrapper. Use
`PayloadBytes::from_vec` only at an existing compatibility boundary, and keep
the zero-copy value through the codec hand-off:

```text
# Before: serialization materializes a Vec<u8> for every outbound message.
let bytes: Vec<u8> = serializer.serialize(&message)?;
let frame = codec.wrap_payload(bytes::Bytes::from(bytes));

# After: the serializer returns the stable shared byte representation.
let bytes: PayloadBytes = serializer.serialize(&message)?;
let frame = codec.wrap_payload(bytes.into_bytes());

# Compatibility only: an older caller that still requires Vec<u8>.
let bytes: Vec<u8> = serializer.serialize_to_vec(&message)?;
```

`serialize_to_vec` is a temporary compatibility shim where provided; new code
should consume `PayloadBytes` directly. `PayloadBytes::into_vec` is likewise an
escape hatch, not the normal transport path.

### Custom codecs

Codecs should store payloads as `Bytes` when possible and override
`frame_payload_bytes` to return a cheap clone. The `wrap_payload` argument is
already `Bytes`, so only the frame type and extraction methods need changing:

```rust
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

Keep `Vec<u8>` conversion at the edge of legacy callers with
`PayloadBytes::from_vec` or `PayloadBytes::into_vec`; do not add per-codec
conversion constructors. See
[ADR 008](adr-008-zero-copy-public-byte-container.md) for the read-only and
edit-on-demand design, and the
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

```rust
// One-time startup construction through the factory.
let server = WireframeServer::new(build_startup_app).bind(addr)?;

// Or construct the application before creating the server.
let app = build_startup_app()?;
let server = WireframeServer::from_app(app).bind(addr)?;
```
