# Wire protocol gap analysis survey

## Status and scope

Surveyed on 2026-11-01 against Wireframe commit
`f346af31b9e654fc1de88d1af5cd34b3c6e35427`, reconciled after rebasing onto
`ed59d71b4869043f6a55e9502d6f31f557d994b4`. This is an architectural audit and
set of proposed extension RFCs, not an interoperability certification or an
implementation commitment. Findings below distinguish source observations from
proposed designs. No protocol server was built or exercised for this survey.
Signal-like cryptographic and application APIs are explicitly out of scope. All
other listed protocols remain in scope for framework-gap closure and toy-server
validation. Production protocol implementations must use Wireframe primitives
and protocol-neutral dependencies; existing implementations may appear in test
code only when their licences permit.

The target is an external adapter that uses supported public APIs, preserves
the peer's wire format, and can reuse Wireframe's routing and runtime services.
Writing a separate server which happens to depend on Wireframe does not meet
that target. Conversely, an absent bundled codec is not itself a blocker.

Wireframe is pre-1.0. This survey imposes no backwards-compatibility
requirement on earlier Wireframe APIs, defaults or wire formats. Its primary
target is a framework and primitives for implementing existing or new
frame-based wire protocols. A compulsory Wireframe envelope or handshake would
obstruct that target. Breaking API changes, replacement traits and removal of
protocol-shaped defaults are acceptable; compatibility with each selected peer
protocol still matters.

Scope resolves the protocol names as follows:

- **WebSocket:** RFC 6455 over TCP, including HTTP upgrade and control frames;
  TLS termination may be external. HTTP/2 extended CONNECT is a separate
  profile.
- **OSCAR:** classic AIM/ICQ FLAP transport and SNAC messaging, with client
  compatibility profiles chosen by the external server.
- **AMQP:** evaluate both Advanced Message Queuing Protocol 0-9-1 and 1.0.
  They are different protocols, not interchangeable codec configurations.
- **RTSP:** Real Time Streaming Protocol, covering 1.0 and 2.0 with
  TCP-interleaved media.
- **IEC 60870-5 over TCP/IP:** primarily IEC 60870-5-104. Serial 101 framing
  tunnelled over TCP is a distinct gateway profile, not equivalent to 104.
- **DNP3:** TCP transport for IEEE Std 1815-2010. Later editions and security
  profiles must not silently substitute for that requested baseline.
- **Modbus:** Modbus TCP. RTU-over-TCP, serial timing and Modbus Security need
  separate, explicit profiles.
- **Apache Kafka:** the broker-facing TCP protocol, with explicitly selected
  API versions and security profiles; not an implementation of broker storage.
- **MySQL/MariaDB:** classic client/server protocols, including negotiation,
  command/result exchanges and native packet splitting; not MySQL X Protocol.
- **PostgreSQL:** frontend/backend protocol 3.x, including startup, extended
  query, COPY and cancellation; replication requires a separately scoped
  profile.
- **OpenRA:** the requested game-network implementation on `bleed`, pinned
  below; mod compatibility is part of the external adapter's selected profile.
- **Quake 3 Arena:** native UDP game traffic, including connectionless and
  connected packets; no TCP substitution is assumed.

## Outcome

Wireframe has useful framing and message abstractions, but its prepared
application driver is not yet a general duplex protocol runtime. The dominant
blockers are independent receive/send progress, explicit session events,
protocol-native metadata, and negotiated session setup. Routing messages from
already-parsed library streams is a lower-priority integration convenience.
Adding bespoke protocol state machines would leave those gaps intact.

Modbus TCP request/response serving has no identified fundamental public-API
blocker. OSCAR packet parsing also fits the available codec surface, but a full
messaging server needs unsolicited output and connection lifecycle control.
WebSocket, RTSP streaming, AMQP, IEC 104 and DNP3 need a coherent external
session-adapter boundary to reuse the prepared application without building a
parallel runtime around it. Kafka adds ordered, version-aware exchanges;
MySQL/MariaDB and PostgreSQL expose phase transitions, transport upgrades and
multi-message replies. OpenRA needs cross-connection order relay. Quake 3 adds
a hard datagram-transport gap. The recommended extensions therefore target
native messages and transport semantics, not compatibility wrappers around
Wireframe's current envelope.

“Blocked” below means blocked under that integration criterion. An external
server can already own its transport and state machine and use selected
Wireframe components; this is a possible workaround, not evidence that the
prepared application supplies the missing capability.

## Current implementation evidence

These links identify the inspected implementation. Design documents explain
intent but do not override the current call paths.

