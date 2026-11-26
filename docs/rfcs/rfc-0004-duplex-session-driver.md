# RFC 0004: Duplex session driver

- Status: proposed
- Scope: define full-duplex connection lifecycle for reads, dispatch,
  unsolicited output, streams, and bounded termination.
- Depends on: RFCs 0003 and 0007. RFC 0005 later integrates negotiated
  transports; its contract can be co-designed without blocking this driver.

## Problem

Wireframe already has output queues. mxd owns a push-handle registry, including
unsolicited disconnect notifications. These facilities do not alone define a
session that advances reads while application work, output streams, deadlines,
or control traffic are pending. Awaiting a handler before reading again can
delay protocol progress; unbounded per-message tasks can reorder dependent
replies. Magitek's Agent Client Protocol (ACP) consumer additionally needs
agent-originated requests and notifications while a host-originated prompt
remains pending. An outbound actor alone does not meet that requirement.

## Contract

One session owns adapter state, reader, writer, bounded queues, and child work.
It selects among inbound frames, application completions, output readiness,
deadlines, cancellation, and EOF. Adapter policy selects serial or bounded
concurrent dispatch and whether completion preserves arrival order. Unsolicited
native output uses the session writer. Push registry integration uses the same
bounded writer and terminal lifecycle; push facilities already exist.

Expose one host-driven connection future and bounded submission handles for
both initiating and accepting peers. Accept supplied read/write halves without
requiring a TCP listener, a server application, a private runtime, or one
thread per connection. RFC 0005 specifies transport setup; an
already-established stream needs no negotiation. There must be one writer and
one semantic correlation owner, not competing SDK and Wireframe dispatch loops.

Output reports accepted, encoded, publication-attempted, flushed, failed,
cancelled, or superseded. These describe local evidence, not peer receipt,
remote execution, or host durability. Superseded is terminal only for an
adapter-declared replaceable frame before publication. Partial-write failure
terminates the stream and never silently replays. Graceful close stops new
admission and drains only a bounded accepted set before its deadline. Immediate
close cancels work and abandons unsafe writes. Both settle each operation once;
actual resource release follows ownership under RFC 0007. Errors distinguish
transport, peer-decode, adapter-contract, application, resource-exhaustion, and
shutdown failures. Distinct recovery paths must not be collapsed.

## Independent progress and work placement

A pending write, handler, response stream, decoder job, or human decision must
not monopolize the session loop. Poll the reader, bounded completions,
deadlines, and cancellation while output is pending, subject to available input
capacity. Keep partial-write state with the sole writer; never cancel a write
future and restart its frame from byte zero. A small supervised task set or an
explicitly polled state machine may implement this contract. Admission bounds
every queued and running job; spawning one task per frame is not a bound.

The adapter's serial dispatch choice serializes dependent application work, not
all transport progress. Protocol-declared reply routing and independent control
work must still advance without waiting on that work. Ordered handlers perform
bounded capture only and must not await later input from the same connection.
Parsing and callbacks that exceed the synchronous work envelope use
host-supplied admitted worker capacity. Timeouts cannot pre-empt arbitrary
blocking code; still-running work keeps its charges and cleanup status.

Assign ingress ordinals before concurrent decode and bound reorder storage.
Apply finite frame, byte, and completion quotas per poll, then yield. State the
executor-service and cooperative-peer assumptions behind progress tests.
Control reserves protect admission after classification; they cannot skip an
unread notification on an ordered stream or interrupt a partially written
frame. If an internal handoff cannot obtain capacity within its deadline,
report scoped overload or close rather than claim unconditional control
liveness. A blocked connection must not monopolize unrelated ready connections.

## Alternatives

A parallel adapter-owned server loses prepared dispatch and lifecycle reuse. A
mandatory serial loop blocks independent work; mandatory concurrency risks
reordering. Adapters select execution semantics while core enforces bounds.

## Acceptance

A stalled request does not block reachable ping, close, or an independent
channel while capacity and executor service permit progress. Kafka-like replies
preserve order under concurrent completion. A PostgreSQL COPY-like stream stays
bounded while cancellation is observed. OpenRA-style fan-out to a slow peer
does not stall unrelated peers. The mandatory RFC 0008 ACP toy combines a
pending prompt, reverse permission request, notification flood, held consumer
leases, partial writes, and cancellation. Each scenario progresses under its
stated assumptions or produces an explicit bounded failure. Exercise EOF, close
races, retained work, and exactly-once terminal settlement.

