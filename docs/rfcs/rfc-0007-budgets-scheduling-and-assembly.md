# RFC 0007: Budgets, scheduling and assembly

- Status: proposed
- Scope: bound native frames, retained representations, logical assembly,
  admitted work, and queued output across connections and streams.
- Depends on: RFCs 0003 and 0006. RFC 0004 consumes these limits in the session
  driver. Contract co-design does not require completed framing machinery.

## Problem

Wire bytes, decoded bytes, assembled messages, decompressed data, and queued
responses are distinct resource dimensions. One global maximum cannot express
native continuation. In multiplexed sessions, one exhausted channel should not
necessarily block control work or an independent channel. Magitek additionally
requires structural JSON limits, bounded waiters and workers, charged
consumer-held events, and shared-host accounting that survives connection
closure and replacement.

## Contract

Expose reusable typed reservation primitives with finite shared-host,
connection-generation, optional logical-key, and stage scopes. Validate a
complete immutable policy before admitting a connection. Acquire a resource
before allocating, retaining, enqueueing, registering, or starting the work it
authorizes. Every reservation has one terminal release; release follows actual
ownership, not merely dequeue, logical completion, cancellation, or close.
RFC 0004 supplies the connection and output lifecycle.

Adapters own protocol keys, correlation, fragment identifiers, duplicate policy,
rollover, acknowledgements, and semantic timeout choices. Core enforces the
selected accounting, deadline, and scheduling contracts without ACP-specific
fields. A finite adapter-defined resource-class vocabulary can express decoded
nodes or other typed units; it must declare units, limits, enforcement points,
and bounded diagnostics at construction. Peer-provided class names or an
unbounded dynamic class registry are not allowed. These APIs must work without
message assembly, a default envelope, or the server router.

## Resource dimensions and policy validation

A bounded profile must express these dimensions independently where applicable:

| Dimension | Required bound and ownership |
| --- | --- |
| Wire frames | Separate inbound/outbound encoded limits and framing overhead; aggregate receive/egress item and byte capacity. |
| Retained representations | Backing allocation capacity for wire, decoded, assembled, decompressed, and temporary encoded data; shared allocations remain charged once. |
| Structural complexity | Per-frame depth/node limits, aggregate retained nodes, identifiers, metadata, and per-frame action count. |
| Work and admission | Queued and running handler/CPU jobs, pending requests in each direction, parked responders, waiting producers, and reorder entries. |
| Protocol populations | Active streams, routes, and validated/unvalidated peer entries, with both count and retained-state bytes charged before insertion. |
| Consumer ownership | Queued and issued leases, including empty/null events; bytes, nodes, and lifetime item permits survive dequeue and close. |
| Protected capacity | Finite control responses, cancellation obligations, and capped terminal-status storage inside enclosing host/connection budgets. |
| Generations | Live and closed-but-retained connection generations, including their bounded state and outstanding obligations. |

_Table 1: Independently bounded resource dimensions._

Required limits and deadlines are finite positive values, not zero, absent
unlimited defaults, or sentinel maxima. Disabled optional capabilities must
reject use explicitly. Use checked arithmetic to validate child/parent limits,
maximum frame plus delimiter/header storage, ordinary capacity after protected
reserves, and aggregate reserves across the admitted connection count. Invalid
policy fails construction rather than being silently clamped.

Adapters declare how many response and cancellation obligations each admitted
operation can create. Validate enough protected slots and bytes for the selected
concurrency profile, while leaving ordinary traffic capacity. Validate the
largest bounded control envelope with the actual encoder, including identifier
escaping and delimiter bytes. A sum of plausible integers alone does not prove
that the reserved response fits. Bound extension/context data and both queued
and running jobs; a byte cap cannot bound populations of zero-byte objects.

## Reservations, transfer, and retirement

Acquire a multi-scope, multi-resource reservation atomically or roll back before
waiting. Never hold one budget's lock or partial permit while awaiting another.
The default non-waiting submission form rejects pressure immediately. An
explicit waiting form bounds waiter count and waiting time; it must not capture
unlimited producer payloads in hidden waiting futures. Caller-retained input
before admission is an explicit exclusion, not unobserved library-owned memory.

Ownership guards or an equivalent ledger must support movement, splitting,
sharing, and destination transfer without an uncharged interval. A stage change
moves its occupancy charge but retains the enclosing allocation charge. Shared
immutable backing storage stays charged once until its last accountable owner
releases it; every new copy or growing allocation requires additional capacity
before allocation. Count retained capacity, not only slice length, and keep
item/node charges for outstanding views. Public owning handles retain their
charge; borrowed byte slices must not expose uncharged owning `Bytes` clones
that can outlive the guard. Enforced paths expose no unmetered sender or raw
connection escape hatch. A destination transfer reserves capacity before
committing ownership; cancellation before commit returns or retains the source,
and after commit the destination owns release. Do not count a move as a new
allocation or omit a temporarily live copy.

