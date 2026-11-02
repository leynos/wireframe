# RFC 0008: External adapter conformance harness

- Status: proposed
- Scope: validate generic APIs and bounded protocol toys through public test
  connectors and independent byte oracles.
- Implementation prerequisite: RFC 0001 public test lifecycle/connectors.
- Validation relationship: RFCs 0003–0007 and 0009 supply later matrix rows;
  this RFC does not depend on those runtime RFCs to start its harness work.

## Problem

Codec round-trips can share encoder/decoder bugs. Parser tests do not establish
lifecycle, negotiation, resource, cancellation or delivery behaviour. Every
examined protocol must have a verified toy that tests the survey implication;
this is explicitly not full interoperability or conformance certification.

## Contract

The harness foundation lands first, alongside the first RFC 0003 adapter toy.
Later rows are added with their corresponding runtime slices. It is a
validation dependency for those slices, not a code dependency of the harness on
unimplemented runtime APIs. RFC 0001 remains the lifecycle connector base.

Extend `wireframe_testing` with deterministic raw stream peers, scripted setup,
short reads/writes, controllable time, cancellation, and bounded datagram
delivery with loss, duplication, reordering and truncation. Fixtures use public
production APIs, publish a bounded supported subset and preserve golden native
bytes with independent provenance. Existing protocol implementations may be
test-only oracles only after review confirms their licence permits the
repository's test and fixture distribution. If not, author expected vectors
independently from the protocol specification and document provenance; never
automatically fall back to unlicensed source or captured artefacts. Production
crates may use generic TLS, cryptographic, and transport libraries, but must
not depend on protocol parser/server implementations.

## Bounded protocol matrix

Every toy implements only the exchanges listed below. A row records exact
profile/version, supported messages, limits, oracle provenance, and exclusions
in its README/test module. Subsets are evidence for framework gaps, never
claims of full peer compatibility.

| Profile                          | Minimal toy exchange                                                                                                                                | Required negative, liveness and budget evidence                                                                                                                                                                       | Independent oracle                                                                                                |
| -------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------- |
| WebSocket RFC 6455               | HTTP/1.1 upgrade; fragmented UTF-8 text; interleaved ping/pong; close                                                                               | Reject bad key/version/mask and invalid split UTF-8; continuation/control interleaving; ping progresses during stalled request; cap fragments                                                                         | RFC 6455 vectors and reviewed test-only Tungstenite if licence permits                                            |
| OSCAR FLAP/SNAC                  | FLAP login-channel record and one SNAC request/reply with sequence                                                                                  | Reject bad marker/length; unsolicited server event progresses; cap record and per-session output                                                                                                                      | Open-source OSCAR source vectors with licence review, otherwise hand-checked FLAP/SNAC vectors                    |
| AMQP 0-9-1                       | protocol header; connection.start/tune/open; channel open; one method/content exchange; close                                                       | Reject bad frame-end/size; heartbeat while handler stalls; cap channel and content assembly                                                                                                                           | RabbitMQ spec vectors; test-only licensed implementation only after review                                        |
| AMQP 1.0                         | protocol header; open/begin/attach; one transfer and disposition; close                                                                             | Reject invalid frame/performative; flow/control progress; enforce link credit and transfer budget                                                                                                                     | OASIS specification examples and independently encoded performatives                                              |
| RTSP 1.0 over TCP                | OPTIONS, SETUP with interleaved channels, PLAY/PAUSE, one `$` RTP record, TEARDOWN                                                                  | Reject malformed CSeq/Content-Length/channel; PAUSE during output; cap headers and interleaved record                                                                                                                 | RFC 2326 examples and manually checked RTP/RTSP byte vectors                                                      |
| RTSP 2.0 over TCP                | OPTIONS, SETUP, PLAY/PAUSE, interleaved RTP record and TEARDOWN                                                                                     | Reject invalid message length/session; control remains live during media; cap body and media queue                                                                                                                    | RFC 7826 vectors; do not imply 1.0 compatibility                                                                  |
| IEC 60870-5-104                  | STARTDT activation/confirmation; one I-format ASDU and S acknowledgement; TESTFR                                                                    | Reject bad start/length/sequence; test timeout/reconnect; bound unacknowledged window and ASDU bytes                                                                                                                  | Public lib60870 test vectors only if licence review allows; otherwise independently checked specification vectors |
| DNP3 IEEE 1815-2010 over TCP     | link request/response with address and CRC; transport/application fragment; confirmation                                                            | Corrupt each CRC block; expire incomplete fragments; bound fragment count, addresses and outstanding confirmations                                                                                                    | Independently compute CRC/reference published OpenDNP3 vectors; no production linkage                             |
| Modbus TCP                       | MBAP transaction, unit, function 03 request and response; exception response                                                                        | Reject length/protocol-ID/function errors; timeout response path; cap MBAP length and concurrent transactions                                                                                                         | Modbus specification examples and independently encoded byte vectors                                              |
| Apache Kafka                     | ApiVersions negotiation including flexible/tagged fields; Metadata request/reply; Produce with acks=0 and no response                               | Reject unknown version/truncated tagged fields; ordered response under concurrency; cap frame and decompression output                                                                                                | Apache protocol fixtures or licensed test-only client after review; pin schema version                            |
| MySQL classic                    | server greeting; client capability response; SSLRequest to TLS phase transition; COM_STMT_CLOSE with no response; bounded COM_QUERY result          | Reject sequence mismatch/oversize continuation; actual SSLRequest phase barrier; reject sequence/length errors; test 0xffffff continuation plus empty exact-multiple terminator; cap packet, logical message and rows | MySQL documentation vectors; test-only licensed client after review                                               |
| MariaDB classic                  | greeting/capability exchange; SSLRequest TLS transition; COM_STMT_CLOSE no-response; bounded query result under pinned MariaDB profile              | Reject divergent/unknown capability and sequence; actual upgrade, packet-sequence mismatch, 0xffffff continuation and empty exact-multiple terminator; cap packets/results                                            | MariaDB documentation vectors; separately licensed test-only client after review                                  |
| PostgreSQL v3                    | StartupMessage; AuthenticationOk/ReadyForQuery; extended Parse/Bind/Execute/Sync; error recovery through Sync; COPY and NotificationResponse subset | Reject malformed startup/message length; cancel long operation on a second connection; recover extended-query error at Sync; cap row, COPY and notification queues                                                    | PostgreSQL protocol documentation vectors and independent `psql` capture corpus                                   |
| OpenRA pinned profile            | server handshake header; handshake order; one frame/order relay and acknowledgement                                                                 | Reject wrong size/mod profile; slow recipient does not stall other peer; enforce 131072-byte source-profile order bound                                                                                               | Pinned OpenRA source/build only as test oracle after GPL separation review; otherwise recorded vectors            |
| Quake 3 Arena pinned UDP profile | connectionless challenge/response; connect packet; one connected sequenced and fragmented message                                                   | Inject loss/duplicate/reorder/translation; expire fragments; cap datagram, fragment and unvalidated-peer state                                                                                                        | Released engine packet corpus/manual vectors; any engine oracle isolated and licence-reviewed                     |
| Hotline profile used by mxd      | client-initiated handshake/auth subset and one correlated request/reply plus series completion                                                      | Reject short header/invalid series; disconnect notification while request stalls; cap series bytes, parts and deadline                                                                                                | mxd's independently captured fixtures with provenance; no production dependency                                   |

