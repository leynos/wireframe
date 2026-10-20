# Migrate serializer output to shared payload bytes

Status: DRAFT — design only; implementation requires separate approval.

## Purpose / big picture

Issue [#538](https://github.com/leynos/wireframe/issues/538) asks for a
migration plan. This document completes that planning scope, not the
implementation. Serializer authors should be able to return already-owned
shared bytes without first producing a vector. The observable result will be
unchanged wire bytes and preservation of a shared payload's data pointer
through codec wrapping.

“Zero-copy” here means the serializer-to-codec hand-off. It does not promise
allocation-free serialization, buffer growth, framing, or socket writes.
`Bytes::from(Vec<u8>)` can already transfer an allocation without copying its
contents. The current limitation is the mandatory vector return type, which
cannot represent an existing shared `Bytes` allocation directly.

## Constraints

- This change edits documentation only. Leave source TODOs, inventory entries,
  and implementation checkboxes open until implementation evidence exists.
- Keep wire encoding, decode budgets, consumed-byte counts, error types, trait
  bounds, and defaulted serializer methods unchanged.
- Keep `Message::to_bytes`, `DecodeWith`, and all deserialize signatures.
- Keep client hook signatures, registration order, error notifications, and
  timing unchanged. Editing belongs to roadmap item 12.2.1.
- Keep public test-helper `Vec<Vec<u8>>` and `Option<Vec<u8>>` results.
- Add no dependency, unsafe code, lint suppression, or parallel serializer API.
- The finite conversion shim is an explicit requirement of #538 and ADR 009,
  not scaffolding to make intermediate implementation commits compile.

## Tolerances (exception triggers)

Stop implementation and revise the design for approval if it needs changes to
wire format, decoding, hook signatures, packet storage, external dependencies,
or an additional public conversion API. Do not solve deprecation warnings with
lint allowances. The proposed ADR 009 amendment must be accepted before using
its exception. Recheck the source inventory if unrelated changes have landed;
mechanical caller updates within the named boundaries need no new design.

## Risks

- High impact, medium likelihood: a missed feature-gated implementation breaks
  downstream builds. Search all implementations and run all-feature gates.
- High impact, low likelihood: routing the raw length-delimited path through
  `wrap_payload` adds framing twice. Retain its raw-payload regression.
- Medium impact, medium likelihood: conversion helpers become permanent.
  Retain ADR 009's release-count removal trigger and explicit review tasks.
- Medium impact, medium likelihood: `BytesMut` growth offsets allocation gains.
  Measure before claiming a throughput or allocation-count improvement.

## Progress

- [x] (2026-10-19) Inspect current contracts, callers, ADRs, and roadmap.
- [x] (2026-10-19) Verify Bincode writer and `BytesMut::freeze` contracts.
- [x] (2026-10-19) Draft contract, compatibility policy, and delivery tasks.
- [x] (2026-10-20) Review proposal status, ADR history, roadmap ownership, and
  deterministic property-test requirements; keep all implementation tasks open.
- [ ] Approve the design and proposed ADR 009 exception.
- [ ] EP-M1: deliver the serializer-to-transport migration atomically.
- [ ] EP-M2: deliver the feature and end-to-end acceptance matrix.

## Surprises & discoveries

The current blanket `EncodeWith` adapter calls `Message::to_bytes`; changing
only `BincodeSerializer` would retain vector encoding. The optional Serde
bridge has its own `encode_to_vec` path. `make test` already enables
`--all-features`, including `serializer-serde`; no feature-enabling Makefile
change is needed for that gate.

The source inventory also contains `FrameDecodeSerializer`, macro-generated
`CountingSerializer` and `FallbackSerializer`, beyond the reference sketch. The
pinned toolchain is `nightly-2026-03-26`; configuration specifies no Polonius
mode. Borrow-checker posture remains unverified, and this design does not rely
on a particular borrow-checker mode.

## Decision log

- 2026-10-19: choose a minimal project-owned `PayloadBytes` over `Bytes`, as
  permitted by ADR 008 and named by ADR 009. Preserve port ownership without
  introducing a new generic buffer parameter across the application.
- 2026-10-19: change all three encode contracts in one coherent implementation
  plateau, including callers. Splitting them would require temporary APIs.
- 2026-10-19: propose documented, finite but initially non-deprecated vector
  conversions. Required hook/helper uses must pass `-D warnings`; neither an
  untestable deprecated API nor warning suppression is acceptable. This is an
  explicit proposed exception to ADR 009, not its current accepted policy.
- 2026-10-19: omit `serialize_to_vec`. It helps callers only; implementers
  still need a signature change, and `into_vec` already covers callers.
- 2026-10-20: keep the serializer clauses in ADRs 005, 008, and 009 as dated
  proposals. ADR 009's accepted helper and deprecation policy remains in force
  until explicit acceptance also reconciles its checked-off roadmap item 1.1.2.
- 2026-10-20: add deterministic property coverage for payload lengths from 0
  through 4096, including empty payloads and Bincode length boundaries 250 and
  251. Select and record the fixed seed and case count with implementation.
- 2026-10-20: model hook ordering with bounded sequences of zero through eight
  transformations and non-commutative edits. Reversed order is a negative
  control that must fail; existing finite integration witnesses remain.

## Context and orientation

`src/serializer.rs` owns `Serializer`, `BincodeSerializer`, and the optional
`SerdeSerializerBridge`. `src/message.rs` owns `EncodeWith`, `DecodeWith`,
`Message`, and `SerdeMessage<T>`. All encode contracts currently return
`Result<Vec<u8>, Box<dyn Error + Send + Sync>>`, except `Message::to_bytes`,
which returns Bincode's concrete error and remains unchanged.

`src/app/outbound_encoding.rs::encode_message_frame` converts the serialized
vector to `Bytes` before calling `FrameCodec::wrap_payload`. The raw helper in
`src/app/outbound_response.rs::send_response_framed` bypasses wrapping because
`LengthDelimitedCodec` adds its own length prefix. The client pipeline in
`src/client/send_pipeline.rs` serializes, invokes vector-mutating hooks, and
sends through its framed sink.

Treat serializer traits as project-owned ports, Bincode as an encoding adapter,
and codecs/sinks as transport adapters. `PayloadBytes` carries immutable owned
output across that boundary. Only the transport edge extracts `Bytes`; encoding
must not acquire a dependency on an application codec. No directory or domain
architecture reorganization is needed for this library.

## Conformance basis

Baseline: commit `d00304c30fc26803b04a5ccf0247a7c79a18b570`, issue #538 as read
on 2026-10-19, [ADR 005][adr-005], [ADR 008][adr-008], and [ADR 009][adr-009].
No separate terms of reference exist. The serializer-specific amendments in
these ADRs remain proposed.

- #538 and ADR 008 shared ownership → EP-M1 → V1 pointer preservation.
- ADR 005 serializer independence → EP-M1 → V2 wire and bridge equivalence.
- ADR 009 finite conversions → EP-M1 → V3 downstream migration examples.
- Main roadmap 11.1.2 → EP-M1 and EP-M2 → V1, V4 transport/hook evidence.
- Dedicated zero-copy roadmap 3.2.2 → EP-M1 and EP-M2 → V1–V4.
- Main roadmap 11.1.1 retains packet storage work; 12.2.1 retains hook editing.
  Neither becomes complete merely because serializer output changes.

## Interfaces and dependencies

Add `src/payload.rs`, with a module comment and documented public examples.
Export `PayloadBytes` from `src/lib.rs` and `src/prelude.rs`. Use a transparent
newtype with a private `bytes::Bytes` field and derive `Clone`, `Debug`,
`PartialEq`, `Eq`, and `Default`. Provide `From<Bytes>`,
`From<PayloadBytes> for Bytes`, `into_bytes`, `AsRef<[u8]>`,
`Deref<Target = [u8]>`, `len`, and `is_empty`. These read-only operations must
not materialize a vector. Do not expose mutable access or edit-on-demand yet.

Add explicit `from_vec(Vec<u8>) -> Self` and `into_vec(self) -> Vec<u8>` as the
finite compatibility shim. `from_vec` transfers ownership using `Bytes::from`;
`into_vec` promises contents, not pointer reuse or allocation freedom. Omit
`From<Vec<u8>>` so conversion remains visible. The ADR 009 proposal defines
removal and warning policy; acceptance is a prerequisite for implementation.

Change the success type of `Serializer::serialize`, `EncodeWith::encode_with`,
and feature-gated `SerdeSerializerBridge::serialize_serde` to `PayloadBytes`.
Keep `Box<dyn Error + Send + Sync>`, all existing bounds, and default methods.
Document that callers can transfer owned output to codecs without copying.

Use a crate-private writer wrapper over `BytesMut` implementing Bincode 2.0.1's
`bincode::enc::write::Writer`. Its `write` appends the entire supplied slice
and returns `Result<(), EncodeError>`; partial writes are forbidden. Invoke
`bincode::encode_into_writer` for the blanket `Message` adapter and
`bincode::serde::encode_into_writer` for the Serde bridge, both with
`config::standard()`. Freeze only successful output into `PayloadBytes` and
retain the boxed `EncodeError` mapping. Do not route the blanket adapter through
`Message::to_bytes`. The writer is private so transport storage does not
become another public serializer abstraction.

### Planned migration examples

These excerpts illustrate future code, not APIs available in this checkout. For
an implementer whose method currently returns
`Result<Vec<u8>, Box<dyn Error + Send + Sync>>`, retain the encoding algorithm
but change the success type to `PayloadBytes`:

```rust,no_run
// Before: the method returns Result<Vec<u8>, Box<dyn Error + Send + Sync>>.
Ok(legacy_encode(value)?)
// After: the method returns Result<PayloadBytes, Box<dyn Error + Send + Sync>>.
Ok(PayloadBytes::from_vec(legacy_encode(value)?))
```

A delegating serializer keeps `value.encode_with(self)` as its body and changes
only the success type. A vector-dependent caller changes explicitly:

```rust,no_run
// Before:
let bytes: Vec<u8> = serializer.serialize(&message)?;
// After, when the downstream operation still requires a vector:
let bytes: Vec<u8> = serializer.serialize(&message)?.into_vec();
// Preferred for a transport accepting shared bytes:
let bytes: bytes::Bytes = serializer.serialize(&message)?.into_bytes();
```

The eventual versioned migration guide must turn these excerpts into complete
compiling examples with imports, bounds, and the application's legacy encoder.

## Plan of work

### EP-M1: shared serializer output reaches transports

First add the V1–V4 regressions described below. Capture the expected missing
`PayloadBytes` or return-type compile failure before production edits. After
the API exists, temporarily substitute a copying hand-off to prove the pointer
regression fails at runtime; remove that negative control before committing.
Existing wire and hook regressions must pass before and after migration.

Implement the type, writer, and three contract changes together. Update every
`Serializer` and `EncodeWith` implementation in `src/`, `examples/`, `tests/`,
and `wireframe_testing/`, including macro bodies. Known implementers include
`BincodeSerializer`, `HeaderSerializer`, `TestSerializer`, `FailingSerializer`,
`ContextCapturingSerializer`, `FrameDecodeSerializer`, `CountingSerializer`, and
`FallbackSerializer`. Delegate to `encode_with` when possible; use
`PayloadBytes::from_vec` only where an implementation still produces a vector.

Pass `payload.into_bytes()` to `wrap_payload` in `encode_message_frame` and
directly to the sink in `send_response_framed`. Remove the two #538 TODOs only
then. Preserve `SendError::Serialize` and `SendError::Io`. Inspect
`send_response` and `codec_driver.rs` for type flow without changing framing.

In `serialize_and_send`, branch on whether `request_hooks.before_send` is
empty. With no hooks, send `payload.into_bytes()` directly. Otherwise, call
`into_vec`, invoke hooks in registration order, and send `Bytes::from(vec)`.
Keep span length calculation after hooks and preserve error-hook/timing order.
Leave `BeforeSendHook` and `invoke_before_send_hooks` signatures unchanged;
comment that roadmap 12.2.1 owns their future editing model.

Use `into_vec` in `src/testkit/fragment_drive.rs`,
`wireframe_testing/src/helpers/fragment_drive.rs`, and
`wireframe_testing/src/echo_server.rs` to preserve public helper results.
Inspect `src/client/tests/streaming_infra.rs`, `src/client/tests/streaming.rs`,
`wireframe_testing/src/helpers/tests/helper_tests.rs`, and all other consumers;
use `into_bytes` for shared-byte destinations and `into_vec` for vector users.
Update the example in `wireframe_testing/src/helpers/slow_io.rs`.

Publish implementer and caller examples in the next applicable versioned
migration guide, selected at implementation time rather than assuming an
unreleased version. Update serializer rustdoc, the users' guide, developers'
guide, ADR status notes, and the inventory to reflect actual delivery.

This plateau satisfies the core contract and local regressions, with all
callers compiling and all gates passing. Compatibility is justified by #538 and
ADR 009; it does not preserve unchanged trait implementations. EP-M2 remains
open. Revert the complete plateau if necessary, not isolated trait signatures.
Check that only the approved public return types changed, no new wire/decode
behaviour appeared, and trace links remain accurate.

### EP-M2: feature and end-to-end acceptance

Exercise default features and `serializer-serde` explicitly through real client
and server paths. Cover a shared-buffer custom serializer, Bincode, the Serde
bridge, the default length-delimited codec, and a protocol-native codec. Cover
no hooks, one mutating hook, ordered hooks, serialize failure, and send
failure. Use finite cases for valid combinations; do not imply every serializer
supports every message adapter. Record each supported combination and its test
name.

This plateau discharges V4's integration matrix and confirms V1–V3 still hold.
Revert this plateau independently if its harness is wrong; keep delivery
checkboxes open until the evidence is repaired. Check source/wire boundaries,
ADR conformance, and unresolved scope before completing roadmap 11.1.2 and
3.2.2. Packet storage and hook API migration remain separate work.

## Verification plan

External axioms are Bincode's standard encoding and all-or-error writer
contract, and `Bytes` ownership/sharing with zero-copy `BytesMut::freeze`.
These are documented third-party interfaces, not proofs of third-party
internals. No new algorithmic lemma, concurrency protocol, or contractual
business logic is introduced, so no exhaustive proof is required. Reassess if
implementation introduces a lemma or contractual business rule.

- V1, ownership: in `src/payload.rs` test pointer preservation for `From<Bytes>`
  and `into_bytes`, with a non-empty heap buffer and a live sibling clone. In
  `src/app/outbound_encoding.rs`, return that clone from a test serializer and
  assert pointer equality after a codec stores it directly. A deliberate
  `Bytes::copy_from_slice` substitution must fail this test. Empty buffers test
  content only; their pointer equality would prove nothing.
- V2, wire compatibility: in `tests/serializer.rs`, compare Bincode output
  with unchanged `Message::to_bytes` for a representative message and a
  deterministic Rust `proptest` property over generated byte payloads of
  lengths 0–4096. Include empty payloads and explicit lengths 250 and 251 to
  exercise the Bincode length-encoding boundary. Record the fixed seed and case
  count when implementing the property; do not select a seed or count in this
  design. Add a `SerdeMessage<T>` round trip through
  `BincodeSerializer::serialize`, not only `encode_with`. Assert full consumed
  length and preserve decode-budget regressions. A changed config or dropped
  writer byte must fail equivalence. This tests repository integration;
  generated cases do not prove equivalence for every possible user encoder.
- V3, adoption: add a deterministic Rust `proptest` conversion property for
  `PayloadBytes::from_vec` and `into_vec`, generating byte vectors of lengths
  0–4096, including the empty vector. Record the fixed seed and case count when
  implementing the property. Also test a custom serializer in
  `tests/features/serializer_boundaries.feature`, with matching fixtures,
  steps, and scenario binding. Compile migration examples against the new
  contract. These checks must reject a conversion that truncates bytes.
- V4, transport and hooks: add a deterministic `proptest` property over zero
  through eight ordered hook transformations. Record the fixed seed and case
  count. Compare non-commutative edits with a registration-order model; include
  a two-hook witness that rejects a reversed-order negative control. Preserve
  finite witnesses for no-hook sends, one mutating hook, registration order,
  serialize failure, and send failure. In addition, preserve
  `send_response_framed_sends_raw_serialized_payload` in `tests/response/` and
  verify client hook mutation, ordering, and no-hook sends in
  `src/client/tests/request_hooks.rs`. Add serialize-failure coverage proving
  before-send hooks are not called and the error hook is called once. Retain
  send-error notification coverage. A double frame, reversed hook order, or
  hook invocation before successful serialization must fail these witnesses.

The V3 behaviour-driven development (BDD) scenario to add is:

```gherkin
Feature: Serializer boundaries and metadata context
  Scenario: Legacy vector serializer output crosses the shared-byte boundary
    Given a custom serializer producing a legacy vector payload
    When the payload is serialized through the compatibility conversion
    Then the shared payload contains the original wire bytes
    And the custom serializer can decode the payload
```

## Concrete steps and validation

Run from the repository root. Before editing code, refresh the implementation
inventory with the following search, then inspect serializer call sites too:

```sh
rg -n 'impl.*(Serializer|EncodeWith)|fn serialize|fn encode_with' src examples tests wireframe_testing
rg -n '\.serialize\(|\.encode_with\(' src examples tests wireframe_testing
```

For implementation, record baseline and red results using `make test` and
`make test-bdd` before production edits. After the API migration, rerun those
commands and expect the new ownership, wire, shim, and hook tests to pass. The
missing-type red failure alone is insufficient: record the runtime negative
control for V1 and the V2–V4 witnesses above.

Run gates sequentially, capturing output with `tee` and `set -o pipefail`:
`make fmt`, `make check-fmt`, `make lint`, `make test`, `make test-doc`,
`make markdownlint`, and `make nixie`. `make test` already covers the Serde
feature; EP-M2 must additionally provide an explicit default-feature Make
target or CI lane that omits `--all-features` and uses the same development
Cargo configuration as existing Make targets. Record its exact command and
result in this plan before completing EP-M2.

For this documentation-only deliverable, run `make fmt`, `make check-fmt`,
`make markdownlint` (including spelling), and `make nixie`, then
`coderabbit review --agent`. Resolve all concerns and repeat affected gates
before requesting another review. Rust implementation gates belong to the later
implementation and must not be reported as executed by this plan.

## Idempotence and recovery

Read the working diff before formatting; preserve unrelated work. Re-run only
failed gates after fixing their cause, then complete remaining gates. Do not
bypass compiler admission or create a separate Cargo cache. Stop on full disks
or unhealthy admission and report the exact failure. Commit only validated
plateaus; recover by reverting the affected coherent commit, not by leaving
mixed old/new contracts.

## Outcomes & retrospective

The design specifies a finite migration and measurable acceptance evidence. No
source implementation or performance improvement is claimed. Design review and
deterministic documentation validation are recorded in the PR; the plan remains
DRAFT until explicitly approved for implementation.

## References

- [Bincode writer contract][bincode-writer].
- [Bincode Serde writer][serde-writer].
- [BytesMut freeze][bytesmut-freeze].

Revision note (2026-10-19): initial design; separates planning completion from
implementation, narrows zero-copy claims, and proposes explicit conversion
warning policy before the breaking contract change.

Revision note (2026-10-20): preserve accepted ADR history in dated addenda,
clarify roadmap ownership and policy acceptance, and specify deterministic
property coverage. All implementation and approval work remains open.

[bincode-writer]:
https://docs.rs/bincode/2.0.1/bincode/enc/write/trait.Writer.html

[serde-writer]:
https://docs.rs/bincode/2.0.1/bincode/serde/fn.encode_into_writer.html

[bytesmut-freeze]:
https://docs.rs/bytes/1.10.1/bytes/struct.BytesMut.html#method.freeze

[adr-005]: ../adr-005-serializer-abstraction.md

[adr-008]: ../adr-008-zero-copy-public-byte-container.md

[adr-009]: ../adr-009-vec-u8-migration-rollout.md
