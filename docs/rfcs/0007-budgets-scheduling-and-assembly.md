# RFC 0007: Budgets, scheduling and assembly

- Status: proposed
- Scope: bound native frames, logical assembly and queued output across streams.
- Depends on: RFCs 0003 and 0006. RFC 0004 consumes these limits in the
session driver.

## Problem

Wire bytes, decoded bytes, assembled messages, decompressed data and queued
responses are distinct resource dimensions. One global maximum cannot express
native continuation. In multiplexed sessions, one exhausted channel should not
necessarily block control work or an independent channel.

## Contract

Core applies finite per-session and per-key limits for wire/decoded bytes,
logical assembly, output bytes, active streams and queued work. Reserve before
allocation/enqueue and release on completion, abort, timeout, cancellation and
disconnect. Adapters provide bounded logical keys, classify control/data, and
report assembly lifecycle. The adapter owns fragment identifiers, duplicate
policy, rollover, acknowledgements and timeout semantics; core owns accounting,
deadlines and fair scheduling.

Adapters may select strict FIFO or replacement of stale unsent snapshots.
Replacement is never implicit. Reliable output is never silently discarded.
Metrics use bounded labels. Invalid or unbounded required limits are rejected.

## Alternatives

One global queue causes head-of-line blocking. Unlimited per-channel queues
create unbounded memory risk. Bounded per-key scheduling with reserved control
capacity keeps policy explicit and resource ownership generic.

## Acceptance

Concurrent fragmented inputs never exceed configured limits; all terminal paths
release reservations. A zero-credit stream cannot prevent an independent ready
stream or bounded control traffic. Snapshot replacement only occurs when an
adapter opts in. Test SQL result streams, IEC/DNP3 segmentation, AMQP credit
and UDP fragments.

## Accounting and scheduling semantics

A reservation is acquired before copying or retaining peer-controlled bytes. It
identifies session, optional logical key, and resource class. Reservation
failure is explicit backpressure/resource exhaustion, not a decoder panic.
Every reservation has one terminal release path. Closing one logical key
releases its partial assembly and queue without cancelling unrelated keys.
Session close releases all remaining reservations and reports abandoned work.

Scheduler readiness means both transport writable and protocol policy allows
send; transport readiness alone does not grant AMQP-like credit. Control
capacity is finite and rate-limited so a peer cannot starve data with control
frames. FIFO ordering applies within an adapter-declared ordered lane. Across
lanes, fairness is observable but need not promise strict round robin. Stale
replacement is legal only for unsent frames in an adapter-declared replaceable
class; accepted/partially written data cannot be rewritten.

## Staging and open decisions

Start with accounting/release invariants and a single session toy, then test
independent lanes, control reserve and replaceable snapshots using RFC 0008.
Choose whether budgets are represented by owned reservation guards or a session
ledger after checking cancellation and cross-task use. Risks include inaccurate
adapter size estimates and excessive reservation granularity. Core must charge
actual owned bytes where possible and cap adapter-declared expansion.

RFC 0007 replaces any interpretation of `MAX_FRAME_LENGTH` as a universal
logical-message limit. Reconcile existing assembly and queue docs during
implementation; do not remove useful APIs solely to preserve compatibility.
