# RFC 0008: External adapter conformance harness

- Status: proposed
- Scope: validate generic APIs and bounded protocol toys through public test
  connectors and independent byte oracles.
- Implementation prerequisite: RFC 0001 public test lifecycle/connectors.
- Validation relationship: RFCs 0003–0007 and 0009 supply later matrix rows;
  this RFC does not depend on those runtime RFCs to start its harness work.

## Problem

Codec round-trips can share encoder/decoder bugs. Parser tests do not establish
lifecycle, negotiation, resource, cancellation, or delivery behaviour. Every
examined protocol must have a verified toy that tests the survey implication;
this is explicitly not full interoperability or conformance certification.

## Contract

The harness foundation lands first, alongside the first RFC 0003 adapter toy.
Later rows are added with their corresponding runtime slices. It is a
validation dependency for those slices, not a code dependency of the harness on
unimplemented runtime APIs. RFC 0001 remains the TCP lifecycle connector base.
Raw peer scripting and controllable I/O must also work independently of a
`WireframeServer`, with separate read/write halves and caller-driven clients.
The existing server-pair convenience API remains usable, not mandatory.

Extend `wireframe_testing` with deterministic raw stream peers, scripted setup,
short reads/writes, controllable time, cancellation, and bounded datagram
delivery with loss, duplication, reordering, and truncation. Include counted
allocation ownership, gated workers/consumers, and faults at selected byte
boundaries. Fixtures use public production APIs, publish a bounded supported
subset, and preserve golden native bytes with independent provenance. Existing
protocol implementations may be test-only oracles only after review confirms
their licence permits the repository's test and fixture distribution. If not,
derive expected vectors from the protocol specification without copying an
implementation, and document provenance; never automatically fall back to
unlicensed source or captured artefacts. Production crates may use generic
serialization, TLS, cryptographic, and transport libraries, but must not depend
on ACP SDKs or other application-protocol parser/server implementations.

## Bounded protocol matrix

Every toy implements only the exchanges listed below. A row records exact
profile/version, supported messages, limits, oracle provenance, and exclusions
in its README/test module. Subsets are evidence for framework gaps, never
claims of full peer compatibility.

