# Architectural decision record (ADR) 018: deliver streaming request bodies to route handlers

## Status

Accepted.

An opt-in `WireframeApp::streaming_route` delivers `RequestParts` and a
`RequestBodyStream` to its handler as a message's frames arrive. Implemented
for roadmap item 8.1.4 by issue
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
    dyn Fn(RequestParts, RequestBodyStream) -> Pin<Box<dyn Future<Output = Option<Vec<u8>>> + Send>>
        + Send
        + Sync,
>;
```

Route identifiers are unique across both maps: registering a streaming route
for an identifier that already has a buffered or streaming handler fails with
`WireframeError::DuplicateRoute`, and the reverse holds too. A message whose
first frame completes the message still reaches the handler as a one-chunk
body, so a streaming handler never has to distinguish the two shapes.

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

A streaming message is never reassembled, so retaining its body in the assembly
state would double-count bytes that have already been delivered.
`MessageAssemblyState` keeps streaming partials in a separate map whose entries
hold only the message key, the order and byte-count state, and the declared
total. Delivered metadata and body bytes are not retained there, and a
streaming partial therefore contributes nothing to `total_buffered_bytes`.

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
and the TCP lifecycle tests in `tests/streaming_request_server.rs`.

## References

- [ADR 002: streaming requests and shared message assembly](adr-0002-streaming-requests-and-shared-message-assembly.md)
- [ADR 015: declared total body length is authoritative](adr-0015-declared-total-body-length-is-authoritative.md)
- [ADR 017: remove the unwired route extractors and application data API](adr-0017-remove-route-extractors-and-app-data.md)
