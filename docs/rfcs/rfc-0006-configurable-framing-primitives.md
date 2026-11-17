# RFC 0006: Configurable framing primitives

- Status: proposed
- Scope: small bounded primitives for native codecs without core protocol
  state machines.
- Dependencies: RFC 0003. RFC 0007 consumes these primitives to account for
  their retained and assembled bytes; the framing contract can be co-designed
  with accounting before either implementation lands.

## Problem

Fixed-width lengths, alternatives, short headers, delimiters, and payload bounds
recur across codecs. mxd has a native 20-byte Hotline header, bincode mapping,
and separate series tracker. Magitek needs newline-delimited JSON (JSONL) with
raw-byte preservation and limits that hold before parsing or allocation.
General framing support should simplify both without embedding either protocol
in core or assuming one payload shape and one universal maximum.

## Contract

Provide checked fixed headers (offset, width, byte order, adjustment), retained
metadata, bounded lengths, delimiter/header-body composition, discriminated
alternatives, and optional pure validation/transformation callbacks. Support
independent inbound/outbound frame types and layouts. Incomplete input reports
needed progress; overflow, invalid length, truncation, and EOF remain distinct.
Check limits before allocation. Document byte consumption on failure and minimum
progress. Resynchronization is opt-in and bounded. Sequence rollover,
compression state, authentication, and reliability remain external Rust adapter
code. Do not create a universal framing language.

Limits distinguish encoded input, encoded output, framing overhead, decoded
representation, and aggregate retained allocation. Constructors reject invalid
or inconsistent policy instead of silently clamping it to the current 64-byte
minimum or 16 MiB default-codec maximum. Document whether a particular length
includes headers or delimiters. Use checked arithmetic for all overhead and
expansion calculations. A codec must be usable with public reservation APIs
without a server router or the default serializer.

## Alternatives

Handwritten codecs maximize flexibility but duplicate boundary logic. A
configuration language for every pattern accumulates protocol-specific flags.
Small primitives plus ordinary Rust handle unique formats without special-case
core APIs.

## Acceptance

Property tests cover every split point, coalescing, overflow, invalid
discriminants, incomplete EOF, exact allocation bounds, and progress.
Independent toys exercise Hotline fixed headers, FLAP, MBAP, Kafka lengths,
MySQL 3-byte lengths, PostgreSQL startup/tagged records, IEC 104, DNP3 CRC
blocks, RTSP mixed records, AMQP headers, OpenRA asymmetric layouts, Quake
datagram payloads, and the mandatory RFC 0008 ACP/JSONL profile. These fixtures
specify bounded subsets, not complete implementations. ACP fixtures test exact
limits, delimiter splits, byte-preserving inspection, structural expansion,
and bounded encoding rather than relying on a successful codec round-trip.

## Pipeline and error ownership

The data path is byte stream -> bounded decoder -> native frame -> admitted
semantic decoding where needed -> RFC 0003 adapter action -> dispatch or native
output. The decoder owns incomplete input until it yields a complete frame or
explicitly consumes bytes in a bounded recovery result. Raw framing does not
require constructing a JSON object or converting bytes to a string. Separate
framing errors from UTF-8, syntax, schema, and application errors.

A recoverable decode result carries a typed error and exact consumed count;
the session applies explicit adapter policy. Fatal errors close. Any recovery
scan belongs to the decoder, has an absolute byte/attempt bound, and must
consume bytes or terminate on each iteration. Magitek's enforced profile closes
on framing/size violations; it must not inherit the existing drop-and-continue
default, or silently downgrade to unchecked byte forwarding.

## Primitive behaviour

A length-field decoder is configured with field offset and width, byte order,
base adjustment, included/excluded field semantics, maximum wire size, and
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
metadata explicitly so it cannot default sequence or channel fields.

## Delimiter framing and byte preservation

Provide a bounded incremental delimiter primitive and a JSONL framing profile.
Its frame owns or shares the original bytes, records the payload span and exact
delimiter, and keeps the corresponding reservation. Inspection can borrow a
view and forward permitted bytes unchanged, including whitespace, field order,
escaping, and admitted extension data. Re-encoding a parsed object is not
byte-preserving forwarding. Native frame APIs need not impose this particular
representation on other codecs.

The JSONL profile emits LF. It may explicitly accept LF-only or LF/CRLF input;
CRLF acceptance is a declared profile choice, not an added ACP requirement.
The payload limit excludes the selected delimiter; reservations include its
one or two bytes. Handle a trailing CR and a split CRLF at the exact limit
without misclassifying it or resetting state. State dispositions for empty
lines, a bare CR, and a non-delimited EOF tail. The enforced ACP toy rejects
empty lines and incomplete tails. An escaped JSON newline is payload, not a
physical delimiter; literal embedded LF ends a candidate frame.

The same input bytes must receive the same acceptance or oversize decision
under every transport chunk partition. Check complete and partial frames before
retaining excess data, including coalesced frames and leftover tails. Reserve
receive capacity before polling a read, and cap the writable receive window;
a generic buffered reader that grows unchecked before `decode` is not enough.
Maintain an incremental scan cursor so repeated tiny reads do not rescan an
unbounded prefix. Apply scan/work quotas without losing progress. Start the
partial-frame deadline at the first byte and do not reset it for trickle input.

## Decoding, encoding, and retained storage

RFC 0007 supplies distinct limits for decoded bytes, structural nodes, nesting,
work admission, and temporary output. An external JSON adapter must reserve a
conservative decoding envelope before materialization or use a budget-aware
visitor. Parsing an unrestricted object tree and checking it afterwards does
not satisfy the bounded contract. Duplicate-key and unsupported-batch policy
belongs to the adapter; the ACP toy rejects both. Bound the number of adapter
actions even for protocols that allow one frame to contain several operations.

The encoder uses reserved bounded output storage, including escaping and
framing overhead. A known conservative bound or incremental bounded sink may
establish capacity; an unrestricted temporary vector followed by a length check
may not. Keep already-encoded frames distinct from payloads requiring wrapping
so no path adds a second delimiter or length prefix. Retain output charges
through any transport buffering and partial write. RFC 0004 determines the
publication evidence and termination after a possible partial transmission.

Charge actual retained backing capacity rather than only a visible byte-slice
length. Shared views keep one allocation reservation alive until its last
accounted owner releases it; copies and copy-on-write growth require new
capacity first. Metadata/view populations still need item bounds. Releasing a
frame from a queue does not release storage retained by a parser or consumer.
Document conservative overcharges and allocations outside the contract.

## Staging and decisions

Implement checked length and delimiter primitives with split/coalescing
properties, then demonstrate mixed text/binary composition and block validation
with RFC 0008 toys. Add JSONL fixtures alongside the primitive rather than
waiting for a complete ACP client. Decide whether variable-width integers belong
in the primitive set only after multiple examples need them. Keep callback
allocation and accounting obligations explicit.

Existing Tokio codec support remains usable behind an audited adapter. This
pre-1.0 proposal permits replacement of all `FrameCodec` methods, including
`decoder`, `encoder`, `frame_payload`, `wrap_payload`, `max_frame_length`,
`frame_payload_bytes`, and `correlation_id`, and their trait bounds. Do not
require one symmetric frame type, contiguous payload, or numeric correlation.
Existing implementations in `src/codec/examples.rs` and
`src/test_helpers/frame_codec.rs`, client framing, and envelope/serializer
bridges must migrate with public compile-time and behavioural tests. ADR 004
remains historical input; update its status when this contract is approved.
