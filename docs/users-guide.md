# Wireframe users' guide

This guide is for Rust developers building servers and clients for binary
protocols with Wireframe v0.4.0-alpha1. It starts from an empty project and
builds up to custom wire formats, production operation, and testing. You do not
need to know Tokio deeply to begin, though you will meet `async` code from the
first example.

Every Rust example here is compiled, and most are run, by the project's test
suite, so what you copy is what was tested. If something does not behave as
described, please [open an issue](https://github.com/leynos/wireframe/issues).

## Find your path

Table 1 suggests where to begin. The sections are ordered to be read from top
to bottom, and each builds on the last, but you can also dip in.

| You want to…                           | Start with                                                                                      |
| -------------------------------------- | ----------------------------------------------------------------------------------------------- |
| See it work in two minutes             | [Your first server](#your-first-server)                                                         |
| Understand how the pieces fit          | [How Wireframe thinks](#how-wireframe-thinks)                                                   |
| Serve typed messages on several routes | [Routes and envelopes](#routes-and-envelopes), then [Middleware](#middleware)                   |
| Speak a protocol that already exists   | [Custom frame codecs](#custom-frame-codecs), then [Connection preambles](#connection-preambles) |
| Call a server from Rust                | [The client runtime](#the-client-runtime)                                                       |
| Send messages larger than a frame      | [Large messages](#large-messages)                                                               |
| Test without opening sockets           | [Testing your protocol](#testing-your-protocol)                                                 |
| Go to production                       | [Running servers](#running-servers), then [Observability](#observability)                       |
| Check whether a feature is ready       | [Building blocks not yet wired into the server](#building-blocks-not-yet-wired-into-the-server) |
| Fix something that is going wrong      | [Troubleshooting](#troubleshooting)                                                             |
| Upgrade from v0.3.0                    | The [migration guide](v0-4-0-migration-guide.md)                                                |

_Table 1: Where to begin, by goal._

## Before you begin

Wireframe needs Rust 1.95 or newer, and runs on Tokio. Add it to `Cargo.toml`,
pinning the exact version during the alpha because one alpha may change what
the last one offered:

<!-- tested-example: guide-dependencies -->

```toml
[dependencies]
wireframe = "=0.4.0-alpha1"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
bincode = "2"
async-trait = "0.1"
```

`bincode` lets you derive encodable messages, and `async-trait` is what you
will write middleware with. Table 2 lists the optional Cargo features.

| Feature              | Default | Adds                                            |
| -------------------- | ------- | ----------------------------------------------- |
| `metrics`            | Yes     | Emits counters through the `metrics` facade.    |
| `serializer-bincode` | Yes     | The default `BincodeSerializer`.                |
| `serializer-serde`   | No      | The optional Serde bridge.                      |
| `pool`               | No      | Client connection pools through `connect_pool`. |
| `testkit`            | No      | Test drivers in `wireframe::testkit`.           |

_Table 2: Cargo features for applications._

The remaining features (`test-support`, `examples`, `advanced-tests`, and
`benchmark-counters`) exist for this repository's own tests and benchmarks.

## Your first server

This complete program starts a server on a free local port, sends it a message
from a client, and prints the reply:

<!-- tested-example: guide-first-server -->

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

const GREET: u32 = 1;

#[derive(bincode::Encode, bincode::BorrowDecode)]
struct Greeting {
    name: String,
}

#[derive(bincode::Encode, bincode::BorrowDecode)]
struct Reply {
    text: String,
}

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

Save it as `src/main.rs` and `cargo run` it. You should see
`Hello, Wireframe!`. There is a fair amount going on, so here it is in order:

1. **Messages.** `Greeting` and `Reply` are ordinary structs. Deriving bincode's
   traits is enough for Wireframe to provide `to_bytes` and `from_bytes`.
2. **Route.** `route(GREET, handler)` says that message ID 1 goes to this
   handler. The ID travels beside the payload, so it is not part of either type.
3. **Handler.** A handler receives `&Envelope` and returns a boxed future of
   `()`. It can observe the request, but it does not produce the reply. Ours
   does nothing.
4. **Middleware.** `Greeter` wraps the route. It decodes the request, calls the
   inner service, and replaces the response bytes. This is where replies come
   from, and it is the part most people find surprising at first.
5. **Server.** Port 0 asks the operating system for any free port. `spawn` runs
   the server in the background and returns a handle for shutting it down.
6. **Client.** `call_correlated` stamps a correlation identifier on the request,
   sends it, reads the reply, and checks that the identifier came back.

Without `Greeter`, the program would still run: the default response repeats
the request's payload, so an unwrapped route is an echo. That makes a good
first experiment, and it is why the `echo` example in `examples/` is so short.

Next, [How Wireframe thinks](#how-wireframe-thinks) gives you the vocabulary
for everything else.

## How Wireframe thinks

Wireframe uses a separate word for what you are holding at each layer, and
these words are used consistently in the API and in this guide. Table 3 defines
them.

| Layer                 | Term     | Meaning                                                                               |
| --------------------- | -------- | ------------------------------------------------------------------------------------- |
| Transport             | Frame    | The unit on the wire, produced and consumed by a `FrameCodec`.                        |
| Routing               | Envelope | A wrapper carrying a message ID, an optional correlation ID, and payload bytes.       |
| Domain                | Message  | The typed value you encode into, and decode from, payload bytes.                      |
| Transport subdivision | Fragment | A bounded slice of a large payload, plus the metadata needed to put it back together. |

_Table 3: The four layers and their terms._

`Packet` is the trait an envelope implements, and `Envelope` is the type
bundled with Wireframe. You can bring your own.

For screen readers: the following flowchart shows an inbound frame passing
through decoding, envelope deserialization, reassembly, and routing into
middleware and the handler, and the reply passing back out through
serialization and encoding.

<!-- tested-example: guide-request-journey -->

```mermaid
flowchart LR
    Socket([Socket]) --> Decode[Codec decode]
    Decode --> Envelope[Deserialize envelope]
    Envelope --> Reassemble[Reassemble fragments]
    Reassemble --> Assemble[Assemble message]
    Assemble --> Route{Route by ID}
    Route --> Middleware[Middleware]
    Middleware --> Handler[Handler]
    Handler --> Middleware
    Middleware --> Fragment[Fragment]
    Fragment --> Serialize[Serialize]
    Serialize --> Wrap[Codec wrap]
    Wrap --> Hook[before_send]
    Hook --> Encode[Codec encode]
    Encode --> Socket
```

_Figure 1: One request's journey through a Wireframe connection._

Three facts follow from this, and they explain most of the API:

- **Handlers observe, and middleware answers.** A handler's signature returns
  `()`, and its future cannot borrow the envelope, so copy out what you need
  before the `async` block. Every routed request sends exactly one response,
  even if it is empty.
- **The order is fixed.** Reassembly happens before routing, so handlers only
  ever see whole messages. On the way out, fragmentation happens before
  serialization, so each fragment is its own frame.
- **An application is immutable once it runs.** You configure a `WireframeApp`
  builder, and the server prepares it once and shares the result among all
  connections.

### Where to import things from

The crate root is deliberately small: `Result`, `WireframeError`, and
`NoProtocolError`. Other items live in a module named for their domain, such as
`wireframe::app` or `wireframe::client`. For convenience,
`wireframe::prelude::*` gathers the most common ones.

## Routes and envelopes

### Registering routes

`route` maps a message ID to a handler, and returns an error if the ID is
already taken, which keeps the dispatch table unambiguous:

<!-- tested-example: guide-routes -->

```rust
use std::sync::Arc;

use wireframe::{
    WireframeError,
    app::{Envelope, Handler, Packet, WireframeApp},
};

const LOGIN: u32 = 1;
const TELEMETRY: u32 = 2;

fn handler(name: &'static str) -> Handler<Envelope> {
    Arc::new(move |envelope: &Envelope| {
        // The returned future is 'static, so copy what it needs out of the borrow.
        let id = envelope.id();
        Box::pin(async move { println!("{name} handled message {id}") })
    })
}

fn main() -> wireframe::Result<()> {
    let app: WireframeApp = WireframeApp::new()?
        .route(LOGIN, handler("login"))?
        .route(TELEMETRY, handler("telemetry"))?;

    let refused = app.route(LOGIN, handler("impostor"));
    assert!(matches!(refused, Err(WireframeError::DuplicateRoute(LOGIN))));
    Ok(())
}
```

A frame with no matching route is logged and dropped without a response. A
frame that fails to decode counts towards a failure limit: a connection closes
after ten failures with no successfully dispatched message in between.

### Custom envelopes

To give handlers your own type rather than `Envelope`, implement `Packet` (and
`CorrelatableFrame`, which it requires) on it, and name it in the application's
parameters.

This changes what handlers see, not what travels on the wire. A connection
still decodes the serializer's `Envelope`, rebuilds your type from its parts
for the handler, and sends the reply as an `Envelope`. To change the layout on
the wire, write a [custom frame codec](#custom-frame-codecs) as well:

<!-- tested-example: guide-custom-envelope -->

```rust
use std::sync::Arc;

use wireframe::{
    app::{Handler, Packet, PacketParts, WireframeApp},
    correlation::CorrelatableFrame,
    serializer::BincodeSerializer,
};

#[derive(bincode::Encode, bincode::BorrowDecode)]
struct Stroke {
    opcode: u16,
    tag: Option<u64>,
    payload: Vec<u8>,
}

impl CorrelatableFrame for Stroke {
    fn correlation_id(&self) -> Option<u64> { self.tag }

    fn set_correlation_id(&mut self, tag: Option<u64>) { self.tag = tag; }
}

impl Packet for Stroke {
    fn id(&self) -> u32 { u32::from(self.opcode) }

    fn into_parts(self) -> PacketParts {
        PacketParts::new(u32::from(self.opcode), self.tag, self.payload)
    }

    fn from_parts(parts: PacketParts) -> Self {
        Self {
            opcode: u16::try_from(parts.id()).unwrap_or(u16::MAX),
            tag: parts.correlation_id(),
            payload: parts.into_payload(),
        }
    }
}

type StrokeApp = WireframeApp<BincodeSerializer, (), Stroke>;

fn main() -> wireframe::Result<()> {
    let draw: Handler<Stroke> = Arc::new(|stroke: &Stroke| {
        let opcode = stroke.opcode;
        Box::pin(async move { println!("drawing opcode {opcode}") })
    });
    let _app = StrokeApp::new()?.route(107, draw)?;
    Ok(())
}
```

### Correlation identifiers

A correlation identifier ties a response to its request. `None` means the frame
is not part of an exchange, such as an unsolicited event, so prefer `None` to
`Some(0)` when there is no identifier. `PacketParts::inherit_correlation` fills
in a missing identifier from the request and logs a mismatch, which is how a
reply keeps the right identifier even if middleware forgets about it.

## Middleware

Middleware is how you read typed requests and write typed replies, and it is
also the place for concerns that span routes, such as counting or
authentication.

### Writing a `Transform`

`wrap` accepts a `Transform` whose output is a `HandlerService`. Two types do
the work: the `Transform` runs once when the application is prepared, and wraps
each route's service in your `Service`, which then runs for every request:

<!-- tested-example: guide-middleware-transform -->

```rust
use std::{
    convert::Infallible,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use async_trait::async_trait;
use wireframe::{
    app::{Envelope, Handler, Packet, WireframeApp},
    middleware::{HandlerService, Service, ServiceRequest, ServiceResponse, Transform},
};

/// Counts every request that passes through, on any route.
struct Tally {
    tallied: Arc<AtomicUsize>,
}

struct TallyService<S> {
    inner: S,
    tallied: Arc<AtomicUsize>,
}

#[async_trait]
impl<S> Service for TallyService<S>
where
    S: Service<Error = Infallible> + Send + Sync + 'static,
{
    type Error = Infallible;

    async fn call(&self, request: ServiceRequest) -> Result<ServiceResponse, Infallible> {
        self.tallied.fetch_add(1, Ordering::Relaxed);
        self.inner.call(request).await
    }
}

#[async_trait]
impl<E: Packet> Transform<HandlerService<E>> for Tally {
    type Output = HandlerService<E>;

    async fn transform(&self, service: HandlerService<E>) -> Self::Output {
        let id = service.id();
        let tallied = Arc::clone(&self.tallied);
        HandlerService::from_service(
            id,
            TallyService {
                inner: service,
                tallied,
            },
        )
    }
}

fn main() -> wireframe::Result<()> {
    let tallied = Arc::new(AtomicUsize::new(0));
    let handler: Handler<Envelope> = Arc::new(|_: &Envelope| Box::pin(async {}));
    let _app: WireframeApp = WireframeApp::new()?
        .route(1, handler)?
        .wrap(Tally { tallied })?;
    Ok(())
}
```

Each middleware layer wraps the application's routes, and the first `wrap` call
ends up outermost, so it sees the request first and the response last. To
reply, call the inner service and then replace `response.frame_mut()`, as
[the first server](#your-first-server) does. A service can edit the incoming
bytes through `request.frame_mut()` too. `ServiceRequest` and `ServiceResponse`
also expose the correlation identifier.

### What `from_fn` is for

`middleware::from_fn` turns an async function into a `Transform`, which is
handy for composing services of your own and for testing. However, it produces
a different service type from the `HandlerService` that `wrap` requires, so
**it cannot be passed to `WireframeApp::wrap`**. For routed middleware, write a
`Transform` as above.

<!-- tested-example: guide-middleware-from-fn -->

```rust
use std::convert::Infallible;

use wireframe::{
    app::Envelope,
    message::Message,
    middleware::{HandlerService, Next, ServiceRequest, ServiceResponse, from_fn},
};

#[derive(bincode::Encode, bincode::BorrowDecode)]
struct Ping(u8);

#[derive(bincode::Encode, bincode::BorrowDecode)]
struct Pong(u8);

async fn decode_and_respond(
    request: ServiceRequest,
    next: Next<'_, HandlerService<Envelope>>,
) -> Result<ServiceResponse, Infallible> {
    let ping = Ping::from_bytes(request.frame()).map(|(message, _)| message).ok();
    let mut response = next.call(request).await?;

    if let Some(Ping(value)) = ping {
        *response.frame_mut() = Pong(value.wrapping_add(1)).to_bytes().expect("encode Pong");
    }
    Ok(response)
}

let _middleware = from_fn(decode_and_respond);
```

### Versioning a message

Middleware is also a tidy way to retire an old message format without breaking
its clients. Accept versions N and N-1 on the way in, rewrite the older one so
that handlers only ever see the current schema, send only version N out, and
count legacy traffic so that you know when it is safe to remove the adapter:

<!-- tested-example: guide-middleware-versions -->

```rust
use std::convert::Infallible;

use async_trait::async_trait;
use wireframe::{
    app::Envelope,
    message::Message,
    middleware::{HandlerService, Service, ServiceRequest, ServiceResponse, Transform},
};

#[derive(bincode::Encode, bincode::BorrowDecode)]
struct PointV1 {
    id: u64,
    name: String,
}

/// Version 2 adds a field, so a version 1 payload is too short to decode as this.
#[derive(bincode::Encode, bincode::BorrowDecode)]
struct PointV2 {
    id: u64,
    display_name: String,
    intensity: u32,
}

struct UpgradePoints;

struct UpgradeService<S> {
    inner: S,
}

#[async_trait]
impl<S> Service for UpgradeService<S>
where
    S: Service<Error = Infallible> + Send + Sync + 'static,
{
    type Error = Infallible;

    async fn call(&self, mut request: ServiceRequest) -> Result<ServiceResponse, Infallible> {
        if PointV2::from_bytes(request.frame()).is_err()
            && let Ok((v1, _)) = PointV1::from_bytes(request.frame())
        {
            let v2 = PointV2 {
                id: v1.id,
                display_name: v1.name,
                intensity: 0,
            };
            *request.frame_mut() = v2.to_bytes().expect("encode PointV2");
        }
        self.inner.call(request).await
    }
}

#[async_trait]
impl Transform<HandlerService<Envelope>> for UpgradePoints {
    type Output = HandlerService<Envelope>;

    async fn transform(&self, service: HandlerService<Envelope>) -> Self::Output {
        let id = service.id();
        HandlerService::from_service(id, UpgradeService { inner: service })
    }
}
```

Note the order of the checks. Bincode happily decodes a prefix, so trying the
older format first could misread a newer message.

## Packets, payloads, and serialization

A serializer converts envelopes to and from bytes. `WireframeApp` uses
`BincodeSerializer` by default. To use another, install it with
`.serializer(s)` on the builder, or start from
`WireframeApp::with_serializer(s)`.

Message types use the adapter traits `EncodeWith<S>` and `DecodeWith<S>`. Any
type deriving bincode's traits implements `Message`, which provides `to_bytes`
and `from_bytes`, and this is usually all you need. For Serde types, enable the
`serializer-serde` feature, wrap values in `SerdeMessage<T>` (or call
`into_serde_message()`), and implement `SerdeSerializerBridge` on your
serializer.

A few details matter if you write your own serializer:

- Implement `MessageCompatibilitySerializer` if it should accept `Message`
  payloads.
- `Serializer` is not object-safe, so `dyn Serializer` is unavailable. If you
  need runtime polymorphism, wrap concrete serializers in your own adapter.
- Implement `Serializer::deserialize_with_context` to inspect metadata through
  `DeserializeContext`.
- `FrameMetadata::parse` can extract routing information cheaply. After a
  successful parse the framework still fully deserializes, unless
  `should_deserialize_after_parse` returns `false`. If parsing fails, it falls
  back to full deserialization, except when a decode limit was exceeded.
  `BincodeSerializer::parse` itself performs a full decode.

### Limits on what a peer can make you decode

The bundled bincode decoders apply a fixed 16 MiB limit (`MAX_FRAME_LENGTH`) to
peer-provided container lengths. This bounds the work and allocation a declared
length can cause. It is separate from your configured frame size, which still
decides which frames are accepted. Reassembled messages use the same 16 MiB
limit, even if their configured size limit is larger.

An inbound `Envelope` frame must contain exactly one value, so trailing bytes
are a decoding error. The limit relies on bincode's accounting, and is not a
general allocator quota, so protocols with further resource needs should
validate peer input themselves.

## Custom frame codecs

The default codec is a 4-byte big-endian length prefix, with a 1024-byte
maximum frame. To change the limit, use `buffer_capacity`; to change the
format, implement `FrameCodec`.

Changing either clears anything derived from the old framing. `buffer_capacity`
and `with_codec` both clear the fragmentation configuration, and `with_codec`
also removes hooks installed with `with_protocol`. Call `with_codec` first,
then configure fragmentation and protocol hooks.

### Writing a `FrameCodec`

A codec supplies a frame type and a Tokio decoder and encoder. This one uses
two bytes of length and four bytes of tag:

<!-- tested-example: guide-frame-codec -->

```rust
use std::{io, sync::Arc};

use bytes::{Buf, BufMut, Bytes, BytesMut};
use tokio_util::codec::{Decoder, Encoder};
use wireframe::{
    app::{Envelope, Handler, WireframeApp},
    codec::FrameCodec,
    serializer::BincodeSerializer,
};

const HEADER_LEN: usize = 6;

#[derive(Debug, Clone)]
struct TaggedFrame {
    tag: u32,
    payload: Bytes,
}

#[derive(Clone)]
struct TagCodec {
    max_frame_length: usize,
}

struct TagDecoder {
    max_frame_length: usize,
}

impl Decoder for TagDecoder {
    type Item = TaggedFrame;
    type Error = io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> io::Result<Option<TaggedFrame>> {
        if src.len() < HEADER_LEN {
            return Ok(None);
        }
        let len = usize::from(u16::from_be_bytes([src[0], src[1]]));
        // Your decoder is what enforces the frame limit.
        if len > self.max_frame_length {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "frame too large"));
        }
        if src.len() < HEADER_LEN + len {
            src.reserve(HEADER_LEN + len - src.len());
            return Ok(None);
        }
        src.advance(2);
        let tag = src.get_u32();
        let payload = src.split_to(len).freeze();
        Ok(Some(TaggedFrame { tag, payload }))
    }
}

struct TagEncoder;

impl Encoder<TaggedFrame> for TagEncoder {
    type Error = io::Error;

    fn encode(&mut self, frame: TaggedFrame, dst: &mut BytesMut) -> io::Result<()> {
        let len = u16::try_from(frame.payload.len())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "payload too large"))?;
        dst.reserve(HEADER_LEN + frame.payload.len());
        dst.put_u16(len);
        dst.put_u32(frame.tag);
        dst.extend_from_slice(&frame.payload);
        Ok(())
    }
}

impl FrameCodec for TagCodec {
    type Frame = TaggedFrame;
    type Decoder = TagDecoder;
    type Encoder = TagEncoder;

    fn decoder(&self) -> TagDecoder {
        TagDecoder {
            max_frame_length: self.max_frame_length,
        }
    }

    fn encoder(&self) -> TagEncoder { TagEncoder }

    fn frame_payload(frame: &TaggedFrame) -> &[u8] { &frame.payload }

    fn frame_payload_bytes(frame: &TaggedFrame) -> Bytes { frame.payload.clone() }

    fn wrap_payload(&self, payload: Bytes) -> TaggedFrame { TaggedFrame { tag: 0, payload } }

    fn correlation_id(frame: &TaggedFrame) -> Option<u64> { Some(u64::from(frame.tag)) }

    fn max_frame_length(&self) -> usize { self.max_frame_length }
}

fn main() -> wireframe::Result<()> {
    let codec = TagCodec {
        max_frame_length: 4096,
    };
    let echo: Handler<Envelope> = Arc::new(|_: &Envelope| Box::pin(async {}));
    let app: WireframeApp<_, _, Envelope, TagCodec> = WireframeApp::<BincodeSerializer>::new()?
        .with_codec(codec)
        .route(1, echo)?;
    assert_eq!(app.codec().max_frame_length(), 4096);
    Ok(())
}
```

When you write a codec, keep these rules in mind:

- `frame_payload` returns only the logical payload, without protocol headers.
- `wrap_payload` adds any headers the wire format needs.
- `correlation_id` is for protocols that store it outside the payload. Wireframe
  uses it only when the deserialized envelope lacks an identifier.
- `max_frame_length` sizes buffers and derives budgets and fragmentation
  defaults, but **your decoder must enforce the limit**.
- Wireframe clones the codec per connection, so make sure `Clone` produces
  independent state if the codec holds any, such as sequence counters.

See `examples/hotline_codec.rs`, `examples/mysql_codec.rs`, and
`examples/resp_codec.rs` for real protocols. The Hotline and MySQL codecs are
also available from `wireframe::codec::examples`.

### Zero-copy payloads

Storing the payload as `Bytes`, as `TaggedFrame` does, and overriding
`frame_payload_bytes` avoids copying it. The clone only increments a reference
count. In the decoder, use `BytesMut::split_to(...).freeze()` rather than
`.to_vec()`. The default `frame_payload_bytes` copies from the `frame_payload`
slice, so codecs using `Vec<u8>` keep working.

### Codec errors

`CodecError` classifies failures as `Framing`, `Protocol`, `Io`, or `Eof`, the
last of which distinguishes a clean close from a close in the middle of a frame
or header. Today **any decoder error closes the connection**. See
[recovery policies](#codec-recovery-policies) for the classification that is
available for your own use.

## Connection preambles

Some protocols begin with a handshake before any frames, such as a magic number
and version. The server reads and decodes a preamble first, then hands anything
left over to the codec:

<!-- tested-example: guide-server-preamble -->

```rust
use std::time::Duration;

use futures::FutureExt;
use tokio::io::AsyncWriteExt;
use wireframe::{app::WireframeApp, client::WireframeClient, server::WireframeServer};

#[derive(bincode::Encode, bincode::BorrowDecode, Debug, Clone, PartialEq)]
struct Hello {
    magic: [u8; 4],
    version: u16,
}

const MAGIC: [u8; 4] = *b"WIRE";

fn build_app() -> WireframeApp {
    WireframeApp::default()
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let server = WireframeServer::new(build_app)
        .with_preamble::<Hello>()
        .preamble_timeout(Duration::from_secs(2))
        .on_preamble_decode_success(|hello: &Hello, stream| {
            let accepted = hello.magic == MAGIC;
            async move {
                stream.write_all(&[u8::from(accepted)]).await?;
                if accepted {
                    Ok(())
                } else {
                    // An error here closes the connection, so write first.
                    Err(std::io::Error::other("unknown protocol"))
                }
            }
            .boxed()
        })
        .bind("127.0.0.1:0".parse()?)?;
    let address = server.local_addr().expect("a bound server has an address");
    let shutdown = server.spawn().await?;

    let client = WireframeClient::builder()
        .with_preamble(Hello {
            magic: MAGIC,
            version: 3,
        })
        .connect(address)
        .await?;
    client.close().await;

    shutdown.stop();
    shutdown.drained().await?;
    Ok(())
}
```

Keep these in mind:

- **Call `with_preamble` first**, because it clears any hooks registered
  earlier.
- **A success handler that returns an error closes the connection.** The server
  writes nothing on your behalf, so send any rejection message before returning.
- **`on_preamble_decode_failure`** receives decode errors and timeouts, along
  with the stream, so that you can reply or log before the connection closes.
- **There is always a deadline.** The read is bounded by
  `DEFAULT_PREAMBLE_TIMEOUT` (10 seconds) by default. `preamble_timeout`
  replaces that, and `disable_preamble_timeout` removes it. `Duration::ZERO`
  clamps to 1 ms. The deadline covers only the read.
- **Even without `with_preamble`**, the server reads a first byte under this
  deadline before it runs your application. This means the server never speaks
  first, and a client that waits silently will be disconnected.
- **Preambles are small.** Decoding is limited to 1 KiB.

The [preamble validator guide](preamble-validator.md) goes further, and
[Client preamble exchange](#client-preamble-exchange) covers the other end.

## Connection lifecycle and state

### Setup and teardown

`on_connection_setup` runs once per connection and produces state, and
`on_connection_teardown` receives that state when the stream finishes. Setting
up changes the application's state type, and it resets any teardown hook, so
register setup first:

<!-- tested-example: guide-lifecycle-hooks -->

```rust
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use wireframe::{
    app::{Envelope, WireframeApp},
    serializer::BincodeSerializer,
};

struct Link {
    connected_at: std::time::Instant,
}

type LinkApp = WireframeApp<BincodeSerializer, Link, Envelope>;

fn build_app(closed: Arc<AtomicUsize>) -> wireframe::Result<LinkApp> {
    let app: WireframeApp = WireframeApp::new()?;
    app.on_connection_setup(|| async {
        Link {
            connected_at: std::time::Instant::now(),
        }
    })?
    .on_connection_teardown(move |link: Link| {
        let closed = Arc::clone(&closed);
        async move {
            println!("connection lasted {:?}", link.connected_at.elapsed());
            closed.fetch_add(1, Ordering::Relaxed);
        }
    })
}

fn main() -> wireframe::Result<()> {
    let _app = build_app(Arc::new(AtomicUsize::new(0)))?;
    Ok(())
}
```

Teardown runs exactly once when both hooks are registered and setup has
produced state, whether the connection ended cleanly or with a decode,
protocol, or transport error. It does not run if a hook is absent, or on a task
panic or cancellation.

### Sharing state with handlers

Setup state reaches teardown, but it is **not passed to handlers or
middleware**. To share state with a handler, capture it when you build the
closure, as `Tally` does with its counter. `app_data` stores values for the
extractor building blocks, which the connection runtime does not read yet; see
[Extractors](#extractors).

### Writing a response directly

`send_response` writes one encoded message to a stream, using the application's
serializer and a length-delimited frame. It lives on the `WireframeApp` builder
rather than on `PreparedApp`, so it is for code that manages its own stream,
not for handlers:

<!-- tested-example: guide-send-response -->

```rust
use tokio::io::{self, AsyncReadExt, AsyncWriteExt};
use wireframe::{
    app::{SendError, WireframeApp},
    message::Message,
};

#[derive(bincode::Encode, bincode::BorrowDecode, Debug, PartialEq)]
struct Ready {
    status: u8,
}

const LENGTH_PREFIX: usize = 4;

#[tokio::main]
async fn main() -> Result<(), SendError> {
    let app: WireframeApp = WireframeApp::default();
    let (mut client, mut server) = io::duplex(64);

    let ready = Ready { status: 1 };
    app.send_response(&mut server, &ready).await?;
    server.shutdown().await?;

    let mut buffer = Vec::new();
    client.read_to_end(&mut buffer).await?;
    let frame = buffer.get(LENGTH_PREFIX..).expect("a length-prefixed frame");
    let (decoded, _) = Ready::from_bytes(frame).expect("decode Ready");
    assert_eq!(decoded, ready);
    Ok(())
}
```

`send_response_framed` writes raw bytes to an existing `Framed` stream, and
`send_response_framed_with_codec` does so through `FrameCodec::wrap_payload`.
All three report failures through `SendError`, and all three bypass
`before_send`. The `Display` output of each variant begins with a stable prefix
you may match on: `serialization error:`, `I/O error:`, or `codec error:`. The
text after the prefix is not part of that contract.

### Driving accepted streams yourself

If you accept streams yourself rather than using `WireframeServer`, prepare the
application once and reuse it. Preparation consumes the builder and builds each
route's middleware chain a single time, so you must register everything first:

<!-- tested-example: guide-prepared-app -->

```rust
use tokio::io::duplex;
use wireframe::app::{PreparedApp, WireframeApp};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let prepared: PreparedApp = WireframeApp::new()?.prepare().await?;
    let (client, server) = duplex(64);
    drop(client);
    prepared.handle_connection_result(server).await?;
    Ok(())
}
```

`handle_connection_result` returns stream and handler I/O errors, and
`handle_connection` logs them instead. The older
`WireframeApp::handle_connection` methods are deprecated because they rebuild
the chains for every connection.

## Large messages

Wireframe has two separate mechanisms, and they are easy to confuse:

- **Fragmentation** is transport-level. Wireframe splits an envelope that will
  not fit in a frame, using its own `FRAG` format, and rejoins it at the other
  end. Both ends need to be Wireframe servers or to speak that format.
- **Message assembly** is protocol-level. Your protocol already defines first
  and continuation frames, and you teach Wireframe to parse them.

### Fragmentation

Fragmentation is off by default. Turn it on with `enable_fragmentation()`,
which derives limits from the frame budget, or supply a `FragmentationConfig`:

<!-- tested-example: guide-fragmentation-config -->

```rust
use std::{num::NonZeroUsize, sync::Arc, time::Duration};

use wireframe::{
    app::{Envelope, Handler, WireframeApp},
    fragment::FragmentationConfig,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = FragmentationConfig::for_frame_budget(
        1024,
        NonZeroUsize::new(16 * 1024).expect("non-zero"),
        Duration::from_secs(30),
    )
    .expect("the frame budget has room for fragments");

    let handler: Handler<Envelope> = Arc::new(|_: &Envelope| Box::pin(async {}));
    let _app: WireframeApp = WireframeApp::new()?
        .fragmentation(Some(config))
        .route(42, handler)?;
    Ok(())
}
```

Payloads larger than `fragment_payload_cap` (the frame budget less some
overhead) are split into fragments, and handlers only ever see the complete
message. Four limits protect the reassembling side:

- `max_message_size` caps the reassembled payload. Exceeding it counts as a
  deserialization failure.
- `reassembly_timeout` evicts stale partial messages.
- `max_concurrent_partials` limits incomplete messages per connection.
- `max_buffered_bytes` caps their total size.

Exceeding either of the last two closes the connection immediately.
`for_frame_budget` sets them to `min(frame_budget, 64)` partial messages and
the larger of `frame_budget × 64` and `max_message_size` bytes. Call
`fragmentation(None)` to disable fragmentation again, for example when a
gateway handles it upstream. The `WireframeClient` has no fragmentation support
of its own, so a client talking to a fragmenting server sees `FRAG` payloads.

To work with fragments directly, `Fragmenter` splits and `Reassembler` rejoins:

<!-- tested-example: guide-fragmenter -->

```rust
use std::{num::NonZeroUsize, time::Duration};

use wireframe::fragment::{FragmentHeader, FragmentIndex, Fragmenter, MessageId, Reassembler};

let fragmenter = Fragmenter::new(NonZeroUsize::new(512).expect("non-zero"));
let batch = fragmenter.fragment_bytes([7_u8; 1400]).expect("fragment");
assert_eq!(batch.len(), 3);
assert!(batch.fragments().last().expect("a fragment").header().is_last_fragment());

let limit = NonZeroUsize::new(4096).expect("non-zero");
let mut reassembler = Reassembler::new(limit, Duration::from_secs(30), None, None);
let id = MessageId::new(9);

let first = FragmentHeader::new(id, FragmentIndex::zero(), false);
assert!(reassembler.push(first, [1_u8; 8]).expect("accepted").is_none());

let last = FragmentHeader::new(id, FragmentIndex::new(1), true);
let whole = reassembler
    .push(last, [2_u8; 8])
    .expect("accepted")
    .expect("the final fragment completes the message");
assert_eq!(whole.payload().len(), 16);
```

An empty input produces one final fragment with an empty payload, so a batch is
never empty. `purge_expired` clears stale partial messages from a standalone
`Reassembler`.

### Message assembly

If your protocol has its own multi-frame format, implement `MessageAssembler`
to parse each frame's header. Once configured, it must parse every inbound
payload, including single-frame messages, and it runs after any fragment
reassembly:

<!-- tested-example: guide-message-assembler -->

```rust
use wireframe::{
    app::WireframeApp,
    message_assembler::{FirstFrameHeader, FrameHeader, MessageAssembler, MessageKey, ParsedFrameHeader},
};

struct DemoAssembler;

impl MessageAssembler for DemoAssembler {
    fn parse_frame_header(&self, _payload: &[u8]) -> Result<ParsedFrameHeader, std::io::Error> {
        Ok(ParsedFrameHeader::new(
            FrameHeader::First(FirstFrameHeader {
                message_key: MessageKey(1),
                metadata_len: 0,
                body_len: 0,
                total_body_len: None,
                is_last: true,
            }),
            0,
        ))
    }
}

let app: WireframeApp = WireframeApp::new().expect("builder");
let _app = app.with_message_assembler(DemoAssembler);
```

Incomplete messages stay buffered per `MessageKey` until they complete or time
out. When one completes, the handler receives one buffered `Envelope` whose
payload is the metadata followed by the body, with no marker between the two.
Parsing and ordering failures count as deserialization failures, towards the
same limit as other inbound failures.

To use the state machine directly, `MessageAssemblyState` handles interleaved
messages:

<!-- tested-example: guide-message-assembly-state -->

```rust
use std::{num::NonZeroUsize, time::Duration};

use wireframe::message_assembler::{
    ContinuationFrameHeader,
    EnvelopeId,
    EnvelopeRouting,
    FirstFrameHeader,
    FirstFrameInput,
    FrameSequence,
    MessageAssemblyState,
    MessageKey,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut state = MessageAssemblyState::new(
        NonZeroUsize::new(1_048_576).expect("non-zero"),
        Duration::from_secs(30),
    );

    // The first frame of message 1.
    let first_one = FirstFrameHeader {
        message_key: MessageKey(1),
        metadata_len: 0,
        body_len: 5,
        total_body_len: Some(15),
        is_last: false,
    };
    let routing = EnvelopeRouting {
        envelope_id: EnvelopeId(1),
        correlation_id: None,
    };
    state.accept_first_frame(FirstFrameInput::new(&first_one, routing, vec![], b"hello")?)?;

    // The first frame of message 2 arrives in between.
    let first_two = FirstFrameHeader {
        message_key: MessageKey(2),
        metadata_len: 0,
        body_len: 5,
        total_body_len: None,
        is_last: false,
    };
    let routing = EnvelopeRouting {
        envelope_id: EnvelopeId(2),
        correlation_id: None,
    };
    state.accept_first_frame(FirstFrameInput::new(&first_two, routing, vec![], b"world")?)?;

    // A continuation completes message 1.
    let last_one = ContinuationFrameHeader {
        message_key: MessageKey(1),
        sequence: Some(FrameSequence(1)),
        body_len: 10,
        is_last: true,
    };
    let message = state
        .accept_continuation_frame(&last_one, b" completed")?
        .expect("message 1 is complete");
    assert_eq!(message.body(), b"hello completed");
    Ok(())
}
```

`MessageSeries` validates the order of one message's frames when your protocol
supplies sequence numbers:

<!-- tested-example: guide-message-series -->

```rust
use wireframe::message_assembler::{
    ContinuationFrameHeader,
    FirstFrameHeader,
    FrameSequence,
    MessageKey,
    MessageSeries,
    MessageSeriesError,
    MessageSeriesStatus,
};

fn continuation(sequence: u32, is_last: bool) -> ContinuationFrameHeader {
    ContinuationFrameHeader {
        message_key: MessageKey(1),
        sequence: Some(FrameSequence(sequence)),
        body_len: 5,
        is_last,
    }
}

let first = FirstFrameHeader {
    message_key: MessageKey(1),
    metadata_len: 0,
    body_len: 10,
    total_body_len: None,
    is_last: false,
};
let mut series = MessageSeries::from_first_frame(&first);

assert_eq!(series.accept_continuation(&continuation(1, false)), Ok(MessageSeriesStatus::Incomplete));
assert!(matches!(
    series.accept_continuation(&continuation(3, false)),
    Err(MessageSeriesError::SequenceMismatch { .. })
));
assert_eq!(series.accept_continuation(&continuation(2, false)), Ok(MessageSeriesStatus::Incomplete));
assert!(matches!(
    series.accept_continuation(&continuation(1, false)),
    Err(MessageSeriesError::DuplicateFrame { .. })
));
assert_eq!(series.accept_continuation(&continuation(3, true)), Ok(MessageSeriesStatus::Complete));
assert!(matches!(
    series.accept_continuation(&continuation(4, false)),
    Err(MessageSeriesError::SeriesComplete)
));
```

The first sequenced frame sets the baseline at any value, so a gap before it
goes undetected. Once a series is tracking sequences, an unsequenced frame is
rejected with `MissingSequence`. Other errors include `KeyMismatch` and
`SequenceOverflow`. Protocols that supply no sequence numbers skip these order
checks.

#### Declared total body length

`FirstFrameHeader::total_body_len` is binding, not a hint. If you set it, the
assembler enforces it:

- `DeclaredTotalTooSmall`: a first frame carries more body than it declares.
- `DeclaredTotalExceeded`: a continuation would pass the total, and is rejected
  before its bytes are buffered.
- `DeclaredTotalMismatch`: the completed body is not the declared length.

A rejected continuation or completion also removes its partial assembly, so a
mismatched message never reaches a handler. An exact total is also the cheapest
way to reject an oversized message early. Leave it as `None` to opt out. The
decision is recorded in
[ADR 015](adr-015-declared-total-body-length-contract.md).

### Per-connection memory budgets

`MemoryBudgets` caps inbound buffering per connection:

<!-- tested-example: guide-memory-budgets -->

```rust
use std::num::NonZeroUsize;

use wireframe::app::{BudgetBytes, MemoryBudgets, WireframeApp};

let bytes = |count: usize| BudgetBytes::new(NonZeroUsize::new(count).expect("non-zero"));
let budgets = MemoryBudgets::new(bytes(16 * 1024), bytes(64 * 1024), bytes(48 * 1024));

let _app: WireframeApp = WireframeApp::new()
    .expect("builder")
    .memory_budgets(budgets);
```

The arguments are bytes per message, per connection, and in flight. Without an
explicit call, Wireframe derives them from `buffer_capacity`, as Table 4 shows.

| Budget                 | Derived value       | With a 1024-byte frame |
| ---------------------- | ------------------- | ---------------------- |
| `bytes_per_message`    | `frame_budget × 16` | 16 KiB                 |
| `bytes_per_connection` | `frame_budget × 64` | 64 KiB                 |
| `bytes_in_flight`      | `frame_budget × 64` | 64 KiB                 |

_Table 4: Default memory budgets._

The budgets protect a connection in three stages. The first applies only with a
`MessageAssembler`, and the others measure whatever fragment and assembly bytes
are held. A default application buffers neither, so these stages have nothing
to do.

1. **Per-frame rejection.** A frame that would exceed the connection or
   in-flight budget is rejected, its partial assembly is freed, and the failure
   counts as a deserialization failure. The effective per-message limit is the
   smaller of `max_message_size` and `bytes_per_message`. A single-frame
   message that completes immediately is not counted, because it never buffers.
2. **Read pacing.** At 80% of the smaller aggregate cap, the connection pauses
   briefly before reading the next frame, passing back-pressure to the sender.
3. **Hard cap.** Above 100% of that cap, the connection closes with
   `InvalidData`. Fragment and assembly bytes are combined for this, so the
   total can exceed the cap even if each layer is within its own limit.

## Protocol hooks

`WireframeProtocol` gathers callbacks for a protocol. Install an implementation
with `with_protocol`, after any `with_codec` call. It must use
`ProtocolError = ()`, and its frame type must be the codec's frame, which is
`Bytes` for the default codec:

<!-- tested-example: guide-protocol-hooks -->

```rust
use bytes::{Bytes, BytesMut};
use wireframe::{
    app::WireframeApp,
    hooks::{ConnectionContext, WireframeProtocol},
};

struct Stamped;

impl WireframeProtocol for Stamped {
    type Frame = Bytes;
    type ProtocolError = ();

    fn before_send(&self, frame: &mut Bytes, _context: &mut ConnectionContext) {
        let mut stamped = BytesMut::from(frame.as_ref());
        stamped.extend_from_slice(b"!");
        *frame = stamped.freeze();
    }
}

let app: WireframeApp = WireframeApp::new().expect("builder");
let _app = app.with_protocol(Stamped);
```

**On a server connection, only `before_send` runs.** It runs on each handler
response frame, including each fragment, after the codec has wrapped it and
just before it is encoded. Each connection has its own `ConnectionContext`,
though state inside your `WireframeProtocol` value is shared by all of them.
Whatever it writes must still fit `max_frame_length`. The default codec rejects
an oversized frame, which fails the send and ends the connection, whereas a
custom codec must enforce the limit itself.

The other hooks (`on_connection_setup`, `on_command_end`, `handle_error`,
`stream_end_frame`, and `on_eof`) belong to `ConnectionActor`, which
`WireframeServer` does not run. See
[the connection actor](#the-connection-actor).

## Running servers

### Configuring and starting

`WireframeServer` starts unbound. `bind` or `bind_existing_listener` gives you
a server that can report its `local_addr` and run. Prefer `from_app` if you
already have an application. `WireframeServer::new` takes a factory instead,
which it calls **once per server run**, not once per connection. Either way,
the application is prepared before the server reports readiness, and failures
arrive as `ServerError` values:

<!-- tested-example: guide-server-configuration -->

```rust
use std::time::Duration;

use tokio::sync::oneshot;
use wireframe::{
    app::WireframeApp,
    server::{BackoffConfig, WireframeServer},
};

fn build_app() -> WireframeApp {
    WireframeApp::default().read_timeout_ms(250)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let backoff = BackoffConfig {
        initial_delay: Duration::from_millis(10),
        max_delay: Duration::from_secs(1),
    };
    let (ready_sender, ready) = oneshot::channel();
    let (stop, stopped) = oneshot::channel::<()>();

    let server = WireframeServer::new(build_app)
        .workers(4)
        .accept_backoff(backoff)
        .ready_signal(ready_sender)
        .bind("127.0.0.1:0".parse()?)?;
    assert_eq!(server.worker_count(), 4);

    let running = tokio::spawn(server.run_with_shutdown(async {
        drop(stopped.await);
    }));

    ready.await?;
    stop.send(()).expect("the server is waiting for shutdown");
    running.await??;
    Ok(())
}
```

Each worker runs an accept loop. The default count follows
`std::thread::available_parallelism`, which respects CPU quotas, and is never
below one. Any `accept` failure makes that worker back off exponentially and
retry, up to `max_delay`. A panic in a connection task is caught, logged, and
counted, and other connections carry on.

Per-connection state belongs in `on_connection_setup`, because the application
is shared among all connections.

### Shutting down

There are three ways to stop a server:

- `run` serves until Ctrl+C.
- `run_with_shutdown` serves until the future you give it completes. Dropping
  the returned future also cancels the accept loops and, in time, releases the
  listener, so keep it alive for as long as the server should run.
- `spawn` runs in the background and returns a cloneable `ServerShutdown`.
  `stop()` requests shutdown and is safe to call repeatedly, and
  `drained().await` waits for it to finish. `ServerError::AbnormalTermination`
  reports an unexpected supervisor exit. Dropping the last clone also requests
  shutdown, so keep one if you care about the outcome.

**Shutdown waits for open connections, and it does not tell them to close.**
There is no drain deadline, so a single idle peer holds up `run`,
`run_with_shutdown`, and `drained()` until it disconnects. If you need a bound,
wrap the wait in `tokio::time::timeout`, or have your protocol close quiet
connections.

### Limits at a glance

Table 5 summarizes the server's defaults.

| Setting                  | Default                 | What it does                                                                             |
| ------------------------ | ----------------------- | ---------------------------------------------------------------------------------------- |
| `buffer_capacity`        | 1024 bytes              | The largest frame; a larger one is a decoder error, which closes the connection.         |
| `read_timeout_ms`        | 100 ms                  | A housekeeping interval: on expiry, stale partial messages are purged.                   |
| Deserialization failures | 10                      | Closes the connection; a fully dispatched message resets the count.                      |
| `preamble_timeout`       | 10 seconds              | Bounds the preamble read; on expiry, the failure handler runs and the connection closes. |
| Decode limit             | 16 MiB                  | Bounds peer-declared container lengths in bundled decoders; fixed.                       |
| Memory budgets           | 16, 64, 64 × frame size | See [memory budgets](#per-connection-memory-budgets).                                    |
| Fragmentation            | Off                     | See [fragmentation](#fragmentation).                                                     |

_Table 5: Server limits and their defaults._

**`read_timeout_ms` does not close idle connections**, despite its name. A
quiet peer stays connected until it disconnects or the process exits. Bound
idle time at the protocol level, such as with a heartbeat, or in front of the
server.

There is no built-in Transport Layer Security (TLS), so terminate it upstream.

## Observability

### Logs

Wireframe emits through both `tracing` and `log`. Connection diagnostics, such
as unrouted messages and rejected frames, go through `log`, while `tracing`
events are also forwarded to any installed `log` logger. Wireframe does not
install anything itself, so without a logger or subscriber these messages are
lost. Install one early in start-up:

<!-- tested-example: guide-tracing-subscriber -->

```rust,no_run
fn main() {
    tracing_subscriber::fmt().init();
    // Start the application here.
}
```

This needs `tracing-subscriber` in your own dependencies. Its default features
include the bridge that receives `log` records.

### Metrics

With the default `metrics` feature, Wireframe records through the
[`metrics`](https://crates.io/crates/metrics) facade, so any recorder your
application installs receives them. With the feature off, the helpers do
nothing, so instrumentation can stay in place. Prometheus takes two steps,
because the exporter is your dependency:

<!-- tested-example: guide-prometheus-dependency -->

```toml
[dependencies]
metrics-exporter-prometheus = "0.18"
```

<!-- tested-example: guide-prometheus-recorder -->

```rust,no_run
use std::time::Duration;

use metrics_exporter_prometheus::PrometheusBuilder;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let prometheus = PrometheusBuilder::new().install_recorder()?;

    // `install_recorder` does not start the exporter's upkeep task, and without
    // it histogram storage grows until a scrape renders it.
    let upkeep_handle = prometheus.clone();
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(Duration::from_secs(5));
            upkeep_handle.run_upkeep();
        }
    });

    // Start the application after installing the recorder, and serve this text
    // however suits your application.
    let _scrape_output = prometheus.render();
    Ok(())
}
```

`PrometheusBuilder::build` and `install` start the upkeep task for you, so use
those if you would rather let the exporter own it. Table 6 lists the server
metrics. Label values are from fixed sets, so application data never becomes a
label.

| Metric                                                    | Kind      | Records                                                  |
| --------------------------------------------------------- | --------- | -------------------------------------------------------- |
| `wireframe_frames_processed_total`                        | Counter   | Frames, by direction.                                    |
| `wireframe_errors_total`                                  | Counter   | Errors, by kind.                                         |
| `wireframe_connection_panics_total`                       | Counter   | Panicking connection tasks.                              |
| `wireframe_decode_rejections_total`                       | Counter   | Decode limit rejections, by path and reason.             |
| `wireframe_preamble_rejections_total`                     | Counter   | Rejected preambles.                                      |
| `wireframe_app_response_sends_total`                      | Counter   | Response send attempts, by `outcome`.                    |
| `wireframe_application_preparations_total`                | Counter   | Preparation attempts, by `outcome`.                      |
| `wireframe_application_preparation_duration_seconds`      | Histogram | Preparation time, by `outcome`.                          |
| `wireframe_prepared_connection_uses_total`                | Counter   | Connections handled by a `PreparedApp`.                  |
| `wireframe_server_startup_failures_total`                 | Counter   | Failed server starts.                                    |
| `wireframe_server_startup_duration_seconds`               | Histogram | Server start time.                                       |
| `wireframe_server_accept_loops_exited_total`              | Counter   | Accept loops that have exited.                           |
| `wireframe_server_supervisor_cancellations_total`         | Counter   | Supervisor cancellations.                                |
| `wireframe_server_supervisor_abnormal_terminations_total` | Counter   | Unexpected supervisor exits.                             |
| `wireframe_connections_active`                            | Gauge     | Live `ConnectionActor` values, **not** accepted sockets. |
| `wireframe_codec_errors_total`                            | Counter   | Recorded only if your code calls `inc_codec_error`.      |

_Table 6: Server metrics._

The `outcome` of `wireframe_app_response_sends_total` is one of `success`,
`serialization_failure`, `codec_rejected`, or `transport_failure`. A failed
send also logs a warning with a matching `category`, along with a `stage` of
`serialize` or `write`, so that logs and metrics can be correlated.

## The client runtime

`WireframeClient` uses the same framing and serialization layers as the server,
so both ends of a protocol share one definition.

### Connecting and calling

Align the frame limit with the server's `buffer_capacity`, because the two are
configured separately and a mismatch fails only once a message crosses the
smaller one:

<!-- tested-example: guide-client-connect -->

```rust,no_run
use std::time::Duration;

use wireframe::{
    app::Envelope,
    client::{ClientCodecConfig, SocketOptions, WireframeClient},
    correlation::CorrelatableFrame,
    message::Message,
};

#[derive(bincode::Encode, bincode::BorrowDecode)]
struct Login {
    username: String,
}

#[derive(bincode::Encode, bincode::BorrowDecode)]
struct LoginAck {
    username: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let socket = SocketOptions::default()
        .nodelay(true)
        .keepalive(Some(Duration::from_secs(30)));

    let mut client = WireframeClient::builder()
        .codec_config(ClientCodecConfig::default().max_frame_length(2048))
        .socket_options(socket)
        .connect("127.0.0.1:7878".parse()?)
        .await?;

    let login = Login {
        username: "guest".to_string(),
    };
    let request = Envelope::new(1, None, login.to_bytes()?);
    let response: Envelope = client.call_correlated(request).await?;
    let (ack, _) = LoginAck::from_bytes(response.payload_bytes())?;
    assert_eq!(ack.username, "guest");
    assert!(response.correlation_id().is_some());

    // Dropping the client without `close` skips the teardown hook.
    client.close().await;
    Ok(())
}
```

To try it against a real server, run the echo example and its login client from
a clone of the repository, in two terminals:

<!-- tested-example: guide-run-client-example -->

```bash
cargo run --example echo --features examples
cargo run --example client_echo_login --features examples
```

Table 7 lists what you can configure on the builder.

| Surface            | API                                    | Default                                        | Use when                                             |
| ------------------ | -------------------------------------- | ---------------------------------------------- | ---------------------------------------------------- |
| Serializer         | `serializer(...)`                      | `BincodeSerializer`                            | The protocol uses another encoding.                  |
| Frame codec        | `codec_config(ClientCodecConfig)`      | 4-byte big-endian prefix, 1024-byte frames     | The server's limit differs from the default.         |
| Socket tuning      | `socket_options(SocketOptions)`        | Operating system defaults                      | You need `TCP_NODELAY` or a keepalive policy.        |
| Preamble           | `with_preamble(T)`                     | Disabled                                       | The protocol negotiates before framed traffic.       |
| Preamble timeout   | `preamble_timeout(Duration)`           | Disabled                                       | A stalled handshake should fail fast.                |
| Setup hook         | `on_connection_setup(...)`             | Disabled                                       | You keep per-connection state.                       |
| Teardown hook      | `on_connection_teardown(...)`          | Disabled                                       | Closing should flush or release something.           |
| Error hook         | `on_error(...)`                        | Disabled                                       | Failures should reach your observability.            |
| Before-send hook   | `before_send(...)`                     | Disabled                                       | You inspect or change bytes before they are written. |
| After-receive hook | `after_receive(...)`                   | Disabled                                       | You inspect or change bytes after they are read.     |
| Tracing            | `tracing_config(TracingConfig)`        | INFO connect and close, DEBUG data; timing off | You want other span levels, or per-command timing.   |
| Pool               | `connect_pool(addr, ClientPoolConfig)` | Disabled; needs the `pool` feature             | You want reusable sockets and bounded fan-out.       |

_Table 7: Client builder configuration._

Call `with_preamble` before `preamble_timeout`, or the timeout is ignored.

For screen readers: the following sequence diagram shows the client lifecycle:
connect, optional preamble exchange, message exchange, error handling, and
teardown.

<!-- tested-example: guide-client-lifecycle -->

```mermaid
sequenceDiagram
    participant App as Application
    participant Builder as WireframeClientBuilder
    participant Stream as TcpStream
    participant Hooks as Client hooks
    App->>Builder: connect(addr)
    Builder->>Stream: apply socket options + connect
    alt preamble configured
        Builder->>Stream: write/read preamble
        Builder->>Builder: preamble success/failure callbacks
    end
    Builder->>Hooks: on_connection_setup
    App->>Stream: send / receive / call / call_correlated
    App->>Hooks: on_error (if operation fails)
    App->>Builder: close()
    Builder->>Hooks: on_connection_teardown
```

_Figure 2: The client connection lifecycle._

### Correlated messages

The client works at two levels. `send`, `receive`, and `call` handle message
types directly. The envelope-aware methods work with any `Packet`, and generate
correlation identifiers from a per-client counter, which `next_correlation_id`
also exposes:

- `send_envelope` sends, generating an identifier if none is set, and returns
  it.
- `receive_envelope` reads the next frame as a packet.
- `call_correlated` does both, and returns `ClientError::CorrelationMismatch`
  if the identifiers differ.

Use `send_envelope` and `receive_envelope` separately when you want more
control:

<!-- tested-example: guide-client-correlation -->

```rust,no_run
use wireframe::{
    app::Envelope,
    client::{ClientError, WireframeClient},
    correlation::CorrelatableFrame,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = WireframeClient::builder()
        .connect("127.0.0.1:7878".parse()?)
        .await?;

    let correlation_id = client.send_envelope(Envelope::new(1, None, b"payload".to_vec())).await?;
    let response: Envelope = client.receive_envelope().await?;
    assert_eq!(response.correlation_id(), Some(correlation_id));

    let outcome: Result<Envelope, ClientError> = client.call_correlated(Envelope::new(1, None, vec![])).await;
    match outcome {
        Ok(response) => println!("{} bytes", response.payload_bytes().len()),
        Err(ClientError::CorrelationMismatch { expected, received }) => {
            eprintln!("expected {expected:?}, received {received:?}");
        }
        Err(other) => eprintln!("request failed: {other}"),
    }
    Ok(())
}
```

### Client preamble exchange

`with_preamble` sends a preamble immediately after connecting. To read the
server's answer, use `on_preamble_success`, which must return any bytes it read
beyond that answer so that they can be replayed to the framing layer:

<!-- tested-example: guide-client-preamble -->

```rust,no_run
use std::time::Duration;

use futures::FutureExt;
use wireframe::{client::WireframeClient, preamble::read_preamble};

#[derive(bincode::Encode, bincode::BorrowDecode)]
struct ClientHello {
    version: u16,
}

#[derive(bincode::BorrowDecode)]
struct ServerAck {
    accepted: bool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = WireframeClient::builder()
        .with_preamble(ClientHello { version: 1 })
        .preamble_timeout(Duration::from_secs(5))
        .on_preamble_success(|_preamble, stream| {
            async move {
                let (ack, leftover) = read_preamble::<_, ServerAck>(stream)
                    .await
                    .map_err(std::io::Error::other)?;
                if !ack.accepted {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::ConnectionRefused,
                        "server rejected preamble",
                    ));
                }
                Ok(leftover)
            }
            .boxed()
        })
        .on_preamble_failure(|error, _stream| {
            async move {
                eprintln!("preamble exchange failed: {error}");
                Ok(())
            }
            .boxed()
        })
        .connect("127.0.0.1:7878".parse()?)
        .await?;
    client.close().await;
    Ok(())
}
```

Failures surface as `ClientError::PreambleTimeout` when the deadline passes,
`ClientError::PreambleRead` when your callback cannot read or decode the
answer, and `ClientError::PreambleEncode` when encoding or writing fails.
`ClientError::PreambleWrite` exists, but the current client never returns it.

### Lifecycle and request hooks

Lifecycle hooks mirror the server's. Setup runs after connecting and any
preamble, and its state goes to teardown when you call `close()`. Dropping a
client without calling `close()` skips teardown. Register setup first, because
it resets the teardown hook.

The error hook sees failures from `send`, `receive`, and `call` operations. It
does not see failures from connecting or from the preamble, because the client
does not exist yet, or errors yielded by a `ResponseStream`.

Request hooks run on bytes. They are synchronous, so capture an `Arc` for any
mutable state, and several of a kind run in the order they were registered:

<!-- tested-example: guide-client-hooks -->

```rust,no_run
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use wireframe::client::WireframeClient;

struct Session {
    requests: AtomicUsize,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let sent = Arc::new(AtomicUsize::new(0));
    let received = Arc::new(AtomicUsize::new(0));
    let (sent_counter, received_counter) = (Arc::clone(&sent), Arc::clone(&received));

    let client = WireframeClient::builder()
        .on_connection_setup(|| async {
            Session {
                requests: AtomicUsize::new(0),
            }
        })
        .on_connection_teardown(|session: Session| async move {
            println!("session ended after {} requests", session.requests.load(Ordering::SeqCst));
        })
        .on_error(|error| {
            let message = error.to_string();
            async move { eprintln!("client error: {message}") }
        })
        .before_send(move |_bytes: &mut Vec<u8>| {
            sent_counter.fetch_add(1, Ordering::SeqCst);
        })
        .after_receive(move |_bytes: &mut bytes::BytesMut| {
            received_counter.fetch_add(1, Ordering::SeqCst);
        })
        .connect("127.0.0.1:7878".parse()?)
        .await?;

    client.close().await;
    Ok(())
}
```

Request hooks fire on `send`, `send_envelope`, `call`, `call_correlated`, the
request in `call_streaming`, `receive`, `receive_envelope`, and
`ResponseStream` polling. Frames from `send_streaming` bypass `before_send`.

### Tracing the client

The client emits `tracing` spans around connection and message operations:
`INFO` for `connect` and `close`, and `DEBUG` for `send`, `receive`, `call`, and
`call_streaming`. `send_streaming` and `receive_streaming` create no spans.
Without a subscriber these cost next to nothing:

<!-- tested-example: guide-client-tracing -->

```rust,no_run
use tracing::Level;
use wireframe::client::{TracingConfig, WireframeClient};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = TracingConfig::default()
        .with_all_levels(Level::TRACE)
        .with_connect_timing(true)
        .with_call_timing(true);

    let client = WireframeClient::builder()
        .tracing_config(config)
        .connect("127.0.0.1:7878".parse()?)
        .await?;
    client.close().await;
    Ok(())
}
```

With timing enabled, an operation emits a `DEBUG` event carrying `elapsed_us`,
on success and on failure. For `call_streaming`, that time ends when the
request has been written, not when the stream ends. Streams also emit
`stream.frames_received` per frame, and `stream.frames_total` when a terminator
arrives.

### Sending large bodies

`send_streaming` sends a body as many frames, prepending your protocol's header
to each chunk:

<!-- tested-example: guide-client-send-streaming -->

```rust,no_run
use std::time::Duration;

use wireframe::client::{SendStreamingConfig, WireframeClient};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = WireframeClient::builder()
        .connect("127.0.0.1:7878".parse()?)
        .await?;

    let header = [0xCA, 0xFE, 0xBA, 0xBE];
    let body = vec![0_u8; 4096];
    let config = SendStreamingConfig::default()
        .with_chunk_size(512)
        .with_timeout(Duration::from_secs(5));

    let outcome = client.send_streaming(&header, &body[..], config).await?;
    println!("sent {} frames", outcome.frames_sent());
    Ok(())
}
```

Without `with_chunk_size`, chunks fill `max_frame_length` less the header, and
an oversized chunk is clamped to that. A chunk size of zero, or a header at
least as long as `max_frame_length`, returns `InvalidInput`. `with_timeout`
bounds the whole operation. On expiry you get
`ClientError::Wireframe(WireframeError::Io(_))` with kind `TimedOut`, and
frames already sent remain sent, so assume partial success. The write side is
then shut down, so do not reuse that connection.

### Reading streamed replies

A server reply can span many frames, ending in a terminator. Teach the client
to recognize yours by overriding `Packet::is_stream_terminator`. **The bundled
`Envelope` has no terminator**, so a stream of `Envelope` values only ends when
the connection does, with an `UnexpectedEof` error.

`call_streaming` sends a request and returns a `ResponseStream`:

<!-- tested-example: guide-client-call-streaming -->

```rust
use futures::{SinkExt, StreamExt};
use tokio::net::TcpListener;
use tokio_util::codec::{Framed, LengthDelimitedCodec};
use wireframe::{
    app::{Packet, PacketParts},
    client::WireframeClient,
    correlation::CorrelatableFrame,
    message::Message,
};

/// An envelope whose message ID 0 means "end of stream".
#[derive(bincode::Encode, bincode::BorrowDecode)]
struct Row {
    id: u32,
    correlation_id: Option<u64>,
    payload: Vec<u8>,
}

impl CorrelatableFrame for Row {
    fn correlation_id(&self) -> Option<u64> { self.correlation_id }

    fn set_correlation_id(&mut self, id: Option<u64>) { self.correlation_id = id; }
}

impl Packet for Row {
    fn id(&self) -> u32 { self.id }

    fn into_parts(self) -> PacketParts {
        PacketParts::new(self.id, self.correlation_id, self.payload)
    }

    fn from_parts(parts: PacketParts) -> Self {
        Self {
            id: parts.id(),
            correlation_id: parts.correlation_id(),
            payload: parts.into_payload(),
        }
    }

    fn is_stream_terminator(&self) -> bool { self.id == 0 }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // A routed Wireframe server sends one reply per request, so this stand-in
    // speaks the same framing with plain Tokio.
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    tokio::spawn(async move {
        let (socket, _) = listener.accept().await.expect("a connection");
        let mut framed = Framed::new(socket, LengthDelimitedCodec::new());
        let bytes = framed.next().await.expect("a request").expect("a frame");
        let (request, _) = Row::from_bytes(&bytes).expect("a valid request");
        for (id, body) in [(1, "first"), (1, "second"), (0, "")] {
            let row = Row {
                id,
                correlation_id: request.correlation_id,
                payload: body.as_bytes().to_vec(),
            };
            let encoded = row.to_bytes().expect("a row encodes");
            framed
                .send(bytes::Bytes::from(encoded))
                .await
                .expect("the client is listening");
        }
    });

    let mut client = WireframeClient::builder().connect(address).await?;
    let request = Row {
        id: 1,
        correlation_id: None,
        payload: vec![],
    };

    let mut rows = client.call_streaming::<Row>(request).await?;
    let mut bodies = Vec::new();
    while let Some(row) = rows.next().await {
        bodies.push(String::from_utf8(row?.payload)?);
    }
    assert_eq!(bodies, ["first", "second"]);
    Ok(())
}
```

If you have already sent the request with `send_envelope`, `receive_streaming`
takes its correlation identifier and returns the same kind of stream. The
stream borrows the client exclusively, so **no other client I/O can happen
until it is drained or dropped**.

The stream checks the identifier on every data frame, yielding
`ClientError::StreamCorrelationMismatch` and ending if one differs. The
terminator is not checked, so any terminator ends the stream cleanly. Transport
and decode failures arrive as `ClientError::Wireframe`.

For protocols that interleave data with notices,
`StreamingResponseExt::typed_with` maps each frame to
`Result<Option<Item>, ClientError>`: `Some` yields an item, `None` skips the
frame, and an error ends the stream:

<!-- tested-example: guide-client-typed-with -->

```rust,no_run
use futures::TryStreamExt;
use wireframe::{
    app::{Packet, PacketParts},
    client::{ClientError, StreamingResponseExt, WireframeClient},
    correlation::CorrelatableFrame,
};

#[derive(bincode::BorrowDecode, bincode::Encode)]
struct Frame {
    id: u32,
    correlation_id: Option<u64>,
    payload: Vec<u8>,
}

impl CorrelatableFrame for Frame {
    fn correlation_id(&self) -> Option<u64> { self.correlation_id }

    fn set_correlation_id(&mut self, id: Option<u64>) { self.correlation_id = id; }
}

impl Packet for Frame {
    fn id(&self) -> u32 { self.id }

    fn into_parts(self) -> PacketParts {
        PacketParts::new(self.id, self.correlation_id, self.payload)
    }

    fn from_parts(parts: PacketParts) -> Self {
        Self {
            id: parts.id(),
            correlation_id: parts.correlation_id(),
            payload: parts.into_payload(),
        }
    }

    fn is_stream_terminator(&self) -> bool { self.id == 0 }
}

struct Row(Vec<u8>);

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = WireframeClient::builder()
        .connect("127.0.0.1:9000".parse()?)
        .await?;
    let request = Frame {
        id: 1,
        correlation_id: None,
        payload: vec![],
    };

    let rows: Vec<Row> = client
        .call_streaming::<Frame>(request)
        .await?
        .typed_with(|frame| match frame.id {
            1 => Ok(Some(Row(frame.payload))),
            2 => Ok(None), // A progress notice, which this caller ignores.
            other => Err(ClientError::from(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("unexpected frame id {other}"),
            ))),
        })
        .try_collect()
        .await?;

    println!("received {} rows", rows.len());
    Ok(())
}
```

Back-pressure comes from TCP flow control: if you read slowly, the receive
window shrinks and the sender's writes eventually wait. No flow-control
messages are involved.

### Client pools

Enable the `pool` feature to use `connect_pool`. A pool keeps a bounded set of
reusable sockets, each **opened on first use**, so `connect_pool` and `acquire`
can succeed with the server down, and the first operation is what reports it:

<!-- tested-example: guide-client-pool -->

```rust,no_run
use std::time::Duration;

use wireframe::client::{ClientPoolConfig, PoolFairnessPolicy, PoolHandle, WireframeClient};

#[derive(bincode::Encode, bincode::Decode, Debug, PartialEq)]
struct Ping(u8);

#[derive(bincode::Encode, bincode::Decode, Debug, PartialEq)]
struct Pong(u8);

#[tokio::main]
async fn main() -> Result<(), wireframe::client::ClientError> {
    let address = "127.0.0.1:7878".parse().expect("a valid address");
    let pool = WireframeClient::builder()
        .connect_pool(
            address,
            ClientPoolConfig::default()
                .pool_size(2)
                .max_in_flight_per_socket(2)
                .idle_timeout(Duration::from_secs(30))
                .fairness_policy(PoolFairnessPolicy::RoundRobin),
        )
        .await?;

    // A lease suits one piece of work.
    let lease = pool.acquire().await?;
    let pong: Pong = lease.call(&Ping(1)).await?;
    assert_eq!(pong, Pong(1));
    drop(lease);

    // A handle suits a long-lived session that should take fair turns.
    let mut session: PoolHandle<_, _, _> = pool.handle();
    let pong: Pong = session.call(&Ping(2)).await?;
    assert_eq!(pong, Pong(2));

    pool.close().await;
    Ok(())
}
```

Table 8 lists the settings. The `Fifo` fairness policy serves waiting handles
in the order they blocked, whereas `RoundRobin`, the default, rotates among
them.

| Setting                    | Default      | Meaning                                                 |
| -------------------------- | ------------ | ------------------------------------------------------- |
| `pool_size`                | 4            | Sockets to maintain.                                    |
| `max_in_flight_per_socket` | 1            | Callers admitted per socket, not parallel writes on it. |
| `max_waiters`              | 64           | Acquisitions that may wait; 0 is treated as 1.          |
| `idle_timeout`             | 600 s        | Replace a socket idle for this long.                    |
| `fairness_policy`          | `RoundRobin` | How blocked handles take turns.                         |

_Table 8: Client pool settings._

Some behaviour worth knowing:

- **Leases forward requests** rather than exposing a client. Socket I/O stays
  serialized per connection. `max_in_flight_per_socket` is an admission budget.
- **Use a lease for split-phase work**, such as `send` followed by `receive`.
  The slot is reserved until `receive` finishes, and calling `call` on that
  lease in between fails and discards the socket. `PoolHandle` does not pin a
  session to one socket, and does not demultiplex responses.
- **Interrupted work discards the socket**, so a late response cannot reach the
  next caller. This covers a cancelled operation and a lease dropped before its
  reply was read. The next operation reconnects.
- **Preambles and setup hooks run per socket.** Reuse keeps preamble state, and
  replacing a socket runs them again.
- **Slots rotate.** Each acquisition starts one slot further round and skips
  full slots. If all are full, it waits.
- **Saturation is immediate.** Once `max_waiters` is reached, `acquire` returns
  `ClientError::PoolSaturated { limit }`. No socket is affected, so retrying
  later is fine.
- **Closing** rejects new acquisitions with `ClientError::PoolClosed`, resolves
  queued ones the same way, and waits for the scheduler task. Existing leases
  are not revoked, and keep their sockets alive until dropped. If the scheduler
  died, you get `ClientError::SchedulerFailed`. Neither error means a socket is
  dirty, and neither is retried automatically.

### Client pool metrics

Table 9 lists the metrics, all with fixed labels, so they never identify slots
or callers.

| Metric                                               | Kind      | Labels and meaning                                                                            |
| ---------------------------------------------------- | --------- | --------------------------------------------------------------------------------------------- |
| `wireframe_pool_acquisitions_total`                  | Counter   | `mode` (`immediate`, `permit_race`), `outcome` (`granted`, `exhausted`, `shutdown`, `error`). |
| `wireframe_pool_permit_races_started_total`          | Counter   | A blocked race began, because every slot was busy.                                            |
| `wireframe_pool_permit_races_total`                  | Counter   | `outcome` (`granted`, `shutdown`, `error`, `cancelled`).                                      |
| `wireframe_pool_permit_race_duration_seconds`        | Histogram | Time spent blocked, by the same `outcome`.                                                    |
| `wireframe_pool_connection_discards_total`           | Counter   | Sockets discarded, by `reason`. Watch this for socket churn.                                  |
| `wireframe_pool_scheduler_failures_total`            | Counter   | Abnormal scheduler exits.                                                                     |
| `wireframe_pool_bookkeeping_poison_recoveries_total` | Counter   | Recoveries from poisoned internal locks.                                                      |

_Table 9: Client pool metrics._

A rising `exhausted` rate means work is arriving faster than permits can serve
it. Dropping your own `acquire()` future does not end a race, which still
records `granted`, so `cancelled` appears only when a pool and all its handles
are dropped without `close()` in the middle of a race. Compare started and
finished races to find any that stay active.

## Building blocks not yet wired into the server

Wireframe ships several parts ahead of the connection runtime that will use
them. Each is tested and usable on its own, but **`WireframeServer` does not
drive any of them**, so adopting one means writing the glue yourself. Table 10
tells you where each stands.

| Building block           | Works today                                  | Not yet                                        |
| ------------------------ | -------------------------------------------- | ---------------------------------------------- |
| `Response` streaming     | Producing frames, through `ConnectionActor`  | Returning a stream from a route handler        |
| `PushQueues`             | Queueing, priorities, rate limits, and a DLQ | Reaching a `WireframeServer` connection        |
| `ConnectionActor`        | Collecting frames into a `Vec`               | Writing to a socket                            |
| `SessionRegistry`        | Tracking handles you insert                  | Being populated automatically                  |
| Streaming request bodies | A channel you own at both ends               | Delivery to handlers                           |
| Extractors               | Running against a request you build          | Being invoked for handlers; `app_data` lookups |
| Recovery policies        | Classifying errors yourself                  | Being consulted when a decoder fails           |
| `on_eof`                 | Implementing the trait method                | Being called by the connection loop            |

_Table 10: What the building blocks do today. DLQ means dead-letter queue._

If one of these is what you need most, say so on its tracking issue:

- [Server connections and streaming responses][issue-874]
- [Push queues and the session registry][issue-875]
- [Streaming request bodies][issue-876]
- [Extractors and `app_data`][issue-877]
- [Recovery policies][issue-878]
- [`on_eof` and the other protocol hooks][issue-879]

[issue-874]: https://github.com/leynos/wireframe/issues/874
[issue-875]: https://github.com/leynos/wireframe/issues/875
[issue-876]: https://github.com/leynos/wireframe/issues/876
[issue-877]: https://github.com/leynos/wireframe/issues/877
[issue-878]: https://github.com/leynos/wireframe/issues/878
[issue-879]: https://github.com/leynos/wireframe/issues/879

### Streaming responses

`Response` models a single frame, a vector of frames, a stream, a multi-packet
channel, or an empty reply. `into_stream` turns any of them into a
`FrameStream`:

<!-- tested-example: guide-response-stream -->

```rust
use async_stream::try_stream;
use futures::TryStreamExt;
use wireframe::response::{FrameStream, Response};

#[derive(Debug, PartialEq)]
struct Frame(u32);

#[tokio::main]
async fn main() {
    let frames: FrameStream<Frame> = Box::pin(try_stream! {
        for value in 0..3 {
            yield Frame(value);
        }
    });
    let response = Response::Stream(frames);

    let collected: Vec<Frame> = response.into_stream().try_collect().await.expect("frames");
    assert_eq!(collected, [Frame(0), Frame(1), Frame(2)]);
}
```

`Response::with_channel` returns a sender and a `Response::MultiPacket`. The
sender waits for room in the channel, which bounds the producer against
whatever is consuming the response. Dropping the last sender ends it:

<!-- tested-example: guide-response-channel -->

```rust
use futures::TryStreamExt;
use wireframe::response::Response;

#[derive(Debug, PartialEq)]
struct Vertex(u32);

#[tokio::main]
async fn main() {
    let (sender, response) = Response::<Vertex>::with_channel(2);

    let producer = tokio::spawn(async move {
        for value in 1..=5 {
            if sender.send(Vertex(value)).await.is_err() {
                return; // The receiver has gone, so stop producing.
            }
        }
    });

    let frames: Vec<Vertex> = response.into_stream().try_collect().await.expect("frames");
    producer.await.expect("the producer finished");
    assert_eq!(frames.len(), 5);
}
```

`Response`, `FrameStream`, and `ConnectionActor` default their protocol error to
`NoProtocolError`, so name the parameter only when a stream carries protocol
failures of its own. `WireframeError` then distinguishes transport failures
from protocol ones.

### The connection actor

`ConnectionActor` merges push queues and a response stream, fairly, under a
cancellation token. `run` collects frames into a `Vec` you supply, and it is up
to you to write them somewhere. So channel capacity bounds a producer against
the actor's polling, and it reaches the socket only if your write path provides
that:

<!-- tested-example: guide-connection-actor -->

```rust
use futures::{StreamExt, stream};
use tokio_util::sync::CancellationToken;
use wireframe::{
    app::{Envelope, Packet},
    connection::ConnectionActor,
    push::PushQueues,
};

#[tokio::main]
async fn main() {
    let (queues, handle) = PushQueues::<Envelope>::builder()
        .high_capacity(8)
        .low_capacity(8)
        .build()
        .expect("valid capacities");

    handle.push_low_priority(Envelope::new(30, None, vec![])).await.expect("queued");
    handle.push_high_priority(Envelope::new(10, None, vec![])).await.expect("queued");

    let reply = stream::iter(vec![Ok(Envelope::new(20, Some(1), vec![]))]).boxed();
    let mut actor: ConnectionActor<Envelope, ()> =
        ConnectionActor::new(queues, handle, Some(reply), CancellationToken::new());

    let mut out = Vec::new();
    actor.run(&mut out).await.expect("the actor drains");
    let mut ids: Vec<u32> = out.iter().map(Envelope::id).collect();
    ids.sort_unstable();
    assert_eq!(ids, [10, 20, 30]);
}
```

Pass a `FairnessConfig` to `set_fairness` to stop sustained high-priority
traffic from starving low-priority frames, and call `enable_fragmentation` to
fragment its output, which is the outbound half only. The actor runs the
protocol hooks that server connections do not:

- `on_connection_setup` receives a `PushHandle` to keep.
- `before_send` and `on_command_end` run for frames and commands.
- `stream_end_frame` provides a terminator for a multi-packet response. The
  default is `None`, and then the client waits until the connection ends. Use
  `set_multi_packet_with_correlation` to stamp the request's identifier; plain
  `set_multi_packet` stamps `None`.

The channel can end as `drained`, `disconnected`, or `shutdown`. The first two
send the terminator through `before_send`. On `shutdown`, the actor logs the
reason and sends none.

`wireframe_connections_active` and `active_connection_count()` count live
actors.

### Push queues

`PushQueues` holds outbound frames at two priorities. Capacities default to 1,
and rate limiting is **on** by default at 100 pushes per second, so configure
both:

<!-- tested-example: guide-push-queues -->

```rust
use tokio::sync::mpsc;
use wireframe::push::PushQueues;

let (dead_letters, _dead_letter_receiver) = mpsc::channel(8);
let (_queues, _handle) = PushQueues::<u8>::builder()
    .high_capacity(8)
    .low_capacity(8)
    .rate(Some(100))
    .dlq(Some(dead_letters))
    .build()
    .expect("valid configuration");

// `unlimited` removes the rate limit.
let (_queues, _handle) = PushQueues::<u8>::builder()
    .high_capacity(8)
    .low_capacity(8)
    .unlimited()
    .build()
    .expect("valid configuration");
```

`build` rejects a zero capacity (`PushConfigError::InvalidCapacity`), and a
rate of zero or above `MAX_PUSH_RATE` (`PushConfigError::InvalidRate`).

`push_high_priority` and `push_low_priority` honour the rate limit, then wait
for channel capacity. `try_push` skips the limiter and applies a `PushPolicy`,
optionally warning and forwarding to the dead-letter queue (DLQ).
**`DropIfFull` without a DLQ loses frames silently.**

If the DLQ is itself full or closed, a throttled `DLQ dropped frames` warning
reports how many frames were lost since the last report, along with the DLQ's
state and the frame's priority, policy, and Rust type, but never its contents.
`dlq_log_every_n` and `dlq_log_interval` set the cadence. Only frames that the
DLQ accepted reach its receiver, and you must consume them from there to
inspect them.

`PushQueues::recv` always prefers the high-priority queue, so it can starve the
low-priority one. Fairness exists only inside `ConnectionActor`. `close`
releases resources when no actor is running.

### Compile-time queue and session configuration

All of the queue builder's setters except `dlq` are `const fn`, so a chain can
run inside a `const fn`, although the builder itself is created at run time.
The frame parameter needs only `Send + 'static`:

<!-- tested-example: guide-const-push-builder -->

```rust
use std::time::Duration;

use wireframe::push::PushQueuesBuilder;

const fn configure<F: Send + 'static>(builder: PushQueuesBuilder<F>) -> PushQueuesBuilder<F> {
    builder
        .high_capacity(8)
        .low_capacity(4)
        .rate(Some(64))
        .dlq_log_interval(Duration::from_secs(5))
}

let builder = configure(PushQueuesBuilder::<Vec<u8>>::default());
drop(builder);
```

`ConnectionId::{new, as_u64}`, `LengthDelimitedFrameCodec::max_frame_length`,
`ConnectionInfo::peer_addr`, `WireframeError::{from_io, from_codec}`, and
`WireframeError::is_clean_close` are `const` too. See also
[Compile-time configuration](#compile-time-configuration).

### Session registry

`SessionRegistry` stores weak references to `PushHandle` values by
`ConnectionId`, so a background task can find a connection without keeping it
alive. Dead entries are pruned on lookup and by `active_handles`. `prune`
removes them without collecting handles. Its `DashMap` holds per-bucket locks
while it does this, so busy systems may prefer running it from a maintenance
task:

<!-- tested-example: guide-session-registry -->

```rust
use wireframe::{
    push::PushQueues,
    session::{ConnectionId, SessionRegistry},
};

let (queues, handle) = PushQueues::<u8>::builder()
    .high_capacity(1)
    .low_capacity(1)
    .build()
    .expect("valid capacities");

let registry: SessionRegistry<u8> = SessionRegistry::default();
let id = ConnectionId::new(42);
registry.insert(id, &handle);
assert!(registry.get(&id).is_some());

drop((queues, handle));
assert!(registry.get(&id).is_none(), "a closed connection leaves no handle behind");
```

`SessionRegistry::default` needs the frame type to implement `Default`, which
`Envelope` does not.

### Streaming request bodies

`RequestParts` separates routing metadata from a body, and `RequestBodyStream`
is a stream of chunks. A bounded `body_channel` connects a sender to a reader,
which `RequestBodyReader` adapts to `AsyncRead`. Today you own both ends, so
back-pressure applies between your own two parties:

<!-- tested-example: guide-request-body -->

```rust
use bytes::Bytes;
use tokio::io::AsyncReadExt;
use wireframe::request::{RequestBodyReader, RequestParts, body_channel};

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let parts = RequestParts::new(42, Some(123), vec![0x01, 0x02]);
    assert_eq!(parts.id(), 42);
    assert_eq!(parts.correlation_id(), Some(123));
    assert_eq!(parts.metadata(), &[0x01, 0x02]);

    let (sender, stream) = body_channel(8);
    tokio::spawn(async move {
        for chunk in [&b"hello "[..], &b"world"[..]] {
            if sender.send(Ok(Bytes::from_static(chunk))).await.is_err() {
                return;
            }
        }
    });

    let mut reader = RequestBodyReader::new(stream);
    let mut body = Vec::new();
    reader.read_to_end(&mut body).await?;
    assert_eq!(body, b"hello world");
    Ok(())
}
```

ADR 002 records the
[design for streaming requests](adr-002-streaming-requests-and-shared-message-assembly.md).

### Extractors

`wireframe::extractor` defines `MessageRequest`, `Payload`, and
`FromMessageRequest`, along with `Message<T>`, `SharedState<T>`,
`ConnectionInfo`, and `StreamingBody`. They resemble Actix Web's extractors,
but handlers cannot declare them as parameters, and the runtime never builds a
`MessageRequest`. To use one, build the request yourself:

<!-- tested-example: guide-extractors -->

```rust
use wireframe::extractor::{ExtractError, FromMessageRequest, MessageRequest, Payload};

struct SessionToken(String);

impl FromMessageRequest for SessionToken {
    type Error = ExtractError;

    fn from_message_request(
        _request: &MessageRequest,
        payload: &mut Payload<'_>,
    ) -> Result<Self, Self::Error> {
        let bytes = payload.as_ref();
        let token = String::from_utf8_lossy(bytes).into_owned();
        let consumed = bytes.len();
        payload.advance(consumed);
        Ok(Self(token))
    }
}

let request = MessageRequest::default();
let mut payload = Payload::new(b"secret");
let token = SessionToken::from_message_request(&request, &mut payload)
    .unwrap_or_else(|_| panic!("the token extracts"));
assert_eq!(token.0, "secret");
```

### Codec recovery policies

`CodecError::default_recovery_policy` classifies an error as shown in Table 11.
The connection loop does not use this, and no builder method installs a
`RecoveryPolicyHook`, so **any decoder error closes the connection today**,
whatever its classification says.

| Error                          | Classification |
| ------------------------------ | -------------- |
| `FramingError::OversizedFrame` | Drop           |
| `FramingError::EmptyFrame`     | Drop           |
| Other `FramingError`           | Disconnect     |
| Any `ProtocolError`            | Drop           |
| Any I/O error                  | Disconnect     |
| Any `EofError`                 | Disconnect     |

_Table 11: The classification from `default_recovery_policy`._

`CodecErrorContext` carries connection details for your own decisions. Implement
`on_eof`, and call it from your own code, to react to end-of-stream conditions:

<!-- tested-example: guide-recovery-hook -->

```rust
use wireframe::{
    codec::{CodecError, CodecErrorContext, EofError, RecoveryPolicy, RecoveryPolicyHook},
    hooks::{ConnectionContext, WireframeProtocol},
};

struct StrictRecovery;

impl RecoveryPolicyHook for StrictRecovery {
    fn recovery_policy(&self, _error: &CodecError, _context: &CodecErrorContext) -> RecoveryPolicy {
        RecoveryPolicy::Disconnect
    }
}

struct Logged;

impl WireframeProtocol for Logged {
    type Frame = bytes::Bytes;
    type ProtocolError = ();

    fn on_eof(&self, error: &EofError, partial: &[u8], _context: &mut ConnectionContext) {
        match error {
            EofError::CleanClose => tracing::info!("connection closed cleanly"),
            EofError::MidFrame { bytes_received, expected } => {
                tracing::warn!(bytes_received, expected, partial = partial.len(), "closed mid-frame");
            }
            EofError::MidHeader { bytes_received, header_size } => {
                tracing::warn!(bytes_received, header_size, "closed mid-header");
            }
        }
    }
}

let context = CodecErrorContext::new()
    .with_connection_id(42)
    .with_correlation_id(123)
    .with_codec_state("seq=5");
assert_eq!(context.connection_id, Some(42));
```

The runtime does not record `wireframe_codec_errors_total` either. It moves
only if your own code calls `wireframe::metrics::inc_codec_error`.

## Testing your protocol

Networks deliver bytes in whatever pieces they like, and your tests should too.
There are two places to look:

- **`wireframe::testkit`**, behind the `testkit` feature, holds drivers and
  assertions you can use from the main crate.
- **The `wireframe_testing` crate** adds codec fixtures, an observability
  harness, a client and server pair, and allocation counting. It re-exports the
  testkit, and enables that feature for you.

<!-- tested-example: guide-testing-dependencies -->

```toml
[dev-dependencies]
wireframe = { version = "=0.4.0-alpha1", features = ["testkit"] }
wireframe_testing = "=0.4.0-alpha1"
```

### Partial frames and slow peers

The chunked drivers encode payloads, then write them in pieces as small as one
byte:

<!-- tested-example: guide-testing-partial-frames -->

```rust
use std::{num::NonZeroUsize, sync::Arc};

use wireframe::{
    app::{Envelope, Handler, WireframeApp},
    codec::LengthDelimitedFrameCodec,
    serializer::{BincodeSerializer, Serializer},
    testkit::drive_with_partial_frames,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let echo: Handler<Envelope> = Arc::new(|_: &Envelope| Box::pin(async {}));
    let app: WireframeApp = WireframeApp::new()?.route(1, echo)?;
    let request = BincodeSerializer.serialize(&Envelope::new(1, Some(7), b"hi".to_vec()))?;

    let codec = LengthDelimitedFrameCodec::new(1024);
    let one_byte = NonZeroUsize::new(1).expect("non-zero");
    let replies = drive_with_partial_frames(app, &codec, vec![request.clone()], one_byte).await?;
    assert_eq!(replies, [request]);
    Ok(())
}
```

The slow-I/O helpers pace reads and writes separately, and are designed for
paused Tokio time:

<!-- tested-example: guide-testing-slow-io -->

```rust
use std::{num::NonZeroUsize, sync::Arc, time::Duration};

use wireframe::{
    app::{Envelope, Handler, WireframeApp},
    codec::LengthDelimitedFrameCodec,
    serializer::{BincodeSerializer, Serializer},
    testkit::{SlowIoConfig, SlowIoPacing, drive_with_slow_codec_payloads},
};

#[tokio::main(flavor = "current_thread", start_paused = true)]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let echo: Handler<Envelope> = Arc::new(|_: &Envelope| Box::pin(async {}));
    let app: WireframeApp = WireframeApp::new()?.route(1, echo)?;
    let request = BincodeSerializer.serialize(&Envelope::new(1, Some(7), b"slow".to_vec()))?;

    let pace = |bytes: usize| {
        SlowIoPacing::new(NonZeroUsize::new(bytes).expect("non-zero"), Duration::from_millis(5))
    };
    let config = SlowIoConfig::new()
        .with_writer_pacing(pace(8))
        .with_reader_pacing(pace(32))
        .with_capacity(64);

    let codec = LengthDelimitedFrameCodec::new(1024);
    let replies = drive_with_slow_codec_payloads(app, &codec, vec![request.clone()], config).await?;
    assert_eq!(replies, [request]);
    Ok(())
}
```

Small capacities combined with reader pacing make an application's writes hit
back-pressure quickly. Table 12 lists the drivers. Many also have a
`_with_capacity` form, and `_mut` forms that borrow the application.

| Function family                   | From                | Feeds                               | Returns          |
| --------------------------------- | ------------------- | ----------------------------------- | ---------------- |
| `drive_with_partial_frames`       | `testkit`           | Payloads, in chunks                 | Payload bytes    |
| `drive_with_partial_codec_frames` | `testkit`           | Payloads, in chunks                 | Decoded frames   |
| `drive_with_fragments`            | `testkit`           | A payload, fragmented               | Payload bytes    |
| `drive_with_fragment_frames`      | `testkit`           | A payload, fragmented               | Decoded frames   |
| `drive_with_partial_fragments`    | `testkit`           | Fragments, in chunks                | Payload bytes    |
| `drive_with_slow_frames`          | `testkit`           | Pre-framed bytes, paced             | Raw output bytes |
| `drive_with_slow_payloads`        | `testkit`           | Default-framed payloads, paced      | Raw output bytes |
| `drive_with_slow_codec_payloads`  | `testkit`           | Codec payloads, paced               | Payload bytes    |
| `drive_with_slow_codec_frames`    | `testkit`           | Codec payloads, paced               | Decoded frames   |
| `drive_with_codec_payloads`       | `wireframe_testing` | Payloads                            | Payload bytes    |
| `drive_with_codec_frames`         | `wireframe_testing` | Payloads                            | Decoded frames   |
| `prepare_and_drive_with_frames`   | `wireframe_testing` | Raw frames, prepared once           | Raw output bytes |
| `drive_prepared_with_frames`      | `wireframe_testing` | Raw frames, reusing a `PreparedApp` | Raw output bytes |

_Table 12: In-memory test drivers._

`encode_payloads_with_codec`, `decode_frames_with_codec`, and
`extract_payloads` help you compose your own patterns.

To keep driving one application across several calls, or to size the in-memory
duplex stream yourself, use the mutable variants that take `&mut WireframeApp`
and a capacity. `wireframe_testing::drive_with_frame_with_capacity_mut` feeds
one frame and `wireframe_testing::drive_with_frames_with_capacity_mut` feeds
several.

### Malformed input

The fixtures in `wireframe_testing` construct raw Hotline-framed bytes
directly, so they can represent data an encoder would refuse to produce:

<!-- tested-example: guide-testing-fixtures -->

```rust
use wireframe::codec::examples::HotlineFrameCodec;
use wireframe_testing::{
    correlated_hotline_wire,
    decode_frames_with_codec,
    oversized_hotline_wire,
    truncated_hotline_header,
    valid_hotline_wire,
};

let codec = HotlineFrameCodec::new(4096);

let wire = valid_hotline_wire(b"hello", 7);
assert_eq!(decode_frames_with_codec(&codec, &wire).expect("decodes").len(), 1);

let wire = oversized_hotline_wire(4096);
assert!(decode_frames_with_codec(&codec, &wire).is_err());

let wire = truncated_hotline_header();
assert!(decode_frames_with_codec(&codec, &wire).is_err());

let wire = correlated_hotline_wire(42, &[b"a", b"b"]);
assert_eq!(decode_frames_with_codec(&codec, &wire).expect("decodes").len(), 2);
```

The others are `valid_hotline_frame`, `mismatched_total_size_wire`,
`truncated_hotline_payload`, and `sequential_hotline_wire`.

### Asserting on reassembly

`wireframe::testkit::reassembly` offers assertions that return a `Result`
rather than panicking, so the same helper works in `rstest` tests and in
`rstest-bdd` steps:

<!-- tested-example: guide-testing-reassembly -->

```rust
use wireframe::{
    message_assembler::MessageKey,
    testkit::{MessageAssemblySnapshot, TestResult, assert_message_assembly_completed_for_key},
};

fn check(snapshot: MessageAssemblySnapshot<'_>) -> TestResult {
    assert_message_assembly_completed_for_key(snapshot, MessageKey(7), b"done")?;
    Ok(())
}
```

For message assembly, there are `assert_message_assembly_incomplete`,
`_completed`, `_completed_for_key`, `_error`, `_buffered_count`,
`_total_buffered_bytes`, and `_evicted`. For fragments, there are
`assert_fragment_reassembly_absent`, `_completed_len`, `_completed_bytes`,
`_error`, `_buffered_messages`, and `_evicted`. Existing
`wireframe_testing::reassembly` imports keep working.

### Logs and metrics

`ObservabilityHandle` captures logs and metrics in one fixture:

<!-- tested-example: guide-testing-observability -->

```rust
use wireframe_testing::ObservabilityHandle;

let mut observability = ObservabilityHandle::new();
observability.clear();

metrics::with_local_recorder(observability.recorder(), || {
    wireframe::metrics::inc_codec_error("framing", "drop");
});

observability.snapshot();
assert_eq!(
    observability.counter(
        wireframe::metrics::CODEC_ERRORS,
        &[("error_type", "framing"), ("recovery_policy", "drop")],
    ),
    1
);
```

Metrics are captured per thread, so the code under test must run on the same
thread as the handle. In async tests, use
`#[tokio::test(flavor = "current_thread")]`. This needs `metrics` in your own
dev-dependencies.

The assertion helpers `assert_counter`, `assert_no_metric`,
`assert_codec_error_counter`, `assert_histogram_recorded`,
`assert_log_contains`, and `assert_log_at_level` **return a `Result` without
panicking**, so propagate them with `?`, or the test asserts nothing. `counter`,
`counter_without_labels`, and `codec_error_counter` read values from the
latest snapshot.

### A real client and server

`wireframe_testing::client_pair` starts a real `WireframeServer` and a connected
`WireframeClient` over a loopback socket, for tests that need the whole
network path. `echo_app_factory` supplies a ready-made application with route 1
and invocation counting:

<!-- tested-example: guide-testing-pair -->

```rust
use std::sync::{Arc, atomic::AtomicUsize};

use wireframe_testing::{TestResult, echo_app_factory, spawn_wireframe_pair};

#[tokio::main]
async fn main() -> TestResult<()> {
    let counter = Arc::new(AtomicUsize::new(0));
    let mut pair = spawn_wireframe_pair(
        echo_app_factory(&counter),
        |builder| builder.max_frame_length(2048),
    )
    .await?;

    assert!(pair.local_addr().port() > 0);

    pair.shutdown().await?;
    Ok(())
}
```

`pair.client_mut()` gives you the client. `spawn_wireframe_pair_default` uses
default client settings. If the client cannot connect, the server is torn down,
so nothing leaks into later tests.

### Counting allocations

`wireframe_testing::benchmark_alloc_support` helps you keep allocation budgets.
Rust permits one `#[global_allocator]` per binary, and a library cannot install
it for you, so a benchmark binary opts in:

<!-- tested-example: guide-allocator -->

```rust,no_run
use std::alloc::System;

use wireframe_testing::benchmark_alloc_support::{INSTRUMENTED_SYSTEM, StatsAlloc, count_allocations};

#[global_allocator]
static GLOBAL_ALLOCATOR: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let count = count_allocations(|| {
        let buffer: Vec<u8> = vec![0; 1024];
        std::hint::black_box(buffer.len());
        Ok(())
    })?;
    println!("{count} allocations");
    Ok(())
}
```

`count_allocations` opens a window, runs your closure, and closes the window
even on error. `count_allocations_or_panic` is for binaries that treat a failed
measurement as fatal. `allocation_count`, `reset_allocation_count`, and
`is_allocation_counting_enabled` work on the current count.

The value includes reallocations, and everything running in the window, such as
the runtime and harness. Compare values across runs of the same binary rather
than treating them as absolute costs.

## Compile-time configuration

Much of the configuration can run in a `const` context, so a value can be
computed by the compiler and checked in a `static` or `const` item. This
matters when a value must exist before `main` runs, and it turns some
configuration mistakes into compile errors. It covers:

- **Application builder.** `WireframeApp::codec`, `fragmentation`, and
  `memory_budgets`, plus `Envelope::new` and `PacketParts::new`.
- **Codec errors.** `CodecError::is_clean_close`, `error_type`, and
  `default_recovery_policy`, and the `RecoveryConfig` methods
  `max_consecutive_drops`, `quarantine_duration`, and `log_dropped_frames`.
- **Top-level errors.** `WireframeError::from_io`, `from_codec`, and
  `is_clean_close`.
- **Error context.** `CodecErrorContext` with `with_connection_id`,
  `with_peer_address`, and `with_correlation_id`.
- **Codec parameters.** `HotlineFrameCodec::new` and `MysqlFrameCodec::new`.
  Their `max_frame_length` is a trait method, and so stays a run-time read.
- **Message assembly.** `ParsedFrameHeader::new`, `FirstFrameHeader`,
  `FrameHeader`, `MessageKey`, `EnvelopeId`, `CorrelationId`,
  `EnvelopeRouting`, and `AssembledMessage::new`.
- **Requests and middleware.** `RequestParts::new`, `id`, `correlation_id`, and
  `metadata_mut`, as well as `FrameContainer`, `FromFn::new`, `Next::new`, and
  the correlation accessors on `ServiceRequest` and `ServiceResponse`.
- **Connection and queue tuning.** `ConnectionActor::set_fairness` and
  `enable_fragmentation`, and the queue builder setters.
- **Fragment metadata.** Constructors and accessors. `FragmentFrame`,
  `FragmentParts`, and `ReassembledMessage` own `Vec<u8>` values, so these can
  be called in a `const fn`, but generally not from a `const` item.

This example builds a recovery policy, an error context, and a codec, then
asserts on the policy without any run-time call:

<!-- tested-example: guide-const-config -->

```rust
use std::{
    net::{Ipv4Addr, SocketAddr, SocketAddrV4},
    time::Duration,
};

use wireframe::codec::{
    examples::HotlineFrameCodec,
    CodecError,
    CodecErrorContext,
    EofError,
    RecoveryConfig,
    RecoveryPolicy,
};

const CLEAN_CLOSE: CodecError = CodecError::Eof(EofError::CleanClose);
const RECOVERY_POLICY: RecoveryPolicy = CLEAN_CLOSE.default_recovery_policy();
const IS_CLEAN_CLOSE: bool = CLEAN_CLOSE.is_clean_close();

// `RecoveryConfig::default()` is not a `const fn`, so a const item spells the
// fields out; the builder methods are `const` and chain from there.
const RECOVERY_CONFIG: RecoveryConfig = RecoveryConfig {
    max_consecutive_drops: 0,
    quarantine_duration: Duration::ZERO,
    log_dropped_frames: false,
}
.max_consecutive_drops(3)
.quarantine_duration(Duration::from_secs(5))
.log_dropped_frames(true);

const ERROR_CONTEXT: CodecErrorContext = CodecErrorContext {
    connection_id: None,
    peer_address: None,
    correlation_id: None,
    codec_state: None,
}
.with_connection_id(7)
.with_peer_address(SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 8080)))
.with_correlation_id(9);

const HOTLINE_CODEC: HotlineFrameCodec = HotlineFrameCodec::new(1024);

const _: () = assert!(matches!(RECOVERY_POLICY, RecoveryPolicy::Disconnect));
const _: () = assert!(IS_CLEAN_CLOSE);
```

A `const fn` cannot run a destructor, which sets the main limit.
`CodecErrorContext` holds an `Option<String>`, and `WireframeError` has a
destructor, so a `const fn` may borrow or return these, but it can never drop
one. This is why `from_io` takes its payload by value and returns the result.

Client configuration works the same way. Most builder setters and socket
shorthands are `const`, along with `TracingConfig`'s setters and
`SendStreamingConfig::with_chunk_size` and `with_timeout`:

<!-- tested-example: guide-const-client -->

```rust
use std::time::Duration;

use wireframe::client::{SocketOptions, TracingConfig, WireframeClientBuilder};

const fn configure(
    builder: WireframeClientBuilder,
    socket_options: SocketOptions,
    tracing: TracingConfig,
) -> WireframeClientBuilder {
    builder
        .socket_options(socket_options)
        .tracing_config(tracing)
        .nodelay(true)
        .keepalive(Some(Duration::from_secs(30)))
        .reuseaddr(true)
}

let builder = configure(
    WireframeClientBuilder::new(),
    SocketOptions::default(),
    TracingConfig::default().with_connect_timing(true),
);
drop(builder);
```

Setters that clamp their input, such as `ClientCodecConfig::max_frame_length`,
`ClientPoolConfig::pool_size`, and `idle_timeout`, are ordinary functions, as
is every `Default::default`. `connect` and `connect_pool` perform I/O, so they
stay at run time. `reuseport` exists only on Unix outside Solaris, illumos, and
Cygwin. The trybuild fixtures in `tests/ui/` keep these promises honest.

## Troubleshooting

### My server does not reply as I expect

- **The reply is my own request.** That is the default: a route with no
  middleware echoes the request's payload. Write the reply in
  [middleware](#middleware).
- **`wrap(from_fn(...))` does not compile.** `wrap` requires a `Transform` that
  outputs a `HandlerService`, [as described here](#what-from_fn-is-for).
- **`WireframeApp::new()` needs type annotations.** Annotate the binding, as in
  `let app: WireframeApp = WireframeApp::new()?;`, so that its default
  parameters apply.
- **My handler will not compile because of a lifetime.** The returned future is
  `'static`, so copy data out of the envelope before the `async` block.
- **`before_send` stopped running.** `with_codec` removes protocol hooks, so
  call `with_protocol` afterwards.
- **Fragmentation stopped working.** `buffer_capacity` and `with_codec` clear
  it, so configure it afterwards.
- **A hook such as `on_command_end` never fires.** Only `before_send` runs on
  server connections, as described in [Protocol hooks](#protocol-hooks).
- **My client is disconnected before it sends anything.** The server reads
  first, under the 10-second [preamble deadline](#connection-preambles), even
  without a preamble.
- **I see no warnings.** You need a `log` logger or a `tracing` subscriber, as
  described in [Logs](#logs).
- **Shutdown hangs.** It waits for open connections, as described in
  [Shutting down](#shutting-down).

### My client gets an error

- **Small requests work and larger ones fail.** Suspect a frame-budget
  mismatch. The client usually reports
  `ClientError::Wireframe(WireframeError::Io(_))`, because the peer closes the
  connection after rejecting the frame. Align
  `ClientCodecConfig::max_frame_length` with the server's `buffer_capacity`.
- **`ClientError::PreambleTimeout`.** The handshake stalled before framed
  traffic began. Check that the server actually reads or replies during the
  preamble.
- **`ClientError::PreambleRead(_)`.** Your success callback could not read or
  decode the answer, usually because of a schema mismatch. Check too that it
  returns any leftover bytes it consumed.
- **`ClientError::PreambleEncode(_)`.** Serializing or writing the preamble
  failed. Write failures are wrapped in `bincode::EncodeError`, so they appear
  here too.
- **The wrong protocol is on that port.** A port expecting TLS, Hypertext
  Transfer Protocol (HTTP), or something else usually produces
  `ClientError::Wireframe(WireframeError::Io(_))` after the first request, or a
  decode error if the reply happens to resemble a frame. Check the host and
  port, and remember that built-in client TLS is future work.
- **`CorrelationMismatch` or `StreamCorrelationMismatch`.** The reply did not
  preserve the request's identifier, so check that the server echoes or stamps
  it.
- **My stream never ends.** The bundled `Envelope` has no terminator. See
  [Reading streamed replies](#reading-streamed-replies).
- **Other calls are blocked.** A `ResponseStream` borrows the client
  exclusively until drained or dropped.
- **`connect_pool` succeeded, then the first call failed.** Pool sockets open
  on first use.
- **Transport disconnects.** Treat
  `ClientError::Wireframe(WireframeError::Io(_))` as a network or peer failure,
  and apply your own reconnect or retry policy. To match on these:

<!-- tested-example: guide-client-error-mapping -->

```rust
use wireframe::{
    WireframeError,
    client::{ClientError, ClientProtocolError},
};

fn describe(error: &ClientError) -> String {
    match error {
        ClientError::Wireframe(WireframeError::Io(error)) => format!("transport failure: {error}"),
        ClientError::Wireframe(WireframeError::Protocol(ClientProtocolError::Deserialize(error))) => {
            format!("decode failure: {error}")
        }
        other => format!("other client error: {other}"),
    }
}
```

## Where to go next

- The [API reference](https://docs.rs/wireframe) documents every public type.
- The `examples/` directory has complete programs, which the
  [README](../README.md#examples) lists.
- The [design document](rust-binary-router-library-design.md) explains the
  architecture and why.
- The [`wireframe_testing` guide](wireframe-testing-crate.md) goes deeper on
  test helpers.
- The
  [production resilience guide](hardening-wireframe-a-guide-to-production-resilience.md)
  covers hardening.
- The [developers' guide](developers-guide.md) is the place to start
  contributing, and the [roadmap](roadmap.md) shows what is coming.