| Area                                                                                           | Available capability                                                                                                                        | Boundary or gap                                                                                                                                                                          |
| ---------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| [Frame codecs](../src/codec/mod.rs)                                                            | `FrameCodec` supplies native frame types, independent Tokio decoder/encoder halves, payload extraction, correlation and size configuration. | `wrap_payload` receives bytes alone; there is no general request-context-to-outbound-frame mapping.                                                                                      |
| [Serialization](../src/serializer.rs) and [message traits](../src/message.rs)                  | Local serializer types can implement `EncodeWith<S>` and `DecodeWith<S>` for `Envelope`; bincode is not compulsory on the wire.             | The prepared application specifically requires `Envelope: DecodeWith<S> + EncodeWith<S>` and `FrameMetadata<Frame = Envelope>`.                                                          |
| [Inbound driver](../src/app/inbound_handler/core.rs)                                           | Per-connection codec clone, metadata parsing, routing, bounded assembly and optional fragmentation.                                         | `process_stream` awaits `handle_frame`, which awaits handler execution and output; it does not concurrently service the next input frame.                                                |
| [Envelope decode](../src/app/frame_handling/decode.rs)                                         | Codec correlation is used when the parsed envelope has no correlation ID.                                                                   | Other native frame metadata is not automatically propagated to handlers.                                                                                                                 |
| [Handler response](../src/app/frame_handling/response.rs)                                      | Response bytes inherit routing and correlation through a new envelope.                                                                      | `ServiceRequest` receives payload and correlation; a response reuses the inbound route ID. Native channels, addresses and control kinds need an explicit mapping.                        |
| [Outbound pipeline](../src/app/codec_driver.rs)                                                | Responses pass through fragmentation, serialization and codec wrapping.                                                                     | `FramePipeline` is private and has no public native-frame/control-frame enqueue operation.                                                                                               |
| [Prepared application](../src/app/prepared_app.rs)                                             | `handle_connection_result` accepts generic asynchronous byte streams.                                                                       | An already framed `Stream`/`Sink`, such as a WebSocket, is not this byte-stream interface.                                                                                               |
| [Built-in server](../src/server/mod.rs) and [preamble](../src/preamble.rs)                     | TCP listener lifecycle and optional bincode preamble.                                                                                       | A bincode preamble is not an HTTP upgrade or a general negotiated transport factory.                                                                                                     |
| [Protocol configuration](../src/app/builder/protocol.rs) and [hooks](../src/hooks.rs)          | `with_protocol` stores hooks; setup, send, completion and EOF callbacks exist.                                                              | The prepared-app loop does not invoke the separate actor's hooks. The builder restricts `ProtocolError` to `()`.                                                                         |
| [Connection actor](../src/connection/mod.rs) and [push queues](../src/push/queues/mod.rs)      | Priority queues, fairness, cancellation and response-stream processing exist.                                                               | `ConnectionActor::run` appends frames to a supplied `Vec`; it is not a transport-writing duplex driver integrated with `process_stream`.                                                 |
| [Assembly](../src/message_assembler/mod.rs) and [fragment adapter](../src/fragment/adapter.rs) | Public header interpretation and reusable assembly/fragmentation abstractions.                                                              | The app's [fragmentation state](../src/app/fragmentation_state.rs) selects its concrete adapter; these abstractions do not imply arbitrary native fragmentation is installed end to end. |
| [Codec errors](../src/codec/error.rs) and [recovery hooks](../src/codec/recovery/hook.rs)      | Structured framing/protocol/EOF errors and recovery policy types exist.                                                                     | `process_stream` returns immediately on a decoder error; the presence of policy types does not establish recover-and-reply behaviour on this path.                                       |

_Table 1: Source-level capabilities and integration boundaries._

An external codec can retain headers inside its payload representation, or a
local serializer can map native packets to and from `Envelope` using public
accessors. These are genuine escape routes. They become awkward when reply
metadata differs from request metadata, several messages share a connection, or
control events must bypass application dispatch. Shared mutable state between
codec halves can also work, but the framework should not require an
undocumented side channel to coordinate every session.

The prepared handler has no first-class successful no-response or multi-message
outcome: even an empty `ServiceResponse` becomes a wire response. Returning an
error suppresses output but records application failure. The separate actor's
`Response` variants do not change this prepared-driver contract.

The current 16 MiB constant is not a universal custom-codec ceiling. It caps
read-buffer reservation and informs default budgets; custom decoders still must
enforce their own declared-length limits. Explicit memory budgets and native
logical-message limits need separate consideration. No native datagram entry
point was found: the prepared runtime requires a byte stream.

The older [codec ADR](adrs/adr-0004-pluggable-protocol-codecs.md) records
limitations and subsequent resolutions. Its historical notes must not be read
as a current claim that serializers, error types or fragment adapters are
absent. Likewise, comments mentioning hooks in the outbound pipeline are not
proof of hook execution: the inspected `FramePipeline::process` fragments and
counts frames.

## Protocol findings

### WebSocket

RFC 6455 requires HTTP upgrade, asymmetric masking rules, variable-length
frames, continuation messages, UTF-8 validation for text, ping/pong, and a
close handshake. Control frames can occur between message fragments.[^1] A
length-prefixed binary payload alone is insufficient.

A custom `FrameCodec` can implement byte parsing and validation. A caller can
also perform an external handshake and pass a compatible byte stream to
`PreparedApp`. However, a mature WebSocket implementation normally exposes
messages through a stream/sink, and flattening that interface loses message
kinds and boundaries unless another adapter recreates them. A slow handler in
the prepared loop can delay reading a ping or close request.

**Verdict:** framing is extensible; seamless use of an existing WebSocket
engine and timely control handling are integration blockers. A
`tokio-tungstenite` bridge is a lower-priority option.[^2] RFCs A–E address the
underlying generic needs. Compression remains an adapter concern with explicit
decompression limits, not an implicit feature of a generic codec.

### OSCAR

OSCAR layers SNAC messages over FLAP records on TCP. The packet hierarchy,
sequence tracking, service negotiation and server-originated messages are more
than a single request/response exchange. Open OSCAR Server provides a concrete
compatibility reference for classic clients.[^3]

FLAP's fixed header and length field suit a configurable header-length codec;
SNAC family/subtype can map to a route selector, with request IDs retained as
correlation. Protocol version, channel, sequence and service state must remain
available to the external adapter. Login redirects and messaging services also
span connections; codec-local state cannot own the whole service.