| Profile                               | Minimal toy exchange                                                                                                                                | Required negative, liveness, and budget evidence                                                                                                                                                                      | Independent oracle                                                                                                              |
| ------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------- |
| WebSocket RFC 6455                    | HTTP/1.1 upgrade; fragmented UTF-8 text; interleaved ping/pong; close                                                                               | Reject bad key/version/mask and invalid split UTF-8; continuation/control interleaving; ping progresses during stalled request; cap fragments                                                                         | RFC 6455 vectors and reviewed test-only Tungstenite if licence permits                                                          |
| OSCAR FLAP/SNAC                       | FLAP login-channel record and one SNAC request/reply with sequence                                                                                  | Reject bad marker/length; unsolicited server event progresses; cap record and per-session output                                                                                                                      | Open-source OSCAR source vectors with licence review, otherwise hand-checked FLAP/SNAC vectors                                  |
| AMQP 0-9-1                            | protocol header; connection.start/tune/open; channel open; one method/content exchange; close                                                       | Reject bad frame-end/size; heartbeat while handler stalls; cap channel and content assembly                                                                                                                           | RabbitMQ spec vectors; test-only licensed implementation only after review                                                      |
| AMQP 1.0                              | protocol header; open/begin/attach; one transfer and disposition; close                                                                             | Reject invalid frame/performative; flow/control progress; enforce link credit and transfer budget                                                                                                                     | OASIS specification examples and independently encoded performatives                                                            |
| RTSP 1.0 over TCP                     | OPTIONS, SETUP with interleaved channels, PLAY/PAUSE, one `$` RTP record, TEARDOWN                                                                  | Reject malformed CSeq/Content-Length/channel; PAUSE during output; cap headers and interleaved record                                                                                                                 | RFC 2326 examples and manually checked RTP/RTSP byte vectors                                                                    |
| RTSP 2.0 over TCP                     | OPTIONS, SETUP, PLAY/PAUSE, interleaved RTP record and TEARDOWN                                                                                     | Reject invalid message length/session; control remains live during media; cap body and media queue                                                                                                                    | RFC 7826 vectors; do not imply 1.0 compatibility                                                                                |
| IEC 60870-5-104                       | STARTDT activation/confirmation; one I-format ASDU and S acknowledgement; TESTFR                                                                    | Reject bad start/length/sequence; test timeout/reconnect; bound unacknowledged window and ASDU bytes                                                                                                                  | Public lib60870 test vectors only if licence review allows; otherwise independently checked specification vectors               |
| DNP3 IEEE 1815-2010 over TCP          | link request/response with address and CRC; transport/application fragment; confirmation                                                            | Corrupt each CRC block; expire incomplete fragments; bound fragment count, addresses and outstanding confirmations                                                                                                    | Independently compute CRC/reference published OpenDNP3 vectors; no production linkage                                           |
| Modbus TCP                            | MBAP transaction, unit, function 03 request and response; exception response                                                                        | Reject length/protocol-ID/function errors; timeout response path; cap MBAP length and concurrent transactions                                                                                                         | Modbus specification examples and independently encoded byte vectors                                                            |
| Apache Kafka                          | ApiVersions negotiation including flexible/tagged fields; Metadata request/reply; Produce with acks=0 and no response                               | Reject unknown version/truncated tagged fields; ordered response under concurrency; cap frame and decompression output                                                                                                | Apache protocol fixtures or licensed test-only client after review; pin schema version                                          |
| MySQL classic                         | server greeting; client capability response; SSLRequest to TLS phase transition; COM_STMT_CLOSE with no response; bounded COM_QUERY result          | Reject sequence mismatch/oversize continuation; actual SSLRequest phase barrier; reject sequence/length errors; test 0xffffff continuation plus empty exact-multiple terminator; cap packet, logical message and rows | MySQL documentation vectors; test-only licensed client after review                                                             |
| MariaDB classic                       | greeting/capability exchange; SSLRequest TLS transition; COM_STMT_CLOSE no-response; bounded query result under pinned MariaDB profile              | Reject divergent/unknown capability and sequence; actual upgrade, packet-sequence mismatch, 0xffffff continuation and empty exact-multiple terminator; cap packets/results                                            | MariaDB documentation vectors; separately licensed test-only client after review                                                |
| PostgreSQL v3                         | StartupMessage; AuthenticationOk/ReadyForQuery; extended Parse/Bind/Execute/Sync; error recovery through Sync; COPY and NotificationResponse subset | Reject malformed startup/message length; cancel long operation on a second connection; recover extended-query error at Sync; cap row, COPY, and notification queues                                                   | PostgreSQL protocol documentation vectors and independent `psql` capture corpus                                                 |
| OpenRA pinned profile                 | server handshake header; handshake order; one frame/order relay and acknowledgement                                                                 | Reject wrong size/mod profile; slow recipient does not stall other peer; enforce 131072-byte source-profile order bound                                                                                               | Pinned OpenRA source/build only as test oracle after GPL separation review; otherwise recorded vectors                          |
| Quake 3 Arena pinned UDP profile      | connectionless challenge/response; connect packet; one connected sequenced and fragmented message                                                   | Inject loss/duplicate/reorder/translation; expire fragments; cap datagram, fragment and unvalidated-peer state                                                                                                        | Released engine packet corpus/manual vectors; any engine oracle isolated and licence-reviewed                                   |
| Hotline profile used by mxd           | client-initiated handshake/auth subset and one correlated request/reply plus series completion                                                      | Reject short header/invalid series; disconnect notification while request stalls; cap series bytes, parts, and deadline                                                                                               | mxd's independently captured fixtures with provenance; no production dependency                                                 |
| ACP v1 JSONL, Magitek bounded profile | Initialize; create session; prompt; session updates; reverse permission request/reply; prompt cancellation and correlated terminal result           | Exact-limit and fragmented framing; structural/retained-work bounds; bilateral overload; honest publication; retained leases across close; byte-preserving enforced proxy                                             | Independently authored ACP specification vectors; pinned licensed SDK or independent scripted peer, never the toy's own encoder |