An event lease keeps its lifetime item permit, bytes, and nodes when yielded.
Consumers may explicitly transfer it to a bounded sink. Copying a borrowed view
into host-owned storage is outside the library ledger and does not release the
source. Closing a logical key discards owned queues/assembly and cancels its
work without revoking issued leases or cancelling unrelated keys. A completed
assembly transfers its payload charge to the next owner rather than freeing
it while the assembled value still exists.

Fence admission at close, but retain charges for issued leases, live workers,
reply contexts, and other outstanding ownership. A generation retires only
when these obligations and its retained state disappear. Its connection-count
permit remains held until retirement, including for empty events; replacement
uses the same shared budget and cannot reset the old account. Deadlines report
outstanding cleanup rather than falsely reclaiming a non-pre-emptible worker.
Bound retired-ID diagnostics instead of accumulating permanent tombstones.

Parsing and decompression reserve a conservative expansion envelope or use
budget-aware allocation before materialization. Framework checks alone cannot
meter arbitrary callback allocations or hidden SDK queues. Qualifying a bounded
adapter requires an ownership map across those handoffs and either enforcement
or explicit exclusions. Ledger limits do not promise a process resident-memory
ceiling: allocator metadata, stacks, kernel buffers, and independent host copies
require separately documented measurement or host limits.

## Scheduling, reserves, and deadlines

Reserve protected control and terminal capacity at connection admission against
the shared budget. Ordinary data cannot borrow it. Before starting a reverse
request handler, reserve its response obligation. If its full reply cannot
obtain ordinary data capacity, it may use a bounded policy-approved refusal;
if that cannot progress, terminate with the original pressure cause. Status
visibility must not depend on draining ordinary event leases.

Scheduler readiness means both transport writable and protocol policy allows
sending; transport readiness does not grant AMQP-like credit. Control capacity
is finite and rate-limited. FIFO ordering applies within declared ordered lanes;
fair service across ready lanes and connections uses finite work batches and
documented service bounds. A profile may require round-robin scheduling across
host-driven connections. State the executor-service assumptions and test quiet
connections under noisy peers. Reserved control cannot bypass unread bytes or
a partial write on one ordered transport.

Deadline classes separately cover admission waiting, initialization, ordinary
request/response work, partial-frame assembly, decoder queue/execution,
write queue/transmission, handler work, permission waits, drain, and close.
Expose monotonic clock injection and document start points. The admission timer
starts at the first blocked handoff; partial framing starts at the first byte;
processing starts at job admission; writing starts at queue admission. Trickle
input and queue handoffs do not reset these deadlines. Long application work,
such as an ACP prompt or a human permission decision, must not inherit an
unrelated short request timer. Expiry is local evidence, not proof of peer
cancellation or process exit.

Adapters may opt into replacing stale snapshots before publication. An already
accepted but still-unpublished replaceable frame may be superseded exactly
once under an atomic reservation exchange; account for any overlapping old/new
allocations. Never rewrite a publication-attempted or partially written frame.
Reliable output and ACP history events are not replaceable snapshots. Terminal
failure, incomplete delivery, and suppressed diagnostic counts remain explicit.

## Alternatives

One global queue causes head-of-line blocking. Unlimited per-channel queues
create unbounded memory risk. Bounded per-key scheduling with reserved control
capacity keeps policy explicit and resource ownership generic. A growing list
of protocol-named limit fields would duplicate adapters in core; typed finite
resource classes plus validated profiles provide the required expressiveness.

## Acceptance

Generate acquire, rollback, move, copy, share, transfer, supersession, timeout,
and cancellation traces with `proptest`. Assert conservation, bounded peak
usage, and exactly-once final release, including failed destination transfers,
shared backing slices, empty leases, and workers that outlive close. Repeated
connection replacement cannot hide retained generations or structural nodes.
Use a bounded model connected to production reservation/publication transitions
for the small ownership state machine; state fairness assumptions separately.

Test SQL streams, IEC/DNP3 segmentation, AMQP credit, UDP fragments, and the
mandatory RFC 0008 ACP toy. Saturate input, output, event, waiter, and worker
budgets together. A quiet ready connection progresses under declared service
conditions or receives an explicit scoped failure; pressure must not fabricate
successful lossless delivery. Verify protected terminal visibility with event
consumption stopped and measure retained-after-close usage separately.

## Staging and open decisions

Start with policy validation and accounting/retirement invariants, then
integrate framing and one session before independent lanes and the ACP toy.
Choose owned guards or a ledger after checking cross-task and
destination-transfer semantics. Risks include inaccurate adapter size
estimates, hidden allocation, and excessive reservation granularity. Expose
bounded snapshots for occupancy, lifetime ownership, oldest waiter/work/write
age, and cleanup. Use fixed metric labels; format/export diagnostics outside
the session driver.

RFC 0007 extends proposed ADR 002. Its additive API and mandatory legacy
envelope compatibility do not constrain this pre-1.0 redesign. ADR 002 remains
proposed until a later decision records adoption. Replace interpretations of
`MAX_FRAME_LENGTH` as a universal message or memory limit. Reconcile existing
assembly/queue docs and retain useful APIs only when they satisfy the stronger
ownership contract. A separately packaged accounting crate is not required.
