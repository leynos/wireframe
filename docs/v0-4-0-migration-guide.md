# v0.4.0 migration guide

This guide is for applications moving from Wireframe v0.3.0 to v0.4.0-alpha1,
which has a target release date of 15 November 2026. Most applications need a
handful of small edits. The changes most likely to surprise you are the ones
that still compile, so please read
[Behaviour that changes silently](#behaviour-that-changes-silently) even if
your build is already green.

## How to upgrade

Start with your dependencies. Pin the exact version during the alpha, and keep
the companion test crate in step if you use it:

<!-- tested-example: migration-dependencies -->

```toml
[dependencies]
wireframe = "=0.4.0-alpha1"

[dev-dependencies]
wireframe_testing = "=0.4.0-alpha1"
```

Then work through these steps:

1. Use Rust 1.95 or newer, which the crate now declares as its minimum.
2. Fix the compile errors using
   [Changes the compiler will find](#changes-the-compiler-will-find).
3. Check each item in
   [Behaviour that changes silently](#behaviour-that-changes-silently) against
   your application.
4. Move off the [deprecated methods](#deprecated-methods) when convenient.

Table 1 maps each change to the kind of code it affects, so you can skip what
does not apply.

| If your code…                                        | Read                                                                          |
| ---------------------------------------------------- | ----------------------------------------------------------------------------- |
| Calls `linger` on client sockets                     | [Client `linger` is removed](#client-linger-is-removed)                       |
| Builds `FragmentationConfig` or `Reassembler`        | [Fragment reassembly limits](#fragment-reassembly-limits)                     |
| Matches error enums exhaustively                     | [Error enums gained variants](#error-enums-gained-variants)                   |
| Names `WireframeError`, `Response`, or `FrameStream` | [The default protocol error changed](#the-default-protocol-error-changed)     |
| Uses the `wireframe_testing` decode helpers          | [Test decode helpers now borrow](#test-decode-helpers-now-borrow)             |
| Passes a factory to `WireframeServer::new`           | [The application factory runs once](#the-application-factory-runs-once)       |
| Uses a connection preamble, or none at all           | [Preambles have a deadline](#preambles-have-a-deadline)                       |
| Writes a custom parser or sends trailing bytes       | [Decoding is bounded and exact](#decoding-is-bounded-and-exact)               |
| Sets `total_body_len`                                | [Declared body lengths are binding](#declared-body-lengths-are-binding)       |
| Enables fragmentation with memory budgets            | [Fragments count towards budgets](#fragments-count-towards-budgets)           |
| Implements `before_send`                             | [Protocol hooks see handler responses](#protocol-hooks-see-handler-responses) |
| Uses client pools                                    | [Client pool behaviour](#client-pool-behaviour)                               |
| Relied on Wireframe for a Prometheus exporter        | [Dependencies you may need to declare](#dependencies-you-may-need-to-declare) |

_Table 1: Which changes affect which code._

## Changes the compiler will find

### Client `linger` is removed

`SocketOptions::linger` and `WireframeClientBuilder::linger` are gone, because
duration-based `SO_LINGER` is not safe on Tokio-managed sockets. Delete the
calls, and call `WireframeClient::close().await` for a graceful shutdown.

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

Exhaustive matches on these enums will stop compiling. Table 2 lists what to
handle.

| Enum                   | New variants                                                              | Add a wildcard arm?          |
| ---------------------- | ------------------------------------------------------------------------- | ---------------------------- |
| `ClientError`          | `PoolSaturated { limit }`, `PoolClosed`, `SchedulerFailed`                | Yes, now `#[non_exhaustive]` |
| `MessageAssemblyError` | `DeclaredTotalTooSmall`, `DeclaredTotalExceeded`, `DeclaredTotalMismatch` | Your choice                  |
| `ReassemblyError`      | `TooManyConcurrentPartials`, `AggregateBufferCapExceeded`                 | Your choice                  |
| `ServerError`          | `FactoryBuild`, `Prepare`, `AbnormalTermination`                          | Already required             |

_Table 2: Error variants added since v0.3.0._

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

### Test decode helpers now borrow

In `wireframe_testing`, `decode_frames`, `decode_frames_with_max`, and
`decode_frames_with_codec` take `&[u8]` rather than `Vec<u8>`. Pass `&bytes`.

## Behaviour that changes silently

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

This applies even if you never configured a preamble, because the server still
reads the first byte under the deadline. Two kinds of protocol need attention:

- **Slow handshakes.** Set a larger bound with `preamble_timeout`.
- **Server-speaks-first protocols.** Disabling the deadline does not make the
  server speak first, because it always reads before it writes. The greeting
  must come from a layer that runs before the preamble read.

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
[ADR 015](adr-015-declared-total-body-length-contract.md) for the decision.

### Fragments count towards budgets

Bytes retained by fragment reassembly now count towards memory-budget read
pacing and hard-cap aborts, even with message assembly disabled. A workload
that fragments heavily may start pacing or closing where it did not before, so
review your `memory_budgets` values under realistic load.

### Protocol hooks see handler responses

`before_send` used to run only for push and multi-packet frames. It now also
runs for every `WireframeApp` handler response, including each fragment of a
fragmented one, after the frame is serialized and wrapped and before it is
encoded:

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

## Deprecated methods

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

`prepare` returns `Result<PreparedApp, PrepareError>`. It cannot fail today,
but handling the error keeps your code ready for preparation steps that can.
`PreparedApp::handle_connection` is the variant that logs errors rather than
returning them.

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

## New in this release

These need no migration, but you may want them:

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

## Not in this release

The zero-copy byte migration is **not** part of v0.4.0-alpha1, so there is
nothing to do yet:

- `Serializer::serialize` still returns `Vec<u8>`.
- There is no `PayloadBytes` type or `serialize_to_vec` method. Earlier drafts
  of this guide described them, in error.
- Middleware and hooks keep using `frame_mut()`, `into_inner()`, and `Vec<u8>`.

`FrameCodec` already offers an optional optimization, unchanged since v0.3.0:
store payloads as `Bytes` and override `frame_payload_bytes` to return a cheap
clone. The [users' guide](users-guide.md#zero-copy-payloads) shows how. The
[zero-copy migration roadmap](zero-copy-frame-and-payload-migration-roadmap.md)
tracks the staged plan.

## If you get stuck

If a change is not covered here, or a step does not work as described, please
[open an issue](https://github.com/leynos/wireframe/issues) with the v0.3.0
code you are moving from. A gap in this guide is a bug in this guide.