_Table 1: Mandatory bounded protocol toys and their independent evidence._

Signal-like cryptographic and service APIs are out of scope. Test-only oracle
choice is conditional on licence compatibility with test distribution. If not
compatible, use independently authored specification vectors with source and
version provenance; never copy implementation code or unlicensed captures. The
production dependency graph must remain free of these protocol parser/server
libraries.

The table deliberately separates AMQP generations, RTSP versions, MySQL and
MariaDB, and pins evolving Kafka, OpenRA and Quake profiles. A row may be split
when implementation work discovers that its handshake or transport profile is
not one coherent bounded exchange.

## Mandatory ACP v1 and JSONL toy

ACP means Agent Client Protocol; JSONL means newline-delimited JSON. The source
requirements are Magitek's
[pinned design](https://github.com/leynos/magitek/blob/bfef49cbf10912ccf8ed246d4760bbb21f96c01b/docs/magitek-design.md)
sections 4–7 and 9–12, and the following ACP v1 contracts, consulted on
17 November 2026:

- [transport](https://agentclientprotocol.com/protocol/v1/transports)
- [initialization](https://agentclientprotocol.com/protocol/v1/initialization)
- [prompt-turn](https://agentclientprotocol.com/protocol/v1/prompt-turn)
- [cancellation](https://agentclientprotocol.com/protocol/v1/cancellation)

The fixture manifest must pin the actual schema/specification revision, any SDK
revision, limits, and supported capabilities; a rolling page or crate major
version is not a wire-version pin.

Deliver a runnable bounded toy agent and an independently scripted client. Also
exercise the production driver in the initiating host role against an
independent agent script. Neither side may generate its expected fixtures with
the implementation under test. Run the same exchange over in-memory separate
halves and a local stdio-style subprocess, with protocol-pure stdout and
independently drained bounded stderr. These tests require no paid inference,
cloud service, container backend, or Magitek release. The toy validates the
shared contracts, not a claim that Magitek or an ACP SDK is already qualified.

The required exchange initializes v1 and negotiated capabilities, creates a
session, and submits a prompt. While that prompt remains pending, the agent
emits ordered `session/update` notifications and sends
`session/request_permission`; the client can reply without blocking unrelated
protocol progress. Finish one prompt normally. Cancel a second with
`session/cancel` and observe its correlated terminal result. Test optional
`$/cancel_request` only under a separately declared supported profile; neither
cancellation notification alone counts as peer-terminal evidence.

The minimum acceptance corpus includes:

1. Native types and identity: preserve string and numeric request IDs without
   coercion, including equal values used in opposite request directions;
   distinguish requests, replies, and notifications; reject illegal IDs and
   unsupported envelopes explicitly. Preserve admitted extension fields and
   verify stale/duplicate replies cannot satisfy a different generation.
   Compile a standalone consumer with asymmetric frame types and no mandatory
   `Envelope`, bincode, numeric route, or server application.
2. Framing and parsing: test every single split point plus generated multi-cut
   partitions, coalescing, split UTF-8, escaped newlines, LF, and the
   explicitly enabled CRLF variant. Test exact limits and one byte over, no
   final delimiter, empty lines, malformed UTF-8/JSON, duplicate keys,
   unsupported batches, excessive depth/nodes, and an unterminated trickle.
   Policy decisions must be partition-invariant and precede excess
   retention/materialization.
3. Encoding and accounting: force escaped identifiers and large response data
   through the bounded encoder. Check delimiter overhead, backing capacity,
   shared slices, temporary copies, decoded nodes, actions, workers, and
   waiting producers. Verify reserved control envelopes fit and a refused
   allocation cannot leave a partial multi-scope reservation behind.
4. Bilateral progress: combine a notification flood, pending permission,
   blocked output, held event leases, and cancellation. Test both cooperative
   peers and obstructed ordered streams. Reachable work progresses under the
   declared service/budget conditions or terminates with a typed cause; no
   timeout claims the driver can bypass unread bytes or a partial write.
5. Publication and closure: fault before publication and at every output byte
   boundary, race withdrawal with publication, and repeat close/cancel. Never
   report not submitted after possible publication, restart a partial line, or
   replay a prompt. Retain worker and lease charges after close, including
   empty/null leases, and reject replacement admission when old generations
   consume the shared budget. A saturated event queue cannot hide terminal
   cause, incomplete delivery, or cleanup evidence.
6. Consumer ordering and proxy enforcement: the toy's external ACP adapter
   preserves the relevant event prefix before exposing its successful result,
   without confusing enqueue, delivery, and durability. In a separate enforced
   proxy mode, inspect and forward permitted raw frame bytes and line endings
   unchanged without constructing an ACP client session. Reject unsupported or
   oversized input and partial EOF; never downgrade to transparent forwarding.
7. Shared-host contention: combine several noisy connections with a quiet
   connection, stalled workers, and retained closed generations. Measure finite
   work batches, protected control capacity, and exact final releases under
   declared executor service. Repeat lifecycle churn and record any remaining
   ownership instead of treating logical close as reclamation.

A report must map each case to RFCs 0003–0007, name exclusions, and distinguish
wire facts from the toy's stricter local profile. JSON duplicate-key rejection,
CRLF acceptance, resource limits, and delivery fences are explicit profile
choices, not newly invented universal ACP rules. ACP session state, permission
policy, and event watermarks remain in the external toy/Magitek adapter, never
in Wireframe core. SDK oracles stay test-only and must not introduce an
uncharged production path or a second semantic dispatcher.

## Acceptance

Every row requires a runnable bounded server toy and an independent peer client
that exchanges the row's stated messages; the ACP row additionally requires the
initiating-host and proxy paths above. Each row needs independent golden
vectors, malformed/truncation and split/coalescing tests, budget checks, and
lifecycle tests. Datagram cases inject loss/reordering; stateful cases exercise
timeout, cancellation, and phase transition. Reports name subsets and avoid
claiming full protocol conformance. Any oracle source, client, or vector
distribution must be licence-reviewed. If an oracle licence is incompatible or
unclear, use independently authored specification vectors with provenance; do
not copy, record, or redistribute that oracle's output. Generic serialization,
TLS/crypto/transport libraries are allowed in production; application-protocol
parser/server implementations remain test-only. The suite runs offline.

## Staging, ownership and open decisions

Stage 1 delivers raw peer scripting, deterministic time, byte capture, and
stream fault injection, composing with RFC 0001 where a server listener is
needed. Add JSONL byte/ownership fixtures with RFC 0006, then the mandatory ACP
exchange with the full-duplex and budget slices. ACP delivery does not wait for
unrelated database, datagram, or WebSocket toys. Stage 2 adds datagram fault
injection and report format before RFC 0009 begins. Each runtime slice adds its
own row in the same change. Stage 3 runs the complete offline matrix in CI and
verifies no production protocol dependency was introduced.

The harness owns transports, clocks, scripted actions, and test artefact
capture; the toy owns protocol state; the independent oracle owns expected
bytes. A toy must not call its own encoder to generate the expected decoder
fixture. Licence review records component, version, licence, usage, and
redistribution compatibility before adding an oracle dependency. Prefer
readable fixture text unless capture fidelity requires binary. Check that tests
and model harnesses actually execute rather than accepting an empty run.

Risks include apparently comprehensive but underspecified toys and mutable
upstream oracle behaviour. Pin provenance and versions, set exact exchanges and
resource ceilings, and review subset changes. This recommendation extends RFC
0001's TCP lifecycle reuse with independently usable raw-I/O fixtures; it does
not turn its convenience pair into a universal client interface.
