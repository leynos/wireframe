# Architectural decision record (ADR) 011: runtime ownership and task-lifetime boundaries

## Status

Proposed.

First proposed on 2026-08-08 in issue
[#636](https://github.com/leynos/wireframe/issues/636); imported and refined on
2026-08-23 following design review.

## Date

2026-08-23.

## Context and problem statement

Tokio does not intrinsically require application state to live behind `Arc`.
The requirement appears when Wireframe creates an independently scheduled
`'static` task, shares one logical resource among genuinely independent owners,
or uses a Tokio primitive whose owned handle embeds shared state.

Wireframe currently mixes those cases with several scope-bound or sole-owner
values that also use `Arc` or owned cancellation futures. The result is a
blurred ownership graph:

- some values cross a real task boundary and correctly require shared
  ownership;
- some values remain within one actor but still sit behind `Arc`;
- some futures could borrow from the current task but instead clone an owned
  handle;
- some coordination state has one conceptual owner but is represented as
  `Arc<Mutex<_>>` plus spawned service tasks;
- some immutable objects acquire one strong reference per callback adapter
  rather than one per independently owned runtime.

Concrete instances of each pattern exist in the current tree. Each is annotated
with its frequency class so baseline and benchmark effort
([#639](https://github.com/leynos/wireframe/issues/639)) concentrates on the
hot cases; the one-time cases are justified on clarity grounds alone:

- the connection actor clones its `CancellationToken` and awaits
  `cancelled_owned()` inside its own `select!` loop (`src/connection/mod.rs`,
  `src/connection/polling.rs`), where the borrowing `cancelled()` future would
  suffice — once per actor-loop iteration;
- `PushHandle::push_with_priority` clones an `mpsc::Sender` before calling
  `send`, although `send` takes `&self` (`src/push/queues/handle.rs`) — once
  per pushed message;
- the client-pool scheduler couples an `Arc<PoolScheduler>` root, a
  `Mutex<SchedulerState>`, an atomic single-flight flag, and spawn-on-demand
  service tasks (`src/client/pool/scheduler.rs`) — once per acquisition or
  lease drop;
- `ProtocolHooks::from_protocol` clones the same `Arc` of the protocol
  object once per callback adapter — six clones for one protocol
  (`src/hooks.rs`) — once per connection;
- the connection actor stores a connection-local fragmenter as
  `Option<Arc<Fragmenter>>` although it is never shared
  (`src/connection/mod.rs`) — once per connection;
- `FnService` stores its middleware function as `Arc<F>` despite being its
  sole owner (`src/middleware.rs`) — once per application build.

Without an explicit policy, local fixes can remove individual clones while
later changes recreate the same topology under different names.

## Prior art

Tokio's own guidance frames the policy this ADR adopts. The Tokio tutorial
states that spawned tasks must satisfy `'static` and that `Arc` is the tool for
data genuinely shared between concurrent tasks — not a default for all task
state.[^1] `tokio_util::sync::CancellationToken` deliberately offers both
`cancelled(&self)`, whose future borrows the token, and
`cancelled_owned(self)`, whose future consumes an owned clone; the borrowed
form exists precisely for persistent owners awaiting cancellation in place.[^2]
`TaskTracker` pairs with `CancellationToken` as the recommended
graceful-shutdown mechanism for genuinely independent tasks.[^3] The actor rule
follows the widely adopted pattern described by Alice Ryhl: one task
exclusively owns the state, handles communicate through a command enum with
oneshot replies, and closing the mailbox is the shutdown signal.[^4]

## Traceability

This ADR governs Epic [#635](https://github.com/leynos/wireframe/issues/635)
and especially the following runtime surfaces:

- `src/connection/mod.rs` and `src/connection/polling.rs`;
- `src/push/queues/handle.rs`;
- `src/middleware.rs`;
- `src/hooks.rs`;
- `src/server/runtime.rs` and `src/server/connection_spawner.rs`;
- `src/client/pool/*`.

It complements rather than supersedes:

- [ADR 010](adr-010-transport-frame-boundary-for-zero-copy.md), which fixes
  the packet/transport-frame ownership boundary;
- [#34](https://github.com/leynos/wireframe/issues/34), which removed a
  historical double-spawn path;
- [#535](https://github.com/leynos/wireframe/issues/535),
  [#548](https://github.com/leynos/wireframe/issues/548), [#550](https://github.com/leynos/wireframe/issues/550),
  and [#593](https://github.com/leynos/wireframe/issues/593), which cover pool
  correctness and test gaps;
- [#547](https://github.com/leynos/wireframe/issues/547) and
  [#549](https://github.com/leynos/wireframe/issues/549), which cover
  protocol-hook and lifecycle correctness.

Implementation of this ADR is sequenced through:

- [#639](https://github.com/leynos/wireframe/issues/639), runtime ownership
  and task-churn baselines;
- [#640](https://github.com/leynos/wireframe/issues/640), removal of
  unambiguous local shared-ownership taxes;
- [#648](https://github.com/leynos/wireframe/issues/648), collapse of nested
  application-owned sharing;
- [#649](https://github.com/leynos/wireframe/issues/649), publication and
  regression guidance.

## Decision drivers

- Make ownership express runtime lifetime rather than compiler appeasement.
- Avoid atomic reference-count operations on event-loop and message hot paths
  when borrowing suffices.
- Keep independent tasks self-contained and cancellation-safe.
- Preserve legitimate cloneable handles and weak registries.
- Make mutable shared state expose a protocol rather than unrestricted lock
  access where one owner can serialize operations.
- Retain ordinary Tokio ergonomics without introducing a custom executor or
  unsafe lifetime extension.
- Give reviews a stable vocabulary for rejecting accidental `'static`
  inflation.

## Options considered

### Option A: treat `Arc` as normal Tokio plumbing

Continue accepting `Arc` whenever a type must satisfy
`Clone + Send + Sync + 'static`, and optimize only after profiling identifies a
hot path.

This minimizes design work but allows ownership topology to drift. It also
makes refactors harder because APIs stop communicating whether sharing is
essential or incidental.

### Option B: prohibit `Arc` in runtime code

Require ownership transfer, borrowing, or actors everywhere and permit `Arc`
only inside dependency types.

This produces simple local rules but is too rigid. `PushHandle`, weak session
registries, application-wide immutable state, and independent connection tasks
have genuine multiple ownership. This option is included to mark the outer
boundary of the design space rather than as a live candidate.

### Option C: use one shared root per independently owned graph and borrow beneath it

Allow `Arc` at explicit lifetime boundaries, then borrow or move fields inside
the task. Use structured concurrency for scope-bound work and actors/channels
for mutable coordination with one conceptual owner.

This preserves legitimate sharing while making nested shared ownership
exceptional and reviewable. Enforcement rests entirely on review vocabulary,
which is the weakest defence against the drift this ADR exists to stop.

### Option D: shared roots plus mechanical enforcement (preferred)

Adopt Option C's rules and back the mechanically recognizable ones with lints.
The repository already runs a Dylint suite in its commit gates, so the
enforcement machinery exists today. Patterns with grep-able signatures — an
owned-cancellation clone inside a persistent loop, a sender clone immediately
before an `&self` `send` — gain `disallowed_methods` or custom Dylint coverage,
with a tightly scoped, justified `#[allow]` as the escape hatch for legitimate
rule R6 sharing. Semantic judgements (graph shape, nested-`Arc` topology)
remain review-enforced through the
[#649](https://github.com/leynos/wireframe/issues/649) checklist.

| Topic                        | Option A: accept `Arc` | Option B: prohibit `Arc` | Option C: shared roots | Option D: C plus lints |
| ---------------------------- | ---------------------- | ------------------------ | ---------------------- | ---------------------- |
| Ownership clarity            | Weak                   | Strong                   | Strong                 | Strong                 |
| Fit with legitimate sharing  | Good                   | Poor                     | Good                   | Good                   |
| Hot-path refcount traffic    | High                   | Low                      | Low                    | Low                    |
| Design and review effort     | Low                    | High                     | Medium                 | Medium                 |
| Resistance to topology drift | Weak                   | Strong                   | Review-dependent       | Strong                 |

_Table 1: Trade-offs for the runtime ownership policy._

## Decision outcome

Adopt Option D: Option C's ownership rules, with mechanical enforcement for the
lint-expressible subset.

Wireframe will apply the following rules. Each rule carries a stable identifier
(R1-R7) so reviews and the
[#649](https://github.com/leynos/wireframe/issues/649) developer-guide
checklist can cite it without paraphrase. The single litmus question behind R2,
R4, and R6 is: **can this value escape its owner independently? If not, it does
not get its own `Arc`.**

### R1. Spawn only for independent lifetime or scheduling

Use `tokio::spawn` or `TaskTracker::spawn` when the child:

- may outlive the current call scope;
- must be independently cancelled or supervised;
- represents a long-lived runtime component;
- or must execute independently across Tokio worker threads.

When all child work is awaited before the parent returns, prefer borrowing
futures through `join!`, `try_join!`, `select!`, `FuturesUnordered`, or an
equivalent scoped future collection.

A spawn is not justified merely to make an async block easier to type or to
obtain `'static`.

### R2. Put `Arc` at graph roots, not on every node

An independently owned task may clone one `Arc` that roots the immutable/shared
object graph it needs. Values reachable exclusively through that root should
ordinarily be stored by value or behind `Box`, not another `Arc`.

Nested `Arc` remains appropriate when a child node has an ownership lifetime
independent of the root, such as a cloneable public handle that can escape by
itself.

### R3. Borrow owned handles within the task

If an actor already owns a `CancellationToken`, sender, route table, protocol
object, or configuration object for the duration of an operation, call borrowed
APIs rather than cloning an owned handle solely to create a future.

Examples governed by this rule include:

- `CancellationToken::cancelled()` instead of cloning then calling
  `cancelled_owned()` inside a persistent actor loop;
- `mpsc::Sender::send(&self, value)` without cloning the sender first;
- borrowing a prepared route table from the application root during one
  connection task.

### Server supervisor cancellation

`WireframeServer::run_with_shutdown` is a supervisor for independently
scheduled accept-loop tasks. It owns their `CancellationToken` and
`TaskTracker`, and `src/server/runtime.rs` holds a named `drop_guard_ref()`
borrow in the supervisor frame. If the supervisor future is dropped, including
through `JoinHandle::abort()`, the guard cancels the accept loops so they
eventually release their listener references. The guarantee is eventual: the
worker tasks must be scheduled to observe cancellation.

When the caller's shutdown future resolves, the supervisor follows the existing
graceful path and waits for tracked work. The drop guard does not cancel
connection tasks that have already been accepted; they retain the existing
graceful-drain semantics. The borrowed guard is a one-time application of R3,
not a per-iteration handle clone.

### R4. Move sole-owned values directly

Values with exactly one runtime owner should be stored directly, even when the
type itself supports concurrent use. Examples include a connection-local
`Fragmenter` and a middleware function owned by one `FnService`.

Thread-safe internals do not imply shared ownership at every call site.

### R5. Use actors for one-owner mutable coordination

When mutable state has one conceptual authority, prefer one task owning the
state and receiving commands over `Arc<Mutex<State>>` plus multiple tasks
mutating it.

This rule applies especially to the client-pool scheduler. The actor's command
handle may be cloneable; the scheduler state itself should have one owner.

An actor is warranted when the state carries at least two coupled invariants or
a shutdown protocol. Otherwise a plain owned struct with `&mut self` methods
suffices; this criterion stops the policy itself from driving actor
proliferation.

### R6. Preserve legitimate shared handles

This ADR explicitly retains shared ownership where it communicates real
semantics:

- `PushHandle` and Tokio channel senders used by independent producers;
- `SessionRegistry` weak references that do not extend connection lifetime;
- application data values returned as owned shared handles;
- `OwnedSemaphorePermit` and the Tokio semaphore state it references;
- one prepared-application root shared by independent connection tasks;
- one client-pool root shared by pool handles and leases.

### R7. Do not substitute `Rc` merely to avoid atomics

Wireframe targets Tokio's multithreaded runtime and exposes `Send` APIs. A
local runtime and `Rc` may be valid in downstream applications, but core
Wireframe runtime types should not switch to thread-local sharing solely to
remove atomic operations.

## Consequences

### Positive

- Types and fields communicate lifetime intent more clearly.
- Event-loop and push hot paths avoid needless refcount operations.
- Connection-local state becomes easier to test and reason about.
- Actor state invariants remain behind a protocol rather than a
  general-purpose lock.
- Future code review can distinguish a legitimate shared handle from
  accidental `'static` inflation.

### Negative

- Some refactors require more explicit lifetime parameters or scoped future
  types.
- Splitting builders, prepared templates, and runtimes creates more named
  types.
- A persistent actor task has lifecycle and shutdown machinery that must be
  tested.
- A single shared root can become a new god-object if capability boundaries
  are ignored.

## Rejected shortcuts

- Replacing every `Arc<T>` with `T: Clone` without checking semantic
  ownership.
- Hiding nested `Arc` behind type aliases while retaining the same ownership
  graph.
- Adding unsafe scoped-task abstractions to bypass Tokio's `'static`
  requirement.
- Treating microbenchmarks as the sole justification for an ownership model;
  correctness and lifetime clarity remain primary.

## Migration plan

### Phase 1: remove unambiguous local taxes

Land behaviour-preserving changes that borrow existing handles or move
sole-owned values directly.

### Phase 2: establish explicit runtime roots

Adopt ADR 012's prepared application root and ADR 013's client-pool
scheduler/slot root.

### Phase 3: collapse nested ownership

Replace nested callback, route, protocol, assembler, scheduler, and slot `Arc`
layers that no longer encode independent lifetimes.

### Phase 4: verify and document

Add benchmarks, shutdown tests, mutation tests, lint coverage for the
mechanically enforceable rules, and developer guidance that makes the intended
ownership boundaries visible. The
[#649](https://github.com/leynos/wireframe/issues/649) developer-guide
checklist is the canonical review artefact derived from rules R1-R7, with this
ADR as its source of truth.

## Verification

Implementation work governed by this ADR should demonstrate:

- no change to externally contracted behaviour in cancellation, fairness,
  backpressure, or graceful shutdown, except for the documented
  server-supervisor drop/abort contract: abandoning `run_with_shutdown`
  eventually cancels its accept loops and releases the listener; the graceful
  shutdown path and in-flight connection-task drain remain unchanged. The
  other intentional behaviour changes are recorded in
  [ADR 012](adr-012-prepared-application-and-connection-runtime.md) (factory
  evaluation frequency, readiness timing, startup error surfacing) and
  [ADR 013](adr-013-client-pool-scheduler-and-slot-ownership.md) (lease-drop
  servicing), and this ADR must not be cited against them;
- no new leaked task or strong-reference cycle;
- no loss of public cloneable-handle semantics;
- benchmark coverage for affected hot paths;
- tests that prove actors and connection tasks terminate after their final
  owning handle is dropped or cancellation is requested;
- server-supervisor tests that prove graceful shutdown remains successful and
  that dropping or aborting the supervisor eventually releases its listener;
- a recorded lint-or-checklist disposition per rule under
  [#649](https://github.com/leynos/wireframe/issues/649): R3 and R4 have
  grep-able signatures and are lint candidates; R1, R2, R5, R6, and R7 are
  judgement calls enforced through the review checklist.

## Outstanding decisions

ADR 012 must decide the exact application preparation frequency and migration
path for `AppFactory`.

ADR 013 must decide the client-pool scheduler command transport, shutdown
protocol, and bounded-waiter integration.

## References

- Epic [#635](https://github.com/leynos/wireframe/issues/635)
- [ADR 010: transport-frame boundary for the zero-copy migration](adr-010-transport-frame-boundary-for-zero-copy.md)
- [#34](https://github.com/leynos/wireframe/issues/34)
- [#535](https://github.com/leynos/wireframe/issues/535)
- [#547](https://github.com/leynos/wireframe/issues/547)
- [#548](https://github.com/leynos/wireframe/issues/548)
- [#549](https://github.com/leynos/wireframe/issues/549)
- [#550](https://github.com/leynos/wireframe/issues/550)
- [#593](https://github.com/leynos/wireframe/issues/593)
- Tokio `spawn`, `CancellationToken`, `mpsc`, and semaphore APIs already used
  by the repository

[^1]: [Tokio tutorial: spawning](https://tokio.rs/tokio/tutorial/spawning),
    on the `'static` bound and sharing data between tasks with `Arc`.

[^2]: [`tokio_util::sync::CancellationToken`](https://docs.rs/tokio-util/latest/tokio_util/sync/struct.CancellationToken.html),
    documenting `cancelled` and `cancelled_owned`.

[^3]: [`tokio_util::task::TaskTracker`](https://docs.rs/tokio-util/latest/tokio_util/task/struct.TaskTracker.html),
    documenting the close-then-wait shutdown contract.

[^4]: Alice Ryhl,
    [Actors with Tokio](https://ryhl.io/blog/actors-with-tokio/).
