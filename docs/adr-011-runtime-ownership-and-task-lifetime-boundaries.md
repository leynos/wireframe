# Architectural decision record (ADR) 011: runtime ownership and task-lifetime boundaries

## Status

Proposed.

## Date

Proposed 2026-08-08. Imported and refined 2026-08-23.

## Context and Problem Statement

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

Concrete instances of each pattern exist in the current tree:

- the connection actor clones its `CancellationToken` and awaits
  `cancelled_owned()` inside its own `select!` loop (`src/connection/mod.rs`,
  `src/connection/polling.rs`), where the borrowing `cancelled()` future would
  suffice;
- `PushHandle::push_with_priority` clones an `mpsc::Sender` before calling
  `send`, although `send` takes `&self` (`src/push/queues/handle.rs`);
- `FnService` stores its middleware function as `Arc<F>` despite being its
  sole owner (`src/middleware.rs`);
- `ProtocolHooks::from_protocol` clones the same `Arc` of the protocol
  object once per callback adapter — six clones for one protocol
  (`src/hooks.rs`);
- the connection actor stores a connection-local fragmenter as
  `Option<Arc<Fragmenter>>` although it is never shared
  (`src/connection/mod.rs`);
- the client-pool scheduler couples an `Arc<PoolScheduler>` root, a
  `Mutex<SchedulerState>`, an atomic single-flight flag, and spawn-on-demand
  service tasks (`src/client/pool/scheduler.rs`).

Without an explicit policy, local fixes can remove individual clones while
later changes recreate the same topology under different names.

## Prior Art

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

## Decision Drivers

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

## Options Considered

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
have genuine multiple ownership.

### Option C: use one shared root per independently owned graph and borrow beneath it (preferred)

Allow `Arc` at explicit lifetime boundaries, then borrow or move fields inside
the task. Use structured concurrency for scope-bound work and actors/channels
for mutable coordination with one conceptual owner.

This preserves legitimate sharing while making nested shared ownership
exceptional and reviewable.

| Topic                        | Option A: accept `Arc` | Option B: prohibit `Arc` | Option C: shared roots |
| ---------------------------- | ---------------------- | ------------------------ | ---------------------- |
| Ownership clarity            | Weak                   | Strong                   | Strong                 |
| Fit with legitimate sharing  | Good                   | Poor                     | Good                   |
| Hot-path refcount traffic    | High                   | Low                      | Low                    |
| Design and review effort     | Low                    | High                     | Medium                 |
| Resistance to topology drift | Weak                   | Strong                   | Strong                 |

_Table 1: Trade-offs for the runtime ownership policy._

## Decision Outcome

Adopt Option C.

Wireframe will apply the following rules.

### 1. Spawn only for independent lifetime or scheduling

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

### 2. Put `Arc` at graph roots, not on every node

An independently owned task may clone one `Arc` that roots the immutable/shared
object graph it needs. Values reachable exclusively through that root should
ordinarily be stored by value or behind `Box`, not another `Arc`.

Nested `Arc` remains appropriate when a child node has an ownership lifetime
independent of the root, such as a cloneable public handle that can escape by
itself.

### 3. Borrow owned handles within the task

If an actor already owns a `CancellationToken`, sender, route table, protocol
object, or configuration object for the duration of an operation, call borrowed
APIs rather than cloning an owned handle solely to create a future.

Examples governed by this rule include:

- `CancellationToken::cancelled()` instead of cloning then calling
  `cancelled_owned()` inside a persistent actor loop;
- `mpsc::Sender::send(&self, value)` without cloning the sender first;
- borrowing a prepared route table from the application root during one
  connection task.

### 4. Move sole-owned values directly

Values with exactly one runtime owner should be stored directly, even when the
type itself supports concurrent use. Examples include a connection-local
`Fragmenter` and a middleware function owned by one `FnService`.

Thread-safe internals do not imply shared ownership at every call site.

### 5. Use actors for one-owner mutable coordination

When mutable state has one conceptual authority, prefer one task owning the
state and receiving commands over `Arc<Mutex<State>>` plus multiple tasks
mutating it.

This rule applies especially to the client-pool scheduler. The actor's command
handle may be cloneable; the scheduler state itself should have one owner.

### 6. Preserve legitimate shared handles

This ADR explicitly retains shared ownership where it communicates real
semantics:

- `PushHandle` and Tokio channel senders used by independent producers;
- `SessionRegistry` weak references that do not extend connection lifetime;
- application data values returned as owned shared handles;
- `OwnedSemaphorePermit` and the Tokio semaphore state it references;
- one prepared-application root shared by independent connection tasks;
- one client-pool root shared by pool handles and leases.

### 7. Do not substitute `Rc` merely to avoid atomics

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

## Rejected Shortcuts

- Replacing every `Arc<T>` with `T: Clone` without checking semantic
  ownership.
- Hiding nested `Arc` behind type aliases while retaining the same ownership
  graph.
- Adding unsafe scoped-task abstractions to bypass Tokio's `'static`
  requirement.
- Treating microbenchmarks as the sole justification for an ownership model;
  correctness and lifetime clarity remain primary.

## Migration Plan

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

Add benchmarks, shutdown tests, mutation tests, and developer guidance that
make the intended ownership boundaries visible.

## Verification

Implementation work governed by this ADR should demonstrate:

- no behaviour change in cancellation, fairness, backpressure, or shutdown;
- no new leaked task or strong-reference cycle;
- no loss of public cloneable-handle semantics;
- benchmark coverage for affected hot paths;
- tests that prove actors and connection tasks terminate after their final
  owning handle is dropped or cancellation is requested.

## Outstanding Decisions

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