**Verdict:** no fundamental packet-decoding blocker; full service integration
is blocked by the lack of an integrated unsolicited-send/session path. RFCs A
and B provide that path; D reduces parsing boilerplate. Buddy lists, presence,
authentication, rate classes and historical-client quirks belong outside core.

### AMQP 0-9-1 and 1.0

AMQP 0-9-1 multiplexes channels and distinguishes method, content-header,
content-body and heartbeat frames. AMQP 1.0 has a different frame layout and
connection/session/link model, including flow control and delivery settlement.
Both require protocol negotiation and autonomous control traffic.[^4][^5]

Length/header codecs can cover their outer framing, but channel IDs are not
request correlation IDs, and link credit is not equivalent to socket
backpressure. A local queue becoming writable must not authorize transmission
when protocol credit is exhausted. Content and transfer fragments need native
assembly rules; Wireframe fragmentation headers must never be inserted into
AMQP traffic.

**Verdict:** blocked for a reusable complete endpoint by native event mapping,
concurrent control handling and channel-aware scheduling. RFCs A, B and E are
central; C handles negotiated transport setup and D shares frame utilities.
Broker exchanges, queues, routing, persistence and settlement policy remain
external. `fe2o3-amqp` is an AMQP 1.0 Rust implementation to evaluate; RabbitMQ
is a useful 0-9-1 interoperability peer. Neither should be represented as a
generic interchangeable AMQP codec.[^6]

### RTSP with streaming

RTSP controls media sessions; it is not the media encoding itself. For
TCP-interleaved delivery, textual RTSP messages and binary media records share
a connection. The binary record uses a dollar marker, channel identifier and
16-bit length. Real-time Transport Protocol (RTP) and RTP Control Protocol
(RTCP) have their own packet semantics. RTSP 1.0 and 2.0 require separate
compatibility decisions.[^7]

A discriminated codec can parse text headers, bounded `Content-Length` bodies
and interleaved records. `PLAY` must not turn the connection into a response
loop that prevents processing `PAUSE`, `TEARDOWN` or RTCP. A stream of
Wireframe envelopes, or its stream terminator convention, is not RTSP
interleaving.

**Verdict:** streaming integration is blocked by independent read/write
progress, native channel metadata and session-owned output. RFCs A–C and E
address these; D supplies bounded text/binary framing composition. Media
pacing, timestamps, codecs, session descriptions and UDP socket management stay
in the external media server. `gortsplib` and MediaMTX are useful reference
implementations and interoperability peers, not Rust codec dependencies.[^8]

### IEC 60870-5-104

IEC 104 carries application service data units (ASDUs) over a TCP-specific
application protocol control information (APCI) layer. Information, supervisory
and unnumbered frames have different roles. Start/stop data transfer, test
frames, send/receive sequence tracking, acknowledgement windows and timers must
progress even without an application request. The `lib60870` guide provides a
public implementation reference, including asynchronous server messages and
configurable connection parameters.[^9]

The start marker and short length header fit the codec surface. The difficult
part is maintaining the connection's transfer state, acknowledgements and
spontaneous events. A TCP acknowledgement is not an IEC acknowledgement, and a
read timeout which only purges assembly state does not implement the protocol's
timers. Serial 101 tunnelling must preserve its own link semantics separately.

**Verdict:** framing is feasible; an integrated timed duplex session is the
blocker. RFCs A, B and E permit an external IEC adapter without embedding
telecontrol state machines in Wireframe. ASDU interpretation, command
activation/termination, interrogation, clock synchronization and redundancy
policy remain adapter/application responsibilities. IEC conformance and any IEC
62351 security profile require their own acceptance criteria.

### DNP3 over TCP/IP

DNP3 retains link framing over TCP, with addressing and cyclic redundancy
checks (CRCs); the CRC layout includes checks across successive data blocks. It
also has transport segmentation and application fragments, confirmations and
unsolicited responses. OpenDNP3's link implementation demonstrates why ordinary
length-prefix framing cannot simply strip one fixed trailer.[^10]

A custom decoder can validate and remove block CRCs, and a custom encoder can
restore them. Generic Wireframe fragment indices are not DNP3 sequence fields.
Native transport and application reassembly must preserve their separate
sequence spaces, duplicate handling and confirmation rules. A response-only
loop cannot provide timely independent confirmations and unsolicited events.

**Verdict:** no inherent byte-codec barrier, but session scheduling and native
segmentation integration block the intended reusable endpoint. RFCs A, B, D and
E address framework gaps. Object groups/variations, event classes,
select/operate semantics and device profiles stay external. Validation must
explicitly target IEEE Std 1815-2010; this audit did not inspect the complete
licensed standard and makes no conformance claim for that edition.

OpenDNP3 is Apache-licensed but archived. Step Function's Rust DNP3 project is
source-available under non-commercial/non-production terms and explicitly says
it is not open source. Treat it as a separately licensed candidate, not an
assumed reusable open-source component.[^11]

### Modbus TCP

Modbus TCP uses a seven-byte Modbus application protocol (MBAP) header. Its
length field counts following bytes, including the unit identifier; transaction
IDs must be copied into responses. The normal application data unit has a
260-byte maximum. Modbus TCP does not add the serial RTU CRC.[^12]

