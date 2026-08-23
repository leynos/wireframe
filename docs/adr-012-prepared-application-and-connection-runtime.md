# Architectural decision record (ADR) 012: prepared application templates and connection-local runtimes

## Status

Proposed.

First proposed on 2026-08-08 in issue
[#637](https://github.com/leynos/wireframe/issues/637); imported and refined on
2026-08-23 following design review.

## Date

2026-08-23.

## Context and problem statement

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

## Decision drivers

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

## Options considered

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
hyper 1.x constructs a per-connection service value in the accept loop while
routing/configuration state stays shared, and axum's `Router` is the closest
structural precedent — a prepared, cheaply cloneable artefact built once and
handed to every connection.

### Option D: expose both per-server and per-connection application strategies immediately

Add explicit strategy types or constructors and preserve both models as
first-class APIs.

This offers flexibility but doubles the lifecycle surface before the project
has evidence that per-connection application construction is needed.
Per-connection resources can already be created through the connection setup
hook.

### Option E: prepare lazily on the first connection

Wrap `PreparedApp` in an async `OnceCell` and prepare it inside the first
accepted connection, keeping the existing factory API and synchronous startup
untouched.

This is the natural incremental step from the current
`OnceCell<Arc<HashMap<...>>>` code and the smallest possible migration, with an
identical steady-state sharing topology. It is rejected because it forfeits the
readiness guarantee and deterministic startup failure: preparation errors
surface on the first client's connection instead of at boot, and the first
connection pays the preparation latency. Both directly contradict the readiness
and deterministic-error decision drivers.

| Topic                       | Option A: per connection | Option B: per worker | Option C: per server | Option D: both APIs | Option E: lazy first use |
| --------------------------- | ------------------------ | -------------------- | -------------------- | ------------------- | ------------------------ |
| Preparation repeated        | Per connection           | Per worker           | Once                 | Depends on choice   | Once                     |
| Readiness meaning           | Undefined                | Partial              | Precise              | Ambiguous           | Undefined                |
| Ownership boundary fidelity | Weak                     | Accidental           | Strong               | Mixed               | Strong                   |
| Public API surface          | Unchanged                | New                  | Small change         | Doubled             | Unchanged                |
| Migration size              | None                     | Medium               | Medium               | Large               | Small                    |

_Table 1: Trade-offs for application preparation frequency._

## Decision outcome

Adopt Option C.

### 1. Separate the lifecycle phases

Wireframe will distinguish the following concepts, whether initially public or
crate-private:

For screen readers: The following diagram shows the three lifecycle phases — the
`WireframeApp` builder is consumed by `prepare().await` into a `PreparedApp`
template, which is `Arc`-cloned at each independent connection-task boundary
into a per-connection `ConnectionRuntime`.

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

_Figure 1: Application lifecycle phases from builder to connection runtime._

The split follows one uniform template-versus-instance rule: `PreparedApp` owns
immutable definitions, configuration, and factories; `ConnectionRuntime` owns
the per-connection mutable instances derived from them. Where the same domain
appears in both lists below — protocol hooks, message assembly, fragmentation —
the prepared side holds the definition and the runtime side holds the
per-connection invocation or reassembly state, so each concrete responsibility
has exactly one owner.

`prepare()` consumes the builder by value and returns `PreparedApp`, making
post-preparation route registration unrepresentable rather than a runtime
error. This mirrors the sealed `Unbound`/`Bound` typestate that
`WireframeServer` already uses. A reusable template is expressed as
`Arc<PreparedApp>`, so no separately cloneable builder snapshot is needed.

#### `WireframeApp`

`WireframeApp` remains the fluent registration/builder surface. It owns mutable
route and middleware registrations and is not used directly to process a stream
after preparation.

The builder deliberately keeps the `WireframeApp` name for compatibility even
though `PreparedApp` becomes the actual application; Phase 5 should consider a
`WireframeAppBuilder` alias so the vocabulary drift is acknowledged rather than
silent.

#### `PreparedApp`

`PreparedApp` is immutable after construction. It owns by value or `Box`:

- the prepared route table and completed middleware chains;
- serializer and codec templates/configuration;
- application data;
- lifecycle callback definitions;
- protocol and message-assembler definitions;
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
  reassembly, and message-assembly instance state;
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

Application build or preparation errors become distinct `ServerError` variants
— for example `ServerError::AppBuild` and `ServerError::Prepare`, with
source-error chaining — so orchestration and rollback automation can
distinguish preparation failure from `Bind` failure. On failure,
`run_with_shutdown` returns the typed error and the readiness channel is
resolved or observably closed rather than silently dropped, so a readiness
waiter learns of the failure promptly instead of hanging.

Preparation runs arbitrary user middleware transforms, so it must be
observable: wrap `prepare()` in a tracing span, log its duration, and consider
an optional preparation timeout mapped to a `ServerError` variant so a hung
transform fails the deploy loudly instead of stalling readiness forever.

No connection is accepted by a server whose application template failed to
prepare. This guarantee should be structural, not merely tested: the
accept-loop spawn takes the `Arc<PreparedApp>` by value, so an accept loop
cannot exist before preparation has succeeded.

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

The disposition list must also cover the adjacent public surface the split
invalidates:

- `WireframeApp::handle_connection_result` and `handle_connection` are `pub`
  and embody the builder/template/runtime hybrid being dissolved; they should
  be deprecated on `WireframeApp` and re-homed on the `PreparedApp`/
  `ConnectionRuntime` path, with removal acceptable pre-1.0 alongside a
  migration note;
- the `AppFactory` trait's `Clone` bound and its rustdoc contract ("build an
  application instance for a new connection") become vestigial under per-server
  evaluation and must be revised together with the `WireframeServer` rustdoc's
  per-worker claim.

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

The guard mechanism must be panic-aware. Teardown callbacks are async, so a
`Drop`-based guard cannot run them during unwind. The terminal paths are
classified explicitly, and each classification carries its own test:

- **Handler panic, recovered.** The connection task's panic surfaces as a
  task join error observed by the spawner (or an equivalent recovery point);
  teardown runs exactly once on that path.
- **Teardown-callback panic, isolated.** The guard disarms before invoking
  teardown, so a panic inside a teardown callback cannot re-trigger the guard,
  fire teardown twice, or poison later connections.
- **Abandoned unwind.** Paths where no recovery point exists cannot run an
  async teardown; the implementation must enumerate them and record each as an
  accepted, documented gap rather than an implicit omission.
- **`panic = "abort"` builds.** Unwind-dependent paths are vacuous; the
  recovered-panic classification then applies only where the spawner observes
  task failure without unwinding through the runtime.

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

## Rejected shortcuts

- Keeping `WireframeApp` as all three phases and only changing
  `OnceCell<Arc<_>>` to `OnceCell<_>`.
- Building one template per worker without thread affinity or a
  worker-local-state requirement.
- Making the whole builder `Clone` and cloning it into every connection.
- Moving connection-local mutable state into the shared prepared root
  behind locks.

## Migration plan

### Phase 1: introduce internal preparation types

Add `PreparedApp` and `ConnectionRuntime` internally, with compatibility
wrappers for current entry points
([#641](https://github.com/leynos/wireframe/issues/641)).

This phase must also ship the test-harness replacement, because roughly fifteen
test files and fixtures drive `WireframeApp::handle_connection_result` over
duplex streams and Phase 1 restructures the type they depend on. Deliver a
`wireframe_testing`/testkit drive helper (builder, then `prepare().await`, then
a connection runtime over a duplex stream) alongside
[#641](https://github.com/leynos/wireframe/issues/641), and migrate existing
tests through one mechanical, documented substitution.

Compatibility wrappers in this plan are internal scaffolding, not API: they are
removed before this ADR flips to Accepted, verified at
[#649](https://github.com/leynos/wireframe/issues/649) closure.

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

Document factory evaluation semantics, update examples, and expose only the
preparation/runtime APIs that downstream users genuinely need. The concrete
documentation obligations, delivered under
[#649](https://github.com/leynos/wireframe/issues/649), are:

- `docs/users-guide.md`: readiness semantics and factory evaluation
  frequency;
- `docs/developers-guide.md`: the ownership section, reproducing the
  builder/template/runtime diagram from this ADR;
- a versioned migration guide if `WireframeServer::new(factory)` semantics
  change observably, following the existing
  `docs/v0-2-0-to-v0-3-0-migration-guide.md` pattern;
- the `WireframeAppBuilder` alias decision noted in section 1.

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
- A readiness waiter observes preparation failure promptly; preparation
  failure is distinguishable from bind failure.
- A handler panic recovered through the task join path still runs teardown
  exactly once.
- A panic inside a teardown callback does not run teardown twice or poison
  later connections.
- Each abandoned-unwind path enumerated by the implementation is recorded
  with a test or an explicit accepted-gap note.
- Preparation duration is visible in tracing output.
- Property-based or bounded-model coverage exercises the lifecycle
  invariants — preparation runs once, readiness never precedes preparation, and
  teardown fires exactly once — across generated interleavings of terminal
  paths (clean close, decode error, transport error, cancellation, and panic),
  using `proptest`, `loom`, or the bounded checkers described in
  [formal-verification-methods-in-wireframe.md](formal-verification-methods-in-wireframe.md),
  mirroring ADR 013's state-machine verification.

## Outstanding decisions before acceptance

- Whether `PreparedApp` and `ConnectionRuntime` remain internal or gain
  public advanced APIs.
- The exact public migration for `WireframeServer::new(factory)`, including
  the `handle_connection_result`/`handle_connection` and `AppFactory` bound
  dispositions from section 3.
- How testkit helpers expose preparation without making tests depend on
  private internals, given the Phase 1 drive-helper deliverable.

The question of whether preparation consumes the builder irreversibly is
resolved in section 1: `prepare()` consumes the builder, and reuse is expressed
through `Arc<PreparedApp>`.

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
- [axum `Router`](https://docs.rs/axum/latest/axum/struct.Router.html)
