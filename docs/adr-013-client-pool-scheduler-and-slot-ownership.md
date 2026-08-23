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
That fairness contract is not axiomatic: it is recorded in the client design
document's scheduling model
([`wireframe-client-design.md`](wireframe-client-design.md)), which makes
`PoolFairnessPolicy::RoundRobin` the default so stable logical sessions take
turns under repeated contention, and it is exercised by the
`client_pool_handle` behavioural suite
(`tests/features/client_pool_handle.feature` and its fixtures). Those
requirements are what motivate a central authority here, and the current
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

### Option E: semaphore-gated admission with a rotation lock and no service tasks

Adopt the sqlx/deadpool shape: one pool-wide fair `tokio::sync::Semaphore` for
capacity, an acquire timeout plus a bounded admission count for
[#550](https://github.com/leynos/wireframe/issues/550), a small synchronous
mutex touched only for slot rotation, and RAII permit drop as the entire
release path.

This eliminates tasks, mailboxes, shutdown protocols, and command-ordering
hazards, and it matches three production-proven pools. It is rejected because
it cannot preserve the per-logical-handle round-robin contract evidenced above
— fairness degrades to semaphore FIFO with best-effort rotation — and because
[#550](https://github.com/leynos/wireframe/issues/550) needs a typed admission
rejection, not only a timeout. If that contract were ever relaxed, this option
should be re-evaluated first.

| Topic                        | Option A: incremental | Option B: scheduler actor | Option C: no scheduler | Option D: async mutex | Option E: semaphore only |
| ---------------------------- | --------------------- | ------------------------- | ---------------------- | --------------------- | ------------------------ |
| Single ownership of state    | Weak                  | Strong                    | None                   | Weak                  | Medium                   |
| Hot-path task spawning       | Reduced               | Eliminated                | Eliminated             | Eliminated            | Eliminated               |
| Fairness preservation        | Good                  | Good                      | Poor                   | Good                  | Partial                  |
| Bounded-admission fit (#550) | Awkward               | Natural                   | Poor                   | Awkward               | Timeout-only             |
| Change size                  | Small                 | Large                     | Medium                 | Medium                | Medium                   |

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

The actor loop must remain responsive while a grant is pending: it `select!`s
over its mailbox and any pending permit race, performs O(1) bookkeeping per
event, and no command handler awaits unboundedly inline. The
pending-acquisition flag is cleared through RAII so that a permit race dropped
mid-`select!` (by shutdown or command handling) cannot leave the flag stuck and
wedge granting. A permit won for a waiter whose reply channel has already
closed is dropped and immediately re-enters the grant loop.

Replacing the current inline mutex-plus-`try_acquire` acquisition with a
command/reply round trip regresses the uncontended path from sub-microsecond to
microsecond-class latency. The implementation must either preserve an
uncontended fast path that cannot violate the fairness invariants, or
demonstrate through the named uncontended benchmark in the Verification section
that the actor round trip meets an explicitly stated latency budget recorded in
[#647](https://github.com/leynos/wireframe/issues/647).

### 2. Commands define the scheduler protocol

The command enum is crate-private, which is what licenses its evolution. The
exact shape may change, but the protocol must cover the following events:

```rust,no_run
Acquire {
    handle_id: u64,
    reply: oneshot::Sender<Result<PooledClientLease<...>, ClientError>>,
}
DeregisterHandle {
    handle_id: u64,
}
Shutdown {
    reply: oneshot::Sender<()>,
}
```

Capacity release is deliberately absent from the command set; section 3 defines
it as a level-triggered signal outside the mailbox.

Logical handle IDs may still come from an atomic counter in the public handle
factory so `WireframeClientPool::handle()` remains synchronous. The actor
treats the first acquire for an ID as registration if necessary, and
deregistration remains idempotent. This avoids requiring a synchronous method
to await a registration acknowledgement.

Message ordering is pinned rather than left open: each `PoolHandle` owns its
own clone of the command sender, so the channel's per-sender FIFO guarantee
ensures a handle's `DeregisterHandle` (sent from its `Drop`) is observed after
any acquire command that handle already submitted. Deregistration is
loss-tolerant: `Drop` may only `try_send`, so a discarded `DeregisterHandle`
must degrade to delayed cleanup, not a leak — during grant sweeps the actor
prunes handles whose waiters' reply channels are all closed, and handle
bookkeeping must not grow without bound under handle churn.

### 3. Observe capacity level-triggered, outside the mailbox

An earlier draft routed capacity release through a `CapacityAvailable` command,
but under this ADR's own ownership rules no declared owner could send it: the
lease owns no command sender (section 5), and `PoolCore` must not own one
(section 4). Giving the lease a sender would reintroduce the lease-to-scheduler
back-edge that the current `release_inner` pointer represents and this ADR
removes.

Capacity signalling is therefore level-triggered and lives outside the bounded
command mailbox. A lease drop releases only its `OwnedSemaphorePermit`; the
actor's pending permit race observes released slot-semaphore capacity directly,
and the actor re-checks actual capacity whenever it processes any command or
completes a grant. A missed edge can delay a grant, never prevent one: a
released permit is always eventually observed. This satisfies the idempotency
invariant by construction — a level-triggered check cannot mint an extra permit
— and removes the edge-triggered lost-wakeup failure mode entirely.

### 4. Separate cloneable control from shared slot storage

Avoid a task/root cycle by separating:

- a cloneable scheduler command handle, whose sender keeps the actor's
  mailbox alive;
- an `Arc<PoolCore>` containing fixed slot storage and shared
  shutdown/connection resources;
- the scheduler task's receiver and owned scheduler state.

`PoolCore` must not own the command sender if the scheduler task owns
`Arc<PoolCore>`. Dropping the final public sender then closes the mailbox,
allowing the task to exit and release its core reference.

### 5. Store slots by value and identify them by index

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

### 6. Race slot permits without per-slot cloning

The requirement is that a grant attempt performs no per-slot `Arc<PoolSlot>`
cloning and no `Vec` allocation per rotation; rotation operates on indices or
an iterator order.

Two compliant implementations exist. Because section 5 retains `Arc<Semaphore>`
per slot, `Semaphore::acquire_owned` already yields `'static` permit futures at
one refcount operation per raced slot, which satisfies the requirement
directly. Alternatively, futures may borrow `PoolCore::slots` through
`FuturesUnordered` or an equivalent scoped collection returning
`(slot_index, OwnedSemaphorePermit)` — noting the real constraint that a
self-referential borrow cannot be stored in the actor's own state and must live
in a loop-scoped binding.

Do not clone every slot merely to default trait-object futures to `'static`.

### 7. Integrate bounded waiter admission

The actor is the sole admission point for
[#550](https://github.com/leynos/wireframe/issues/550). It will enforce
configured waiter capacity and/or acquisition deadlines and return a typed
error when admission fails.

Admission counts must include queued requests whose oneshot receivers have not
yet been observed as cancelled. Cancelled receivers are pruned on every grant
attempt and at admission-check time, so abandoned requests do not consume
capacity indefinitely. Waiter deadlines are mandatory, not optional: a
saturated pool must reject or time out waiters rather than queue them
unboundedly.

Admission rejection and timeout use dedicated typed errors — for example
`ClientError::PoolSaturated { limit }` and `ClientError::AcquireTimeout` —
neither of which reports `should_recycle_connection() == true`, since no socket
was involved. `ClientError` gains `#[non_exhaustive]` before these variants
land, which is cheap at 0.3.0 and painful later.

### 8. Preserve cancellation safety of physical sockets

This ADR does not resolve
[#548](https://github.com/leynos/wireframe/issues/548) by itself. Lease and
checkout operations must retain or strengthen the rule that cancelling an
in-flight operation invalidates or re-synchronizes the physical socket before
reuse.

The scheduler refactor must not treat release of admission capacity as proof
that the socket is clean.

### 9. Make shutdown awaitable

`WireframeClientPool::close` delivers `Shutdown`, waits for the scheduler
acknowledgement and task completion, then releases pool resources. `Shutdown`
delivery must be guaranteed non-blocking — reserved mailbox capacity, a
dedicated channel, or a `CancellationToken` — so a saturated pool cannot
deadlock its own shutdown.

On observing `Shutdown`, the actor enters a draining state: mailbox order is
the fence, and every command observed after `Shutdown` — including acquisitions
submitted through still-live cloned senders — is resolved with a dedicated
`ClientError::PoolClosed` variant. Queued waiters are resolved with the same
error. `PoolClosed` is distinct from `ClientError::disconnected()`, which
reports a transport-level peer close and wrongly classifies as recyclable;
`PoolClosed` reports `should_recycle_connection() == false` and lets callers
programmatically distinguish "pool closed, retry elsewhere" from a real peer
failure.

`close` waits for the actor and queued waiters, not for outstanding leases.
After actor termination by either route — explicit `Shutdown` or mailbox
closure when the final public sender drops — outstanding leases remain valid;
their permit releases require no scheduler observation, which is coherent
because capacity observation is level-triggered (section 3).

### 10. Detect and report scheduler failure

Centralizing scheduler state also centralizes its failure, and removing
`std::sync::Mutex` poison recovery removes the one existing loud panic
tripwire. Actor-termination detection explicitly replaces poison recovery as
the panic-signalling mechanism, and
[#539](https://github.com/leynos/wireframe/issues/539) is resolved in that
direction.

The design requires:

- the scheduler task's `JoinHandle` is retained (by `PoolCore` or the close
  path), so the panic is observed rather than swallowed;
- abnormal termination maps to a dedicated error — for example
  `ClientError::SchedulerFailed` — distinct from `PoolClosed`, so callers and
  operators can tell a crash from a clean shutdown;
- abnormal termination emits an error-level structured log event and a
  metric carrying the pool identity;
- the failure policy is fail-pool-permanently: subsequent acquisitions
  return `SchedulerFailed`, and automatic actor restart is out of scope for
  this ADR.

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
- Acquire events incur channel traffic, and the uncontended path regresses
  unless the fast path or latency budget in section 1 is honoured.
- `Drop` paths can only send non-blocking commands; sections 2 and 3 define
  the loss-tolerant behaviour this forces.
- Splitting pool core and scheduler control introduces more internal types.
- A scheduler actor can become a throughput bottleneck if it performs socket
  I/O rather than only admission bookkeeping.
- The actor is a new single point of failure; section 10 defines its
  detection and failure policy.

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
8. Capacity observation is idempotent and cannot create an extra permit.
9. A dirty/cancelled socket is never made reusable merely because its lease
   was dropped.
10. A released permit is always eventually observed; a lost edge delays a
    grant but never prevents one.
11. Scheduler handle bookkeeping is bounded: a lost `DeregisterHandle`
    degrades to delayed cleanup via grant-sweep pruning, never an unbounded
    leak.

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
compatibility wrappers around existing `PoolHandle` and pool APIs; the wrappers
are internal scaffolding removed before this ADR flips to Accepted.

### Phase 4: use scoped permit races

Remove `ordered_slots()` Arc cloning and `'static` boxed slot futures
([#646](https://github.com/leynos/wireframe/issues/646)). The permit-race
rework deliberately follows the actor phase so the races are written once
against the actor's grant loop rather than twice.

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
  cancellation, deregistration, and shutdown races, naming at minimum: the
  pending-flag RAII clear when a permit race is dropped mid-`select!`, and a
  permit won exactly as its waiter's receiver drops. No loom coverage exists
  for the pool today; the only loom suite exercises the push queues.
- A panic injected into the scheduler task surfaces as `SchedulerFailed`
  with the required log event and metric, not as a silent hang or a clean
  `PoolClosed`.
- Filling the mailbox with acquisitions and then dropping leases still
  grants: no lost-wakeup wedge.
- Criterion/allocation benchmarks compare uncontended and contended
  acquire/drop before and after, explicitly naming uncontended single-caller
  acquire/drop latency, where the actor design is most likely to regress
  against the mutex.
- Day-two observability exists: per-pool gauges/counters for queued waiters,
  admission rejections, and scheduler-task liveness, so "is the actor alive and
  is the queue draining?" is answerable without a debugger.

## Outstanding Decisions Before Acceptance

- Bounded versus unbounded internal command mailbox; waiter admission must
  remain bounded either way, and `Shutdown` delivery must remain guaranteed
  regardless of the choice.
- Whether explicit `close` waits on the stored `JoinHandle`, the oneshot
  acknowledgement, or both; section 10 requires the `JoinHandle` to be retained
  in any case.
- Whether [#535](https://github.com/leynos/wireframe/issues/535) is closed
  as superseded or retained for smaller transition helpers inside the actor.

The following earlier open questions are now resolved in the text: waiter
rejection and timeout errors are named in section 7; command ordering across
cloned senders is pinned in section 2 (per-handle sender clones);
[#539](https://github.com/leynos/wireframe/issues/539) is resolved by section
10, with actor-termination detection replacing poison recovery.

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