A custom codec can retain the transaction/unit identifiers, and a local
serializer can map function codes and native request/response bodies through
`Envelope`. Unit addressing must survive dispatch; storing only a transaction
ID is insufficient. Exception responses are protocol output, not a reason to
return a generic handler error that the driver logs and drops.

**Verdict:** no identified hard blocker for an ordinary bounded TCP server.
Native metadata mapping is awkward but possible through public APIs. RFC A
makes that mapping explicit and D removes length-header boilerplate. RFC B
becomes useful when independent concurrent transactions or slow device access
matter. A licensed independent client can validate the toy server.[^13]
Function semantics, register maps, device access and gateway ordering remain
external.

### Apache Kafka

Kafka uses TCP with a signed 32-bit big-endian size prefix excluding that
prefix. Requests carry an API key, API version and correlation ID; responses
need the corresponding request's version context. Flexible versions add compact
encodings and tagged fields. `ApiVersions`, authentication and the selected
listener security profile determine the supported exchange.[^14]

Ordinary bounded request/response APIs fit custom framing and serialization.
Kafka's connection ordering must be preserved even if handlers execute
asynchronously; its correlation ID does not authorize arbitrary response
reordering. `Produce` with `acks=0` requires no response. Long-poll fetches,
large record batches and compressed records require bounded waiting, output and
decompression. A codec must not confuse batch CRCs with transport framing.

**Verdict:** no fundamental framing blocker for a limited broker-compatible
endpoint. A full adapter needs explicit successful completion without a reply,
version-aware reply context and ordered streaming output; hiding those in an
encoder or returning a handler error is an integration workaround. RFCs A, B, D
and E supply the generic contracts, with C for authentication/transport setup.
Broker storage, replication, group coordination and transactions remain
external. The `kafka-protocol` crate provides generated, version-aware message
codecs, not a broker; pin its supported schema set and use real Kafka clients
as independent peers.[^15]

### MySQL and MariaDB

The classic protocol has a three-byte little-endian payload length and a
one-byte packet sequence. Payloads of `0xffffff` bytes continue into another
packet; an exact multiple needs an empty terminating packet. Sequence numbers
advance within an exchange and reset for a new command. They are not request
correlation identifiers. Compression adds another framing layer.[^16]

The server speaks first, advertising capabilities and authentication data.
Negotiation can switch to TLS after `SSLRequest`, change authentication method
and enable compression. Command responses include ordered result metadata, rows
and capability-dependent terminators; prepared statements also use binary
encodings. Some commands, including `COM_STMT_CLOSE`, send no response. MySQL
and MariaDB share the basic packet format but have divergent capabilities and
authentication behaviour; select a supported product/version profile.[^17]

Wireframe already includes a feature-gated
[`MysqlFrameCodec` example](../src/codec/examples.rs). It parses individual
packet headers, but `wrap_payload` sets `sequence_id` to zero and
`correlation_id` exposes that sequence byte. It does not implement negotiated
sessions or multi-packet message termination. Reusing it unchanged is not a
MySQL server implementation.

**Verdict:** packet parsing is available; a full server adapter is blocked by
integrated server-first negotiation, shared exchange sequencing, transport
transitions and bounded multi-message output. External setup and a custom codec
can work around parts of this, but integration should not require
reimplementing the driver. RFCs A–E address these capabilities. SQL execution,
prepared statement storage and database semantics stay external. Test-only
clients may validate each profile when their licences permit; protocol
libraries are not production dependencies.[^18]

### PostgreSQL

The frontend/backend protocol has an untagged startup packet and, in normal
operation, a one-byte message type followed by a big-endian length including
the four-byte length field itself. Startup negotiates protocol and
authentication; TLS can use `SSLRequest` with a one-byte server response or a
supported direct-TLS profile. The PostgreSQL 18 documentation covers protocol
3.2 as well as the long-lived 3.0 baseline; pin the minor version rather than
assuming every PostgreSQL endpoint has the same startup fields.[^19]

Simple queries produce multiple messages. Extended queries separate Parse,
Bind, Execute and Sync, with ordered pipelining and an error-recovery rule that
discards input until Sync. COPY introduces inbound, outbound and bidirectional
streams; notifications can be unsolicited. Cancellation arrives on a separate
connection carrying session key data and receives no direct reply.[^20]

**Verdict:** native packet decoding is straightforward; integrated phase
transitions, output streams, recovery-to-Sync, cross-connection cancellation
and transport upgrades are the blockers. A local serializer is insufficient for
the whole session. RFCs A–C and E should expose these behaviours without
PostgreSQL-specific callbacks in core. SQL parsing, execution, transaction
semantics, type catalogues and replication payloads stay external.

`pgwire` is an especially relevant Rust server-side candidate: it exposes
startup/query APIs and supports COPY and cancellation. Its README separately
marks streaming/logical replication support incomplete; COPY support alone must
not be reported as full replication support. A `psql` smoke test should be
supplemented by an extended-query client and cancellation/COPY tests.[^21]

### OpenRA

This survey inspected the requested `OpenRA.Game/Network` code and the matching
server connection/dispatch code at `bleed` commit
`9636b63eaee62fcb60ef2e896e7985bdf2f1ee68`. This is a source-defined protocol
profile, not a promise of compatibility with every OpenRA release or mod. The
files are reference material for an external implementation; no OpenRA
contribution is proposed.[^22]

