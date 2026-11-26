# Migration guide: v0.3.0 to v0.4.0

## Who should read this

This guide is for applications moving from Wireframe v0.3.0 to v0.4.0-alpha1.
Most need a handful of small edits. The changes most likely to surprise you are
the ones that still compile, so please read
[Check behaviour that changes silently](#3-check-behaviour-that-changes-silently)
even if your build is already green.

## Impact at a glance

Table 1 classifies each change, so you can skip what does not apply.

| Priority           | Area                                      | What to do                                                                                         |
| ------------------ | ----------------------------------------- | -------------------------------------------------------------------------------------------------- |
| Required           | Dependencies and Rust 1.95                | [Update your dependencies](#1-update-your-dependencies)                                            |
| Required if called | Client `linger`                           | [Delete the calls](#client-linger-is-removed)                                                      |
| Required if called | Extractors and `app_data`                 | [Replace them](#removed-extractors-and-app_data)                                                   |
| Required if called | `FragmentationConfig`, `Reassembler`      | [Supply the new limits](#fragment-reassembly-limits)                                               |
| Required if called | Exhaustive error matches                  | [Add the new arms](#error-enums-gained-variants)                                                   |
| Required if called | Protocol errors of `()`                   | [Name the parameter](#the-default-protocol-error-changed)                                          |
| Required if called | `ProtocolHooks` literals                  | [Use `CustomProtocolHooks`](#protocol-hook-ownership-and-dispatch)                                 |
| Required if called | `message_assembler()`                     | [Borrow instead of cloning](#application-owned-values-beneath-preparedapp)                         |
| Required if called | `ConnectionContext`, borrowed serializers | [Adjust construction and bounds](#connection-contexts-and-serializers)                             |
| Required if called | `wireframe_testing` decode helpers        | [Pass a borrow](#test-decode-helpers-now-borrow)                                                   |
| Review if enabled  | `WireframeServer::new`                    | [The factory runs once](#the-application-factory-runs-once)                                        |
| Review if enabled  | Preambles, or none                        | [Deadlines and empty preambles](#preambles-have-a-deadline)                                        |
| Review if enabled  | Custom parsers, trailing bytes            | [Decoding is bounded and exact](#decoding-is-bounded-and-exact)                                    |
| Review if enabled  | `total_body_len`                          | [Declared lengths are binding](#declared-body-lengths-are-binding)                                 |
| Review if enabled  | Fragmentation with memory budgets         | [Fragments count](#fragments-count-towards-budgets)                                                |
| Review if enabled  | `WireframeProtocol`                       | [All hooks now run](#protocol-hooks-see-handler-responses)                                         |
| Review if enabled  | Graceful shutdown, teardown               | [Connections end differently](#shutdown-and-teardown)                                              |
| Review if enabled  | Push queues and `SessionRegistry`         | [Pushes and sessions](#push-queues-and-sessions)                                                   |
| Review if enabled  | Multi-packet correlation                  | [Frames keep their own identifiers](#multi-packet-correlation)                                     |
| Review if enabled  | Client pools                              | [Client pool behaviour](#client-pool-behaviour)                                                    |
| Review if enabled  | Metrics, logs, and exporters              | [Metrics](#metrics-that-mean-something-new), [dependencies](#dependencies-you-may-need-to-declare) |
| Recommended        | `WireframeApp::handle_connection`         | [Move off deprecated methods](#4-move-off-deprecated-methods)                                      |
| Optional           | Streamed replies, pushes, recovery        | [Adopt what is new](#5-adopt-what-is-new)                                                          |

_Table 1: Obligations when moving from v0.3.0 to v0.4.0._

## 1. Update your dependencies

Use Rust 1.95 or newer, which both crates now declare as their minimum. Pin the
exact version during the alpha, and keep the companion test crate in step if
you use it:

<!-- tested-example: migration-dependencies -->
```toml
[dependencies]
wireframe = "=0.4.0-alpha1"

[dev-dependencies]
wireframe_testing = "=0.4.0-alpha1"
```

## 2. Fix what the compiler finds

### Client `linger` is removed

`SocketOptions::linger` and `WireframeClientBuilder::linger` are gone, because
duration-based `SO_LINGER` is not safe on Tokio-managed sockets. Delete the
calls, and call `WireframeClient::close().await` for a graceful shutdown.

### Removed: extractors and `app_data`

`wireframe::extractor`, `AppDataStore`, and `WireframeApp::app_data` are gone.
They were public, but nothing in the runtime called them, so a handler could
never receive what they promised.
[ADR 017](adrs/adr-0017-remove-route-extractors-and-app-data.md) records the
decision. Table 2 says what to use instead.

| Removed                                | Use instead                                            |
| -------------------------------------- | ------------------------------------------------------ |
| `FromMessageRequest`, `MessageRequest` | A function that decodes the envelope                   |
| `Payload`                              | `Envelope::payload_bytes`                              |
| `Message<T>`                           | `T::from_bytes`, or your serializer's decoder          |
| `SharedState<T>`, `app_data`           | An `Arc<T>` captured by the handler                    |
| `ConnectionInfo`                       | `ConnectionContext::peer_addr`, in protocol hooks only |
| `StreamingBody`                        | `RequestBodyReader::new`, on a stream you own          |
| `ExtractError`                         | An error type of your own                              |

_Table 2: Replacements for the removed items._

Before:

<!-- tested-example: migration-extractors-before -->
```text
let app = WireframeApp::new()?
    .app_data(counter)
    .route(1, handler)?;
```

After:

<!-- tested-example: migration-extractors-after -->
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

    let app: WireframeApp = WireframeApp::new()?.route(1, handler)?;
    Ok(app)
}
```

### Fragment reassembly limits

`FragmentationConfig` gains `max_concurrent_partials` and `max_buffered_bytes`.
Add both to any struct literal. Use `None` to disable a limit:

<!-- tested-example: migration-fragmentation-config -->
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
assert_eq!(config.max_concurrent_partials.map(NonZeroUsize::get), Some(64));
```

`Reassembler::new` takes the same two limits after its existing arguments. Pass
`None, None` to keep the old unbounded behaviour:

<!-- tested-example: migration-reassembler-new -->
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
assert_eq!(reassembler.buffered_len(), 0);
```

If you use `FragmentationConfig::for_frame_budget` or `enable_fragmentation()`,
there is nothing to edit, but limits now apply. They allow at most
`min(frame_budget, 64)` partial messages, and buffer the larger of
`frame_budget × 64` and `max_message_size` bytes. A connection exceeding either
aggregate cap closes immediately. Per-message size violations are still counted
as deserialization failures.

### Error enums gained variants

Exhaustive matches on these enums will stop compiling. Table 3 lists what to
handle.

| Enum                   | New variants                                                              | Add a wildcard arm?          |
| ---------------------- | ------------------------------------------------------------------------- | ---------------------------- |
| `ClientError`          | `PoolSaturated { limit }`, `PoolClosed`, `SchedulerFailed`                | Yes, now `#[non_exhaustive]` |
| `MessageAssemblyError` | `DeclaredTotalTooSmall`, `DeclaredTotalExceeded`, `DeclaredTotalMismatch` | Your choice                  |
| `ReassemblyError`      | `TooManyConcurrentPartials`, `AggregateBufferCapExceeded`                 | Your choice                  |
| `PushError`            | `RateLimited`, `FrameTooLarge { size, max }`                              | Already required             |
| `ServerError`          | `FactoryBuild`, `Prepare`, `AbnormalTermination`                          | Already required             |

_Table 3: Error variants added since v0.3.0._

For example, `ClientError` needs a wildcard arm. This is also a good moment to
decide how your application treats a saturated pool, which is temporary and
says nothing about the health of any socket:

<!-- tested-example: migration-client-error-match -->
```rust
use wireframe::client::ClientError;

fn saturation_limit(error: ClientError) -> Option<usize> {
    match error {
        ClientError::PoolSaturated { limit } => Some(limit),
        _ => None,
    }
}
```

And `MessageAssemblyError`, whose new variants all report a protocol violation:

<!-- tested-example: migration-assembly-error-match -->
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

### The default protocol error changed

`WireframeError`, `Response`, `FrameStream`, and `ConnectionActor` used to
default their protocol error parameter to `()`. They now default to
`NoProtocolError`, which lets the default `wireframe::Result<T>` work with
`Box<dyn Error>`. Code that has no protocol errors usually needs no edit. Code
that relied on the old default must now say so:

<!-- tested-example: migration-protocol-error -->
```rust
use wireframe::WireframeError;

let explicit: WireframeError<()> = WireframeError::Protocol(());
assert!(matches!(explicit, WireframeError::Protocol(())));
```

### Protocol hook ownership and dispatch

`ProtocolHooks` was a struct of optional callbacks. It is now an enum of `None`,
`Protocol`, and `Custom`, so that a connection holds one reference to its
protocol. The six callbacks, with the same names, move to `CustomProtocolHooks`.

Before:

<!-- tested-example: migration-protocol-hooks-before -->
```text
let hooks = ProtocolHooks {
    before_send: Some(Box::new(|_frame: &mut Vec<u8>, _context| {})),
    ..ProtocolHooks::default()
};
```

After:

<!-- tested-example: migration-protocol-hooks-after -->
```rust
use wireframe::hooks::{CustomProtocolHooks, ProtocolHooks};

let hooks: ProtocolHooks<Vec<u8>, ()> = CustomProtocolHooks {
    before_send: Some(Box::new(|_frame: &mut Vec<u8>, _context| {})),
    ..CustomProtocolHooks::default()
}
.into();
assert!(matches!(hooks, ProtocolHooks::Custom(_)));
```

Code that inspected the fields to detect an installed protocol should match on
`ProtocolHooks::Protocol(_)`. `ProtocolHooks::from_protocol` keeps its
signature, and `on_connection_setup` runs at most once for each hooks value.
`WireframeApp::protocol()` still returns a clone of the application's `Arc`, to
which each live connection adds exactly one further reference.

### Application-owned values beneath `PreparedApp`

`Arc<PreparedApp>` is the one shared owner for every connection, so the values
beneath it no longer have reference counts of their own. The assembler
accessors on `WireframeApp` and `PreparedApp` now lend a reference:

Before:

<!-- tested-example: migration-message-assembler-before -->
```text
let assembler: Option<Arc<dyn MessageAssembler>> = app.message_assembler().cloned();
```

After:

<!-- tested-example: migration-message-assembler-after -->
```rust
use wireframe::{app::WireframeApp, message_assembler::MessageAssembler};

fn assembler(app: &WireframeApp) -> Option<&dyn MessageAssembler> {
    app.message_assembler()
}
```

If an assembler must outlive the application, keep your own `Arc`, and install
a small type that forwards to it. Handlers are still shared through the `Arc`
you register. Connections no longer need the serializer to be `Clone`.

### Connection contexts and serializers

- **`ConnectionContext`** is no longer a unit struct, and is
  `#[non_exhaustive]`. Replace literals with `ConnectionContext::default()`.
  Contexts from the server carry `connection_id()` and `peer_addr()`.
- **Serializers must be `'static`** to handle a connection, whether through
  `PreparedApp` or the deprecated `WireframeApp::handle_connection*` methods,
  so one that borrows local data no longer compiles.
- **`SessionRegistry::insert`** returns a `#[must_use]` `bool`. See
  [Push queues and sessions](#push-queues-and-sessions).

### Test decode helpers now borrow

In `wireframe_testing`, `decode_frames`, `decode_frames_with_max`, and
`decode_frames_with_codec` take `&[u8]` rather than `Vec<u8>`. Pass `&bytes`.

## 3. Check behaviour that changes silently

These changes still compile, so check each one that applies to you.

### The application factory runs once

`WireframeServer::new` used to call its factory for each connection. It now
calls it once per server run, prepares the result before reporting readiness,
and shares one immutable application across every worker and connection.
Failures surface from the run as `ServerError::FactoryBuild` or
`ServerError::Prepare`.

If your factory created per-connection state, it now creates one shared copy.
Move that state to `on_connection_setup`. If the application is already built,
`WireframeServer::from_app` is the clearer way to say so:

<!-- tested-example: migration-server-from-app -->
```rust
use wireframe::{
    app::WireframeApp,
    server::{ServerError, WireframeServer},
};

fn build_app() -> WireframeApp {
    WireframeApp::default()
}

fn servers() -> Result<(), ServerError> {
    let address = "127.0.0.1:0".parse().expect("a valid address");

    // One-time startup construction through the factory.
    let from_factory = WireframeServer::new(build_app).bind(address)?;

    // Or construct the application before creating the server.
    let from_app = WireframeServer::from_app(build_app()).bind(address)?;

    drop((from_factory, from_app));
    Ok(())
}

fn main() {
    let runtime = tokio::runtime::Runtime::new().expect("a Tokio runtime");
    let _guard = runtime.enter();
    servers().expect("both servers bind");
}
```

### Preambles have a deadline

The server now bounds the preamble read with `DEFAULT_PREAMBLE_TIMEOUT`, which
is 10 seconds. Before, a peer that connected and sent nothing kept a task alive
indefinitely. The deadline covers only the read, not your callbacks or message
handling.

A preamble that encodes to no bytes, including the default `()` when you never
configured one, now reads nothing at all. The server used to wait for a first
byte; it now goes straight to your application, which may speak first. Two
kinds of protocol need attention:

- **Slow handshakes.** Set a larger bound with `preamble_timeout`.
- **Server-speaks-first protocols with a preamble.** Disabling the deadline does
  not make the server speak first, because a preamble that has bytes is read
  before anything is written. The greeting must come from a layer that runs
  before the preamble read.

<!-- tested-example: migration-preamble-timeout -->
```rust
use std::time::Duration;

use wireframe::{app::WireframeApp, server::WireframeServer};

fn build_app() -> WireframeApp {
    WireframeApp::default()
}

fn main() {
    let slower_handshake = WireframeServer::new(build_app).preamble_timeout(Duration::from_secs(30));
    let no_deadline = WireframeServer::new(build_app).disable_preamble_timeout();
    drop((slower_handshake, no_deadline));
}
```

The last of the two setters called wins. `Duration::ZERO` clamps to 1 ms, so use
`disable_preamble_timeout()` to remove the deadline, which also brings back
the availability risk it was added to close.

A preamble success callback that returns an error now **closes the
connection**. Before, the error was logged and the connection carried on. Write
any rejection message before returning the error, and do not return `Err` for a
condition you consider non-fatal.

### Decoding is bounded and exact

The bundled bincode decoders now limit the work a peer can demand with a
declared container length. `Message::from_bytes`, `BincodeSerializer`,
reassembled messages, and the Serde bridge share a 16 MiB budget
(`MAX_FRAME_LENGTH`), and preamble decoding has its own 1 KiB budget. Custom
decoders and resource needs specific to your types still need their own
validation.

An inbound `Envelope` frame must now contain exactly one value, so **trailing
bytes are a decoding error**. If a producer appended data after an envelope,
move that data inside the envelope or send another frame. The same rule binds
custom serializers: a `FrameMetadata::parse` implementation that skips full
deserialization, or a custom `deserialize`, must report consuming the whole
frame.

Lower-level APIs keep their prefix behaviour: `Message::from_bytes` still
returns the bytes consumed, and `read_preamble` still returns leftovers. The new
`wireframe_decode_rejections_total` counter shows what is being rejected.

### Declared body lengths are binding

`FirstFrameHeader::total_body_len` was a hint; it is now a commitment. A
message whose body does not match its declared total is rejected, counted as a
deserialization failure, and never reaches a handler. If your totals were only
advisory, set the field to `None`, which keeps the old behaviour. See
[ADR 015](adrs/adr-0015-declared-total-body-length-is-authoritative.md) for the
decision.

### Fragments count towards budgets

Bytes retained by fragment reassembly now count towards memory-budget read
pacing and hard-cap aborts, even with message assembly disabled. A workload
that fragments heavily may start pacing or closing where it did not before, so
review your `memory_budgets` values under realistic load.

### Protocol hooks see handler responses

Server connections now run every `WireframeProtocol` hook. `before_send` used
to run only for push and multi-packet frames. It now runs exactly once for
every outbound frame, including each fragment of a fragmented reply, after the
frame is serialized and wrapped and before it is encoded:

<!-- tested-example: migration-before-send-order -->
```text
serialize -> FrameCodec::wrap_payload -> before_send -> codec encode -> socket
```

If your hook marks or edits frames, it now affects ordinary replies as well.
Its output must stay within the codec's `max_frame_length`. The default codec
rejects an oversized frame, which fails the send, while a custom encoder must
enforce the limit itself. The direct helpers `send_response`,
`send_response_framed`, and `send_response_framed_with_codec` still bypass the
hook.

The other hooks now fire too, so look for code of yours that stood in for them:

- **`on_connection_setup`** runs once, before the first frame is read, with a
  handle for pushing frames to that connection.
- **`on_command_end`** runs after each completed reply, including an ordinary
  `route` reply. It is not a cleanup guarantee: a cancelled reply skips it, as
  does one whose terminator could not be written.
- **`handle_error`** receives a protocol error from a streamed reply. The
  connection then closes. A malformed frame does not reach it.
- **`on_eof`** runs when a peer vanishes part-way through a frame, and not after
  a clean close. A custom codec must return `CodecError::Eof` with
  `EofError::MidHeader` or `EofError::MidFrame` from `decode_eof` to take part.

The [users' guide](users-guide.md#protocol-hooks) has the whole contract.

### Shutdown and teardown

- **Shutdown ends idle connections.** It used to wait for every peer to
  disconnect. A connection waiting for input now closes when the server stops.
  Running handlers are still waited for, without a deadline. A streamed reply
  continues while it makes progress, and a write that would block is abandoned.
  See
  [ADR 014](adrs/adr-0014-server-shutdown-ownership-and-terminal-observation.md).
- **Teardown runs after errors and handler panics.** A stream-processing error
  used to skip it. It now runs exactly once, after the transport is released,
  for every ending the connection task survives. A handler panic is raised
  again once teardown succeeds, so it is still logged and counted.
- **Teardown has limits.** It does not run if setup was absent, failed, or never
  finished, or if the connection future is dropped or aborted first. Stopping
  the server is not one of those: the connection ends and teardown runs. A
  teardown that panics is not retried, and `panic = "abort"` builds neither
  recover a handler panic nor run destructors.

### Push queues and sessions

Server connections now have push queues, and register themselves with the
application's `SessionRegistry`.

- **Identifiers come from the registry.** They start at 1 and are never reused.
  Address a connection by the one it publishes, not one you chose.
- **`insert` refuses to replace a live entry**, and returns `false`. Call
  `remove` first if you mean to replace one, or keep your own entries in a
  separate registry.
- **`try_push` obeys the rate limit.** It used to bypass it, and now returns
  `PushError::RateLimited` when there is room but no permit.
- **The awaited pushes reserve capacity first**, and then wait for a permit. The
  order used to be the other way round.
- **`with_codec` clears the queue template and registry**, as it does the
  protocol.

### Decoder failures

With no recovery hook installed, a decoder failure still closes the connection.
Each failure now increments `wireframe_codec_errors_total`. To choose a policy,
see [Codec recovery policies](users-guide.md#codec-recovery-policies).

### Multi-packet correlation

When a request carries no correlation identifier, a multi-packet frame that
supplies its own now keeps it, rather than having it cleared. A frame that
differs from the request's identifier is still overwritten, with a warning. On
a server connection, a terminator keeps whatever identifier `stream_end_frame`
gave it.

### Client pool behaviour

- **Waiters are capped.** At most `max_waiters` acquisitions may queue, 64 by
  default. Further attempts return `ClientError::PoolSaturated { limit }` at
  once. Raise it with `ClientPoolConfig::max_waiters` for bursty callers.
- **Closed pools report it.** Acquiring from a closed pool used to return a
  recyclable I/O disconnection, and now returns `ClientError::PoolClosed`. A
  scheduler that died abnormally returns `ClientError::SchedulerFailed`.
  Neither means a socket is dirty, so discard the pool and reconnect instead of
  retrying.
- **`close()` waits longer.** It now returns once every queued acquisition has
  been resolved and the scheduler task has finished.
- **Interrupted work discards the socket.** A socket is no longer reused after a
  cancelled operation or an unread split-phase reply, so a late response cannot
  reach the next caller. Finish `send` and `receive` on the same lease to keep
  the socket warm. `wireframe_pool_connection_discards_total` counts these.

<!-- tested-example: migration-pool-closed -->
```rust,no_run
use wireframe::client::{ClientError, ClientPoolConfig, WireframeClient};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let address = "127.0.0.1:7878".parse()?;
    let pool = WireframeClient::builder()
        .connect_pool(address, ClientPoolConfig::default().max_waiters(128))
        .await?;
    let mut handle = pool.handle();
    pool.close().await;

    let refused = handle.acquire().await;
    assert!(matches!(refused, Err(ClientError::PoolClosed)));
    Ok(())
}
```

### Metrics that mean something new

- **`wireframe_connections_active`** counts accepted connections, from `accept`
  until the connection task ends.
- **`wireframe_codec_errors_total`** is recorded by the server. An oversized
  frame has the `error_type` of `framing`, so an alert on `io` must match that
  too.

### Dependencies you may need to declare

Wireframe no longer brings these along at run time. Declare them yourself if
you were relying on feature unification to provide them:

- **`metrics-exporter-prometheus`** is now only a development dependency.
- **`tracing-subscriber`** was never a part of Wireframe's API, and is now only
  a development dependency. Wireframe has no subscriber of its own, and an
  application that installs one keeps working.

<!-- tested-example: migration-observability-dependencies -->
```toml
[dependencies]
metrics-exporter-prometheus = "0.18"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
```

See [Observability](users-guide.md#observability) for how to install them.

### Smaller changes

- Dead-letter queue warnings no longer include frame contents, which matters if
  you scrape those logs.
- `SendError`'s message now begins `serialization error:`, not
  `serialisation error:`. The prefixes are a stable contract from this release.
- Dropping a `run_with_shutdown` future now cancels the accept loops and
  releases the listener, so keep the future alive for as long as the server
  should run.

## 4. Move off deprecated methods

`WireframeApp::handle_connection` and `WireframeApp::handle_connection_result`
are deprecated, which fails builds using `-D warnings`. They rebuild the
middleware chain for every connection. If you accept streams yourself, prepare
the application once and reuse the result:

<!-- tested-example: migration-prepared-app -->
```rust
use tokio::io::duplex;
use wireframe::app::{PreparedApp, WireframeApp};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Register every route and middleware first: `prepare` consumes the builder.
    let prepared: PreparedApp = WireframeApp::new()?.prepare().await?;

    let (client_one, server_one) = duplex(64);
    drop(client_one);
    prepared.handle_connection_result(server_one).await?;

    let (client_two, server_two) = duplex(64);
    drop(client_two);
    prepared.handle_connection_result(server_two).await?;
    Ok(())
}
```

`prepare` returns `Result<PreparedApp, PrepareError>`. It fails with
`PrepareError::InvalidPushConfig` if a push-queue template has an unusable
capacity or rate. `PreparedApp::handle_connection` is the variant that logs
errors rather than returning them.

The `wireframe_testing` builder drivers such as `drive_with_frames` carry no
deprecation warning, but they use the deprecated path internally. Tests that
want to prove one-time middleware preparation should move to the prepared
drivers:

<!-- tested-example: migration-prepared-test-drivers -->
```rust
use wireframe::app::WireframeApp;
use wireframe_testing::{drive_prepared_with_frames, prepare_and_drive_with_frames};

async fn drive() -> Result<(), Box<dyn std::error::Error>> {
    let app: WireframeApp = WireframeApp::new()?;
    let _once = prepare_and_drive_with_frames(app, Vec::new()).await?;

    let app: WireframeApp = WireframeApp::new()?;
    let prepared = app.prepare().await?;
    let _reused = drive_prepared_with_frames(&prepared, Vec::new()).await?;
    Ok(())
}
```

`WireframeApp::with_push_dlq` is deprecated too. Its sink received serialized
payloads, and nothing is sent to it now, which a warning says when it is
configured. Put a `dlq` sender on the template you give to `with_push_queues`,
and dropped frames arrive as codec frames.

## 5. Adopt what is new

These need no migration, but you may want them:

- **`WireframeApp::route_response`** lets a handler return a reply of one frame,
  several, or a stream. See
  [Response-producing routes](users-guide.md#response-producing-routes).
- **`with_push_queues` and `with_session_registry`** let a server send frames
  nobody asked for. See
  [Server connections and push queues](users-guide.md#server-connections-and-push-queues).
- **`with_recovery_hook` and `recovery_config`** decide what a decoder failure
  does. See [Codec recovery policies](users-guide.md#codec-recovery-policies).
- **`WireframeServer::spawn`** runs the server in the background and returns a
  cloneable `ServerShutdown` with `stop()` and `drained()`. See
  [Running servers](users-guide.md#running-servers).
- **`benchmark_alloc_support`** in `wireframe_testing` measures allocations. See
  [Counting allocations](users-guide.md#counting-allocations).
- **`drive_with_frame_with_capacity_mut` and
  `drive_with_frames_with_capacity_mut`** are now exported from the
  `wireframe_testing` crate root. See
  [Testing your protocol](users-guide.md#testing-your-protocol).
- **More `const fn`s** across configuration. See
  [Compile-time configuration](users-guide.md#compile-time-configuration).

### Copied-byte counters

`wireframe::benchmark_counters` is new since v0.3.0, so this concerns you only
if you followed development builds. `CopiedBytes` gained a `deserialized`
field, for the copy made when an inbound frame becomes an envelope, and
`record_deserialized` records it. A literal that names every field must add it:

Before:

<!-- tested-example: migration-copied-bytes-before -->
```text
let copied = CopiedBytes {
    serialized: 0,
    handoff: 0,
    framed: 0,
};
```

After:

<!-- tested-example: migration-copied-bytes-after -->
```rust
use wireframe::benchmark_counters::CopiedBytes;

let copied = CopiedBytes {
    serialized: 0,
    handoff: 0,
    framed: 0,
    deserialized: 0,
};
assert_eq!(copied.total(), 0);
```

## What does not need to change

- **Route handlers.** The `Handler<E>` signature, `route`, and its default echo
  reply are unchanged.
- **Middleware.** `Transform`, `Service`, and `wrap` are unchanged.
- **Memory budgets and fragmentation wire format.** Values and bytes keep their
  meaning, aside from the limits described above.

The zero-copy byte migration is **not** part of this release either, so there
is nothing to do for it yet:

- `Serializer::serialize` still returns `Vec<u8>`.
- There is no `PayloadBytes` type or `serialize_to_vec` method. Earlier drafts
  of this guide described them, in error.
- Middleware and hooks keep using `frame_mut()`, `into_inner()`, and `Vec<u8>`.

`FrameCodec` already offers an optional optimization, unchanged since v0.3.0:
store payloads as `Bytes` and override `frame_payload_bytes` to return a cheap
clone. The [users' guide](users-guide.md#zero-copy-payloads) shows how. The
[zero-copy migration roadmap](zero-copy-frame-and-payload-migration-roadmap.md)
tracks the staged plan.

## Migration checklist

- [ ] Build with Rust 1.95 or newer, and pin both crates.
- [ ] Remove `linger`, extractors, and `app_data`.
- [ ] Add the fragmentation limits and the new error arms.
- [ ] Update `ProtocolHooks`, `ConnectionContext`, and assembler code.
- [ ] Move per-connection state out of the application factory.
- [ ] Check preambles, trailing bytes, and declared body lengths.
- [ ] Check protocol hooks for work that now happens twice.
- [ ] Review shutdown, teardown, pushes, and client pools.
- [ ] Update dashboards, alerts, and log scraping.
- [ ] Declare exporter and subscriber dependencies you used indirectly.
- [ ] Move off the deprecated methods.

## Common errors and fixes

- **Error:** ``unresolved import `wireframe::extractor` ``
  - **Fix:** [replace the extractor](#removed-extractors-and-app_data).
- **Error:** ``no method named `app_data` found for struct``
  - **Fix:** [capture an `Arc`](#removed-extractors-and-app_data).
- **Error:** ``cannot find value `ConnectionContext` in this scope``
  - **Fix:** use
    [`ConnectionContext::default()`](#connection-contexts-and-serializers).
- **Error:** `cannot create non-exhaustive struct using struct expression`
  - **Fix:** if it names `ConnectionContext`, use
    [`ConnectionContext::default()`](#connection-contexts-and-serializers).
- **Error:** ``non-exhaustive patterns: `_` not covered``
  - **Fix:** add [the arms for the new variants](#error-enums-gained-variants).
- **Error:** ``expected `NoProtocolError`, found `()` ``
  - **Fix:** [name the protocol error](#the-default-protocol-error-changed).
- **Error:** `push rate limit reached`
  - **Fix:** raise the rate, wait with `push_low_priority`, or
    [retry later](#push-queues-and-sessions).
- **Error:** `client pool waiter limit exceeded (64)`
  - **Fix:** raise [`max_waiters`](#client-pool-behaviour), or shed load.

## If you get stuck

If a change is not covered here, or a step does not work as described, please
[open an issue](https://github.com/leynos/wireframe/issues) with the v0.3.0
code you are moving from. A gap in this guide is a bug in this guide.
