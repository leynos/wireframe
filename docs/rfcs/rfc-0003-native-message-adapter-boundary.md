# RFC 0003: Native message adapter boundary

- Status: proposed
- Scope: replace mandatory `Envelope` mapping with a typed boundary between
  native frames, application operations, and native replies.
- Implementation prerequisite: none. Co-design with RFCs 0004 and 0008; those
  links describe later integration and validation, not prerequisites to define
  this contract.

## Problem

A protocol's route, correlation token, channel, kind, and reply context need
not match Wireframe's route and correlation fields. mxd maps its native 20-byte
Hotline header through `route_id_for(header.ty)` and a bincode `Envelope`, then
unwraps the envelope to encode. This works but makes native applications
translate protocol semantics into framework-shaped messages.

Magitek's proposed bounded Agent Client Protocol (ACP) client adds a second
consumer boundary: newline-delimited JSON (JSONL), bilateral JSON-RPC requests,
notifications, and byte-preserving proxy inspection. Its
[pinned design](https://github.com/leynos/magitek/blob/bfef49cbf10912ccf8ed246d4760bbb21f96c01b/docs/magitek-design.md)
requires reusable framing and accounting without adopting a server router or a
second protocol engine. These are requirements, not implemented capabilities.

## Contract

The session first feeds bounded bytes to a decoder implementing the bounded
framing contract. RFC 0006 may supply reusable primitives but is not a
prerequisite: adapters may supply their own decoder. Only complete native
`Frame` values cross this boundary; the decoder retains incomplete bytes and
owns bounded recovery/resynchronization. The adapter receives one such frame
and connection-local context, then returns bounded actions: dispatch with
opaque reply context; send control output; complete without output; close; or
request recovery. Application completion returns the opaque context. The
adapter maps zero, one, or many native frames. Inbound and outbound frame types
may differ. Envelope conversion is an optional example with no privileged
runtime path and no compatibility obligation.

Each `Dispatch` action creates its own independently owned reply context and
operation ticket, so multiple dispatches from one frame cannot alias a mutable
context. A control action has no dispatch ticket or reply context; its output
still has the acceptance and accounting lifecycle in RFC 0004. Each operation
context is consumed once by completion or explicitly settled on cancellation,
timeout, recovery, or terminal close. Cancellation is a terminal outcome, not
an illegal drop; any still-retained allocation keeps its RFC 0007 reservation.
Errors distinguish malformed input, adapter failure, application failure,
resource exhaustion, and transport termination. Successful no-reply is not an
error. Core owns dispatch, cancellation, and context accounting; the adapter
owns protocol-field meaning; the application owns domain semantics. Core has no
protocol-name branches. Already-parsed message-stream routing is optional and
lower priority than raw negotiation and native framing.

## Native types and reusable boundaries

The public boundary must not require `Envelope`, `PacketParts`, bincode traits,
a numeric route, or `Option<u64>` correlation. Associated types or equivalent
typed adapters must support independent received-frame, transmitted-frame,
request, completion, and reply-context types. A frame may carry headers,
multiple byte spans, retained delimiters, or a typed message rather than one
mandatory payload slice. Borrowed inspection and shared immutable storage must
not force a copy or erase the allocation's reservation. Metadata and opaque
context also have finite byte and item charges.

Wire correlation belongs to the adapter and remains distinct from a local
operation ticket. Preserve string versus numeric identifiers, missing versus
explicit null fields, and bounded extension data; the protocol profile decides
which forms are legal. Do not hash or cast native identifiers into a universal
integer. Scope matching by connection generation and request direction so
simultaneous requests from opposite peers cannot alias. Replies and
notifications must not acquire a fabricated reply obligation or stream
terminator. Adapter state, pending correlations, identifiers, and retired-ID
history all remain bounded; framework-generated tickets never wrap into reuse.

Offer framing and reservation APIs independently of `WireframeServer`,
`WireframeApp`, route registration, and the default client. A standalone
consumer may own JSON-RPC correlation and ACP sessions while using only those
primitives or the RFC 0004 driver. Exactly one component owns semantic dispatch
and pending replies on a given path. A bounded transport feeding an unqualified
SDK queue does not establish a bounded end-to-end connection.

Keep raw frame delimitation separate from admitted semantic decoding. The
session owner performs only bounded capture and state transitions. Expensive
parsing or schema conversion uses admitted work with finite queued/running
jobs and charged results; RFCs 0004 and 0007 define progress and ownership.
An ingress ordinal precedes concurrent decoding, and bounded reordering restores
source order before order-sensitive adapter decisions. JSON syntax, duplicate
keys, capabilities, permissions, and ACP event watermarks remain external
adapter or application policy, not core protocol branches.

## Alternatives

Packing metadata into payload hides reply context and defeats independent
accounting. Duplicating a server runtime in each adapter prevents reuse of
Wireframe dispatch. A universal message type erases peer distinctions.

## Acceptance

A Hotline toy preserves native header and correlation through a reply without
bincode; Modbus preserves transaction/unit identifiers; PostgreSQL preserves
tagged reply context. The mandatory ACP toy in RFC 0008 preserves string and
numeric IDs, notifications, extension fields, and raw permitted frame bytes.
It uses different inbound/outbound types and exercises a standalone consumer
without a mandatory envelope or server application. External examples compile
using public APIs only. Generated action, completion, and cancellation traces
must prove each dispatch context settles exactly once, including recovery and
close races, while outstanding ownership retains its charge.

## Conceptual operations and invariants

Names below describe the contract, not compiled API. An implementation can use
associated types or an enum if it preserves these properties:

- `decode(frame, session) -> Vec<Action<Request, ReplyContext>>` consumes one
  complete native frame. The vector is bounded by the configured per-frame
  action limit before allocation or dispatch. `Dispatch` owns request data and
  reply context; `SendControl` owns a native control value; `Complete` has zero
  or more bounded outputs; `Close` specifies graceful or immediate termination.
- `encode(reply_context, completion) -> bounded native frames` consumes reply
  context exactly once. Context cannot be cloned or silently dropped; abort,
  timeout, recovery, and terminal close explicitly settle it. Cancellation
  remains a distinct terminal outcome. Reservations release only when the
  represented ownership ends, including copies held by outstanding work.
- Invalid action combinations (dispatch after close, control send without a
  live session, duplicate completion, or context from another session) are
  rejected as adapter contract errors. They do not panic or become peer decode
  errors.

The adapter runs on the session owner from RFC 0004. It does not retain a
borrow of the input buffer across asynchronous dispatch. Framework accounting
charges owned context and output; adapter-provided sizes are checked rather
than trusted. Route selection is an adapter result, not an implicit numeric
cast. Encoding must use reserved bounded storage, not allocate an unrestricted
representation and check its size afterwards.

## mxd migration example

At mxd commit `e0dbf2b8289f3b296d8b2825ba533b31ca82363b`,
`src/wireframe/codec/frame.rs` handles native 20-byte headers, maps `header.ty`
through `route_id_for`, and translates with a bincode `Envelope`. The proposed
adapter lets this codec return the Hotline message type and correlation token
directly. Its inbound-series reassembly and configured bounds remain a
session/protocol policy; RFC 0007 determines which accounting primitives can
replace or complement that code. This is evidence for a generic native
boundary, not a Hotline-specific core API.

## Staging and decisions

Prototype this contract alongside the RFC 0008 harness foundation, then wire
dispatch and reply completion through RFC 0004. Decide before implementation
which boundaries require `Send` and how cancellation reaches handlers. Multiple
independent dispatches are permitted only within an explicit finite per-frame
limit; the ACP toy rejects batches rather than expanding them implicitly.

Risks include making context too opaque to inspect in diagnostics and making
native borrowing impossible for zero-copy users. Provide safe debug metadata
and owned/shared byte handles without exposing private driver internals.
Supersede the mandatory-envelope assumptions in ADR 004 and ADR 005; preserve
useful codec/serializer implementations only where they remain simpler than the
new native path. Update affected codec, packet, correlation, envelope, and
serializer APIs together with migration examples and compile-time tests. No
new shared crate or Magitek production dependency is required by this RFC.
