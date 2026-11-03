# RFC 0003: Native message adapter boundary

- Status: proposed
- Scope: replace mandatory `Envelope` mapping with a typed boundary between
  native frames, application operations and native replies.
- Implementation prerequisite: none. Co-design with RFCs 0004 and 0008; those
  links describe later integration and validation, not prerequisites to define
  this contract.

## Problem

A protocol's route, correlation token, channel, kind, and reply context need
not match Wireframe's route and correlation fields. mxd maps its native 20-byte
Hotline header through `route_id_for(header.ty)` and a bincode `Envelope`, then
unwraps the envelope to encode. This works but makes native applications
translate protocol semantics into framework-shaped messages.

## Contract

The session first feeds bounded bytes to a decoder implementing the bounded
framing contract. RFC 0006 may supply reusable primitives but is not a
prerequisite: adapters may supply their own decoder. Only complete native
`Frame` values cross this boundary; the decoder retains incomplete bytes and
owns bounded recovery/resynchronization. The adapter receives one such frame
and connection-local context, then returns bounded actions: dispatch with
opaque reply context; send control output; complete without output; close; or
request recovery. Application completion returns the opaque context. The
adapter maps zero, one or many native frames. Inbound and outbound frame types
may differ. Envelope conversion is an optional example with no privileged
runtime path and no compatibility obligation.

Each `Dispatch` action creates its own independently owned reply context and
operation ticket, so multiple dispatches from one frame cannot alias a mutable
context. A control action has no reply context. Each operation context is
consumed once by completion or explicitly released on cancellation, timeout,
recovery, or terminal close; cancellation is a terminal outcome, not an illegal
drop. Errors distinguish malformed input, adapter failure, application failure,
resource exhaustion and transport termination. Successful no-reply is not an
error. Core owns dispatch, cancellation, and context accounting; the adapter
owns protocol-field meaning; the application owns domain semantics. Core has no
protocol-name branches. Already-parsed message-stream routing is optional and
lower priority than raw negotiation and native framing.

## Alternatives

Packing metadata into payload hides reply context and defeats independent
accounting. Duplicating a server runtime in each adapter prevents reuse of
Wireframe dispatch. A universal message type erases peer distinctions.

## Acceptance

A Hotline toy preserves native header and correlation through a reply without
bincode; Modbus preserves transaction/unit identifiers; PostgreSQL preserves
tagged reply context. External examples compile using public APIs only. RFC
0008 defines the full verification matrix.

## Conceptual operations and invariants

Names below describe the contract, not compiled API. An implementation can use
associated types or an enum if it preserves these properties:

- `decode(frame, session) -> Vec<Action<Request, ReplyContext>>` consumes one
  complete native frame. The vector is bounded by the configured per-frame
  action limit. `Dispatch` owns request bytes and reply context; `SendControl`
  owns a native control value; `Complete` has zero or more outputs; `Close`
  specifies graceful or immediate termination.
- `encode(reply_context, completion) -> bounded native frames` consumes reply
  context exactly once. Context cannot be cloned or silently dropped; abort,
  timeout and disconnect explicitly discard it and release its budget.
- Invalid action combinations (dispatch after close, control send without a
  live session, duplicate completion, or context from another session) are
  rejected as adapter contract errors. They do not panic or become peer decode
  errors.

The adapter runs on the session owner from RFC 0004. It does not retain a
borrow of the input buffer across asynchronous dispatch. Framework accounting
charges owned context and output; adapter-provided sizes are checked rather
than trusted. Route selection is an adapter result, not an implicit numeric
cast.

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
whether reply context must be `Send`, how cancellation is surfaced to handlers,
and whether an adapter may emit several independent dispatch actions from one
frame. The default must bound these actions and reject the rest.

Risks include making context too opaque to inspect in diagnostics and making
native borrowing impossible for zero-copy users. Provide safe debug metadata
and owned/shared byte handles without exposing private driver internals.
Supersede the mandatory-envelope assumptions in ADR 004 and ADR 005; preserve
useful codec/serializer implementations only where they remain simpler than the
new native path.
