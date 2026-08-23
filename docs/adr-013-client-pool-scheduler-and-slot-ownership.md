# Architectural decision record (ADR) 013: single-owner client-pool scheduler and slot graph

## Status

Proposed.

## Date

Proposed 2026-08-08. Imported and refined 2026-08-23.

## Context and Problem Statement

The client pool has one conceptual scheduler, one fixed set of physical socket
slots, and many logical handles and leases. Its current ownership graph is
substantially more distributed. `WireframeClientPool` wraps one
`Arc<ClientPoolInner>` shaped as:

```text
Arc<ClientPoolInner>
  ├── Arc<[Arc<PoolSlot>]>
  │       └── Arc<Semaphore>
  └── Arc<PoolScheduler>
          ├── Mutex<SchedulerState>
          ├── AtomicBool service guard
          └── spawn-on-demand service tasks
```

This creates several costs and reasoning hazards:

- `ordered_slots()` clones every `Arc<PoolSlot>` for each acquisition;
- boxed permit futures default to `'static` and therefore own those slot
  clones even though the race is scoped to one acquisition;
- a lease owns both an `Arc<PoolSlot>` and optionally an
  `Arc<ClientPoolInner>` for release notification;
- dropping a lease can clone the pool and scheduler roots and spawn
  `service_waiters` even when the waiter queue is empty;
- fairness state, restart logic, waiter cancellation, shutdown, and capacity
  notification are divided between a mutex, atomics, callers, and transient
  tasks;