## Event ordering and invalid actions

The session is the sole owner of transport writes and adapter state. Events are
observed in a total order by that owner, even when work is concurrent. Each RFC
0003 `Dispatch` operation receives its own monotonically increasing local
ticket; a control action receives no dispatch ticket. Several dispatches from
one frame therefore have distinct tickets. End a generation before ticket
exhaustion. The adapter selects serial handling or a bounded in-flight window.
In ordered mode, reply emission waits behind earlier tickets in its declared
lane; unordered mode requires explicit permission. Do not impose a global
application order across independent lanes.

A control action may overtake unrelated unsent data only at a documented frame
boundary. It must not overtake a causal prerequisite, such as cancellation
passing the request it cancels. A completion for an unknown ticket, second
completion, send after terminal close, or queue item exceeding its reservation
is an internal contract error. Reject it, emit diagnostics, and close or fail
the session according to the error class; never treat it as malformed input. A
recoverable framing error follows RFC 0006 and reaches the adapter's explicit
policy. Fatal errors close. Recovery never replays dispatched work. EOF arrives
once after buffered complete frames; a truncated tail is a decode termination,
not clean EOF. Read-half closure does not itself establish external process
exit or require discarding valid pending output; the profile bounds draining.

## Publication and terminal evidence

Make queued-to-publication-attempted atomic with local withdrawal. A request
can report not submitted only if rejection or removal precedes every possible
publication attempt, including handoff to a buffering transport. Otherwise
failure is conservatively outcome unknown. Flush is local transport evidence,
not a peer acknowledgement. Dropping an admitted caller future cannot lose the
operation record or trigger automatic replay. Keep bounded outcome storage
available through the handle or connection status.

Reserve a terminal-status cell and its capped diagnostic storage at connection
admission. A saturated event consumer must still observe the first failure,
closure, and outstanding cleanup without needing ordinary event capacity. Do
not synthesize successful protocol completion when a stream truncates. Report
transport progress, consumer pressure, and cleanup separately. ACP
session-event delivery watermarks and host durability remain Magitek policy;
generic ingress order or a flushed frame cannot stand in for either.

## Ownership and backpressure

The session owns reader, writer, cancellation token, child work, and terminal
notification. Handler jobs own their request and reply context. A bounded
submission interface reserves item, byte, and work capacity before acceptance.
Payload charges survive dequeue, serialization, buffering, and partial writes
until their owners release them. Reserve a bounded response obligation before
admitting a reverse request; a full ordinary output budget must not prevent a
small policy-approved refusal. Control capacity itself is finite and charged.

Quiescing stops new work but permits admitted obligations to settle under a
drain deadline. Closing fences the generation, settles waiters and contexts,
and reports incomplete cleanup when workers outlive the close deadline.
Outstanding leases and non-pre-emptible workers remain charged; closing a
connection is not budget retirement. Concurrent close callers observe one
primary terminal cause, with bounded secondary cleanup evidence. An
application-owned registry such as mxd's may publish submission handles without
moving that registry into Wireframe or keeping dead generations unaccounted.

## Staging, risks and open decisions

Prove event order and shutdown races with a deterministic fake transport, then
integrate one prepared TCP connection and the same driver over separate
supplied read/write halves. RFC 0008 supplies the executable contract toys.
Decide whether ordered emission uses a session reorder buffer or per-stream
lanes; either choice must preserve global bounds. Review protocol-close output
versus handler cancellation explicitly. Use `proptest` for action, completion,
publication, cancellation, and close interleavings. Reuse RFC 0002's pure
scheduling-decision approach for bounded state exploration of production
transitions; do not claim Loom schedules ordinary Tokio primitives.

This proposal refines the ownership concerns in ADR 011 and ADR 012. The
current actor's sink-driven response integration is not full duplex:
independent inbound dispatch and writer progress remain implementation work.
Extend that connection-local runtime rather than introduce a parallel actor.
