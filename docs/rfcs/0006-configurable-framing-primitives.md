# RFC 0006: Configurable framing primitives

- Status: proposed
- Scope: small bounded primitives for native codecs without core protocol
  state machines.
- Dependencies: RFC 0003. RFC 0007 consumes these primitives to account for
  their retained and assembled bytes.

## Problem

Fixed-width lengths, alternatives, short headers and payload bounds recur
across codecs. mxd already has a native 20-byte Hotline header, bincode mapping
and separate series tracker. General framing support should simplify that work
without making Hotline the library's identity.

## Contract

Provide checked fixed headers (offset, width, byte order, adjustment), retained
metadata, bounded lengths, delimiter/header-body composition, discriminated
alternatives and optional pure validation/transformation callbacks. Support
asymmetric inbound/outbound layouts. Incomplete input reports needed progress;
overflow, invalid length, truncation and EOF remain distinguishable. Check
limits before allocation. Document byte consumption on failure and minimum
progress. Resynchronization is opt-in and bounded. Sequence rollover,
compression state, authentication, and reliability remain ordinary external
Rust adapter code. Do not create a universal framing language.

## Alternatives

Handwritten codecs maximize flexibility but duplicate boundary logic. A
configuration language for every pattern accumulates protocol-specific flags.
Small primitives plus ordinary Rust handle unique formats without special-case
core APIs.

## Acceptance

Property tests cover every split point, coalescing, overflow, invalid
discriminants, incomplete EOF, exact allocation bounds and progress.
Independent toys exercise Hotline fixed headers, FLAP, MBAP, Kafka lengths,
MySQL 3-byte lengths, PostgreSQL startup/tagged records, IEC 104, DNP3 CRC
blocks, RTSP mixed records, AMQP headers, OpenRA asymmetric layouts, and Quake
datagram payloads. These fixtures specify bounded subsets, not complete
implementations.

## Pipeline and error ownership

The data path is byte stream -> bounded decoder -> native `Frame` -> RFC 0003
adapter action -> dispatch or native output. The decoder alone owns its input
buffer until it yields a frame or explicitly consumes bytes in a bounded
recovery result. The adapter never sees incomplete raw bytes as a frame. A
recoverable decode result carries a typed error and exact consumed count; the
session applies its recovery policy. Fatal errors close. Any resynchronization
scan belongs to the decoder, has an absolute byte/attempt bound, and must
consume bytes or terminate on each iteration. This prevents a recovery callback
from repeating against the same failing prefix forever.

## Primitive behaviour

A length-field decoder is configured with field offset and width, byte order,
base adjustment, included/excluded field semantics, maximum wire size and
maximum decoded payload size. Arithmetic uses checked operations. It reports
`NeedMore(n)`, `Frame(frame, retained_metadata)`,
`Recoverable(error, consumed)`, or `Fatal(error)`. The recoverable variant
reports the exact consumed byte count; a resynchronization attempt must make
positive progress or terminate. `NeedMore` must not consume bytes needed to
decode the frame. Invalid or oversized declared lengths fail before
reserve/allocate. EOF with no buffered bytes is clean; EOF within a header or
body is typed truncation.

Composition is ordered and deterministic. A discriminator selects exactly one
branch; zero or multiple matching branches is a configuration error. A pure
validator can reject a completed candidate but cannot cause an unbounded
rescan. Stateful checksum accumulation may be a codec implementation over
public byte access, not a hidden core registry. Outbound encoding receives
metadata explicitly so it cannot accidentally default sequence or channel
fields.

## Staging and decisions

Implement checked length arithmetic and split/coalescing properties first, then
demonstrate mixed text/binary composition and block validation with RFC 0008
toys. Decide whether variable-width integers belong in the primitive set only
after multiple independent examples need them. Risks include a flag-heavy
configuration API and callbacks that allocate outside framework budgets. Keep
the surface small. Core reserves input-buffer budget before reading or
retaining bytes, including leftover bytes after coalesced frames. Release each
reservation on consumption, abort, timeout, cancellation, or session close.

Existing Tokio codec support remains usable behind an adapter. This pre-1.0
proposal permits breaking changes to all `FrameCodec` methods, including
`decoder`, `encoder`, `frame_payload`, `wrap_payload`, `max_frame_length`,
`frame_payload_bytes`, and `correlation_id`. Existing implementations in
`src/codec/examples.rs` and `src/test_helpers/frame_codec.rs` must migrate.
Codec constructors and `Envelope` serializers are likewise replaceable. ADR 004
remains historical input; update its status when this contract is approved.