The game connection uses TCP. The server first writes handshake version and
client index. Thereafter client-to-server traffic has a length followed by
frame number and order bytes; server-to-client traffic additionally carries the
originating client index outside that length-counted payload. Frame zero
carries immediate orders. Handshake orders negotiate mod/version and identity;
subsequent traffic includes synchronization hashes, acknowledgements, pings and
tick-scale messages. The server relays orders between clients rather than
answering each packet with an independent RPC response.[^22]

**Verdict:** the asymmetric length/header layouts fit configurable codecs. The
blockers are server-first setup and integrated, bounded fan-out with
client/frame metadata and independent control progress. RFCs A–E should let an
external session coordinator submit native packets to multiple connections and
handle slow peers without blocking the whole session. An OpenRA game frame is a
simulation step, not a transport frame or correlation ID. Lockstep order
scheduling, lobby/mod validation, world-dependent order decoding, replay and
desynchronization policy stay outside Wireframe. The GPL-licensed OpenRA source
and a matching built client are references/peers, not Rust codec dependencies.

### Quake 3 Arena

Quake 3 uses UDP, not a TCP byte stream. Connectionless discovery/challenge
messages and connected sequenced messages share the socket. Client packets
carry a `qport` used with the address to recognize a session despite
source-port translation. The original netchan implementation adds native
fragment offsets and lengths, including a final empty fragment at an exact
fragment-size boundary. Datagram loss, duplication and reordering are expected
input conditions, not stream-decoder failures.[^23]

Reliable commands and replaceable snapshots have different delivery rules.
Payload interpretation uses bit-level fields, Huffman coding, delta baselines
and connection-dependent XOR obfuscation; that obfuscation is not encryption.
The supplied protocol guide explains these layers and is checked against the
released engine source for the transport and encoding boundaries.[^24]

**Verdict:** the current TCP/`AsyncRead + AsyncWrite` runtime is a hard
transport integration blocker. Putting UDP packets into a synthetic reliable
byte stream would destroy semantics; a stateless custom `FrameCodec` does not
fix that. RFC G adds a datagram boundary, with A, B and E providing session
events, deadlines and bounded output. D may offer bounded bit readers and
writers if several adapters need them, but Quake-specific Huffman tables, XOR
state, reliability rules and snapshot delta history remain in the external
adapter. Simulation, prediction and anti-cheat logic remain application
concerns.

Use the released engine or a deliberately selected ioquake3 protocol profile as
the independent peer. Forks can change protocol details, so pin the build and
packet corpus. No existing engine's GPL source should be mistaken for a
permissively licensed library to copy into Wireframe.

## Shared requirements from the extended survey

| Requirement                             | Protocol pressure                | Core responsibility                                          | External responsibility                                 |
| --------------------------------------- | -------------------------------- | ------------------------------------------------------------ | ------------------------------------------------------- |
| No, one or many replies                 | Kafka, SQL protocols, OpenRA     | Explicit completion and bounded native output                | Which replies the protocol permits                      |
| Phase and transport changes             | MySQL/MariaDB, PostgreSQL        | Buffer ownership, flush barriers and stream replacement      | Negotiation, authentication and state transitions       |
| Ordered replies with active reads       | Kafka, PostgreSQL, MySQL/MariaDB | Configurable ordering and cancellation                       | Command dependencies and transaction semantics          |
| Session coordination across connections | OpenRA, PostgreSQL cancellation  | Stable handles, lifecycle observation and bounded submission | Session registry, authorization and routing policy      |
| Datagram-native delivery                | Quake 3                          | Peer metadata, packet boundaries and bounded scheduling      | Challenges, reliability, sequencing and snapshot policy |

_Table 2: Additional requirements without protocol-specific core APIs._

## Recommended extension RFCs

These proposals map directly to
[RFC 0003](rfcs/rfc-0003-native-message-adapter-boundary.md) through
[RFC 0009](rfcs/rfc-0009-datagram-session-transport.md). Existing APIs and wire
formats may be replaced. Production dependencies must remain protocol-neutral;
protocol implementations may be used in test code only when licences permit.

Each proposal needs an external-crate acceptance example: no `pub(crate)`
access, copied driver, hidden global state, or protocol-name branches in core.

### A. Native frame and application-message adapter boundary

Introduce one typed adapter between transport frames, session events and
application requests. Its connection-local state should receive the complete
native frame and emit bounded actions such as dispatch, native send, wait or
close. Outbound application replies should carry the adapter's opaque typed
reply context, including channel/address/opcode data where needed.

Separate framing from optional body serialization, redesigning `FrameCodec`,
`Serializer` and `Packet` where needed. Remove mandatory `Envelope` conversion
from the runtime; an envelope-based example is optional and has no privileged
path. Do not force all protocols into `u32` routing plus `Option<u64>`
correlation, or require encoding metadata into application payload bytes. Keep
control events distinct from routed messages and allow zero, one or several
output frames per event. Represent successful no-response completion
explicitly, independently of errors. Do not require inbound and outbound
message types to be identical. Preserve negotiated version and reply ordering
context without encoding them into application payload bytes.

Acceptance: an external Modbus adapter preserves unit and transaction IDs; an
external mixed text/binary adapter preserves frame kind and channel through an
asynchronous reply. Both produce peer-native bytes without extra envelopes.

### B. One duplex connection driver with session events

Connect the existing actor's queue/fairness work to a real transport sink and
the prepared application's dispatch path. Keep reading while handlers or
streaming responses are pending. Give one owner responsibility for protocol
state, with events for inbound frames, application completions, queue
availability, deadlines, EOF and cancellation.

