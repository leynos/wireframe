# RFC 0005: Negotiated transport entry points

- Status: proposed
- Scope: establish and transition connections while preserving buffered bytes
  and transport ownership.
- Depends on: RFCs 0003 and 0004; RFC 0008 validates the boundary. Setup
semantics may be co-designed before the duplex driver lands.

## Problem

Protocols may speak first, negotiate capabilities, upgrade to TLS, or move from
untagged startup to tagged frames. External setup works today, but direct
socket writes in handshake hooks can split writer ownership. Replacing a
decoder after it has buffered bytes can lose or misclassify peer data.

## Contract

Listener setup receives an accepted transport and peer metadata; it returns
rejection or an owned stream plus session metadata and initial phase. An
in-session phase transition is an explicit action after a confirmed writer
flush barrier. It transfers unread bytes, reader/writer ownership, cancellation
and peer identity atomically. Buffered bytes reach the new phase exactly once.
Consumed bytes cannot be reinterpreted. Setup bounds bytes, time, rounds and
cancellation.

First priority is raw stream negotiation: server-first greeting, client-first
startup, TLS handoff and in-band phase changes. Adapting an existing parsed
stream/sink is supported as a lower-priority optional connector and preserves
message kinds and limits. Core provides setup mechanics, not TLS policy,
authentication or protocol state machines.

## Alternatives

An external listener may finish simple handshakes before Wireframe accepts the
stream. It cannot own later in-band transitions. Flattening parsed messages
into bytes erases boundaries; reopening a socket loses session state.

## Acceptance

Scripted peers cover both greeting directions, coalesced handshake and first
message, positive/negative upgrade, flush order, rejection, timeout and
cancellation. No byte is lost, duplicated or replayed across a phase change. A
WebSocket parsed-stream bridge is a separate optional example, not a condition
for raw setup support.

## Transition semantics

Conceptually, setup returns `Reject(error)` or
`Established(stream, metadata, phase)`. A phase transition is requested by the
current adapter and committed by the session writer after all earlier bytes
flush. The transition result contains the new phase parser and unread bytes.
Only the session commits it. At request time the session inserts a writer-owned
barrier. All already-encoded earlier frames must flush before the barrier.
Accepted but not-yet-encoded operations are cancelled and their contexts
released; they are never encoded under the new phase. This cancellation rule
prevents a barrier waiting on a handler whose completion itself depends on
post-upgrade input. The adapter receives cancellation outcomes and may
explicitly retry under the new phase as new operations. Concurrent transitions,
transition after close, or partial-write crossing the barrier are contract
errors and terminate.

TLS is an opaque transport handoff: the API states who owns the secured stream
and peer identity, but does not make a TLS library or authentication policy a
Wireframe protocol dependency. Setup callbacks cannot write around the session
writer after ownership has transferred. Cancellation at every setup round
closes owned transport and releases buffered-byte budget. Unrecognized or
excess setup bytes fail within configured bounds.

## Staging, trade-offs and open decisions

First prove raw byte setup with server-first and client-first toys in RFC 0008.
Then add a flush-barrier transition and TLS-style wrapper test. Parsed-stream
connectors can follow if an initial consumer needs one; their types must keep
message boundaries and provide backpressure. Decide whether transition tokens
are statically typed or represented by a closed phase enum. A type-erased phase
graph is flexible but risks obscuring ownership and errors.

The key risk is upgrading with bytes already prefetched by a codec. The
contract therefore transfers the actual unread buffer and forbids hidden second
readers. Existing bincode preamble handling is an example, not the required
setup abstraction. Revisit ADR 010's transport/frame boundary and ADR 012's
prepared-app boundary when implementing this RFC; breaking replacement is
allowed.
