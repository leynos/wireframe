# RFC 0009: Datagram session transport

- Status: proposed
- Scope: expose packet boundaries and peer metadata for generic datagram
  adapters.
- Depends on: RFCs 0003, 0004, and 0007; RFC 0008 will validate the transport.

## Problem

A stream does not represent packet loss, duplication, truncation, reordering,
or source-address changes. A synthetic stream shim hides these semantics. Quake
3 therefore needs a native transport boundary; stream protocols remain free to
use stream entry points. Datagram support is a general framework capability.

## Contract

A datagram includes immutable bytes, remote peer address and available local
destination/interface metadata. Deliver each received datagram event once as
one unit; duplicate network packets generate separate events for the adapter;
truncation is explicit. Sending is one bounded packet with acceptance or
failure result. Core never splits, retries, acknowledges, or deduplicates. It
gives no network-order guarantee; an adapter may select a local ordered send
lane. Adapters choose session keys, connectionless behaviour, expiry, native
fragmentation and reliability. Session lifecycle follows RFC 0004; packet bytes
and peer state consume RFC 0007 budgets. The peer/session-entry class charges
validated and unvalidated entries, including retained bytes and entry count,
before insertion. Validate address changes against protocol identity before
updating mappings. Bound unvalidated peer state and response rates.

## Alternatives

A stream shim loses boundaries and complicates identity. Built-in reliable UDP
would impose sequencing and retransmission on protocols that do not require it.
The API exposes datagram semantics and leaves delivery rules external.

## Acceptance

A Quake-style toy handles challenge and bounded fragments under loss,
duplication, reordering, truncation, and translated source ports. A second
stateless datagram toy uses the same API without Quake fields. Tests cover send
failure, expiry, cancellation, anti-amplification bounds, and cleanup. Stream
transports retain their stream contracts. Use `proptest` to generate datagram
loss, duplication, reordering, expiry, and cancellation traces; verify event
boundaries and exactly-once entry release without assuming network ordering.

## Packet lifecycle and failure rules

Receive yields one owned datagram event with peer and available local endpoint
metadata. Oversize receive/truncation is reported explicitly; the
implementation does not pass a silently shortened packet to a decoder. The
adapter either handles the packet statelessly or returns a bounded session key
after validation. Session creation is charged before insertion. Expiry,
cancellation, transport close, and adapter rejection each remove the key and
release state once.

A send action represents exactly one datagram. It is either rejected before
acceptance, accepted and handed to the socket, or failed with the transport
error. There is no promise of peer receipt. Oversize packets fail before send;
core does not fragment. Concurrent sends preserve each datagram atomically but
may reorder unless an adapter selects an ordered lane. Such ordering is a local
scheduling promise, not a network delivery guarantee.

## Staging, risks and decisions

Prototype one connectionless challenge toy, then a session-keyed Quake subset
with translated addresses and native fragmentation. Use RFC 0008's simulated
loss rather than nondeterministic network tests. Decide how portable
destination metadata is represented and how socket errors reach session
handles. Risks are address spoofing, amplification, and unbounded peer tables;
admission limits, response budgets and expiry are mandatory before exposing a
listener.

The stream transport remains the default path for stream protocols. This RFC
adds a generic datagram transport and does not add Quake field names,
reliability semantics, or parser code to production Wireframe.
