# Architectural decision record (ADR) 012: prepared application templates and connection-local runtimes

## Status

Proposed.

## Date

Proposed 2026-08-08. Imported and refined 2026-08-23.

## Context and Problem Statement

`WireframeApp` currently represents three different lifecycle phases:

1. a mutable builder that collects routes, middleware, lifecycle callbacks,
   protocol hooks, serializer, codec configuration, message assembly, and
   application data;
2. an immutable application template read by every connection;
3. a value constructed afresh by `AppFactory::build` inside each connection
   task.

Those roles have incompatible ownership requirements.

The builder must own mutable registration collections. The immutable template
should build middleware chains once and be shared by independent connection
tasks. The connection runtime should own stream state, lifecycle state, codec
state, message assembly, fragmentation state, and other values that exist for
exactly one connection.

The current hybrid creates several observable problems:

- route chains are built lazily from a fresh app for each connection;
- the route table is stored as
  `OnceCell<Arc<HashMap<u32, HandlerService<E>>>>` and cloned into connection
  handling even though the app remains alive for the whole connection;
- the generic server and examples use different application-sharing
  topologies: `examples/support/runtime_bootstrap.rs` shares one
  `Arc<WireframeApp>` across connections and bypasses `AppFactory` entirely;
- documentation variously describes an app per worker and an app per
  connection; the rustdoc for `WireframeServer` states that each worker
  "receives its own `WireframeApp`", while the code builds one per accepted
  connection;
- application-owned callbacks and protocol objects acquire nested `Arc`
  layers because the shared template itself is not explicit;
- factory failures happen after a connection has already been accepted and
  are logged rather than reported as startup failure; `ServerError` currently
  has only `Bind` and `Accept` variants;
- lifecycle teardown and protocol-hook correctness must be threaded through
  a type that also carries builder-only state.

## Traceability