- the current model makes race recovery difficult enough that mutation
  survivors covering shutdown and slot rotation are accepted as untested
  interleavings ([#593](https://github.com/leynos/wireframe/issues/593)), and
  scheduler bookkeeping relies on `std::sync::Mutex` poison recovery
  ([#539](https://github.com/leynos/wireframe/issues/539));
- [#550](https://github.com/leynos/wireframe/issues/550) requires bounded
  waiter admission, which adds another invariant to distributed scheduler state.

The scheduler is already actor-shaped in behaviour: callers submit requests,
one authority orders them, and replies return through oneshot channels. Its
ownership model should make that authority explicit.

## Prior Art

The mainstream Rust connection pools all avoid a central scheduler task: sqlx
gates acquisition with a fairness-configurable semaphore and bounds waiting by
an acquire timeout;[^1] deadpool advertises "no background task" as a feature
and admits callers through a `tokio::sync::Semaphore`;[^2] bb8 uses lock-based
state with a FIFO/LIFO `QueueStrategy`.[^3] None of them, however, provides
Wireframe's contract of per-logical-handle round-robin fairness across a fixed
slot set, and none needs a single admission point for bounded waiter counts.
Those requirements are what motivate a central authority here, and the current
spawn-on-demand servicing already pays for one without gaining its benefits.

The persistent-actor shape follows the pattern described by Alice Ryhl: one
task exclusively owns the state, commands arrive through a bounded mailbox with
oneshot replies, mailbox closure is the shutdown signal, and cycles between the
actor and its handles must be avoided.[^4] A bounded mailbox also gives
admission control a natural home, which the semaphore-only pools lack. sqlx's
history is a caution for the waiter queue: its early list-of-waiters design
leaked entries when acquisitions were cancelled, and the fix was RAII-based
cancel safety rather than explicit list removal — the same discipline the
scheduler actor must apply when pruning abandoned oneshot receivers.[^1]

## Traceability

This ADR governs the client-pool half of Epic
[#635](https://github.com/leynos/wireframe/issues/635) and implements ADR 011's
single-owner coordination rule.

Primary code surfaces:

- `src/client/pool/client_pool.rs`;
- `src/client/pool/scheduler.rs`;
- `src/client/pool/handle.rs`;
- `src/client/pool/lease.rs`;
- `src/client/pool/slot.rs`;
- `src/client/pool/policy.rs`;
- `src/client/pool/sync.rs`.

Related issues:

- Epic [#635](https://github.com/leynos/wireframe/issues/635);
- ADR 011 proposal [#636](https://github.com/leynos/wireframe/issues/636);
- [#535](https://github.com/leynos/wireframe/issues/535), scheduler
  state-transition simplification;
- [#539](https://github.com/leynos/wireframe/issues/539), pool
  poison-recovery integration coverage;
- [#548](https://github.com/leynos/wireframe/issues/548), cancellation can
  return a dirty socket;
- [#550](https://github.com/leynos/wireframe/issues/550), blocked waiter
  admission is unbounded;
- [#593](https://github.com/leynos/wireframe/issues/593), shutdown and
  slot-rotation mutation survivors.

Implementation of this ADR is sequenced through:

- [#645](https://github.com/leynos/wireframe/issues/645), `PoolCore` and
  index-based pooled leases;
- [#646](https://github.com/leynos/wireframe/issues/646), borrowed pool-slot
  permit races without per-slot `Arc` cloning;
- [#647](https://github.com/leynos/wireframe/issues/647), persistent
  scheduler actor replacing spawn-on-demand servicing.

## Decision Drivers

- Give scheduler invariants one owner.
- Eliminate scheduler task creation from the acquire/drop hot path.
- Avoid O(pool-size) slot refcount operations per acquisition.
- Preserve FIFO and round-robin fairness semantics.
- Integrate bounded waiter admission and cancellation handling.
- Keep pool shutdown deterministic and awaitable.
- Avoid strong-reference cycles between the scheduler task and public pool
  handles.
- Preserve `OwnedSemaphorePermit` where a permit must outlive a borrowed
  acquisition future.

## Options Considered

### Option A: retain the current mutex and spawn-on-demand service model

Add an empty-queue fast path before `kick`, lifetime-parameterize permit
futures, and remove nested slot Arcs independently.

This can remove much of the overhead with smaller changes, but scheduler
ownership remains distributed between a mutex, atomics, lease drops, and
transient tasks. Adding bounded waiters and robust cancellation would increase
that coordination surface.

### Option B: use a persistent scheduler actor with command/reply messages (preferred)

Run one long-lived task per pool. It owns `SchedulerState`, waiter queues,
fairness rotation, admission counts, and grant sequencing. Pool handles send
commands; the scheduler returns leases or errors through oneshot replies.

Store physical slots by value behind one shared pool-core root. Leases identify
a slot by index rather than owning a second slot Arc.

### Option C: remove the central scheduler and let acquisitions race permits directly

Each caller races slot permits and applies local rotation. This minimizes
central machinery but cannot preserve logical-handle fairness without
rebuilding shared coordination elsewhere.

### Option D: use an async mutex around scheduler state without service tasks

Each acquisition locks state, enqueues, and attempts to grant work directly.

This removes transient task spawning but lets arbitrary caller tasks become
scheduler executors. Cancellation and fairness transitions still occur under
distributed ownership, and lock hold times become more delicate.

| Topic                        | Option A: incremental | Option B: scheduler actor | Option C: no scheduler | Option D: async mutex |
| ---------------------------- | --------------------- | ------------------------- | ---------------------- | --------------------- |
| Single ownership of state    | Weak                  | Strong                    | None                   | Weak                  |
| Hot-path task spawning       | Reduced               | Eliminated                | Eliminated             | Eliminated            |
| Fairness preservation        | Good                  | Good                      | Poor                   | Good                  |
| Bounded-admission fit (#550) | Awkward               | Natural                   | Poor                   | Awkward               |
| Change size                  | Small                 | Large                     | Medium                 | Medium                |

_Table 1: Trade-offs for the client-pool scheduler ownership model._

## Decision Outcome

Adopt Option B.

### 1. One persistent scheduler task per pool

Creating a pool starts one scheduler task. The task owns:

- FIFO and round-robin waiter queues;
- logical handle bookkeeping;
- fairness rotation state;
- bounded-waiter admission state;
- whether a slot-permit acquisition is currently pending;
- shutdown and drain transitions.

No lease drop or acquire call spawns an additional scheduler service task.

### 2. Commands define the scheduler protocol

The exact enum may evolve, but the protocol must cover the following events:

```rust,no_run
Acquire {
    handle_id: u64,
    reply: oneshot::Sender<Result<PooledClientLease<...>, ClientError>>,
}
DeregisterHandle {
    handle_id: u64,
}
CapacityAvailable
Shutdown {
    reply: oneshot::Sender<()>,
}
```

Logical handle IDs may still come from an atomic counter in the public handle
factory so `WireframeClientPool::handle()` remains synchronous. The actor
treats the first acquire for an ID as registration if necessary, and
deregistration remains idempotent. This avoids requiring a synchronous method
to await a registration acknowledgement.

The implementation must document message-order assumptions and ensure a handle
cannot be deregistered before its already-submitted acquire command is observed.

### 3. Separate cloneable control from shared slot storage

Avoid a task/root cycle by separating:

- a cloneable scheduler command handle, whose sender keeps the actor's
  mailbox alive;
- an `Arc<PoolCore>` containing fixed slot storage and shared
  shutdown/connection resources;
- the scheduler task's receiver and owned scheduler state.

`PoolCore` must not own the command sender if the scheduler task owns
`Arc<PoolCore>`. Dropping the final public sender then closes the mailbox,
allowing the task to exit and release its core reference.

### 4. Store slots by value and identify them by index

Replace nested slot ownership conceptually shaped as:

```rust,no_run
Arc<[Arc<PoolSlot>]>
```

with:

```rust,no_run
Arc<PoolCore> {
    slots: Box<[PoolSlot]>,
    // ...
}
```

A lease owns:

```rust,no_run
Arc<PoolCore>
slot_index: usize
OwnedSemaphorePermit
```

The core keeps every slot alive for the lease lifetime, so a separate
`Arc<PoolSlot>` is unnecessary.

The `Arc<Semaphore>` inside a slot may remain because `OwnedSemaphorePermit`
genuinely owns semaphore capacity independently of the permit-acquisition
future.

### 5. Race borrowed slot futures

Permit acquisition is scope-bound to the scheduler's current grant attempt. Use
lifetime-parameterized futures, `FuturesUnordered`, or an equivalent scoped
collection that borrows `PoolCore::slots` and returns
`(slot_index, OwnedSemaphorePermit)`.

Do not clone every slot merely to default trait-object futures to `'static`.

Rotation should operate on indices or an iterator order rather than allocating
a fresh `Vec<Arc<PoolSlot>>`.

### 6. Integrate bounded waiter admission

The actor is the sole admission point for
[#550](https://github.com/leynos/wireframe/issues/550). It will enforce
configured waiter capacity and/or acquisition deadlines and return a typed
error when admission fails.

Admission counts must include queued requests whose oneshot receivers have not
yet been observed as cancelled. Cancelled receivers should be pruned promptly
enough that abandoned requests do not consume capacity indefinitely.

### 7. Preserve cancellation safety of physical sockets

This ADR does not resolve
[#548](https://github.com/leynos/wireframe/issues/548) by itself. Lease and
checkout operations must retain or strengthen the rule that cancelling an
in-flight operation invalidates or re-synchronizes the physical socket before
reuse.

The scheduler refactor must not treat release of admission capacity as proof
that the socket is clean.

### 8. Make shutdown awaitable

`WireframeClientPool::close` sends `Shutdown`, rejects new acquisitions,
resolves queued waiters with `ClientError::disconnected`, waits for the
scheduler acknowledgement/task completion, then releases pool resources.

Dropping the final pool/control handle without explicit close must also let the
actor terminate when its mailbox closes.

## Consequences

### Positive

- Scheduler invariants have one owner and one serial event stream.
- Uncontended lease drop no longer spawns a task.
- Acquisition avoids O(pool-size) slot Arc clones.
- Waiter admission, cancellation, fairness, and shutdown live in one state
  machine.
- `std::sync::Mutex` poison recovery is removed from scheduler state if no
  other caller shares it.
- Loom/state-machine testing can target a command protocol rather than
  incidental lock interleavings.

### Negative

- Every pool owns one long-lived Tokio task and mailbox.
- Acquire and capacity events incur channel traffic.
- `Drop` paths can only send non-blocking commands; closed/full mailbox
  handling must be defined.
- Splitting pool core and scheduler control introduces more internal types.
- A scheduler actor can become a throughput bottleneck if it performs socket
  I/O rather than only admission bookkeeping.

## Invariants

The implementation must maintain:

1. At most the configured number of permits is granted per slot.
2. Every granted permit belongs to exactly one live lease until drop.
3. FIFO policy preserves request arrival order among admitted live waiters.
4. Round-robin policy gives each active logical handle turns according to
   the existing contract.
5. Cancelled or deregistered waiters receive no later lease.
6. Shutdown resolves every queued reply and terminates the task.
7. No strong-reference cycle keeps the pool alive after all public handles
   are gone.
8. Capacity notification is idempotent and cannot create an extra permit.
9. A dirty/cancelled socket is never made reusable merely because its lease
   was dropped.

## Rejected Shortcuts

- Keeping `PoolSlot` behind `Arc` solely because permit futures were boxed
  as `'static` trait objects.
- Spawning one task per capacity notification with an empty-queue check as
  the final architecture.
- Letting the scheduler actor perform user request/response I/O.
- Using an unbounded mailbox without separately enforcing
  [#550](https://github.com/leynos/wireframe/issues/550)'s waiter admission
  limit.
- Detaching the scheduler task without an explicit shutdown/drain protocol.

## Migration Plan

### Phase 1: characterize behaviour

Land the pool baseline/verification issue
([#639](https://github.com/leynos/wireframe/issues/639)) and existing
[#593](https://github.com/leynos/wireframe/issues/593) tests before changing
the scheduler topology.

### Phase 2: introduce pool core and index-based leases

Remove the nested slot Arc layer while retaining current scheduler behaviour
([#645](https://github.com/leynos/wireframe/issues/645)).

### Phase 3: add actor command protocol

Move waiter queues, fairness, admission, and shutdown state into the long-lived
task ([#647](https://github.com/leynos/wireframe/issues/647)). Keep
compatibility wrappers around existing `PoolHandle` and pool APIs.

### Phase 4: use scoped permit races

Remove `ordered_slots()` Arc cloning and `'static` boxed slot futures
([#646](https://github.com/leynos/wireframe/issues/646)).

### Phase 5: integrate existing safety work

Complete or coordinate the following, plus any remaining useful scope from
[#535](https://github.com/leynos/wireframe/issues/535) and
[#539](https://github.com/leynos/wireframe/issues/539):

- [#548](https://github.com/leynos/wireframe/issues/548);
- [#550](https://github.com/leynos/wireframe/issues/550);
- [#593](https://github.com/leynos/wireframe/issues/593).

## Verification

- Uncontended acquire/drop creates no scheduler task after pool
  construction.
- Pool task count remains constant under repeated acquisitions.
- Pool sizes greater than one rotate/grant across slots as configured.
- FIFO and round-robin behavioural tests preserve ordering.
- Saturated pools enforce waiter bounds and acquisition timeouts.
- Dropped acquire futures are pruned and do not receive leases.
- Shutdown resolves blocked waiters promptly and the actor terminates.
- Dropping all public handles without `close` releases the pool core.
- Loom or deterministic state-machine tests cover enqueue, capacity,
  cancellation, deregistration, and shutdown races. No loom coverage exists for
  the pool today; the only loom suite exercises the push queues.
- Criterion/allocation benchmarks compare uncontended and contended
  acquire/drop before and after.

## Outstanding Decisions Before Acceptance

- Bounded versus unbounded internal command mailbox; waiter admission must
  remain bounded either way.
- Exact typed error(s) for waiter rejection and acquire timeout.
- Whether explicit `close` waits on a stored `JoinHandle`, oneshot
  acknowledgement, or both.
- How to preserve strict command ordering across cloned scheduler senders
  and synchronous `handle()` creation.
- Whether [#535](https://github.com/leynos/wireframe/issues/535) is closed
  as superseded or retained for smaller transition helpers inside the actor.
- Whether [#539](https://github.com/leynos/wireframe/issues/539) remains
  relevant once scheduler mutex poison recovery disappears.

## References

- Epic [#635](https://github.com/leynos/wireframe/issues/635)
- [ADR 011: runtime ownership and task-lifetime boundaries](adr-011-runtime-ownership-and-task-lifetime-boundaries.md)
- [#535](https://github.com/leynos/wireframe/issues/535)
- [#539](https://github.com/leynos/wireframe/issues/539)
- [#548](https://github.com/leynos/wireframe/issues/548)
- [#550](https://github.com/leynos/wireframe/issues/550)
- [#593](https://github.com/leynos/wireframe/issues/593)

[^1]: [sqlx pool internals](https://github.com/launchbadge/sqlx/blob/main/sqlx-core/src/pool/inner.rs),
    semaphore-gated acquisition with `acquire_timeout` and RAII
    `DecrementSizeGuard` cancel safety.

[^2]: [deadpool](https://docs.rs/deadpool/latest/deadpool/), which performs
    no background actions and admits callers through a semaphore.

[^3]: [bb8](https://docs.rs/bb8/latest/bb8/), lock-based pooling with a
    configurable `QueueStrategy`.

[^4]: Alice Ryhl,
    [Actors with Tokio](https://ryhl.io/blog/actors-with-tokio/).
