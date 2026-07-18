# RFC 0001: Protocol-agnostic test harness lifecycle

## Preamble

- RFC number: 0001
- Status: Proposed
- Created: 2026-07-18
- Audience: Wireframe maintainers and downstream protocol-crate authors
- Extends: roadmap item `17.3.2`
- Related: [Wireframe testing crate design](../wireframe-testing-crate.md),
  [client design](../wireframe-client-design.md), and
  [the original pair-harness execution plan][pair-plan]

<!-- markdownlint-disable-next-line MD013 -->
[pair-plan]: ../execplans/17-3-2-in-process-server-and-client-pair-test-harness.md

## 1. Summary

`wireframe_testing::client_pair` currently couples server lifecycle management
with one concrete client configuration. That convenience API remains useful for
the default length-delimited protocol, but it cannot host downstream protocols
that select a different codec, negotiate a typed preamble, or construct their
own client.

This RFC adds a lower-level, protocol-agnostic server lifecycle primitive. It
accepts a fully configured, unbound `WireframeServer`, reserves or accepts a
loopback listener, waits for readiness, and returns a
`RunningWireframeServer`. An optional asynchronous connector composes that
handle with any caller-owned client. Existing `WireframePair` functions remain
source-compatible wrappers for the default client.

The proposal also makes `wireframe_testing` an explicit quality-gate target.
The root package is currently the only default workspace member, so the
Makefile's unqualified Cargo commands do not validate the companion crate's own
tests and doctests. Issue #578 records the resulting documentation drift.

## 2. Problem

The first pair harness solved repeated loopback setup for Wireframe's default
client. Its public shape deliberately stayed concrete because no downstream
consumer had yet demonstrated a broader requirement. The implementation now
contains five constraints that block a real protocol integration:

1. `WireframePair` stores a concrete
   `WireframeClient<BincodeSerializer, RewindStream<TcpStream>, ()>`.
2. The client callback must return the same
   `WireframeClientBuilder<BincodeSerializer, (), ()>` type it receives.
3. Type-changing client methods, including `with_preamble` and
   `on_connection_setup`, cannot pass through that callback.
4. The helper constructs the server internally, so callers cannot configure a
   server preamble, preamble hooks, worker policy, or other server options.
5. No server-only lifecycle handle exists for a protocol-specific client.

The [mxd server behavioural-test design][mxd-design] is the first concrete
downstream case. It needs Wireframe's loopback lifecycle and readiness
handling, but it speaks Hotline framing and performs a Hotline preamble
exchange. A byte adapter cannot repair this mismatch because the incompatible
choices occur before framed traffic begins.

[mxd-design]: https://github.com/leynos/mxd/pull/401

Without an upstream primitive, each downstream crate must reimplement listener
reservation, readiness signalling, task ownership, connection-failure cleanup,
explicit shutdown, and defensive drop behaviour. Repetition at that boundary
invites port races and orphaned tasks.

## 3. Goals and non-goals

### 3.1. Goals

- Separate server lifecycle ownership from the default client convenience API.
- Accept every valid `WireframeServer` serializer, context, packet, codec, and
  preamble combination without erasing its configuration.
- Bind the listener before task spawn and wait for an explicit readiness signal.
- Provide bounded, idempotent, and cancellation-safe explicit shutdown.
- Clean up the server when a caller-supplied client connector fails.
- Keep the existing `WireframePair`, `spawn_wireframe_pair`, and
  `spawn_wireframe_pair_default` APIs source-compatible.
- Preserve real loopback Transmission Control Protocol (TCP) coverage.
- Give failures enough lifecycle-stage context to diagnose startup and cleanup.
- Validate the companion crate directly, including its doctests.

### 3.2. Non-goals

- Do not add a universal client trait or require all clients to implement a
  Wireframe-specific close operation.
- Do not move test-only lifecycle APIs into the production crate surface.
- Do not replace in-memory codec, fragmentation, or slow-I/O drivers.
- Do not add a test-framework-specific runtime adapter to `wireframe_testing`.
- Do not generalize transport beyond TCP; roadmap item `18.1.1` owns that work.
- Do not remove or deprecate the current pair helpers.

## 4. Current architecture

The current `spawn_wireframe_pair` function performs six operations in one
function:

1. reserve a loopback listener with `unused_listener()`;
2. construct a default `WireframeServer` from an app factory;
3. force one worker and bind the existing listener;
4. attach readiness and shutdown channels, then spawn the server task;
5. configure and connect the default `WireframeClient`; and
6. return a concrete pair that owns both resources.

The lifecycle work is reusable. Steps 2 and 5 contain the protocol assumptions.
The target design exposes the reusable middle and retains the current function
as a thin default-protocol façade.

## 5. Proposed design

### 5.1. Layered public surface

The API has three layers:

```plaintext
Protocol-specific test
        |
        +-- spawn_wireframe_server(configured server)
        |       |
        |       `-- RunningWireframeServer
        |
        +-- spawn_wireframe_server_and_connect(server, connector)
        |       |
        |       `-- (RunningWireframeServer, protocol client)
        |
        `-- spawn_wireframe_pair(...)
                |
                `-- existing default WireframePair façade
```

_Figure 1: The server lifecycle is the common foundation. Protocol-specific
clients and the existing default pair compose above it._

This layering keeps the generic boundary at construction time. The running
handle is non-generic because shutdown, task joining, and the local address do
not depend on application or protocol types.

### 5.2. Running server handle

The illustrative public shape is:

```rust,no_run
use std::net::{SocketAddr, TcpListener};

use wireframe::server::{Unbound, WireframeServer};
use wireframe_testing::TestResult;

pub struct RunningWireframeServer {
    // local address, shutdown sender, task handle, and timeout policy
}

impl RunningWireframeServer {
    pub fn local_addr(&self) -> SocketAddr;
    pub async fn shutdown(&mut self) -> TestResult<()>;
}

pub async fn spawn_wireframe_server<F, T, Ser, Ctx, E, Codec>(
    server: WireframeServer<F, T, Unbound, Ser, Ctx, E, Codec>,
) -> TestResult<RunningWireframeServer>;

pub async fn spawn_wireframe_server_on<F, T, Ser, Ctx, E, Codec>(
    server: WireframeServer<F, T, Unbound, Ser, Ctx, E, Codec>,
    listener: TcpListener,
) -> TestResult<RunningWireframeServer>;
```

The omitted generic bounds are exactly those already required by
`WireframeServer`. The implementation must not introduce default serializer,
connection-state, packet, codec, or preamble bounds.

`spawn_wireframe_server` calls `unused_listener()` and delegates to
`spawn_wireframe_server_on`. The latter:

1. binds the supplied listener with `bind_existing_listener`;
2. records `local_addr` before spawning;
3. installs a harness-owned readiness sender;
4. starts `run_with_shutdown` in a Tokio task;
5. waits under a bounded readiness timeout; and
6. returns the non-generic running handle.

The caller configures the server before passing it to the helper. This includes
its app factory, worker count, codec-bearing app type, typed preamble, preamble
success and failure hooks, preamble timeout, and accept backoff. The helper does
not silently replace those settings. The readiness sender is the sole reserved
setting because the harness must know when connection attempts are safe.

The default helper uses documented readiness and shutdown timeouts. A compact
options type may expose overrides if the implementation spike proves that fixed
defaults are insufficient. Timeout configuration must not become a second
server builder.

### 5.3. Lifecycle guarantees

`RunningWireframeServer::shutdown` has these guarantees:

- it is idempotent;
- it signals graceful shutdown before awaiting the task;
- it retains enough internal state for `Drop` to recover if its future is
  cancelled;
- it bounds the join and aborts the task only after timeout;
- it reports task panic, server error, and timeout as distinct failures; and
- it releases the listener before returning success.

`Drop` remains a safety net, not the normal path. Inside a Tokio runtime it
allows a short bounded grace period before aborting. Outside a runtime it aborts
immediately because synchronous drop cannot drive graceful asynchronous
shutdown.

Startup follows the same failure discipline. If the server exits before
readiness, the helper joins the task and returns the underlying join or server
error. A readiness timeout initiates cleanup before returning.

### 5.4. Caller-supplied client connector

A second helper removes the remaining connection boilerplate without claiming
ownership of arbitrary client teardown semantics:

```rust,no_run
use std::{future::Future, net::SocketAddr};

use wireframe_testing::{RunningWireframeServer, TestResult};

pub async fn spawn_wireframe_server_and_connect<S, C, Connect, ConnectFuture>(
    server: S,
    connect: Connect,
) -> TestResult<(RunningWireframeServer, C)>
where
    Connect: FnOnce(SocketAddr) -> ConnectFuture,
    ConnectFuture: Future<Output = TestResult<C>>;
```