Expose bounded unsolicited sending and typed protocol errors. Define when a
send is accepted, encoded, flushed or failed; cancellation must not silently
replay a partially written frame. Support graceful protocol close with a
bounded drain deadline and exactly-once terminal notification. Public extension
events must execute on the documented path, with tests proving that fact.
Existing hook names and signatures need not survive the redesign.

Acceptance: a deliberately stalled handler cannot block a synthetic ping,
acknowledgement or close event; independent channels continue to progress.
Shutdown and write failure reclaim every task and queued frame. This is a
runtime contract, not a built-in WebSocket or IEC state machine.

Add adapter-selected serial or concurrent dispatch with ordered completion
where required. Multi-message replies must stream through a bounded sink,
without collecting an entire result set. Public connection handles should let
an external coordinator submit output or cancellation across connections; core
does not own a global game-session or database-query registry. Acceptance must
cover ordered database replies, cancellation during a long operation, and a
slow fan-out recipient that cannot stall unrelated peers.

### C. Negotiated and already framed transport entry points

Add an asynchronous setup boundary that can return an established byte stream
or a typed frame stream/sink plus connection metadata. Preserve unread bytes
across handshake completion, enforce handshake size/time limits, and make
ownership of TLS, peer identity and cancellation explicit.

Provide generic byte-stream and native framed entry points, without preserving
the current signatures. An external listener should integrate without losing
lifecycle observation; a TCP listener should accept a setup adapter without
protocol-specific branches. Do not flatten WebSocket messages into an
undocumented byte stream.

Support server-first and client-first setup, bounded multi-round negotiation,
and transport replacement after a protocol flush barrier. Ownership transfer
must include buffered input/output and explicit rules for bytes at the upgrade
boundary; never replay plaintext as encrypted application input. A typed
session enum can change parsing mode without runtime trait-object sprawl.

Acceptance: a native external WebSocket toy server passes raw-wire fixtures; a
`tokio-tungstenite` bridge is an optional comparison. MySQL-style greeting/TLS
upgrade and PostgreSQL-style startup/tagged-message transitions use the same
setup contract. Setup rejection, timeout and shutdown leave no detached task.
Coordinate with existing
[RFC 0001](rfcs/rfc-0001-protocol-agnostic-test-harness-lifecycle.md) for
testing.

### D. Configurable framing primitives with bounded composition

Offer a small collection of reusable building blocks, preferably wrapping
existing Tokio codec facilities where sufficient:

- Fixed headers with length-field offset, width, endianness, checked length
  adjustment, retained metadata and separate payload/wire limits, including
  three-byte integers and different layouts for each direction.
- Bounded delimiter/header-body parsing and discriminated alternatives, for
  mixed textual messages and binary records.
- Optional pure validation/transformation callbacks for magic values, trailers,
  checksums, byte escaping or block layouts that cannot be expressed by one
  length-field configuration.
- Bounded bit-field access when independently useful. Stateful compression,
  authentication and historical delta transforms belong behind the adapter
  boundary, not in an expanding set of codec flags.

Specify overflow behaviour, incomplete-input retention, EOF classification,
allocation limits and progress guarantees. Resynchronization must be opt-in and
bounded; a decoder must never repeatedly reject the same bytes forever.
Masking, compression and protocol semantic validation remain external when they
exceed these primitives. Avoid a universal framing configuration language.

Acceptance: independent fixtures cover MBAP lengths, FLAP-style headers,
RTSP-style mixed records, PostgreSQL length accounting, MySQL continuation
boundaries and block-CRC framing. Property tests vary every TCP split point,
concatenation, malformed length and truncation boundary. Keep fixtures in
tests; no complete protocol session implementation is required.

### E. Channel-aware budgets, scheduling and assembly policies

Unify queues, memory budgets and assembly into one coherent subsystem, reusing
existing implementations only where their contracts fit. Allow adapter-defined
logical stream keys, bounded per-stream queues and readiness decisions.
Distinguish application credit, local memory pressure and transport
writability. Reserve bounded capacity for control traffic while limiting
control floods and preserving data fairness.

Expose native segmentation policy and completion/abort events. The adapter owns
sequence rollover, duplicate handling, confirmations and credit changes; core
owns resource accounting and scheduling. Bound decoded bytes, decompressed
bytes, incomplete messages, stream count and queued output. Separate
wire-frame, logical-message and queued-byte limits; do not hard-code the
current 16 MiB bincode policy as every protocol's maximum. Configuration
remains bounded and must reject oversized declarations before allocation.

RFC 0007 extends proposed ADR 002's protocol-neutral streaming, layered
assembly, continuity, and bounds goals. It supersedes ADR 002's mandatory
additive compatibility and legacy defaults for this pre-1.0 redesign; ADR 002
remains proposed until a later decision records adoption.

Permit an external policy to replace stale unsent snapshots while preserving
reliable commands. A shared scheduler must not impose retransmission or
latest-value replacement on protocols that require ordered delivery.

Acceptance: a zero-credit channel does not block another channel or control
traffic; incomplete-message floods stay within budgets; closing one stream
cleans only its resources. Native segmentation and completion add no
Wireframe-specific headers or terminators. A slow database reader bounds row
buffers; a game snapshot queue drops superseded unsent state only when the
adapter explicitly permits it.

### F. External-adapter conformance harness

