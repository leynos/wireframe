# Architectural decision record (ADR) 018: deliver streaming request bodies to route handlers

## Status

Accepted.

An opt-in `WireframeApp::streaming_route` delivers `RequestParts` and a
`RequestBodyStream` to its handler as a message's frames arrive, for the one
message that owns the connection's dispatch slot. A message that completes
while another body owns that slot is parked and delivered afterwards as a
pre-filled body. Implemented for roadmap item 8.1.4 by issue
[#876](https://github.com/leynos/wireframe/issues/876).

## Date

2026-11-15.

## Context and problem statement

[ADR 002](adr-0002-streaming-requests-and-shared-message-assembly.md) decided
that handlers MAY receive `RequestParts` plus `RequestBodyStream` rather than a
fully reassembled body. Its addendum of 2026-11-09 recorded that the shapes
were never wired into dispatch, and
[ADR 017](adr-0017-remove-route-extractors-and-app-data.md) removed the
`StreamingBody` extractor and the application data store, leaving
`RequestParts`, `RequestBodyStream`, and `RequestBodyReader` as caller-owned
primitives.

That left the primitives without a server-side consumer. A protocol crate could
build a body channel, but nothing in the connection runtime passed the
receiving end to a handler, and nothing fed it from incoming frames. Every
large inbound message therefore had to be reassembled in full before a handler
could start work, even when the handler only ever reads it sequentially.

Roadmap item 8.1.4 closes that gap: route handlers must receive streaming
request bodies.

## Decision drivers

- Existing buffered routes and their guarantees must not change.
- A streaming body is peer-controlled state. Its memory must be bounded by the
  same budgets that protect assembly, and it must never outlive the connection.
- Cancellation must be complete. Dropping the connection must drop the handler,
  the body, and any in-flight admission rather than leaving detached work.
- The handler signature must stay expressible without a new envelope type, and
  its reply must travel the same codec path as any other response.
- A handler that abandons its body must not leave the assembler waiting for
  frames nobody will read.

## Decision

### 1) Opt-in route registration

`WireframeApp::streaming_route(id, handler)` registers a `StreamingHandler`:

```rust,no_run
pub type StreamingHandler = Arc<
    dyn Fn(RequestParts, RequestBodyStream) -> Pin<Box<dyn Future<Output = Option<Bytes>> + Send>>
        + Send
        + Sync,
>;
```

Route identifiers are unique across both maps: registering a streaming route
for an identifier that already has a buffered or streaming handler fails with
`WireframeError::DuplicateRoute`, and the reverse holds too. A message whose
first frame completes the message still reaches the handler as a one-chunk
body, so a streaming handler never has to distinguish the two shapes.

The reply payload is `Bytes`.
[ADR 008](adr-0008-zero-copy-public-byte-container.md) makes the zero-copy
container the stable public payload type, and
[ADR 009](adr-0009-rollout-strategy-for-the-vec-u8-to-zero-copy-migration.md)
confines `Vec<u8>` to a named compatibility set that a new public surface must
not join. The handler's payload is therefore converted once, at the reply
boundary, while the middleware and envelope path still carries `Vec<u8>`.

Middleware does not run for streaming routes. Middleware transforms a whole
payload, while a streaming handler owns the body incrementally and may never
materialize it. The reply takes the same path as a buffered reply: its payload
becomes an ordinary response envelope for the route and correlation
identifiers, and passes through the outbound codec pipeline. Returning `None`
sends no frame.

### 2) One dispatch slot per connection

A connection delivers at most one streaming request body at a time. The slot
owns the handler future, the byte-accounted sender, and at most one pending
chunk admission. The connection loop polls the handler, the admission, and the
inbound socket from one place; no task is detached.

This keeps three properties that separate tasks would make conditional:

- dropping the connection drops the handler and every chunk it had not read;
- admission cannot reorder chunks, because only one admission is ever in
  flight; and
- the handler and the socket are both polled, so a slow consumer suspends
  inbound reading for that body instead of starving the handler.

A second streaming message that starts while the slot is busy cannot be
delivered. Complete messages that arrive meanwhile are parked in completion
order: buffered messages as envelopes, and streaming-route messages as
`RequestParts` plus their assembled body. A parked streaming-route message is
later delivered as a pre-filled, single-chunk body. Parked messages are
dispatched, in order, before the loop reads another frame once the slot frees,
and again at end of connection.

### 3) Streaming bytes stay outside the assembly budgets

A message delivered through the live slot is never reassembled, so retaining
its body in the assembly state would double-count bytes that have already been
delivered. `MessageAssemblyState` keeps streaming partials in a separate map
whose entries hold only the message key, the order and byte-count state, and
the declared total. The byte-count state records the first frame's metadata
length so that later size checks charge it, but retains neither the metadata
bytes nor the delivered body bytes, and a streaming partial therefore
contributes nothing to `total_buffered_bytes`.

A streaming-route message that completes while the slot is occupied is not a
streaming body and is not an exception to the accounting above. Nothing can
reach its handler until the slot frees, so it is assembled by the ordinary
buffered path: its bytes are counted by the assembly budgets while it is
incomplete, and by the parked-message account once it is complete and waiting.
It is delivered later as a pre-filled, single-chunk body.

The bytes the connection does retain are accounted elsewhere and added back
into the pressure calculation:

- body chunks queued for the handler, charged through a byte-accounted channel
  and released when the handler takes the chunk or drops the body; and
- parked messages awaiting the dispatch slot.

Chunk admission uses the stricter of `bytes_per_connection` and
`bytes_in_flight`, the same aggregate cap that protects assembly and retained
fragments. One exception is deliberate: a chunk is always admitted to an empty
queue. Without it, a chunk larger than the remaining budget could never make
progress, because the producer would wait for room that only a consumer could
create while the consumer had nothing to take. The hard-cap policy observes the
queued bytes immediately afterwards, so the exception cannot hide an overrun.

### 4) Per-message limits are enforced before delivery

The per-message cap and the declared total body length
([ADR 015](adr-0015-declared-total-body-length-is-authoritative.md)) are
checked when a continuation is admitted, before its bytes reach the handler. An
overshoot, a short final frame, or a frame that would exceed the cap is a
terminal failure: the partial is removed and the handler's body receives an
error. A recoverable continuity error, such as a duplicate or a gap, retains
the partial so a retransmission can still complete the message.

### 5) Body lifecycle outcomes

The handler observes exactly one termination:

| Event                                    | Body outcome                        |
| ---------------------------------------- | ----------------------------------- |
| Assembly expiry                          | `TimedOut`                          |
| Connection end of stream                 | `UnexpectedEof`                     |
| Inbound decode failure                   | the decoder's error kind            |
| Per-message cap or declared-total breach | `InvalidData`                       |
| Handler drops the body                   | remaining frames validated, dropped |
| Aggregate hard cap breached              | connection closed, handler dropped  |

When the handler drops its body stream, the message key switches to discard
mode: remaining frames are still validated for continuity and size, so the
connection does not lose framing, but nothing is delivered.

A reply the handler produces while the body is being closed is still written to
the connection, because a peer that half-closed its sending direction may still
be reading. If the peer has gone entirely, that write fails and ends the
connection; the handler's observation of the body error is unaffected.

## Addendum (2026-11-18): replies are peer-observed while they are written

Section 5 says a reply produced while the body is being closed is written to
the connection, and that the write fails and ends the connection when the peer
has gone entirely. That holds only while the peer's read half is observed
during the write.

Forwarding a reply writes to the transport, which blocks for as long as the
peer declines to read, and a reply may stream for an unbounded time. The loop
therefore polls the reader alongside every reply-producing action: dispatching
a frame, draining parked messages, forwarding the reply of a handler that
resolved while the loop waited, and failing a body whose request was cut short.
A peer that half-closes is observed during the write, which arms the actor's
peer-close grace; a write that makes no progress within the grace is abandoned
and the connection stops.

The delivery outcome of that write decides whether anything else may be served.
A reply that fails or is cancelled can leave a partial frame on the wire, so a
`Dispatch::Stop` from failing the body suppresses the parked drain that follows
a peer's end of stream; those messages would otherwise be read as part of the
broken stream.

The consequences above name `stream_dispatch.rs`, `stream_slot.rs`, and
`stream_parked.rs`. The loop's maintenance actions now live in
`stream_actions.rs`, its peer-observed reply path in `stream_observed.rs`, and
its peer-observed chunk admission in `stream_admission_watch.rs`, so each
module stays within the repository's size limit.

`src/app/inbound_handler/stream_reply_watch_tests.rs` pins the behaviour: a
reply that outgrows the transport buffer ends at the peer-close grace once the
peer stops reading, and a cancelled reply serves no parked message. Reverting
the dispatch to an unobserved write makes both tests fail by never completing.

## Addendum (2026-11-18): body outcomes and occupancy are observable

Section 5 lists the terminations a handler observes, but the connection's own
view of a body was invisible: a delivered body, a body failed terminally, a
body abandoned by an early reply, and a chunk that never cleared admission all
left the same trace, which was a handful of unstructured log lines. An operator
could see that a connection was busy without being able to say which of those
had happened, and a stalled handler and a peer that stopped reading looked
identical from outside.

Three metric series now separate them. All three are gated behind the `metrics`
feature and are no-ops otherwise.

- `wireframe_streaming_body_outcomes_total` counts each body decision with a
  fixed `outcome` label: `delivered`, `terminal`, `abandoned`,
  `admission_timeout`, `delivery_timeout`, or `no_handler`. The label
  vocabulary is a closed set so application data cannot create new series.
  Three of the labels are also the `error_category` of a diagnostic event:
  `admission_timeout` on `prefilled_body`, and `delivery_timeout` and
  `no_handler` on `terminal_body` (with `no_handler` also naming a
  `stream_start` event). Those three categories are defined from the outcome
  vocabulary rather than repeated as literals, so the counter and the event
  cannot drift apart. The other categories name something the counter does not:
  `abandoned_body` reports the chunk-dropping event whose counter label is
  `abandoned`, and `closed`, `receiver_gone` and `reply_abandoned` describe a
  delivery or reply outcome rather than a body decision.
- `wireframe_streaming_body_bytes_queued` gauges the body bytes a process
  holds for handlers. A chunk is charged where its reservation is installed, so
  the number includes a chunk that is still waiting for channel capacity rather
  than only queued ones, and it is released by the byte guard whoever drops it.
- `wireframe_streaming_bodies_parked` gauges completed messages waiting behind
  the live body.

The two gauges are process-wide totals rather than per-connection values. A
per-connection gauge would need a connection label, and connection identity is
unbounded; a single process-wide total with no labels reports the same
occupancy without turning a connection identifier into a metric dimension.
Concurrent connections therefore add to one total, which is guarded by a
`Mutex` and published on every change. The change and the publish happen under
the one lock, because `Gauge::set` assigns rather than advances: two releases
that each computed their total outside the lock could publish in the opposite
order, and the gauge would stay above the real total for good. Release
saturates, so a release that outranks its addition floors at zero instead of
wrapping to `usize::MAX`. A connection that ends with messages still parked
releases them in one step, so the gauge cannot keep counting work nobody will
dispatch.

The counter records one observation per decision, with two exceptions noted in
the vocabulary: an expired admission wait is recorded per expiry, so one body
that waits across several reads contributes several `admission_timeout`
observations, and an abandoned pending chunk is recorded once when the exchange
is torn down rather than per chunk that never arrived.

Every decision also emits one structured `tracing` event, carrying an
`operation` (`stream_start`, `chunk_delivery`, `prefilled_body`, or
`terminal_body`), the `message_id` or assembly `key`, and an `error_category`.
Payload bytes are never logged. The `warn` level is reserved for the cases
where a message could not be served: a pre-filled body that did not fit the
connection budget in time, a terminal failure the handler's channel would not
accept, a frame for a streaming route with no live handler, a terminal failure
with no live handler left to receive it, and a handler that outlived the
connection's wait for its reply. A body abandoned by an early reply or by the
hard cap is a `debug` record, because the handler chose that outcome.

A chunk that cannot clear admission keeps the loop off the reader for as long
as the handler declines to take it, so the read half is now polled alongside
the admission as well: the peer's departure is what ends such a connection. A
frame that arrives meanwhile is buffered for the step after admission rather
than dispatched, because a continuation offered while its own chunk is still
queued would be refused as an abandoned body. The buffer holds one frame, which
is the same bound the reply-observation path uses.
`src/app/inbound_handler/stream_admission_watch_tests.rs` pins that
observation: without it the stalled connection never ends.

`src/metrics/streaming_bodies.rs` holds the series and its tests, and
`src/app/inbound_handler/stream_diagnostics.rs` holds the event vocabulary.
`src/app/inbound_handler/stream_outcome_tests.rs` pins the two outcomes that
only their deciding branch can count. The occupancy arithmetic is exercised on
local tallies rather than the shared statics, because the statics are
process-wide and an absolute assertion on them would race with concurrently
running tests.

## Consequences

- Protocol crates can process large inbound messages incrementally without
  writing their own channel plumbing or budget accounting.
- A connection carries at most one streaming request body, so a protocol that
  needs concurrent bodies must open concurrent connections. This is an explicit
  simplicity and cancellation trade-off, not an accident.
- Streaming handlers bypass middleware. A protocol that relies on middleware
  for authentication or logging must apply those checks inside the handler or
  keep the route buffered.
- The connection loop now owns the dispatch slot, the parking queue, and the
  pressure calculation together. The loop lives in
  `src/app/inbound_handler/stream_dispatch.rs`, with the slot in
  `stream_slot.rs` and parking in `stream_parked.rs`, to keep each module
  within the repository's size limit.

## Evidence

Unit tests cover streaming assembly
(`src/message_assembler/streaming_tests.rs`) and the byte-accounted channel
(`src/request/accounted_tests.rs`). Behavioural coverage lives in
`tests/features/streaming_request_dispatch.feature` with its step definitions,
and the TCP lifecycle tests in `tests/streaming_request_server/`.

## References

- [ADR 002: streaming requests and shared message assembly](adr-0002-streaming-requests-and-shared-message-assembly.md)
- [ADR 008: zero-copy public byte container](adr-0008-zero-copy-public-byte-container.md)
- [ADR 009: owned-byte migration rollout](adr-0009-rollout-strategy-for-the-vec-u8-to-zero-copy-migration.md)
- [ADR 015: declared total body length is authoritative](adr-0015-declared-total-body-length-is-authoritative.md)
- [ADR 017: remove the unwired route extractors and application data API](adr-0017-remove-route-extractors-and-app-data.md)