Signal-like cryptographic and service APIs are out of scope. Test-only oracle
choice is conditional on licence compatibility with test distribution. If not
compatible, use checked-in vectors with source, version, capture method and
review record; never copy implementation code. The production dependency graph
must remain free of these protocol parser/server libraries.

The table deliberately separates AMQP generations, RTSP versions, MySQL and
MariaDB, and pins evolving Kafka, OpenRA and Quake profiles. A row may be split
when implementation work discovers that its handshake or transport profile is
not one coherent bounded exchange.

## Acceptance

Every row requires a runnable bounded server toy and an independent peer client
that exchanges the row's stated messages. It also requires independent golden
vectors, malformed/truncation and split/coalescing tests, budget checks and
lifecycle tests. Datagram cases inject loss/reordering; stateful cases exercise
timeout, cancellation and phase transition. Reports name subsets and avoid
claiming full protocol conformance. Any oracle source, client or vector
distribution must be licence-reviewed. If an oracle licence is incompatible or
unclear, use independently authored vectors derived from specifications with
provenance; do not copy, record or redistribute that oracle's output. Generic
TLS/crypto/transport libraries are allowed in production; protocol
parser/server implementations remain test-only. The suite runs offline.

## Staging, ownership and open decisions

Stage 1 delivers raw peer scripting, deterministic time, byte capture and
stream fault injection using RFC 0001 lifecycle hooks. Stage 2 adds datagram
fault injection and report format before RFC 0009 begins. Each runtime slice
adds its own row in the same change, so the matrix grows with capabilities
rather than blocking all implementation until every toy exists. Stage 3 runs
the complete offline matrix in CI and verifies no production dependency was
introduced.

The harness owns sockets, clocks, scripted actions and test artefact capture;
the toy owns protocol state; the independent oracle owns expected bytes. A toy
must not call its own encoder to generate the expected decoder fixture. Licence
review must record the component, version, licence, usage and distribution
compatibility before adding an oracle dependency. Unresolved decisions include
where profile manifests live and whether independent captures are checked in as
binary fixtures or compact hexadecimal text. Prefer readable text unless
capture fidelity requires binary.

Risks include apparently comprehensive but underspecified toys and mutable
upstream oracle behaviour. Pin all fixture provenance and versions, set exact
exchanges and resource ceilings, and review any proposed subset change. The
recommendation supersedes the generic test-only wording in RFC 0001 where the
protocol matrix adds byte-level, datagram and independent-oracle requirements;
RFC 0001 continues to define server lifecycle reuse.