Here `S` abbreviates the fully bounded, unbound `WireframeServer` type from the
previous section. The implementation passes the ready server's address to the
connector. If connection fails, it shuts down and joins the server before
returning the connector error.

On success, the tuple preserves ordinary ownership. The caller closes or drops
the protocol client according to its own contract, then explicitly shuts down
the server. A generic pair object is intentionally absent because an arbitrary
`C` may require an asynchronous protocol logout, expose only `Drop`, or have no
close operation at all.

When connection and cleanup both fail, diagnostics must retain both failures;
the cleanup error must not replace the primary connector error. Single failures
should continue to use typed `TestError` variants. The rare combined case may
use a contextual `TestError::Msg` unless a more structured, dependency-safe
error shape emerges during implementation.

### 5.5. Existing pair compatibility

The current public functions remain:

```rust,no_run
pub async fn spawn_wireframe_pair(/* existing parameters */)
    -> TestResult<WireframePair>;

pub async fn spawn_wireframe_pair_default(/* existing parameters */)
    -> TestResult<WireframePair>;
```

Their signatures and observable behaviour do not change. Internally they:

1. build the current default server and set one worker;
2. call the new server-and-connector helper;
3. store the resulting default client and running server; and
4. call `WireframeClient::close` before server shutdown.

`WireframePair` remains concrete. Turning it into `WireframePair<C>` would make
client teardown either incomplete or trait-bound, while creating needless type
noise for the common case. The lower layer supplies extensibility without
burdening the convenience layer with unrelated generic parameters.

### 5.6. Module boundaries

The implementation should extract lifecycle code from `client_pair.rs`:

```plaintext
wireframe_testing/src/
|-- server_harness/
|   |-- mod.rs
|   |-- lifecycle.rs
|   `-- connector.rs
|-- client_pair.rs
`-- lib.rs
```

_Figure 2: `server_harness` owns protocol-neutral lifecycle code, while
`client_pair` remains the default-client façade._

No source file should exceed the repository's 400-line limit. Shared pending-
server and bounded-shutdown machinery belongs in `server_harness`, not in both
modules.

## 6. Behavioural and diagnostic requirements

The implementation must preserve these invariants:

- Listener ownership begins before server task spawn, eliminating the released-
  port race.
- A successful spawn means the readiness signal has fired, not merely that the
  task exists.
- Each running handle owns exactly one shutdown sender and task handle.
- Explicit shutdown and connector-failure cleanup join the server task.
- Dropping one handle cannot affect another parallel test.
- Protocol-specific setup remains outside the lifecycle module.
- Error text identifies the lifecycle stage: bind, readiness, connect,
  shutdown, join, or abort-after-timeout.
- The original connector failure remains primary when cleanup also fails.

The helpers continue to use real loopback TCP. In-memory `duplex` drivers remain
the faster choice for isolated application and codec tests.

## 7. Verification plan

### 7.1. Lifecycle integration tests

Add `rstest` coverage for:

- a configured server becoming ready and accepting a connection;
- a caller-supplied listener retaining its selected local address;
- startup failure before readiness returning the underlying error;
- readiness timeout cleaning up the task and listener;
- idempotent explicit shutdown;
- cancelled shutdown followed by `Drop` cleanup;
- defensive drop inside and outside a Tokio runtime;
- connector failure cleaning up the server;
- simultaneous connector and cleanup failures retaining both diagnostics; and
- parallel handles using distinct listeners without shared state.

### 7.2. Protocol-generic proof

A downstream-style test must configure all boundaries that the current pair
cannot represent:

- a non-default `FrameCodec`;
- a typed server preamble;
- server preamble success or failure hooks;
- a client connector that performs the matching preamble; and
- a protocol-specific client type that is not `WireframeClient`.

The test should exchange at least one framed request and response after the
handshake. A small Hotline-shaped fixture is acceptable; the proof must use the
public `wireframe_testing` API rather than private test helpers.

Behaviour-driven development coverage should describe observable lifecycle and
compatibility behaviour. It should not assert private channel or task fields.

### 7.3. Compatibility coverage

Existing `client_pair_harness` integration and behavioural suites remain green.
Add compile-time coverage demonstrating that the old calls infer the same
public types after the internal refactor.

### 7.4. Companion-crate quality gates

