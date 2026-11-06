# 📡 Wireframe

[![Ask DeepWiki][dw]][deepwiki] [![Crates.io Version][crates]][package]

[dw]: https://deepwiki.com/badge.svg
[deepwiki]: https://deepwiki.com/leynos/wireframe
[crates]: https://img.shields.io/crates/v/wireframe "crates.io package"
[package]: https://crates.io/crates/wireframe

*Route binary protocols in Rust, without writing the plumbing twice.*

Wireframe is a Tokio library for building servers and clients that speak custom
binary protocols. You describe your messages, routes, and framing; it handles
connections, length limits, reassembly, correlation, and shutdown.

Website: <https://df12.studio/wireframe>

______________________________________________________________________

## Why Wireframe?

Handwritten protocol servers tend to grow the same tangle of frame parsing,
dispatch tables, and half-finished limits. Wireframe gives each concern one
home:

- **Routing by message ID**: Map an identifier to a handler, and let middleware
  shape requests and replies around it.
- **Your wire format, not ours**: Use the default length-delimited framing, or
  describe an existing protocol once with a `FrameCodec`.
- **One definition for both ends**: The client shares the server's codec,
  serializer, and envelope, so the two cannot quietly disagree.
- **Limits by default**: Frame sizes, decode budgets, preamble deadlines, and
  per-connection memory budgets are on before you configure anything.
- **Tests at the frame**: Feed your application partial frames, one byte at a
  time, without opening a socket.

______________________________________________________________________

## Quick start

