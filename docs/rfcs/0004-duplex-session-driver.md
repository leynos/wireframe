# RFC 0004: Duplex session driver

- Status: proposed
- Scope: define connection lifecycle for reads, dispatch, unsolicited output,
  streams and bounded termination.
- Depends on: RFCs 0003 and 0007. RFC 0005 later integrates negotiated
  transports; its contract can be co-designed without blocking this driver.

## Problem

Wireframe already has output queues. mxd owns a push-handle registry, including
unsolicited disconnect notifications. These facilities do not alone define a
session that advances reads while application work, output streams, deadlines
or control traffic are pending. Blocking receive on a handler can delay
protocol progress; unbounded per-message tasks can reorder dependent replies.

## Contract

One session owns adapter state, reader, writer, bounded queues and child tasks.
It selects among inbound frames, application completions, output readiness,
deadlines, cancellation and EOF. Adapter policy selects serial or bounded
concurrent dispatch and whether completion preserves arrival order. Unsolicited
native output uses the session writer. Push registry integration uses the same
bounded writer and terminal lifecycle; push facilities already exist.

Output reports accepted, encoded, flushed, failed or cancelled. Partial-write
failure terminates the stream and never silently replays. Graceful close stops
admission, drains only a bounded accepted set, then emits one terminal result.
Cancellation reclaims tasks, queues and adapter context. Errors preserve
transport, peer-decode, application and shutdown distinctions.

## Alternatives

A parallel adapter-owned server loses prepared dispatch and lifecycle reuse. A
mandatory serial loop blocks independent work; mandatory concurrency risks
reordering. Adapters select execution semantics while core enforces bounds.

## Acceptance

A stalled request does not block ping, close or an independent channel.
Kafka-like replies preserve order under concurrent completion. A PostgreSQL
COPY-like stream stays bounded while cancellation is observed. OpenRA-style
fan-out to a slow peer does not stall unrelated peers. Exercise short writes,
cancellation, EOF, close races and task cleanup.

## Event ordering and invalid actions

The session is the sole owner of socket writes and adapter state. Events are
observed in a total order by that owner, even when work is concurrent. Each RFC
0003 `Dispatch` operation receives its own monotonically increasing local
ticket; a control action receives none. Several dispatches from one frame
therefore have distinct tickets. The adapter selects serial handling or a
bounded in-flight window. In ordered mode, reply emission waits behind earlier
tickets; in unordered mode, only the adapter's explicit declaration permits
reordering. A control action may overtake queued data only at a documented
frame boundary, never in the middle of a partial write.

A completion for an unknown ticket, second completion, send after terminal
close, or queue item exceeding its reservation is an internal contract error.
Reject it, emit diagnostics, and close or fail the session according to the
error class; never treat it as malformed peer input. A recoverable framing
error follows RFC 0006 and is passed to the adapter, which may continue, close,
or request bounded resynchronization. The decoder owns the buffer and validates
positive progress; the session caps bytes, attempts and deadline. Fatal framing
errors close. Recovery never replays dispatched work. EOF is delivered once
after buffered complete frames; truncated trailing input is a decode
termination, not clean EOF.

## Ownership and backpressure

The session owns reader, writer, cancellation token, child tasks and terminal
notification. Handler tasks own their request and reply context. A bounded
submission queue applies backpressure before accepting output. Once accepted,
output remains charged until flush or terminal failure. Shutdown cancels work,
then drains only policy-approved accepted writes until a deadline. An
application-owned registry such as mxd's can publish handles; this RFC lets
those handles submit through session acceptance and observe terminal outcomes
without moving that registry into Wireframe.

## Staging, risks and open decisions

Prove event order and shutdown races with a deterministic fake transport, then
integrate one prepared TCP connection. RFC 0008 supplies the executable
contract toys. Decide whether ordered emission uses a session reorder buffer or
per-stream lanes; either choice must preserve global memory bounds. Review
whether graceful shutdown should flush protocol close output before cancelling
handlers. Risks are task leaks, head-of-line blocking and ambiguous acceptance
semantics. Tests must assert each ticket terminates exactly once.

This proposal refines, rather than erases, the ownership concerns in ADR 011
and proposed ADR 012. It supersedes any assumption that the current actor's
`Vec` output is already a transport-writing session loop.
