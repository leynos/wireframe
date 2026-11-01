# Wire protocol gap analysis survey

## Status and scope

Surveyed on 2026-11-01 against Wireframe commit
`f346af31b9e654fc1de88d1af5cd34b3c6e35427`, reconciled after rebasing onto
`ed59d71b4869043f6a55e9502d6f31f557d994b4`. This is an architectural audit and
set of proposed extension RFCs, not an interoperability certification or an
implementation commitment. Findings below distinguish source observations from
proposed designs. No protocol server was built or exercised for this survey.

The target is an external adapter that uses supported public APIs, preserves
the peer's wire format, and can reuse Wireframe's routing and runtime services.
Writing a separate server which happens to depend on Wireframe does not meet
that target. Conversely, an absent bundled codec is not itself a blocker.

Scope resolves the protocol names as follows:

- **WebSocket:** RFC 6455 over TCP, including HTTP upgrade and control frames;
  TLS termination may be external. HTTP/2 extended CONNECT is a separate
  profile.
- **OSCAR:** classic AIM/ICQ FLAP transport and SNAC messaging, with client
  compatibility profiles chosen by the external server.
- **Signal:** distinguish the Signal cryptographic protocol from compatibility
  with Signal Messenger's service. Both are discussed; neither is a single TCP
  framing specification.
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

## Outcome

Wireframe has useful framing and message abstractions, but its prepared
application driver is not yet a general duplex protocol runtime. The dominant
blockers are independent receive/send progress, explicit session events,
protocol-native metadata, and integration with existing framed transports.
Adding seven bespoke protocol state machines would leave those gaps intact.

Modbus TCP request/response serving has no identified fundamental public-API
blocker. OSCAR packet parsing also fits the available codec surface, but a full
messaging server needs unsolicited output and connection lifecycle control.
WebSocket, RTSP streaming, AMQP, IEC 104 and DNP3 need a coherent external
session-adapter boundary to reuse the prepared application without building a
parallel runtime around it. Signal service compatibility has substantial
application requirements beyond any Wireframe extension.

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

The older [codec ADR](adr-004-pluggable-protocol-codecs.md) records limitations
and subsequent resolutions. Its historical notes must not be read as a current
claim that serializers, error types or fragment adapters are absent. Likewise,
comments mentioning hooks in the outbound pipeline are not proof of hook
execution: the inspected `FramePipeline::process` fragments and counts frames.

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
engine and timely control handling are integration blockers. Prefer an external
`tokio-tungstenite` adapter, preserving text/binary/control semantics, over
another WebSocket implementation in Wireframe.[^2] Proposed RFCs A, B and C
below address the boundary. Compression remains an adapter concern with
explicit decompression limits, not an implicit feature of a generic codec.

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

### Signal

`libsignal` implements cryptographic protocols, including the Double Ratchet;
it is not a drop-in Signal Messenger server or TCP codec.[^4] Carrying opaque
Signal ciphertext in an application-owned Wireframe protocol presents no
identified framing blocker, but does not make that service compatible with
Signal clients.

Signal Messenger service compatibility involves HTTP/WebSocket APIs,
authenticated devices, delivery queues, acknowledgements, account and key
services, and external persistence. Signal-Server and its WebSocket resources
are the implementation reference, rather than a frozen general-purpose wire
standard.[^5] Compatibility must pin a client/server revision and supported
endpoint set; this survey does not assert complete current API coverage.

**Verdict:** the WebSocket/runtime gaps apply to a compatible service adapter;
cryptographic correctness and service completeness are separate blockers that
Wireframe cannot solve. RFCs A–C would enable integration. Keep Signal schemas,
cryptography and service logic in external packages, with independent licence
and compatibility review. Do not terminate end-to-end message encryption in
Wireframe merely to route ciphertext.

### AMQP 0-9-1 and 1.0

AMQP 0-9-1 multiplexes channels and distinguishes method, content-header,
content-body and heartbeat frames. AMQP 1.0 has a different frame layout and
connection/session/link model, including flow control and delivery settlement.
Both require protocol negotiation and autonomous control traffic.[^6][^7]

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
generic interchangeable AMQP codec.[^8]

### RTSP with streaming

RTSP controls media sessions; it is not the media encoding itself. For
TCP-interleaved delivery, textual RTSP messages and binary media records share
a connection. The binary record uses a dollar marker, channel identifier and
16-bit length. Real-time Transport Protocol (RTP) and RTP Control Protocol
(RTCP) have their own packet semantics. RTSP 1.0 and 2.0 require separate
compatibility decisions.[^9]

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
implementations and interoperability peers, not Rust codec dependencies.[^10]

### IEC 60870-5-104