Extend the existing test harness and RFC 0001 with public, protocol-neutral
raw-byte, framed-peer and datagram connectors, deterministic time and
controllable short reads/writes, packet loss, duplication and reordering. Test
protocol hooks through the same driver used in production. Add byte-level
golden fixtures and an independent peer, not just codec round-trips that can
share the same mistake.

Acceptance suites should cover malformed input, slow consumers, cancellation,
EOF in every framing phase, interleaved control/data and resource exhaustion.
Use external protocol adapters as development dependencies or separate
repositories. A test harness may model generic transitions; core must not own
OSCAR login, AMQP link, RTSP playback, database sessions, game sessions or
telecontrol state machines. Include no-response operations, multi-packet result
sets, phase changes, fragmented UDP and independent client interoperability.

### G. Datagram transports and externally owned sessions

Expose complete datagrams with source/destination metadata and bounded receive
and send queues. Keep datagram truncation distinct from stream EOF, and allow
connectionless traffic without allocating a persistent session. An external
adapter should select a session key after protocol validation; a UDP source
port alone cannot represent every protocol's identity.

Provide timer and expiry events, cancellation, packet-size limits and
observable send failures through the same session runtime concepts as TCP. Do
not promise reliability or ordering that UDP does not supply, and do not
silently split oversized output. The adapter decides native fragmentation,
duplicate rejection, acknowledgements, retransmission and address-update
policy. Bound unvalidated peer state and allow admission/rate policies before
sending amplified replies.

Acceptance: an external Quake-style adapter handles connectionless challenge
traffic, translated source ports, lost/duplicate/out-of-order datagrams and
native fragmented messages. A second stateless datagram adapter uses the same
entry point without inheriting Quake fields, reliability or session machinery.
A transport shim that turns packets into a synthetic TCP stream fails this gate.

## Sequencing and decision gates

Start with the native adapter, framing, and test harness using Hotline and
Modbus toy servers to validate mxd-first needs and generic reuse. Integrate the
duplex runtime and bounded assembly before negotiated database protocols. Add
timed telecontrol, streaming, fan-out, and datagrams as subsequent vertical
slices. Add routing for pre-parsed library frames last; it is lower priority
than native protocol support. RFC 0008 establishes the test harness alongside
the first adapter and expands its matrix as each toy server lands.

Use database phase transitions and OpenRA-style fan-out to challenge the
session API. Prefer replacing the current envelope driver over preserving it
through compatibility layers. Migration notes may explain breaking releases,
but compatibility shims are not acceptance criteria.

Before accepting the extension surface, require two unrelated external adapters
to use the same APIs: for example, WebSocket plus IEC-style timed control, or
RTSP interleaving plus AMQP-style credit. Include at least one database session
and one datagram adapter before declaring the framework surface sufficient.
Reject any proposal whose second adapter requires a protocol-specific method on
the core driver.

The existing
[transport-boundary ADR](adrs/adr-0010-transport-frame-boundary-for-the-zero-copy-migration.md)
and
[runtime ownership ADR](adrs/adr-0011-runtime-ownership-and-task-lifetime-boundaries.md)
are historical design inputs, not requirements to retain earlier public
contracts. Supersede them explicitly where the protocol-first design differs.
These recommendations make no borrow-checker-dependent API promise; the pinned
nightly alone does not establish a verified borrow-checker posture for a future
implementation.

## Rebase reconciliation

The updated baseline adds lint cleanup, public `const fn` qualifications and
compile-time API fixtures, plus regenerated spelling configuration. Inspection
of the codec and connection changes found no change to the runtime boundaries
or protocol verdicts above. Retain those findings and inherit the updated
spelling configuration unchanged.