You will need [Rust](https://rustup.rs/) 1.95 or newer. The program below
starts a server on a free local port, sends it a message from a client, and
prints the reply. It takes about two minutes, most of which is the first
compile.

### Installation

Create a project and add the dependencies:

<!-- tested-example: readme-new-project -->

```bash
cargo new hello-wireframe
cd hello-wireframe
```

Then add these lines to `Cargo.toml`. Pin the exact version during the alpha,
because one alpha may change what the last one offered.

<!-- tested-example: readme-dependencies -->

```toml
[dependencies]
wireframe = "=0.4.0-alpha1"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
bincode = "2"
async-trait = "0.1"
```

### Your first server and client

Replace `src/main.rs` with this program:

<!-- tested-example: readme-first-server -->

```rust
use std::{convert::Infallible, error::Error, sync::Arc};

use async_trait::async_trait;
use wireframe::{
    app::{Envelope, Handler, WireframeApp},
    client::WireframeClient,
    message::Message,
    middleware::{HandlerService, Service, ServiceRequest, ServiceResponse, Transform},
    server::WireframeServer,
};

// A message ID selects the route; it travels beside the payload, not inside it.
const GREET: u32 = 1;

#[derive(bincode::Encode, bincode::BorrowDecode)]
struct Greeting {
    name: String,
}

#[derive(bincode::Encode, bincode::BorrowDecode)]
struct Reply {
    text: String,
}

// Handlers observe a request; middleware like this writes the reply.
struct Greeter;

struct GreeterService<S> {
    inner: S,
}

#[async_trait]
impl<S> Service for GreeterService<S>
where
    S: Service<Error = Infallible> + Send + Sync + 'static,
{
    type Error = Infallible;

    async fn call(&self, request: ServiceRequest) -> Result<ServiceResponse, Infallible> {
        let greeting = Greeting::from_bytes(request.frame()).map(|(greeting, _)| greeting);
        let mut response = self.inner.call(request).await?;
        if let Ok(greeting) = greeting {
            let reply = Reply {
                text: format!("Hello, {}!", greeting.name),
            };
            *response.frame_mut() = reply.to_bytes().expect("a short reply encodes");
        }
        Ok(response)
    }
}

#[async_trait]
impl Transform<HandlerService<Envelope>> for Greeter {
    type Output = HandlerService<Envelope>;

    async fn transform(&self, service: HandlerService<Envelope>) -> Self::Output {
        let id = service.id();
        HandlerService::from_service(id, GreeterService { inner: service })
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let handler: Handler<Envelope> = Arc::new(|_envelope: &Envelope| Box::pin(async {}));
    let app: WireframeApp = WireframeApp::new()?.route(GREET, handler)?.wrap(Greeter)?;

    // Port 0 asks the operating system for any free port.
    let server = WireframeServer::from_app(app).bind("127.0.0.1:0".parse()?)?;
    let address = server.local_addr().expect("a bound server has an address");
    let shutdown = server.spawn().await?;

    let mut client = WireframeClient::builder().connect(address).await?;
    let greeting = Greeting {
        name: "Wireframe".into(),
    };
    let request = Envelope::new(GREET, None, greeting.to_bytes()?);
    let response: Envelope = client.call_correlated(request).await?;
    let (reply, _) = Reply::from_bytes(response.payload_bytes())?;
    println!("{}", reply.text);

    client.close().await;
    shutdown.stop();
    shutdown.drained().await?;
    Ok(())
}
```

Run it:

<!-- tested-example: readme-run-first-server -->

```bash
cargo run
```

You should see `Hello, Wireframe!`. The
[users' guide walks through this program](docs/users-guide.md#your-first-server)
a step at a time, including why the reply lives in middleware.

______________________________________________________________________

## Features

Wireframe v0.4.0-alpha1 provides:

- routing by message ID, with custom envelopes through the `Packet` trait and
  correlation identifiers;
- middleware through `Transform`, and per-connection setup and teardown hooks;
- length-delimited framing by default, with worked `FrameCodec` implementations
  for Hotline, MySQL, and RESP in `examples/`;
- bincode serialization by default, a pluggable `Serializer` trait, and an
  optional Serde bridge;
- connection preambles, with a 10-second default deadline on the server;
- transport fragmentation, protocol message assembly, and per-connection memory
  budgets with read pacing;
- a server with configurable workers, readiness signals, graceful shutdown, and
  accept backoff, which keeps a panicking connection away from the others;
- a client with correlated calls, request and lifecycle hooks, tracing, streamed
  replies, outbound streaming sends, and optional connection pools;
- `metrics` counters, `tracing` and `log` events, and test drivers behind the
  `testkit` feature and in the `wireframe_testing` crate.

______________________________________________________________________

## Release and development status

Wireframe v0.4.0-alpha1 has a target release date of 15 November 2026. It is an
alpha for early adopters: expect interfaces to change before 1.0, and pin the
version. Upgrading from v0.3.0? The
[migration guide](docs/v0-4-0-migration-guide.md) lists every change to act on.

A few things are worth knowing before you build on it:

- **One reply per request.** A routed request produces exactly one response
  frame, and a handler cannot return a stream.
- **Some parts ship ahead of the wiring.** Push queues, `ConnectionActor`,
  streaming request bodies, extractors, and codec recovery policies exist and
  are tested, but `WireframeServer` does not drive them yet. The
  [users' guide](docs/users-guide.md#building-blocks-not-yet-wired-into-the-server)
  says how far each one goes.
- **There is no built-in TLS.** Terminate it in front of the server.
- **Idle connections stay open.** `read_timeout_ms` is a housekeeping interval,
  and shutdown waits for open connections to finish, so bound them in your
  protocol or with your own deadline.

______________________________________________________________________

## Examples

The `examples/` directory holds complete programs. To run them, clone the
repository and start the echo server:

<!-- tested-example: readme-run-echo-server -->

```bash
git clone https://github.com/leynos/wireframe.git
cd wireframe
cargo run --example echo --features examples
```

Then, in a second terminal in the same directory, log in against it:

<!-- tested-example: readme-run-echo-client -->

```bash
cargo run --example client_echo_login --features examples
```

The other programs run the same way:

- `ping_pong`: typed requests and replies through middleware, explained in
  [its walkthrough](examples/ping_pong.md).
- `packet_enum`: telling packet types apart with a bincode enum.
- `metadata_routing`: routing from frame metadata without decoding the payload.
- `hotline_codec`, `mysql_codec`, `resp_codec`: custom framing for real
  protocols.
- `async_stream`, `multi_packet`: building responses with the streaming types,
  explained in the [stream](examples/async_stream.md) and
  [channel](examples/multi_packet.md) walkthroughs.

______________________________________________________________________

## Learn more

- [Users' guide](docs/users-guide.md): from a first server to production and
  custom protocols.
- [Migration guide](docs/v0-4-0-migration-guide.md): moving from v0.3.0.
- [API reference](https://docs.rs/wireframe): every public type, on docs.rs.
- [Design document](docs/rust-binary-router-library-design.md): architecture and
  rationale.
- [Developers' guide](docs/developers-guide.md): development workflow and
  quality gates.
- [Roadmap](docs/roadmap.md): completed foundations and planned work.

______________________________________________________________________

## Licence

ISC — see [LICENSE](LICENSE) for details.

______________________________________________________________________

## Contributing

Contributions are welcome. Start with the
[developers' guide](docs/developers-guide.md); automated contributors should
also follow [AGENTS.md](AGENTS.md).