IEC 104 carries application service data units (ASDUs) over a TCP-specific
application protocol control information (APCI) layer. Information, supervisory
and unnumbered frames have different roles. Start/stop data transfer, test
frames, send/receive sequence tracking, acknowledgement windows and timers must
progress even without an application request. The `lib60870` guide provides a
public implementation reference, including asynchronous server messages and
configurable connection parameters.[^11]

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
length-prefix framing cannot simply strip one fixed trailer.[^12]

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
assumed reusable open-source component.[^13]

### Modbus TCP

Modbus TCP uses a seven-byte Modbus application protocol (MBAP) header. Its
length field counts following bytes, including the unit identifier; transaction
IDs must be copied into responses. The normal application data unit has a
260-byte maximum. Modbus TCP does not add the serial RTU CRC.[^14]

A custom codec can retain the transaction/unit identifiers, and a local
serializer can map function codes and native request/response bodies through
`Envelope`. Unit addressing must survive dispatch; storing only a transaction
ID is insufficient. Exception responses are protocol output, not a reason to
return a generic handler error that the driver logs and drops.

**Verdict:** no identified hard blocker for an ordinary bounded TCP server.
Native metadata mapping is awkward but possible through public APIs. RFC A
makes that mapping explicit and D removes length-header boilerplate. RFC B
becomes useful when independent concurrent transactions or slow device access
matter. `tokio-modbus` already supplies TCP-server support and is a useful
reuse candidate and independent test peer.[^15] Function semantics, register
maps, device access and gateway ordering remain external.

## Recommended extension RFCs

These are proposal labels, not allocated repository RFC numbers. They should
extend existing abstractions and preserve the default Wireframe protocol. Each
proposal needs an external-crate acceptance example: no `pub(crate)` access,
copied driver, hidden global state, or protocol-name branches in core.

### A. Native frame and application-message adapter boundary

Introduce one typed adapter between transport frames, session events and
application requests. Its connection-local state should receive the complete
native frame and emit bounded actions such as dispatch, native send, wait or
close. Outbound application replies should carry the adapter's opaque typed
reply context, including channel/address/opcode data where needed.

Preserve `FrameCodec` for framing and `Serializer` for body serialization.
Provide a default envelope adapter for compatibility. Do not force all
protocols into `u32` routing plus `Option<u64>` correlation, or require
encoding metadata into application payload bytes. Keep control events distinct
from routed messages and allow zero, one or several output frames per event.

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
bounded drain deadline and exactly-once terminal notification. Compatibility
hooks must execute on the documented path, with tests proving that fact.

Acceptance: a deliberately stalled handler cannot block a synthetic ping,
acknowledgement or close event; independent channels continue to progress.
Shutdown and write failure reclaim every task and queued frame. This is a
runtime contract, not a built-in WebSocket or IEC state machine.

### C. Negotiated and already framed transport entry points

Add an asynchronous setup boundary that can return an established byte stream
or a typed frame stream/sink plus connection metadata. Preserve unread bytes
across handshake completion, enforce handshake size/time limits, and make
ownership of TLS, peer identity and cancellation explicit.

Retain today's generic byte-stream entry point. An external listener should be
able to integrate without losing lifecycle observation; the built-in TCP server
should accept a setup adapter without acquiring protocol-specific branches. Do
not flatten WebSocket messages into an undocumented byte stream.

Acceptance: a `tokio-tungstenite` bridge preserves message kinds; coalesced
handshake and first-message input loses no bytes. Setup rejection, timeout and
shutdown leave no detached task. Coordinate with existing
[RFC 0001](rfcs/0001-protocol-agnostic-test-harness-lifecycle.md) for testing.

### D. Configurable framing primitives with bounded composition

Offer a small collection of reusable building blocks, preferably wrapping
existing Tokio codec facilities where sufficient:

- Fixed headers with length-field offset, width, endianness, checked length
  adjustment, retained metadata and separate payload/wire limits.
- Bounded delimiter/header-body parsing and discriminated alternatives, for
  mixed textual messages and binary records.
- Optional pure validation/transformation callbacks for magic values, trailers,
  checksums, byte escaping or block layouts that cannot be expressed by one
  length-field configuration.

Specify overflow behaviour, incomplete-input retention, EOF classification,
allocation limits and progress guarantees. Resynchronization must be opt-in and
bounded; a decoder must never repeatedly reject the same bytes forever.
Masking, compression and protocol semantic validation remain external when they
exceed these primitives. Avoid a universal framing configuration language.

Acceptance: independent fixtures cover MBAP lengths, FLAP-style headers,
RTSP-style mixed records and block-CRC framing. Property tests vary every TCP
split point, concatenation, malformed length and truncation boundary. Keep
fixtures in tests; no complete protocol session implementation is required.

### E. Channel-aware budgets, scheduling and assembly policies

Extend existing queues, memory budgets and assembly rather than adding a second
parallel subsystem. Allow adapter-defined logical stream keys, bounded
per-stream queues and readiness decisions. Distinguish application credit,
local memory pressure and transport writability. Reserve bounded capacity for
control traffic while limiting control floods and preserving data fairness.

