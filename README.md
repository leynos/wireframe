# Wireframe

[![Ask DeepWiki](https://deepwiki.com/badge.svg)](
https://deepwiki.com/leynos/wireframe)

**Wireframe** is an experimental Rust library that simplifies building servers
and clients for custom binary protocols. The design borrows heavily from
[Actix Web](https://actix.rs/) to provide a familiar, declarative API for
routing and middleware.

## Motivation

Manual handling of binary protocols typically involves verbose serialization
code, custom frame parsing, and complex dispatch logic. `wireframe` aims to
reduce this boilerplate through layered abstractions:

- **Transport adapter** built on Tokio I/O
- **Framing layer** for length‑prefixed or custom frames
- **Connection preamble** with customizable validation callbacks [^1]
- Call `with_preamble::<T>()` before registering success or failure callbacks
- **Serialization engine** using `bincode` or a `wire-rs` wrapper
- **Routing engine** that dispatches messages by ID
- **Handler invocation** with access to the routed envelope
- **Middleware chain** for request/response processing
- **[Connection lifecycle hooks](#connection-lifecycle)** for per-connection
  setup and teardown

These layers correspond to the architecture outlined in the design document[^2].

## API overview

Applications are configured using a builder pattern similar to Actix Web. A
`WireframeApp` defines routes and middleware, while `WireframeServer` manages
connections and runs the Tokio event loop:

```rust
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use wireframe::{
    app::{Envelope, Handler, WireframeApp},
    message::Message,
    serializer::BincodeSerializer,
    server::WireframeServer,
};

const LOGIN: u32 = 1;

#[derive(bincode::Encode, bincode::BorrowDecode)]
struct LoginRequest {
    username: String,
}

let attempts = Arc::new(AtomicU64::new(0));
let handler: Handler<Envelope> = {
    let attempts = Arc::clone(&attempts);
    Arc::new(move |envelope: &Envelope| {
        let payload = envelope.payload_bytes().to_vec();
        let attempts = Arc::clone(&attempts);
        Box::pin(async move {
            if let Ok((login, _)) = LoginRequest::from_bytes(&payload) {
                attempts.fetch_add(1, Ordering::Relaxed);
                tracing::info!(user = %login.username, "login attempt");
            }
        })
    })
};

WireframeServer::new(move || {
    WireframeApp::new()?
        .serializer(BincodeSerializer)
        .route(LOGIN, Arc::clone(&handler))
})
.bind("127.0.0.1:7878")?
.run()
.await
```

By default, the number of worker tasks equals the number of CPU cores. If the
CPU count cannot be determined, the server falls back to a single worker.

`Handler<E>` is a shared closure that receives the routed envelope and returns
a boxed future. There is no type-based parameter resolution step: a route
handler receives only `&E`, and the application echoes the envelope after the
handler completes. Shared state reaches a handler by capturing an `Arc` in the
closure, as the example does, so ownership and lifetime stay explicit[^3].

## Example

A minimal echo server routes one message ID and lets the application return the
envelope unchanged:

```rust
use std::sync::Arc;

use wireframe::{
    app::{Envelope, Handler, WireframeApp},
    serializer::BincodeSerializer,
    server::WireframeServer,
};

let handler: Handler<Envelope> = Arc::new(|_: &Envelope| {
    Box::pin(async {
        tracing::info!("echo request received");
        // `WireframeApp` echoes the envelope after the handler completes.
    })
});

WireframeServer::new(move || {
    WireframeApp::new()?
        .serializer(BincodeSerializer)
        .route(1, Arc::clone(&handler))
})
.bind("127.0.0.1:8000")?
.run()
.await
```

This mirrors [`examples/echo.rs`](examples/echo.rs), which shows the complete
runnable server and its shutdown handling.

## Custom envelopes

`WireframeApp` defaults to a simple `Envelope` containing a message ID and raw
payload bytes. Applications can supply their own envelope type by calling
`WireframeApp::<_, _, MyEnv>::new()`. The custom type must implement the
`Packet` trait:

```rust
use wireframe::{
    app::{Packet, PacketParts, WireframeApp},
    correlation::CorrelatableFrame,
};

#[derive(bincode::Encode, bincode::BorrowDecode)]
struct MyEnv { id: u32, correlation_id: Option<u64>, payload: Vec<u8> }

impl CorrelatableFrame for MyEnv {
    fn correlation_id(&self) -> Option<u64> { self.correlation_id }
    fn set_correlation_id(&mut self, id: Option<u64>) { self.correlation_id = id; }
}

impl Packet for MyEnv {
    fn id(&self) -> u32 { self.id }
    fn into_parts(self) -> PacketParts {
        PacketParts::new(self.id, self.correlation_id, self.payload)
    }
    fn from_parts(parts: PacketParts) -> Self {
        let id = parts.id();
        let correlation_id = parts.correlation_id();
        let payload = parts.payload();
        Self { id, correlation_id, payload }
    }
}

let app = WireframeApp::<_, _, MyEnv>::new()
    .unwrap()
    .route(1, std::sync::Arc::new(|env: &MyEnv| Box::pin(async move { /* ... */ })))
    .unwrap();
```

A `None` correlation ID denotes an unsolicited event or server-initiated push.
Use `None` rather than `Some(0)` when a frame lacks a correlation ID. See
[PacketParts](docs/api.md#packetparts) for field details.

This allows integration with existing packet formats without modifying
`handle_frame`.

## Response serialization and framing

A route handler returns `()`, and the application echoes the routed envelope.
To send a different reply, call `send_response` or `send_response_framed`. Both
are methods on `WireframeApp` and need a writer that the caller owns and
implements `AsyncWrite + Unpin`. A `Handler<E>` receives only `&E`, so a
handler cannot call them itself; the code that owns the connection makes the
call. Each helper encodes the value with the application's configured
serializer and reports serialization or I/O failures as `SendError`[^4].

```rust
use tokio::net::TcpStream;
use wireframe::app::{SendError, WireframeApp};

#[derive(bincode::Encode, bincode::BorrowDecode)]
struct Ready(u8);

// `server` is the caller-owned writer, for example an accepted connection.
async fn announce(app: &WireframeApp, server: &mut TcpStream) -> Result<(), SendError> {
    app.send_response(server, &Ready(1)).await
}
```

Frames are length prefixed using `tokio_util::codec::LengthDelimitedCodec`. The
prefix length and byte order are configurable and default to a 4‑byte
big‑endian header[^5].

## Push Queues

Push queues buffer frames before they are written to a connection. Configure
them with capacities, rate limits, and an optional dead-letter queue:

```rust,no_run
use tokio::sync::mpsc;
use wireframe::push::PushQueues;

# async fn demo() {
let (dlq_tx, _dlq_rx) = mpsc::channel(8);
let (_queues, _handle) = PushQueues::<u8>::builder()
    .high_capacity(8)
    .low_capacity(8)
    .rate(Some(100))
    .dlq(Some(dlq_tx)) // frames drop if the DLQ is absent or full
    .build()
    .expect("failed to build PushQueues");
# drop((_queues, _handle));
# }
```

Disable throttling with the `unlimited` convenience:

```rust,no_run
use wireframe::push::PushQueues;
let (_queues, _handle) = PushQueues::<u8>::builder()
    .high_capacity(8)
    .low_capacity(8)
    .unlimited()
    .build()
    .expect("failed to build PushQueues");
```

## Connection Lifecycle

Protocol callbacks are consolidated under the `WireframeProtocol` trait,
replacing the individual `on_connection_setup`/`on_connection_teardown`
closures. The trait methods are synchronous so the trait remains object safe,
but callbacks can spawn asynchronous tasks when needed. A protocol
implementation registers hooks for connection setup, frame mutation and command
completion. The associated `ProtocolError` type is used by other parts of the
API, such as request handling.

```rust
pub trait WireframeProtocol: Send + Sync + 'static {
    type Frame: FrameLike;
    type ProtocolError;

    fn on_connection_setup(
        &self,
        handle: PushHandle<Self::Frame>,
        ctx: &mut ConnectionContext,
    );

    fn before_send(&self, frame: &mut Self::Frame, ctx: &mut ConnectionContext);

    fn on_command_end(&self, ctx: &mut ConnectionContext);
}

struct MySqlProtocolImpl;

impl WireframeProtocol for MySqlProtocolImpl {
    type Frame = Vec<u8>;
    type ProtocolError = ();

    fn on_connection_setup(
        &self,
        handle: PushHandle<Self::Frame>,
        _ctx: &mut ConnectionContext,
    ) {
        // Spawn an async task to send a heartbeat after setup
        tokio::spawn(async move {
            let _ = handle.push_high_priority(b"ping".to_vec()).await;
        });
    }

    fn before_send(&self, _frame: &mut Self::Frame, _ctx: &mut ConnectionContext) {}

    fn on_command_end(&self, _ctx: &mut ConnectionContext) {}
}

```

```rust
let app = WireframeApp::new().with_protocol(MySqlProtocolImpl);
```

## Session registry

The \[`SessionRegistry`\] stores weak references to \[`PushHandle`\]s for
active connections. Background tasks can look up a handle by \[`ConnectionId`\]
to send frames asynchronously without keeping the connection alive. Entries are
pruned on lookup and when calling `active_handles()`. `DashMap::retain` holds
per-bucket write locks while collecting, so heavy traffic may experience
contention. Invoke `prune()` from a maintenance task when only removal of dead
entries is required, without collecting handles.

```rust
use wireframe::{
    session::{ConnectionId, SessionRegistry},
    push::PushHandle,
    ConnectionContext,
};

let registry: SessionRegistry<MyFrame> = SessionRegistry::default();

// inside a `WireframeProtocol` implementation
fn on_connection_setup(&self, handle: PushHandle<MyFrame>, _ctx: &mut ConnectionContext) {
    let id = ConnectionId::new(42);
    registry.insert(id, &handle);
}
```

## Decoding requests

Handlers own request decoding. Use `message::Message::from_bytes` for a bincode
payload, or call the matching serializer when a protocol uses another format.
Validation that would otherwise repeat across handlers belongs in a plain
helper function:

```rust
use wireframe::{app::Envelope, message::Message};

#[derive(bincode::Encode, bincode::BorrowDecode)]
struct SessionToken(String);

fn parse_token(envelope: &Envelope) -> Option<SessionToken> {
    let (token, _) = SessionToken::from_bytes(envelope.payload_bytes()).ok()?;
    (!token.0.is_empty()).then_some(token)
}
```

The envelope remains borrowed for the whole handler call, so a decode helper
can read the payload without copying it unless the decoded type owns its data.

## Middleware

Middleware allows inspecting or modifying requests and responses. The `from_fn`
helper builds middleware from an async function or closure:

```rust
use wireframe::middleware::from_fn;

let logging = from_fn(|req, next| async move {
    tracing::info!("received request: {:?}", req);
    let res = next.call(req).await?;
    tracing::info!("sending response: {:?}", res);
    Ok(res)
});
```

## Examples

Example programs are available in the `examples/` directory:

- `echo.rs` — minimal echo server using routing
- `ping_pong.rs` — showcases serialization and middleware in a ping/pong
  protocol. See [examples/ping_pong.md](examples/ping_pong.md) for a detailed
  overview.
- [`packet_enum.rs`](examples/packet_enum.rs) — shows packet type discrimination
  with a bincode enum and a frame containing container types like `HashMap` and
  `Vec`.

Run an example with Cargo:

```bash
cargo run --example echo
```

Try the echo server with netcat:

```bash
$ cargo run --example echo
# in another terminal
$ printf '\x00\x00\x00\x00\x01\x00\x00\x00' | nc 127.0.0.1 7878 | xxd
```

Try the ping‑pong server with netcat:

```bash
$ cargo run --example ping_pong
# in another terminal
$ printf '\x00\x00\x00\x08\x01\x00\x00\x00\x2a\x00\x00\x00' | nc 127.0.0.1 7878 | xxd
```

## Current limitations

Connection handling now processes frames and routes messages. Although the
server is still experimental, it now compiles in release mode for evaluation or
production use.

## Roadmap

Development priorities are tracked in [docs/roadmap.md](docs/roadmap.md). Key
tasks include building the Actix‑inspired API, implementing middleware, and
providing example applications[^6].

## Licence

Wireframe is distributed under the terms of the ISC licence. See
[LICENSE](LICENSE) for details.

[^1]: [docs/preamble-validator.md](docs/preamble-validator.md)
[^2]: [design §4.2](docs/rust-binary-router-library-design.md#42-high-level-architecture)
[^3]: [design §5.2](docs/rust-binary-router-library-design.md#52-handler-functions)
[^4]: [design §4.4](docs/rust-binary-router-library-design.md#44-message-serialization-and-deserialization)
[^5]: [design §4.3](docs/rust-binary-router-library-design.md#43-frame-definition-and-processing)
[^6]: [docs/roadmap.md](docs/roadmap.md)
