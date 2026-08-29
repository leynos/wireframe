# v0.3.0 to v0.4.0 migration guide

This guide covers the prepared-application transition for applications that
drive accepted streams directly. It explains how to move route and middleware
setup out of connection handling while retaining the existing server factory
workflow.

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

## Server factory compatibility

`WireframeServer` continues to accept an `AppFactory` and retains its existing
factory-evaluation semantics in this release. Applications that construct a
fresh builder per connection therefore do not automatically share a prepared
application. Preparing the application factory before server readiness, and
moving server connection tasks onto a prepared root, are tracked separately in
[issue #642](https://github.com/leynos/wireframe/issues/642).