The Makefile and Continuous Integration (CI) workflow must execute the
companion crate explicitly. At minimum:

```bash
cargo test -p wireframe_testing --all-targets --all-features
cargo test -p wireframe_testing --doc --all-features
```

The implementation must repair the currently failing doctests tracked by
[issue #578][issue-578] before publishing the lifecycle extension. Root-package
success alone is not sufficient evidence because `default-members = ["."]` and
the current Make targets omit `--workspace` and `-p wireframe_testing`.

[issue-578]: https://github.com/leynos/wireframe/issues/578

## 8. Migration and release sequencing

The change is additive and should land in five stages:

1. Extract `RunningWireframeServer` and prove lifecycle behaviour without
   changing the existing pair implementation's public surface.
2. Refactor `WireframePair` to delegate to the new lifecycle primitive.
3. Add the asynchronous connector and protocol-generic proof.
4. Add explicit companion-crate tests and doctests to local and CI gates, then
   resolve issue #578.
5. Adopt the primitive in a downstream protocol crate, with mxd as the first
   candidate, before declaring the extension mature.

No downstream migration is mandatory. Existing callers continue to use the
pair helpers. Protocol crates can replace only their local server lifecycle and
keep their own scenario world, database fixtures, codec, and client.

## 9. Risks and mitigations

<!-- markdownlint-disable MD013 -->

| Risk | Mitigation |
| --- | --- |
| The generic server signature produces unreadable diagnostics. | Keep generic bounds in one private helper and expose concrete examples with inferred types. |
| Readiness ownership conflicts with a caller-provided signal. | Reserve `ready_signal` for the harness and document that constraint explicitly. |
| Generic client teardown becomes underspecified. | Return the caller-owned client separately instead of inventing a universal close trait. |
| Connector failure leaks a running server. | Perform explicit shutdown and join before returning the connection error. |
| Drop hides shutdown faults. | Treat explicit `shutdown` as normative and keep `Drop` only as a bounded safety net. |
| The convenience API becomes a breaking generic rewrite. | Keep `WireframePair` concrete and delegate internally. |
| The companion crate drifts again. | Add package-specific all-feature and doctest gates before release. |

<!-- markdownlint-enable MD013 -->

_Table 1: Principal design risks and their containment strategies._

## 10. Rejected alternatives

### 10.1. Generalize only the client-builder callback

A callback returning the same builder type cannot express `with_preamble` or
`on_connection_setup`. Allowing an arbitrary return type merely moves the
problem into the concrete `WireframePair` field and does not expose server
configuration.

### 10.2. Make `WireframePair` generic over every protocol type

This would expose serializer, stream, connection state, preamble, and codec
parameters at the convenience layer. It still would not define how an arbitrary
client closes. The resulting type would be technically general but difficult to
infer, document, and close safely.

### 10.3. Accept raw bytes through the current pair

The mismatch occurs at server construction, client codec selection, and the
preamble exchange. A raw payload adapter begins too late in the connection
lifecycle.

### 10.4. Leave lifecycle ownership to downstream crates

That preserves duplicated listener, readiness, task, and cleanup code in every
protocol integration. These mechanics are Wireframe-specific enough to belong
in its testing companion crate.

### 10.5. Move the primitive into `wireframe::testkit`

The feature is test-facing and already has a natural home in
`wireframe_testing`. Moving it would widen the main crate's optional public
surface without solving a production requirement.

### 10.6. Accept an opaque server-start closure

A callback that receives a listener, readiness sender, and shutdown future can
host any server, but it also lets each caller reinterpret the lifecycle
contract. Accepting Wireframe's public unbound-server typestate keeps binding,
readiness, errors, and shutdown inside one implementation while retaining every
protocol configuration supported by the server builder.

## 11. Open implementation questions

The implementation spike should settle two deliberately narrow questions:

1. whether readiness and shutdown timeout overrides justify a small options
   type in the first release; and
2. whether the combined connector-and-cleanup error needs a new structured
   `TestError` variant or a contextual message is sufficient.

Neither question changes the architectural boundary: configured server in,
non-generic running handle out, optional caller-owned client above it.

## 12. Recommendation

Accept the layered lifecycle design and schedule roadmap items `17.3.3` through
`17.3.5`. The first implementation should extract the running server handle,
retain the existing pair API as a wrapper, prove a custom preamble and codec
through a caller-supplied connector, and make `wireframe_testing` a required
quality-gate target.
