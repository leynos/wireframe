# Wireframe

**Wireframe** is an experimental Rust library that simplifies building servers
and clients for custom binary protocols. The design borrows heavily from
[Actix Web](https://actix.rs/) to provide a familiar, declarative API for
routing, extractors, and middleware.

## Motivation

Manual handling of binary protocols typically involves verbose serialization
code, custom frame parsing, and complex dispatch logic. `wireframe` aims to
reduce this boilerplate through layered abstractions:

- **Transport adapter** built on Tokio I/O
- **Framing layer** for length‑prefixed or custom frames
- **Connection preamble** with customizable validation callbacks \[[docs](docs/preamble-validator.md)\]
- Call `with_preamble::<T>()` before registering success or failure callbacks
- **Serialization engine** using `bincode` or a `wire-rs` wrapper
- **Routing engine** that dispatches messages by ID
- **Handler invocation** with extractor support
- **Middleware chain** for request/response processing
- **[Connection lifecycle hooks](#connection-lifecycle)** for per-connection
  setup and teardown

These layers correspond to the architecture outlined in the design
document【F:docs/rust-binary-router-library-design.md†L292-L344】.

## API Overview

Applications are configured using a builder pattern similar to Actix Web. A
`WireframeApp` defines routes and middleware, while `WireframeServer` manages
connections and runs the Tokio event loop:

```rust
WireframeServer::new(|| {
    WireframeApp::new()
        .frame_processor(MyFrameProcessor::new())
        .app_data(state.clone())
        .route(MessageType::Login, handle_login)
        .wrap(MyLoggingMiddleware::default())
})
.bind("127.0.0.1:7878")?
.run()
.await
```

By default, the number of worker tasks equals the number of CPU cores. If the
CPU count cannot be determined, the server falls back to a single worker.

The builder supports methods like `frame_processor`, `route`, `app_data`, and
`wrap` for middleware configuration. `app_data` stores any `Send + Sync` value
keyed by type; registering another value of the same type overwrites the
previous one. Handlers retrieve these values using the `SharedState<T>`
extractor【F:docs/rust-binary-router-library-design.md†L616-L704】.

Handlers are asynchronous functions whose parameters implement extractor traits
and may return responses implementing the `Responder` trait. This pattern
mirrors Actix Web handlers and keeps protocol logic concise【F:docs/rust-binary-router-library-design.md†L676-L704】.

## Example

The design document includes a simple echo server that demonstrates routing
based on a message ID and the use of a length‑prefixed frame processor:

```rust
async fn handle_echo(req: Message<EchoRequest>) -> WireframeResult<EchoResponse> {
    Ok(EchoResponse {
        original_payload: req.payload.clone(),
        echoed_at: time_now(),
    })
}

WireframeServer::new(|| {
    WireframeApp::new()
        .serializer(BincodeSerializer)
        .route(MyMessageType::Echo, handle_echo)
})
.bind("127.0.0.1:8000")?
.run()
.await
```

This example showcases how derive macros and the framing abstraction simplify a
binary protocol server【F:docs/rust-binary-router-library-design.md†L1120-L1150】.

## Response Serialization and Framing

Handlers can return types implementing the `Responder` trait. These values are
encoded using the application's configured serializer and written
back through the `FrameProcessor`【F:docs/rust-binary-router-library-design.md†L718-L724】.

The included `LengthPrefixedProcessor` illustrates a simple framing strategy
based on a big‑endian length prefix【F:docs/rust-binary-router-library-design.md†L1076-L1117】.

## Connection Lifecycle

`WireframeApp` can run callbacks when a connection is opened or closed. The state
produced by `on_connection_setup` is passed to `on_connection_teardown` when the
connection ends.

```rust
let app = WireframeApp::new()
    .on_connection_setup(|| async { 42u32 })
    .on_connection_teardown(|state| async move {
        println!("closing with {state}");
    });
```

## Current Limitations

Connection processing is not implemented yet. After the optional
preamble is read, the server logs a warning and immediately closes the
stream. Release builds fail to compile to prevent accidental production
use.

## Roadmap

Development priorities are tracked in [docs/roadmap.md](docs/roadmap.md). Key
tasks include building the Actix‑inspired API, implementing middleware and
extractor traits, and providing example applications【F:docs/roadmap.md†L1-L24】.

## License

Wireframe is distributed under the terms of the ISC license.
See [LICENSE](LICENSE) for details.
