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
- **Serialization engine** using `bincode` or a `wire-rs` wrapper
- **Routing engine** that dispatches messages by ID
- **Handler invocation** with extractor support
- **Middleware chain** for request/response processing

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
        .app_data(SharedState::new(state.clone()))
        .route(MessageType::Login, handle_login)
        .wrap(MyLoggingMiddleware::default())
})
.bind("127.0.0.1:7878")?
.run()
.await
```

By default, the number of worker tasks equals the number of CPU cores.

The builder supports methods like `frame_processor`, `route`, `app_data`, and
`wrap` for middleware configuration【F:docs/rust-binary-router-library-design.md†L616-L704】.

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
        .serialization_format(SerializationFormat::Bincode)
        .route(MyMessageType::Echo, handle_echo)
})
.bind("127.0.0.1:8000")?
.run()
.await
```

This example showcases how derive macros and the framing abstraction simplify a
binary protocol server【F:docs/rust-binary-router-library-design.md†L1120-L1150】.

## Roadmap

Development priorities are tracked in [docs/roadmap.md](docs/roadmap.md). Key
tasks include building the Actix‑inspired API, implementing middleware and
extractor traits, and providing example applications【F:docs/roadmap.md†L1-L24】.

## License

Wireframe is distributed under the terms of the ISC license.
See [LICENSE](LICENSE) for details.