Future extension RFC implementations should follow the
[developer guide's const-API guidance](developers-guide.md#the-const-api-surface):
extend the existing fixture for each public API family when adding a
`const fn`, rather than introducing a parallel fixture. Public queue adapters
should use the callable `Send + 'static` bounds described in the
[users' guide](users-guide.md#compile-time-queue-and-session-configuration),
without naming the crate-visible `FrameLike` trait. This guidance applies while
using those APIs; it does not require retaining them in a breaking redesign and
creates no additional protocol blocker.

## Evidence limits and follow-up

The repository audit traced concrete public traits and the prepared-app and
actor paths using CodeGraph/Leta navigation and source inspection. Web research
used Firecrawl against standards publishers and upstream projects. Tooling
status is a dated observation, not a dependency-selection decision.

Remaining implementation research is explicit: select exact protocol profiles,
pin interoperating peer revisions, review candidate licences and maintenance,
and obtain the normative IEC/DNP3 material needed for conformance. Verify
security profiles separately. The survey establishes framework gaps and
candidate experiments; it does not claim production readiness or measured
latency, throughput or memory performance.

## Consumer evidence

At pinned `mxd` commit `e0dbf2b8289f3b296d8b2825ba533b31ca82363b`, its
[frame codec](https://github.com/leynos/mxd/blob/e0dbf2b8289f3b296d8b2825ba533b31ca82363b/src/wireframe/codec/frame.rs)
parses native frames, then translates them into a bincode `Envelope`. Its
[message assembly](https://github.com/leynos/mxd/blob/e0dbf2b8289f3b296d8b2825ba533b31ca82363b/src/wireframe/message_assembly.rs)
uses `InboundSeriesTracker` for limits and timeouts. The
[handshake](https://github.com/leynos/mxd/blob/e0dbf2b8289f3b296d8b2825ba533b31ca82363b/src/wireframe/handshake.rs)
replies on `TcpStream`. It already has registry-backed push and broadcast.
Priorities are native message integration without compulsory envelopes,
reusable bounded assembly, negotiated session progress, and independent
receive/send progress. Hotline does not define the framework's sole purpose.

## Sources

[^1]: [RFC 6455, especially sections 4–7](https://www.rfc-editor.org/rfc/rfc6455).
[^2]: [Tokio Tungstenite](https://github.com/snapview/tokio-tungstenite).
[^3]: [Open OSCAR Server](https://github.com/mk6i/open-oscar-server) and
    [AIM OSCAR Server's FLAP/SNAC overview](https://github.com/ox/aim-oscar-server).
[^4]: [AMQP 0-9-1 specification](https://www.rabbitmq.com/resources/specs/amqp0-9-1.pdf)
    and [RabbitMQ heartbeat documentation](https://www.rabbitmq.com/docs/heartbeats).
[^5]: [OASIS AMQP 1.0 transport specification](https://docs.oasis-open.org/amqp/core/v1.0/os/amqp-core-transport-v1.0-os.html).
[^6]: [fe2o3-amqp](https://github.com/minghuaw/fe2o3-amqp).
[^7]: [RTSP 1.0, RFC 2326](https://www.rfc-editor.org/rfc/rfc2326) and
    [RTSP 2.0, RFC 7826](https://www.rfc-editor.org/rfc/rfc7826.html), especially
    the embedded/interleaved binary data sections.
[^8]: [gortsplib](https://github.com/bluenviron/gortsplib), developed for
    [MediaMTX](https://github.com/bluenviron/mediamtx).
[^9]: [lib60870 user guide](https://github.com/mz-automation/lib60870/blob/master/user_guide.adoc)
    and [project and licensing information](https://github.com/mz-automation/lib60870).
[^10]: [OpenDNP3 link frame implementation](https://github.com/dnp3/opendnp3/blob/release/cpp/lib/src/link/LinkFrame.cpp)
    and [link header](https://github.com/dnp3/opendnp3/blob/release/cpp/lib/src/link/LinkHeader.h).
[^11]: [OpenDNP3 status and licence](https://github.com/dnp3/opendnp3) and
    [Step Function DNP3 status and licensing](https://github.com/stepfunc/dnp3).
[^12]: [Modbus TCP implementation guide](https://www.modbus.org/file/secure/messagingimplementationguide.pdf)
    and [application protocol specification](https://www.modbus.org/file/secure/modbusprotocolspecification.pdf).
[^13]: [tokio-modbus and TCP-server feature](https://github.com/slowtec/tokio-modbus).

[^14]: [Apache Kafka protocol guide](https://kafka.apache.org/44/design/protocol/).
[^15]: [kafka-protocol generated Rust codecs](https://github.com/kafka-protocol-rs/kafka-protocol-rs).
[^16]: [MySQL basic packets](https://dev.mysql.com/doc/dev/mysql-server/latest/page_protocol_basic_packets.html)
    and [MariaDB packet format](https://mariadb.com/docs/server/reference/clientserver-protocol/0-packet).
[^17]: [MySQL connection phase](https://dev.mysql.com/doc/dev/mysql-server/latest/page_protocol_connection_phase.html),
    [MariaDB connection negotiation](https://mariadb.com/docs/server/reference/clientserver-protocol/1-connecting/connection)
    and [MySQL statement close](https://dev.mysql.com/doc/dev/mysql-server/9.7.1/page_protocol_com_stmt_close.html).
[^18]: [OpenSrv and its MySQL/MariaDB server implementation](https://github.com/databendlabs/opensrv).
[^19]: [PostgreSQL protocol overview](https://www.postgresql.org/docs/18/protocol-overview.html)
    and [message formats](https://www.postgresql.org/docs/18/protocol-message-formats.html).
[^20]: [PostgreSQL protocol flow](https://www.postgresql.org/docs/18/protocol-flow.html).
[^21]: [pgwire server implementation and support matrix](https://github.com/sunng87/pgwire).
[^22]: OpenRA commit `9636b63eaee62fcb60ef2e896e7985bdf2f1ee68`:
    [client connection](https://github.com/OpenRA/OpenRA/blob/9636b63eaee62fcb60ef2e896e7985bdf2f1ee68/OpenRA.Game/Network/Connection.cs),
    [order encoding](https://github.com/OpenRA/OpenRA/blob/9636b63eaee62fcb60ef2e896e7985bdf2f1ee68/OpenRA.Game/Network/OrderIO.cs),
    [handshake](https://github.com/OpenRA/OpenRA/blob/9636b63eaee62fcb60ef2e896e7985bdf2f1ee68/OpenRA.Game/Network/Handshake.cs),
    [server connection](https://github.com/OpenRA/OpenRA/blob/9636b63eaee62fcb60ef2e896e7985bdf2f1ee68/OpenRA.Game/Server/Connection.cs)
    and [server dispatch](https://github.com/OpenRA/OpenRA/blob/9636b63eaee62fcb60ef2e896e7985bdf2f1ee68/OpenRA.Game/Server/Server.cs).
[^23]: [Released Quake III netchan implementation](https://github.com/id-Software/Quake-III-Arena/blob/master/code/qcommon/net_chan.c).
[^24]: [Quake 3 protocol guide](https://www.jfedor.org/quake3/)
    and [released server netchan encoding](https://github.com/id-Software/Quake-III-Arena/blob/master/code/server/sv_net_chan.c).