This ADR governs the application half of Epic
[#635](https://github.com/leynos/wireframe/issues/635) and implements ADR 011's
shared-root rule.

Primary code surfaces:

- `src/app/builder/core.rs`;
- `src/app/builder/routing.rs`;
- `src/app/builder/lifecycle.rs`;
- `src/app/builder/protocol.rs`;
- `src/app/inbound_handler.rs`;
- `src/server/mod.rs`;
- `src/server/runtime.rs`;
- `src/server/runtime/accept.rs`;
- `src/server/connection_spawner.rs`;
- `examples/support/runtime_bootstrap.rs`.

Related issues and decisions:

- Epic [#635](https://github.com/leynos/wireframe/issues/635);
- ADR 011 proposal [#636](https://github.com/leynos/wireframe/issues/636);
- [ADR 010](adr-010-transport-frame-boundary-for-zero-copy.md), whose
  packet/transport-frame boundary remains unchanged;
- [#547](https://github.com/leynos/wireframe/issues/547), normal app
  responses bypass configured protocol hooks;
- [#549](https://github.com/leynos/wireframe/issues/549), teardown is
  skipped after stream-processing errors;
- [#598](https://github.com/leynos/wireframe/issues/598), public protocol
  and message-assembler accessors lack direct coverage;
- [#538](https://github.com/leynos/wireframe/issues/538), zero-copy
  serializer output migration, which remains a separate concern.

Implementation of this ADR is sequenced through:

- [#641](https://github.com/leynos/wireframe/issues/641), `PreparedApp` and
  one-time route/middleware preparation;
- [#642](https://github.com/leynos/wireframe/issues/642), preparing the
  application before server readiness;
- [#643](https://github.com/leynos/wireframe/issues/643), the
  connection-local runtime and centralized lifecycle finalization;
- [#644](https://github.com/leynos/wireframe/issues/644), consolidated
  protocol ownership and hook dispatch;
- [#648](https://github.com/leynos/wireframe/issues/648), collapse of
  nested application-owned sharing beneath `PreparedApp`.

## Decision Drivers

- Prepare immutable routing and middleware state once, before serving
  traffic.
- Make readiness mean that application preparation has succeeded.
- Give each connection explicit ownership of connection-local state.
- Preserve one clear shared ownership root for independent connection
  tasks.
- Surface application construction errors deterministically.
- Avoid arbitrary per-worker state duplication on a multithreaded executor
  where accept-loop tasks are not thread-affine.
- Keep the migration reviewable and avoid coupling it to ADR 010 or the
  zero-copy public API migration.

## Options Considered

### Option A: retain a fresh `WireframeApp` per connection

Continue invoking `AppFactory::build` in the connection task and optimize
individual fields in place.

This preserves current runtime behaviour but repeats application construction,
leaves startup readiness underspecified, and keeps the builder/template/runtime
roles entangled.

### Option B: prepare one application per accept-loop worker

Invoke the factory once per configured worker and share that prepared app among
connections accepted by the worker.

This reduces per-connection construction, but accept-loop tasks are not pinned
to executor threads. Per-worker application state therefore partitions state by
an implementation detail rather than by a meaningful ownership boundary, and it
duplicates route/middleware preparation `workers` times. The per-worker model
suits actix-web, whose workers are single-threaded runtimes with thread-affine
state; Wireframe's accept loops run on a shared multithreaded runtime, so the
analogy does not transfer.

### Option C: prepare one application template per server and create one runtime per connection (preferred)

Invoke the application factory once during server startup, consume the
resulting builder into an immutable prepared template, and share one
`Arc<PreparedApp>` among independent connection tasks. Create connection-local
runtime state after accept.

This aligns preparation, readiness, sharing, and connection ownership. It
matches the shape used across the ecosystem: tower's `MakeService` builds one
factory whose cheap per-connection services derive from shared configuration,
and hyper 1.x constructs a per-connection service value in the accept loop
while routing/configuration state stays shared.

### Option D: expose both per-server and per-connection application strategies immediately

Add explicit strategy types or constructors and preserve both models as
first-class APIs.

This offers flexibility but doubles the lifecycle surface before the project
has evidence that per-connection application construction is needed.
Per-connection resources can already be created through the connection setup
hook.

| Topic                       | Option A: per connection | Option B: per worker | Option C: per server | Option D: both APIs |
| --------------------------- | ------------------------ | -------------------- | -------------------- | ------------------- |
| Preparation repeated        | Per connection           | Per worker           | Once                 | Depends on choice   |
| Readiness meaning           | Undefined                | Partial              | Precise              | Ambiguous           |
| Ownership boundary fidelity | Weak                     | Accidental           | Strong               | Mixed               |
| Public API surface          | Unchanged                | New                  | Small change         | Doubled             |
| Migration size              | None                     | Medium               | Medium               | Large               |

_Table 1: Trade-offs for application preparation frequency._

## Decision Outcome

Adopt Option C.

### 1. Separate the lifecycle phases

Wireframe will distinguish the following concepts, whether initially public or
crate-private:

```text
WireframeApp builder
        │
        │ prepare().await
        ▼
PreparedApp
        │ Arc clone at independent connection-task boundary
        ▼
ConnectionRuntime
```

#### `WireframeApp`

`WireframeApp` remains the fluent registration/builder surface. It owns mutable
route and middleware registrations and is not used directly to process a stream
after preparation.

#### `PreparedApp`

`PreparedApp` is immutable after construction. It owns by value or `Box`:

- the prepared route table and completed middleware chains;
- serializer and codec templates/configuration;
- application data;
- lifecycle callback definitions;
- protocol and message-assembler implementations;
- immutable fragmentation, memory-budget, timeout, and push configuration.

The server owns one `Arc<PreparedApp>` and clones it once per independent
connection task. Values that cannot escape independently should not add another
`Arc` layer merely because the prepared root is shared.

#### `ConnectionRuntime`

`ConnectionRuntime` owns values whose lifetime is exactly one connection:

- accepted/rewound stream and framed codec state;
- connection setup state `C`;
- `ConnectionContext` and protocol-hook invocation state;
- inbound frame pipeline, deserialization failure count, fragment
  reassembly, and message assembly;
- outbound connection actor state, connection-local `Fragmenter`, queues,
  and cancellation observations;
- peer metadata and teardown guard.

Connection-local resources are created through setup/runtime construction, not
by rebuilding the entire application.

### 2. Prepare before readiness

`run_with_shutdown` will:

1. evaluate `AppFactory` once;
2. prepare routes and middleware chains;
3. construct the shared `PreparedApp` root;
4. spawn accept loops;
5. send the readiness signal through the existing `ready_tx` oneshot
   channel.

Application build or preparation errors become `ServerError` variants and fail
startup. No connection is accepted by a server whose application template
failed to prepare.

### 3. Treat `AppFactory` as a startup factory

The existing `WireframeServer::new(factory)` surface may remain during
migration, but the factory is evaluated once per server run rather than once
per connection.

The implementation issue must assess semver impact and choose one of:

- document the new evaluation semantics as a bug/performance correction if
  no supported contract promised per-connection invocation;
- introduce a direct `WireframeServer::from_app(app)` or
  `from_prepared_app` constructor and deprecate ambiguous factory semantics;
- retain an explicitly named per-connection factory API only if a concrete
  use case cannot be expressed through connection setup state.

The final public API disposition must be recorded before this ADR is accepted.

### 4. Build route chains exactly once

Middleware transformation may remain asynchronous, so `prepare()` may be async.
The resulting route map is owned directly by `PreparedApp`; no inner
`Arc<HashMap<...>>` or per-connection `OnceCell` clone is required.

### 5. Make lifecycle cleanup structural

Once setup succeeds, teardown must run exactly once on every terminal path,
including decode, protocol, transport, cancellation, and panic-recovery paths
where execution can safely continue. Today, `handle_connection_result` returns
early when stream processing fails and skips the teardown callback entirely.
The connection runtime should use an explicit guard/finalization path rather
than a clean-path-only tail call. This coordinates with
[#549](https://github.com/leynos/wireframe/issues/549).

### 6. Apply protocol hooks consistently

The prepared protocol definition and connection-local hook context must cover
normal app responses as well as actor-driven push/streaming output. This
coordinates with [#547](https://github.com/leynos/wireframe/issues/547) while
preserving ADR 010's packet-oriented hook decision.

### 7. Unify examples with the production runtime

Examples should stop maintaining a separate `Arc<WireframeApp>` bootstrap
topology. They should exercise the same preparation and server runtime used by
library consumers, unless an example deliberately demonstrates a lower-level
API.

## Consequences

### Positive

- Route and middleware preparation no longer repeats per connection.
- Readiness acquires a precise meaning.
- One `Arc<PreparedApp>` reflects one real shared ownership boundary.
- Connection-local invariants become visible in a dedicated type.
- Startup failures become deterministic and observable.
- Lifecycle and protocol-hook fixes gain one structural home.
- The generic server and examples share one topology.

### Negative

- Existing factory side effects or per-connection assumptions may change
  behaviour.
- `prepare()` introduces an explicit asynchronous startup phase.
- `ServerError` grows application-build/preparation variants.
- More internal types and transition code increase short-term refactor
  size.
- Tests that called `WireframeApp::handle_connection_result` directly may
  need a preparation helper or lower-level harness.

## Rejected Shortcuts

- Keeping `WireframeApp` as all three phases and only changing
  `OnceCell<Arc<_>>` to `OnceCell<_>`.
- Building one template per worker without thread affinity or a
  worker-local-state requirement.
- Making the whole builder `Clone` and cloning it into every connection.
- Moving connection-local mutable state into the shared prepared root
  behind locks.

## Migration Plan

### Phase 1: introduce internal preparation types

Add `PreparedApp` and `ConnectionRuntime` internally, with compatibility
wrappers for current entry points
([#641](https://github.com/leynos/wireframe/issues/641)).

### Phase 2: move route and middleware preparation

Consume handler and middleware registrations during preparation and remove
per-connection route initialization
([#641](https://github.com/leynos/wireframe/issues/641)).

### Phase 3: switch server startup

Evaluate and prepare the app before spawning accept loops and readiness
notification. Thread one prepared root into connection tasks
([#642](https://github.com/leynos/wireframe/issues/642)).

### Phase 4: collapse nested ownership

Replace application-owned `Arc` callback/assembler layers with direct or boxed
ownership where they cannot escape independently
([#648](https://github.com/leynos/wireframe/issues/648)). Consolidate protocol
ownership under [#644](https://github.com/leynos/wireframe/issues/644).

### Phase 5: settle public API and documentation

Document factory evaluation semantics, update examples, add migration notes if
required, and expose only the preparation/runtime APIs that downstream users
genuinely need.

## Verification

- A counter-based test proves the app factory and middleware transforms run
  once per server run, not once per connection.
- Multiple simultaneous connections share the same prepared route table
  without rebuilding it.
- Readiness is not signalled before preparation completes.
- Factory/preparation failure prevents acceptance and returns a typed
  server error.
- Setup/teardown runs exactly once across clean close and error paths.
- Protocol `before_send` applies to ordinary responses and actor-driven
  outputs according to ADR 010.
- Existing handler ordering, codec, fragmentation, message assembly,
  memory-budget, and shutdown tests continue to pass.
- Connection-startup benchmarks compare the old and prepared paths.

## Outstanding Decisions Before Acceptance

- Whether `PreparedApp` and `ConnectionRuntime` remain internal or gain
  public advanced APIs.
- The exact public migration for `WireframeServer::new(factory)`.
- Whether preparation consumes the builder irreversibly or supports a
  separately cloneable reusable prepared template.
- How testkit helpers expose preparation without making tests depend on
  private internals.

## References

- Epic [#635](https://github.com/leynos/wireframe/issues/635)
- ADR 011 proposal [#636](https://github.com/leynos/wireframe/issues/636)
- [ADR 010: transport-frame boundary for the zero-copy migration](adr-010-transport-frame-boundary-for-zero-copy.md)
- [#538](https://github.com/leynos/wireframe/issues/538)
- [#547](https://github.com/leynos/wireframe/issues/547)
- [#549](https://github.com/leynos/wireframe/issues/549)
- [#598](https://github.com/leynos/wireframe/issues/598)
- [tower `MakeService`](https://docs.rs/tower/latest/tower/trait.MakeService.html)
- [hyper 1.x server guide](https://hyper.rs/guides/1/server/hello-world/)
- [actix-web server model](https://actix.rs/docs/server/)
