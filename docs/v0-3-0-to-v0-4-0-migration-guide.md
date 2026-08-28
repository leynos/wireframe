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

## Server factory compatibility

`WireframeServer` continues to accept an `AppFactory` and retains its existing
factory-evaluation semantics in this release. Applications that construct a
fresh builder per connection therefore do not automatically share a prepared
application. Preparing the application factory before server readiness, and
moving server connection tasks onto a prepared root, are tracked separately in
[issue #642](https://github.com/leynos/wireframe/issues/642).