Expose native segmentation policy and completion/abort events. The adapter owns
sequence rollover, duplicate handling, confirmations and credit changes; core
owns resource accounting and scheduling. Bound decoded bytes, decompressed
bytes, incomplete messages, stream count and queued output.

Acceptance: a zero-credit channel does not block another channel or control
traffic; incomplete-message floods stay within budgets; closing one stream
cleans only its resources. An external adapter can disable Wireframe-specific
fragment headers and terminators throughout the transport path.

### F. External-adapter conformance harness

Extend the existing test harness and RFC 0001 with public, protocol-neutral
raw-byte and framed-peer connectors, deterministic time and controllable short
reads/writes. Test protocol hooks through the same driver used in production.
Add byte-level golden fixtures and an independent peer, not just codec
round-trips that can share the same mistake.

Acceptance suites should cover malformed input, slow consumers, cancellation,
EOF in every framing phase, interleaved control/data and resource exhaustion.
Use external protocol adapters as development dependencies or separate
repositories. A test harness may model generic transitions; core must not own
OSCAR login, AMQP link, RTSP playback or telecontrol state machines.

## Sequencing and decision gates

Start with A and B, using Modbus as the small native-mapping experiment and a
synthetic duplex protocol as the scheduling experiment. Add C with an existing
WebSocket engine. Develop D only where those experiments demonstrate repeated
framing code; a new helper is not required merely because a protocol exists.
Build E against a multiplexed credit-controlled experiment. F accompanies every
step and must not wait until the abstractions are frozen.

Before accepting the extension surface, require two unrelated external adapters
to use the same APIs: for example, WebSocket plus IEC-style timed control, or
RTSP interleaving plus AMQP-style credit. Reject any proposal whose second
adapter requires a protocol-specific method on the core driver.

The existing
[transport-boundary ADR](adr-010-transport-frame-boundary-for-zero-copy.md) and
[runtime ownership ADR](adr-011-runtime-ownership-and-task-lifetime-boundaries.md)
should inform the RFCs. These recommendations make no borrow-checker-dependent
API promise; the pinned nightly alone does not establish a verified
borrow-checker posture for a future implementation.

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
without naming the crate-visible `FrameLike` trait. These are implementation
constraints for the proposals, not additional protocol blockers.

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

## Sources

[^1]: [RFC 6455, especially sections 4–7](https://www.rfc-editor.org/rfc/rfc6455).
[^2]: [Tokio Tungstenite](https://github.com/snapview/tokio-tungstenite).
[^3]: [Open OSCAR Server](https://github.com/mk6i/open-oscar-server) and
    [AIM OSCAR Server's FLAP/SNAC overview](https://github.com/ox/aim-oscar-server).
[^4]: [Signal's libsignal](https://github.com/signalapp/libsignal).
[^5]: [Signal-Server](https://github.com/signalapp/Signal-Server) and its
    [WebSocket resources](https://github.com/signalapp/Signal-Server/tree/main/websocket-resources).
[^6]: [AMQP 0-9-1 specification](https://www.rabbitmq.com/resources/specs/amqp0-9-1.pdf)
    and [RabbitMQ heartbeat documentation](https://www.rabbitmq.com/docs/heartbeats).
[^7]: [OASIS AMQP 1.0 transport specification](https://docs.oasis-open.org/amqp/core/v1.0/os/amqp-core-transport-v1.0-os.html).
[^8]: [fe2o3-amqp](https://github.com/minghuaw/fe2o3-amqp).
[^9]: [RTSP 1.0, RFC 2326](https://www.rfc-editor.org/rfc/rfc2326) and
    [RTSP 2.0, RFC 7826](https://www.rfc-editor.org/rfc/rfc7826.html), especially
    the embedded/interleaved binary data sections.
[^10]: [gortsplib](https://github.com/bluenviron/gortsplib), developed for
    [MediaMTX](https://github.com/bluenviron/mediamtx).
[^11]: [lib60870 user guide](https://github.com/mz-automation/lib60870/blob/master/user_guide.adoc)
    and [project and licensing information](https://github.com/mz-automation/lib60870).
[^12]: [OpenDNP3 link frame implementation](https://github.com/dnp3/opendnp3/blob/release/cpp/lib/src/link/LinkFrame.cpp)
    and [link header](https://github.com/dnp3/opendnp3/blob/release/cpp/lib/src/link/LinkHeader.h).
[^13]: [OpenDNP3 status and licence](https://github.com/dnp3/opendnp3) and
    [Step Function DNP3 status and licensing](https://github.com/stepfunc/dnp3).
[^14]: [Modbus TCP implementation guide](https://www.modbus.org/file/secure/messagingimplementationguide.pdf)
    and [application protocol specification](https://www.modbus.org/file/secure/modbusprotocolspecification.pdf).
[^15]: [tokio-modbus and TCP-server feature](https://github.com/slowtec/tokio-modbus).
