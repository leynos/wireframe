# 10.2.1 Capture allocation, copied-byte, throughput, and latency baselines

This execution plan (ExecPlan) is a living document. The sections
`Constraints`, `Tolerances`, `Risks`, `Progress`, `Surprises and discoveries`,
`Decision log`, `Outcomes and retrospective`, `Conformance basis`, and
`Verification plan` must be kept up to date as work proceeds.

Status: DRAFT

No `PLANS.md` exists in this repository as of 2026-08-23.

## Purpose / big picture

Roadmap item `10.2.1` is the measurement gate that the rest of the
`Frame = Vec<u8>` migration depends on. Items `10.2.2`, `11.1.2`, `11.2.3`, and
`13.2.1` all compare later work against numbers that do not exist yet. Without
those numbers, "remove the final default-path copy" is an assertion nobody can
falsify.

This plan produces a reproducible baseline for four stages of the default
codec path — inbound decode, middleware pass-through, request hooks, and
outbound encode — across four metrics: allocation events, allocated bytes,
copied bytes, and throughput with latency.

After this work, a maintainer can observe success by running two commands and
reading one document:

```bash
make baseline           # deterministic counters; prints a table, passes tests
make baseline-copy      # Valgrind copy profile; prints copied-byte totals
```

and then reading `docs/frame-vec-u8-baseline.md`, which records:

- For each of the four stages, at both payload classes, the exact number of
  allocation events and allocated bytes, reproducible bit-for-bit on any
  machine.
- For each stage at the large payload class, the number of bytes copied
  through `memcpy`-family calls.
- Indicative throughput and latency figures with a recorded hardware and
  toolchain stamp, plus an explicit statement that these figures are
  machine-dependent and must be re-measured, not compared across machines.
- A named, falsifiable claim about where the default path copies payload
  bytes today, so that item `11.1.2` has something concrete to remove and
  item `11.2.3` has something concrete to guard.

The baseline is not merely written down. The deterministic counters are
rendered into an `insta` snapshot and asserted by a test, and a further test
asserts that the table embedded in `docs/frame-vec-u8-baseline.md` is
byte-identical to the rendered report. A baseline that silently drifts away
from the code it describes is worse than no baseline, so the plan makes drift
a test failure.

## Context and orientation

This section assumes no prior knowledge of the repository.

### What wireframe is

`wireframe` is a Rust library for building servers and clients that speak
custom binary protocols. A server application is a
`WireframeApp<S, C, E, F>` where `S` is a serializer, `E` is a packet
(envelope) type, and `F` is a frame codec. The default instantiation, declared
at `src/app/builder/core.rs:29-34`, is:

```rust
WireframeApp<BincodeSerializer, (), Envelope, LengthDelimitedFrameCodec>
```

Its `Default` implementation (`src/app/builder/core.rs:52-80`) leaves
`middleware` empty and `protocol`, `fragmentation`, and `message_assembler`
all `None`. On the default path, therefore, middleware, protocol hooks,
fragmentation, and reassembly are inert. Only decode, dispatch, and encode do
real work. Throughout this plan, "the default codec path" means exactly this
configuration.

### The four stages, precisely

**Inbound decode.** `WireframeApp::process_stream`
(`src/app/inbound_handler.rs:158-226`) wraps the transport in a
`tokio_util::codec::Framed` built from `CombinedCodec`
(`src/app/combined_codec.rs:8-18`). Each frame reaches `handle_frame`
(`src/app/inbound_handler.rs:228-276`), then
`build_dispatchable_envelope` (`:278-322`), which calls
`parse_envelope` (`:74-95`). That in turn calls `BincodeSerializer::parse`
(`src/serializer.rs:150-157`), which calls `Envelope::from_bytes`, resolving
to the blanket `Message::from_bytes` (`src/message.rs:106-111`) and finally
into `bincode`. The `Vec<u8>` allocation that becomes `Envelope.payload` is
made inside `bincode`, not inside `wireframe`. It is observable only through
allocator instrumentation, never through a `wireframe`-owned copy call.

**Middleware pass-through.** `frame_handling::forward_response`
(`src/app/frame_handling/response.rs:29-63`) builds
`ServiceRequest::new(env.payload, env.correlation_id)` at line 41 and calls
`service.call(request)` at line 42. `ServiceRequest`
(`src/middleware.rs:39-76`) holds a `FrameContainer<Vec<u8>>`.
`RouteService::call` (`src/middleware.rs:315-339`) rebuilds the packet with
`E::from_parts(PacketParts::new(id, correlation_id, req.into_inner()))`. Every
step here is a move. `WireframeApp::build_chains`
(`src/app/inbound_handler.rs:146-156`) iterates the middleware vector in
reverse; on the default path that vector is empty and the loop body never
executes.

**Request hooks.** These are the client-side hooks in `src/client/hooks.rs`:
`BeforeSendHook = Arc<dyn Fn(&mut Vec<u8>) + Send + Sync>` (line 152) and
`AfterReceiveHook = Arc<dyn Fn(&mut BytesMut) + Send + Sync>` (line 176),
collected in `RequestHooks` (lines 184-189) and invoked by
`invoke_before_send_hooks` and `invoke_after_receive_hooks`
(`src/client/messaging.rs:335-346`) from `src/client/send_pipeline.rs:41` and
`src/client/messaging.rs:295`.

The server-side `WireframeProtocol::before_send` hooks in `src/hooks.rs` are a
different mechanism. On the default path `protocol` is `None`, so
`ProtocolHooks::before_send` (`src/hooks.rs:172-177`) is a no-op. They are out
of scope; see `Decision log` entry D-4.

**Outbound encode.** `encode_message_frame`
(`src/app/outbound_encoding.rs:22-38`) is the whole of it:

```rust
let bytes = serializer.serialize(msg).map_err(SendError::Serialize)?;
// Keep this bridge behaviour-preserving until issue #538 changes the
// public serializer contract to return a Bytes-native container.
let frame = codec.wrap_payload(Bytes::from(bytes));
```

It is called from `send_envelope` (`src/app/codec_driver.rs:120-152`, line
132) and `flush_pipeline_output` (`:164-180`). ADR-010
(`docs/adr-010-transport-frame-boundary-for-zero-copy.md:196-206`) confirms
this is the sole production caller of `FrameCodec::wrap_payload`.

### A correction the baseline must record

The roadmap and ADRs describe a "final default-path `Vec<u8>` copy between
serialization and `FrameCodec::wrap_payload`". Read literally, that phrase
names two operations that are both free on the default codec:

- `Bytes::from(Vec<u8>)` at `src/app/outbound_encoding.rs:36` takes ownership
  of the vector's buffer. It does not copy.
- `LengthDelimitedFrameCodec::wrap_payload` (`src/codec.rs:260-285`) is the
  identity function; `type Frame = Bytes` and the payload is returned
  unchanged.

The real cost is one step earlier: `Serializer::serialize`
(`src/serializer.rs:61-113`) returns `Vec<u8>`, so `bincode` must allocate a
fresh vector and copy the envelope payload into it. That allocation and that
copy are what item `11.1.2` must remove, by giving the serializer a
`Bytes`-native output contract. The baseline must therefore attribute the
outbound cost to `Serializer::serialize`, not to `Bytes::from` or
`wrap_payload`, or item `11.1.2` will be measured against the wrong number.

The same asymmetry appears inbound: `FrameCodec::frame_payload_bytes` has a
default body of `Bytes::copy_from_slice` (`src/codec.rs:90`), which copies,
but `LengthDelimitedFrameCodec` overrides it with `frame.clone()`
(`src/codec.rs:260-285`), a reference-count bump. The default path does not
pay that copy; a protocol-native codec that does not override the method
would.

### What already exists

Roadmap item `9.6.1` (see `docs/execplans/9-6-1-codec-performance-benchmarks.md`,
status COMPLETE) built codec-level benchmarks:

- `benches/codec_performance.rs` defines the Criterion groups `codec/encode`,
  `codec/decode`, and `codec/fragmentation_overhead`.
- `benches/codec_performance_alloc.rs` defines the group `codec/allocations`
  and contains a private `CountingAllocator` (lines 34-77) installed as
  `#[global_allocator]`.
- `wireframe_testing/src/codec_benchmarks/` holds the shared support code:
  `SMALL_PAYLOAD_BYTES = 32`, `LARGE_PAYLOAD_BYTES = 64 * 1024`,
  `VALIDATION_ITERATIONS = 16`, `payload_for_class`, `CodecUnderTest`,
  `PayloadClass`, `BenchmarkWorkload`, `benchmark_workloads`, `measure_encode`,
  `measure_decode`, `Measurement`, `MeasurementExt`, `AllocationBaseline`, and
  `allocation_label`.
- `make bench-codec` (`Makefile:66-67`) runs both bench binaries with
  `--features test-support`.

Three gaps matter for this item. First, the existing coverage stops at the
codec boundary: there is nothing for middleware, hooks, or the
serializer-to-codec bridge. Second, `CountingAllocator` counts allocation
*events* only — it never reads `layout.size()`, so it cannot report bytes —
and it uses process-global atomics, which is why its own documentation
(lines 79-84) calls it a "noisy relative baseline". Third, nothing anywhere in
the repository measures copied bytes, and no document defines what a "copied
byte" is.

### Terms defined

- **Allocation event**: one call into the global allocator's `alloc`,
  `alloc_zeroed`, or `realloc` entry point.
- **Allocated bytes**: the sum of `Layout::size()` over allocation events. For
  `realloc`, the new size.
- **Copied byte**: a byte written to a second location by a `memcpy`,
  `memmove`, `strcpy`, or `bcopy` call while an equivalent live copy exists.
  This is the operational definition adopted by this plan; see `Decision log`
  entry D-2 and ADR-011.
- **Payload class**: `Small` (32 bytes) or `Large` (65,536 bytes), as already
  defined in `wireframe_testing::codec_benchmarks`.
- **Probe**: a deterministic, in-process function that drives one production
  stage exactly once for one payload class, with no transport and no runtime
  scheduling.
- **Deterministic counter**: a measurement that yields an identical value on
  every machine and every run. Allocation events, allocated bytes, and copied
  bytes are deterministic counters. Throughput and latency are not.

## Conformance basis

There is no Terms of Reference document in this repository. The upstream
artefacts are:

- `docs/roadmap.md` section 10.2, item `10.2.1` (lines 470-481), revision as at
  commit `d77dd76`.
- `docs/adr-008-zero-copy-public-byte-container.md` (Accepted). The binding
  technical requirement is line 85-86: "The default codec path must be able to
  move from serialization to `wrap_payload` without materializing a fresh
  `Vec<u8>`." Its non-goal at line 181 explicitly declines to guarantee zero
  allocation for mutation paths, so the baseline must separate read-only from
  mutating hook shapes.
- `docs/adr-009-vec-u8-migration-rollout.md` (Accepted). Line 270-272 flags
  `PayloadBytes::into_vec` as a known allocating-and-copying escape hatch.
- `docs/adr-010-transport-frame-boundary-for-zero-copy.md` (Accepted). Lines
  189-192 name the residual serializer-to-codec bridge and link issue
  [leynos/wireframe#538](https://github.com/leynos/wireframe/issues/538); lines
  196-206 pin the sole production `wrap_payload` call site.
- `docs/frame-vec-u8-inventory.md`, in particular lines 98-105 and 118 (the
  middleware `Vec<u8>` island), 124-127 (inbound decode), 132-140 (the
  middleware-to-codec return path), 200-204 (client hooks), and 226-228 (the
  serializer contract).
- `docs/zero-copy-frame-and-payload-migration-roadmap.md`, items 1.2.1 and
  1.2.2, the precursor wording that `docs/roadmap.md` 10.2.1 condenses.

Trace links:

```plaintext
ADR-008-REQ-default-path-no-fresh-vec -> EP-M3 -> EP-M4
  -> tests::baseline_report::outbound_encode_allocates_serializer_vec
ADR-010-RISK-serializer-bridge -> EP-M5
  -> docs/frame-vec-u8-baseline.md#outbound-encode
ROADMAP-10.2.1-alloc -> EP-M1, EP-M3, EP-M4
  -> tests::baseline_report::snapshot_matches_committed_baseline
ROADMAP-10.2.1-copied-bytes -> EP-M5
  -> docs/frame-vec-u8-baseline.md#copied-bytes
ROADMAP-10.2.1-throughput-latency -> EP-M5
  -> benches/pipeline_baseline.rs
INVENTORY-middleware-island -> EP-M3
  -> tests::baseline_report::middleware_pass_through_is_move_only
EP-M2 (measurement soundness) -> verus/wireframe_proofs.rs
  and src/../alloc_probe.rs kani harnesses
```

Item `10.2.1` requires `10.1.1`, which is complete (`docs/roadmap.md:460-462`),
so this item is unblocked.

## Constraints

- The four stages, the metric set, and "the default codec path" are fixed by
  `docs/roadmap.md:472-474`. Do not narrow them.
- Production behaviour must not change. Every probe drives production code; no
  probe may re-implement a production step. Where a production function is
  crate-private, expose it through a `test-support`-gated shim rather than
  copying its body into the probe. A probe that re-implements the thing it
  measures proves only that the probe is self-consistent.
- The `test-support` Cargo feature is the only permitted mechanism for widening
  visibility. Nothing new may become unconditionally public on the `wireframe`
  crate.
- Deterministic counters must be exactly reproducible. No test may assert on
  wall-clock time, throughput, or latency.
- Allocation instrumentation must be thread-local, not process-global, so that
  a parallel test harness running on other threads cannot pollute a
  measurement. This is a hard requirement, not a preference: the existing
  process-global counter is documented as noisy, and a noisy baseline cannot
  support item `10.2.2`'s thresholds.
- Copied-byte measurement must not require modifying production code. It is an
  external observation.
- Use `rstest` for unit tests, `rstest-bdd` v0.5.0 for behavioural tests,
  `googletest` matchers and `pretty_assertions` for assertion clarity,
  `proptest` for generated input domains, and `insta` for the rendered
  baseline snapshot.
- All documentation must follow `docs/documentation-style-guide.md`: en-GB
  Oxford spelling, sentence-case headings, prose wrapped at 80 columns, code
  at 120, language identifiers on every fence, captions on every table, no
  first-person or second-person pronouns.
- Register every new document in `docs/contents.md`.
- Record the measurement methodology in a new ADR
  (`docs/adr-011-byte-migration-baseline-methodology.md`) and reference it from
  `docs/frame-vec-u8-baseline.md` and `docs/developers-guide.md`.
- Append an entry under `## Unreleased` in `CHANGELOG.md`.
- Mark `docs/roadmap.md` item `10.2.1` done only after every gate passes. Edit
  the roadmap with `mapsplice` per `docs/developers-guide.md:512-540`, and use
  inline links rather than footnotes in that file.
- Run gates with `set -o pipefail` and `tee` to a log under `/tmp`. Delegate
  full gate runs to the `scrutineer` sub-agent; do not run them inline.
- Commit after each milestone. Every commit must pass `make check-fmt`,
  `make lint`, and `make test`.

## Tolerances (exception triggers)

- **Scope**: if implementation touches more than 32 files or exceeds 2,500 net
  changed lines, stop and re-scope.
- **Dependencies**: `insta` is the only new dependency this plan authorizes,
  and only as a `dev-dependency`. If anything else is required — including
  `dhat`, `stats_alloc`, `iai-callgrind`, or `critcmp` — stop and escalate.
- **Public interface**: if any change must be visible without the
  `test-support` feature, stop and escalate.
- **Verus**: this would be the repository's first Verus proof. If the proof in
  EP-M2 is not discharged within four attempts, stop, record the failure, and
  escalate with the option of descoping to the Kani harness plus the proptest
  model. Do not weaken the property to make it provable.
- **Kani**: if a harness exceeds ten minutes at its stated unwind bound, reduce
  the bound and record the reduction rather than raising the timeout.
- **Valgrind**: if `valgrind --tool=dhat --mode=copy` cannot attribute copies
  to the probe under test, stop and escalate before inventing a substitute
  metric.
- **Measurement instability**: if a deterministic counter varies between two
  consecutive runs on the same machine, stop. That is an instrument defect, not
  noise to be averaged away.
- **Benchmark runtime**: if a single Criterion target exceeds six minutes
  locally, reduce sample size and record the change.
- **Ambiguity**: if the correct attribution of a measurement to a stage is
  genuinely unclear, stop and present the options rather than choosing
  silently.

## Risks

- Risk: thread-local state accessed from inside a `GlobalAlloc` implementation
  can recurse or fail during thread-local storage initialization and teardown,
  because initializing thread-local storage may itself allocate.
  Severity: high. Likelihood: medium.
  Mitigation: declare the counters with `thread_local!` using a
  `const { Cell::new(0) }` initializer so that no lazy allocation occurs on
  first access, and reach them through `try_with`, treating a failure as "not
  measuring" rather than panicking. Add a test that allocates during thread
  teardown and asserts the process does not abort.

- Risk: LLVM inlines small `memcpy` calls into load and store sequences, which
  Valgrind's DHAT copy mode cannot see, so copied bytes are undercounted for
  small payloads.
  Severity: medium. Likelihood: high.
  Mitigation: treat copied bytes as normative only for the `Large` payload
  class, record the `Small` class with an explicit undercount caveat, and
  cross-check both against allocated bytes, which bound copied bytes from above
  for allocate-then-fill buffers. Record this limitation in ADR-011 rather than
  hiding it.

- Risk: the bytes copied inside `bincode` and inside `tokio_util` are not
  attributable to `wireframe` source lines, so a naive reading of a DHAT
  profile blames the wrong frame.
  Severity: medium. Likelihood: high.
  Mitigation: run one probe process per stage with a `--stage` argument, and
  subtract a `--stage noop` calibration run, so each figure is a difference
  between two whole-process totals rather than an attribution to a stack.

- Risk: this is the repository's first Verus proof, and
  `docs/developers-guide.md:376-379` states `make run-verus` is expected to fail
  until `verus/wireframe_proofs.rs` exists. Establishing the proof harness may
  cost more than the proof.
  Severity: medium. Likelihood: medium.
  Mitigation: EP-M2 is a separable milestone with its own tolerance. The
  measurement work in EP-M1 and EP-M3 does not depend on it.

- Risk: `insta` is new to the repository, so there is no existing convention
  for snapshot location or review.
  Severity: low. Likelihood: high.
  Mitigation: place snapshots under `tests/snapshots/`, document the
  re-blessing procedure in `docs/developers-guide.md`, and keep the snapshot
  restricted to deterministic counters so it never fails for environmental
  reasons.

- Risk: the baseline document and the snapshot drift apart, leaving a document
  that describes code that no longer exists.
  Severity: medium. Likelihood: medium.
  Mitigation: a test asserts that the fenced table in
  `docs/frame-vec-u8-baseline.md` is byte-identical to the rendered report.

- Risk: the `test-support` shims widen the surface that later refactors must
  keep working, creating drag on items 11.x and 12.x.
  Severity: low. Likelihood: medium.
  Mitigation: the shims are thin re-exports of existing crate-private
  functions, feature-gated, and documented as unstable. When 11.1.2 changes
  `Serializer::serialize`, the shim changes with it and the baseline is
  re-blessed; that is the intended workflow, not breakage.

- Risk: benchmarks do not currently run in CI at all, so nothing prevents the
  baseline from rotting.
  Severity: medium. Likelihood: medium.
  Mitigation: the deterministic half of the baseline runs under `make test`,
  which CI does run. Only the timing half and the Valgrind half are
  operator-invoked. Record this split explicitly so item `10.2.2` does not
  assume a CI gate that does not exist.

## Verification plan

The implementation introduces one genuinely non-trivial piece of logic: the
allocation-accounting state machine. Everything downstream — every threshold in
`10.2.2`, every regression assertion in `11.2.3`, every claim in `13.2.1` —
rests on that accounting being exact. It is therefore verified rather than
merely tested.

The probes themselves introduce no invariant; they are straight-line calls into
production code. They are covered by parameterized tests and a negative
control.

### Axioms

These are assumed, not verified:

- The system allocator satisfies the `GlobalAlloc` contract.
- `bincode` and `tokio_util` behave as documented; their internals are not
  verified. Their observable allocation behaviour is measured, not proven.
- Valgrind's DHAT copy mode counts bytes passed to `memcpy`, `memmove`,
  `strcpy`, and `bcopy`, as documented in the Valgrind manual, section 10.5.
- `thread_local!` with a `const` initializer performs no heap allocation on
  first access on the supported targets.
- Criterion's timing methodology is sound; this plan does not verify it and
  does not assert on its output.

### Obligation V-1: accounting exactness over unbounded event sequences

- **Obligation**: for any finite sequence of allocator events
  `e_0 .. e_{n-1}`, the accounting function satisfies
  `total_events = |{e : e is Alloc or Realloc}|`,
  `total_bytes = Σ size(e) over allocating events`,
  `curr_bytes = Σ size of live blocks`, and `curr_bytes ≤ total_bytes`. The
  `Realloc` case is the non-trivial one: it must count as one allocating event
  of the new size, release the old size from `curr_bytes`, and contribute
  `min(old, new)` to `realloc_moved_bytes`.
- **Method**: Verus deductive proof by induction over `Seq<Event>`.
- **Rationale**: the sequence length is unbounded — a large-payload probe
  performs thousands of events — so bounded exploration cannot discharge the
  property. The realloc case makes the induction step non-trivial, so the proof
  is not a restatement of the assumption.
- **Domain**: all finite sequences over
  `Event = Alloc(size) | Realloc(old, new) | Dealloc(size)`, with sizes in
  `nat`.
- **Artefact**: `verus/wireframe_proofs.rs` and
  `verus/wireframe_proofs_alloc.rs`.
- **Evidence**: `make run-verus` currently fails with a missing-proof-file
  diagnostic; after EP-M2 it must report zero verification errors. Record the
  transcript.
- **Non-vacuity**: the proof must include a witness lemma exhibiting a
  three-event sequence `[Alloc(8), Realloc(8, 16), Dealloc(16)]` with the
  concrete expected totals, proving the antecedents are inhabited. Before
  declaring the proof complete, seed a fault by changing the `Realloc` arm to
  add `old` instead of `new` to `total_bytes` and confirm Verus rejects it.
  Inspect the proof for stray `assume` statements; any surviving `assume` fails
  this obligation.

### Obligation V-2: the real allocator refines the specification

- **Obligation**: the `GlobalAlloc` implementation in
  `wireframe_testing::codec_benchmarks::alloc_probe` records, for any bounded
  sequence of allocation operations, exactly the totals that the V-1
  specification function computes for the corresponding event sequence.
- **Method**: Kani bounded model checking.
- **Rationale**: V-1 proves the specification correct; V-2 connects the
  specification to the code that actually runs. Kani explores every path
  through the real `alloc`, `realloc`, and `dealloc` arms including the
  enable/disable gating and the `try_with` fallback, which Verus cannot reach
  because those paths use `unsafe` and thread-local storage.
- **Domain**: sequences of up to four operations with symbolic sizes bounded to
  a small range, plus a symbolic enable/disable schedule. Stated bound:
  `#[kani::unwind(5)]`.
- **Artefact**: `#[cfg(kani)]` harnesses in
  `wireframe_testing/src/codec_benchmarks/alloc_probe.rs`, run by
  `make kani-baseline`.
- **Evidence**: `cargo kani --harness verify_alloc_probe_refines_spec` reports
  no failed properties. Record the transcript including the explored bound.
- **Non-vacuity**: assert that at least one reachable execution has
  `total_events > 0` and one has counting disabled, so neither branch is
  vacuously excluded. Seed a fault by dropping the `layout.size()` addition in
  the `realloc` arm and confirm Kani produces a counter-example. Do not
  `kani::assume` a fixed size; the sizes must remain symbolic.

### Obligation V-3: measurement scoping is exact and non-reentrant

- **Obligation**: for a measured scope `S`, the reported counters equal the
  counters of exactly the allocator events that occurred on the measuring
  thread between entering and leaving `S`; nested scopes do not double-count;
  events on other threads are excluded.
- **Method**: `proptest` over generated scope schedules, plus an `rstest`
  parameterized test that spawns concurrent allocating threads during a
  measurement.
- **Rationale**: this is a property over orderings and interleavings rather
  than over arithmetic, and the thread-exclusion half cannot be expressed in
  the single-threaded Verus or Kani models.
- **Domain**: generated sequences of `Enter`, `Allocate(size)`, `Leave`, and
  `SpawnAllocatingThread` operations, depth-bounded to 3.
- **Artefact**: `wireframe_testing/tests/alloc_probe_scoping.rs`.
- **Evidence**: the property fails before the thread-local change (the current
  process-global counter cannot exclude other threads) and passes after.
  Record both transcripts. Use `proptest`'s classification output to show that
  generated cases actually reach nesting depth 3 and actually spawn threads;
  if a class is unreached, the generator is wrong and the run does not count.
- **Non-vacuity**: negative control — a test variant that deliberately shares
  the counter across threads must fail the property.

### Obligation V-4: the probes exercise the stages they claim to

- **Obligation**: each of the four probes performs a non-zero, stage-specific
  amount of work, and the reported counters change when the corresponding
  production code changes.
- **Method**: `rstest` parameterized tests with `googletest` matchers, plus a
  seeded-fault probe variant.
- **Rationale**: a baseline of zero is indistinguishable from a probe that
  never ran. This obligation exists to make that failure mode impossible.
- **Domain**: the four stages crossed with the two payload classes.
- **Artefact**: `tests/baseline_probes.rs`.
- **Evidence**: every probe reports `total_events > 0` for at least one payload
  class; the middleware probe reports zero *payload-sized* allocations,
  matching the "moves only" claim; the outbound-encode probe reports at least
  one allocation of at least `payload_len` bytes, matching the
  `Serializer::serialize` claim.
- **Non-vacuity**: the seeded-fault control. A `#[cfg(test)]` probe variant
  inserts one additional `payload.to_vec()` into the middleware stage. The
  test asserts that this variant reports strictly more allocated bytes than the
  faithful variant, by at least `payload_len`. This proves the instrument can
  see exactly the class of copy that item `11.1.2` will remove. If the fault
  variant reports the same numbers, the whole baseline is worthless and the
  milestone fails.

### Obligation V-5: the recorded baseline matches the code

- **Obligation**: the table in `docs/frame-vec-u8-baseline.md` equals the
  report rendered from a live run.
- **Method**: `insta` snapshot assertion plus a document-synchronization test.
- **Rationale**: prevents the documented baseline from silently decaying into
  fiction.
- **Domain**: the full rendered report.
- **Artefact**: `tests/baseline_report.rs` and
  `tests/snapshots/baseline_report__deterministic_baseline.snap`.
- **Evidence**: `make test` fails if either the snapshot or the document
  diverges. Re-blessing is `cargo insta accept` followed by pasting the
  snapshot body into the document, which the test then re-checks.
- **Non-vacuity**: a negative control test mutates one digit in a copy of the
  document fixture and asserts the synchronization check rejects it.

### Obligation V-6: copied-byte capture is reproducible

- **Obligation**: two consecutive `make baseline-copy` runs on the same machine
  and binary report identical copied-byte totals per stage.
- **Method**: behavioural test with `rstest-bdd` over the recorded profile
  artefacts, plus an operator-run reproducibility check.
- **Rationale**: Valgrind is deterministic by construction, so any variation
  indicates the harness is measuring something other than the probe.
- **Domain**: the four stages at the `Large` payload class.
- **Artefact**: `tests/features/baseline_copy_profile.feature` with steps in
  `tests/steps/baseline_copy_profile_steps.rs`.
- **Evidence**: the scenario parses two recorded profile summaries and asserts
  equality. Where Valgrind is unavailable the scenario is skipped with an
  explicit diagnostic, never silently passed.
- **Non-vacuity**: the fixture set includes one deliberately mismatched pair
  that the scenario must reject.

### Why Verus and Kani are both used, and where they are not

Verus discharges V-1 over unbounded sequences but cannot reason about the
`unsafe` `GlobalAlloc` implementation or thread-local storage. Kani discharges
V-2 over the real implementation but only to a bound of five operations, far
below the thousands a real probe performs. Neither alone is sufficient; the
pair is. The residual gap is that V-2's bound does not cover long sequences,
which V-1 covers only for the specification. That gap is stated here rather
than papered over.

No proof is attempted for the probes, the report renderer, or the Criterion
benches. They introduce no invariant over an unbounded domain, and
parameterized tests with a seeded-fault control give proportionate rigour.

## Plan of work

### Stage A — understand and confirm (no code changes)

Read `src/app/outbound_encoding.rs`, `src/app/inbound_handler.rs`,
`src/app/frame_handling/response.rs`, `src/middleware.rs`,
`src/client/hooks.rs`, `src/client/messaging.rs`, `src/client/send_pipeline.rs`,
`src/codec.rs`, and `src/serializer.rs`, and confirm the four stage boundaries
described in `Context and orientation` against the current tree.

Confirm two open questions recorded during planning and write the answers into
`Surprises and discoveries`:

1. `src/app/codec_driver.rs:1-12` documents that the frame pipeline applies
   protocol hooks via `before_send`, but no such invocation was located in
   `FramePipeline::process` (`:56-71`) or in
   `src/app/frame_handling/response.rs`. Determine whether the documentation is
   stale or whether the invocation lives in an untraced path such as
   `src/connection/frame.rs`. Do not fix it in this plan; record it and, if it
   is a genuine defect, raise a separate issue.
2. `docs/repository-layout.md` is referenced by `AGENTS.md:41-43`,
   `docs/documentation-style-guide.md`, and `docs/roadmap.md`, but does not
   exist. Record the gap; creating it is out of scope.

Validation: no code changes; the two answers are written into the plan.

### Stage B — red tests

Write the failing tests before the implementation, in this order:

1. `wireframe_testing/tests/alloc_probe_scoping.rs` — the V-3 property. It
   fails to compile until `alloc_probe` exists.
2. `tests/baseline_probes.rs` — the V-4 tests including the seeded-fault
   control.
3. `tests/baseline_report.rs` — the V-5 snapshot and document-synchronization
   tests. The snapshot file does not exist yet, so `insta` reports a new
   snapshot; the document test fails because
   `docs/frame-vec-u8-baseline.md` does not exist.
4. `tests/features/baseline_copy_profile.feature` and its steps — the V-6
   scenario, failing because no profile artefacts exist.

Validation: each test fails for its intended reason, recorded as a transcript
in `Artefacts and notes`. A test that fails to compile counts as red only if
the compilation error names the missing item this plan will add.

### Stage C — implementation and verification together

Milestones EP-M1 through EP-M5 below. Each ends with its own validation.

### Stage D — documentation, cleanup, and wider validation

Milestone EP-M6.

## Milestones and plateaus

### EP-M1 — byte-aware, thread-local allocation instrument

- **Outcome**: `wireframe_testing::codec_benchmarks::alloc_probe` exists and
  `benches/codec_performance_alloc.rs` uses it instead of its private
  `CountingAllocator`. The existing `codec/allocations` Criterion group and its
  label format are unchanged.
- **Requirements**: `ROADMAP-10.2.1-alloc`.
- **Interfaces**: in
  `wireframe_testing/src/codec_benchmarks/alloc_probe.rs`, define:

  ```rust
  /// Counters describing allocator traffic observed on one thread.
  #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
  pub struct AllocationCounters {
      pub allocation_events: u64,
      pub allocated_bytes: u64,
      pub deallocation_events: u64,
      pub deallocated_bytes: u64,
      pub reallocation_events: u64,
      pub realloc_moved_bytes: u64,
  }

  /// A `GlobalAlloc` that counts traffic on the measuring thread only.
  pub struct ProbeAllocator;

  // SAFETY invariants are documented on the impl.
  unsafe impl core::alloc::GlobalAlloc for ProbeAllocator { /* ... */ }

  /// Runs `operation` with counting enabled on the current thread and
  /// returns the counters attributable to it.
  pub fn measure<T, F: FnOnce() -> T>(operation: F) -> (T, AllocationCounters);
  ```

  The counters live in `thread_local!` `Cell`s declared with
  `const { Cell::new(0) }` and are reached with `try_with`; a `try_with`
  failure means "not measuring" and must never panic. `measure` is re-entrant
  safe: a nested `measure` attributes events to the innermost scope and adds
  them to the outer scope on exit.
- **Acceptance evidence**: `wireframe_testing/tests/alloc_probe_scoping.rs`
  passes, including the concurrent-thread exclusion case that the previous
  process-global counter could not satisfy. `make bench-codec` still emits the
  same `wrap_allocs_<n>` and `decode_allocs_<n>` label shapes.
- **Conformance check**: no public `wireframe` API changed; only
  `wireframe_testing` gained a module. No new dependency.
- **Recovery**: the change is additive to `wireframe_testing` and a
  single-file substitution in the bench. Revert the bench file to restore the
  prior behaviour.
- **Remaining gaps**: the instrument is unverified until EP-M2.
- **Compatibility decision**: none required. `wireframe_testing` is a test
  support crate below 1.0 with no external consumer commitment.

### EP-M2 — verification of the accounting

- **Outcome**: V-1 discharged in Verus, V-2 discharged in Kani.
- **Requirements**: measurement soundness for `ROADMAP-10.2.1-alloc`.
- **Interfaces**: `verus/wireframe_proofs.rs` as the entry point that `mod`s
  `verus/wireframe_proofs_alloc.rs`, mirroring `AllocationCounters` as a spec
  struct and `Event` as a spec enum. A Makefile target `kani-baseline` runs the
  Kani harnesses; `make run-verus` gains a working proof file for the first
  time.
- **Acceptance evidence**: `make run-verus` reports zero errors;
  `make kani-baseline` reports no failed properties; both seeded faults
  described in V-1 and V-2 are shown to be rejected, with transcripts.
- **Conformance check**: `docs/developers-guide.md:354-393` describes the
  formal tooling entry points and states that `run-verus` fails until a proof
  file exists; that statement must be updated in EP-M6.
- **Recovery**: the proof files are outside the Cargo build. Deleting them
  restores the prior state exactly.
- **Remaining gaps**: V-2's bound of five operations, stated explicitly.
- **Compatibility decision**: none.

### EP-M3 — stage probes

- **Outcome**: four probes drive the four production stages in-process, and a
  seeded-fault variant exists for the negative control.
- **Requirements**: `ROADMAP-10.2.1-alloc`, `INVENTORY-middleware-island`,
  `ADR-008-REQ-default-path-no-fresh-vec`.
- **Interfaces**: in
  `wireframe_testing/src/codec_benchmarks/pipeline_baseline.rs`:

  ```rust
  /// One measurable stage of the default codec path.
  #[derive(Clone, Copy, Debug, PartialEq, Eq)]
  pub enum Stage {
      InboundDecode,
      MiddlewarePassThrough,
      RequestHooksReadOnly,
      RequestHooksMutating,
      OutboundEncode,
  }

  /// Prepared, reusable inputs for one stage at one payload class.
  pub struct StageFixture { /* ... */ }

  pub fn fixture(stage: Stage, class: PayloadClass)
      -> Result<StageFixture, BaselineError>;

  /// Drives the production stage exactly once.
  pub fn run_stage(fixture: &StageFixture) -> Result<(), BaselineError>;
  ```

  In `src/app/baseline_support.rs`, gated by
  `#![cfg(feature = "test-support")]`, expose thin `pub` wrappers over the
  crate-private stage entry points: `encode_message_frame`
  (`src/app/outbound_encoding.rs:22`), the decode path used by
  `parse_envelope` (`src/app/inbound_handler.rs:74`), and
  `RequestHooks::invoke_before_send_hooks`
  (`src/client/messaging.rs:335`). Each wrapper is a single delegating call
  with a doc comment stating it exists for baseline measurement and carries no
  stability guarantee.
- **Acceptance evidence**: `tests/baseline_probes.rs` passes, including the
  seeded-fault control from V-4. The middleware probe reports zero
  payload-sized allocations; the outbound-encode probe reports at least one
  allocation of at least `payload_len` bytes.
- **Conformance check**: every new `wireframe` item is behind `test-support`.
  Confirm with `cargo public-api` if available, otherwise by inspection of the
  feature gates.
- **Recovery**: probes and shims are additive; deleting
  `src/app/baseline_support.rs` and its `mod` declaration reverts cleanly.
- **Remaining gaps**: no numbers are recorded yet.
- **Compatibility decision**: none. The shims are feature-gated and the crate
  is pre-1.0.

### EP-M4 — deterministic baseline report

- **Outcome**: a rendered report of deterministic counters, snapshotted and
  cross-checked against the document, plus a `make baseline` target.
- **Requirements**: `ROADMAP-10.2.1-alloc`.
- **Interfaces**:

  ```rust
  /// One row of the deterministic baseline.
  pub struct BaselineRow {
      pub stage: Stage,
      pub payload_class: PayloadClass,
      pub counters: AllocationCounters,
  }

  /// Renders rows in a stable, sorted order as a Markdown table.
  pub fn render_report(rows: &[BaselineRow]) -> String;
  ```

  `render_report` must sort by `(stage, payload_class)` so that the output does
  not depend on collection order.
- **Acceptance evidence**: `make baseline` prints the table and exits zero;
  `tests/baseline_report.rs` passes both the snapshot assertion and the
  document-synchronization assertion; the negative control from V-5 is
  rejected.
- **Conformance check**: `insta` appears only under `[dev-dependencies]`.
- **Recovery**: delete the snapshot and re-run `cargo insta accept`.
- **Remaining gaps**: copied bytes and timings are not yet recorded.
- **Compatibility decision**: none.

### EP-M5 — copied bytes, throughput, and latency

- **Outcome**: `benches/pipeline_baseline.rs` measures throughput and latency
  for the four stages; a probe binary plus `make baseline-copy` captures
  copied-byte totals under Valgrind; both sets of figures are recorded.
- **Requirements**: `ROADMAP-10.2.1-copied-bytes`,
  `ROADMAP-10.2.1-throughput-latency`, `ADR-010-RISK-serializer-bridge`.
- **Interfaces**: a `test-support`-gated binary target
  `baseline-copy-probe` accepting `--stage <name>` and `--iterations <n>`,
  where `--stage noop` performs setup and teardown only. `make baseline-copy`
  runs each stage and the calibration under
  `valgrind --tool=dhat --mode=copy` and reports, per stage, the difference
  between the stage total and the calibration total, divided by the iteration
  count.
- **Acceptance evidence**: two consecutive `make baseline-copy` runs produce
  identical per-stage figures (V-6). The Criterion groups
  `pipeline/inbound_decode`, `pipeline/middleware_pass_through`,
  `pipeline/request_hooks`, and `pipeline/outbound_encode` appear in
  `make bench-codec` output with `Throughput::Bytes` set.
- **Conformance check**: no production code changed for the copy measurement;
  Valgrind is invoked externally.
- **Recovery**: the probe binary and bench are additive and independently
  removable. If Valgrind is unavailable the copy target fails loudly with a
  diagnostic naming the missing tool; it must not silently record zeros.
- **Remaining gaps**: copied bytes for the `Small` payload class carry a stated
  undercount caveat.
- **Compatibility decision**: none.

### EP-M6 — documentation, roadmap, and final gates

- **Outcome**: the baseline is published, the methodology is recorded as an
  ADR, the internal conventions are documented, and the roadmap is ticked.
- **Requirements**: all of the above.
- **Deliverables**:
  - `docs/frame-vec-u8-baseline.md` — the baseline itself: the deterministic
    table (byte-identical to the snapshot), the copied-byte table for the
    `Large` class with the `Small`-class caveat, the indicative timing table
    with a hardware, operating-system, and toolchain stamp, and a short section
    stating the falsifiable claim about where the default path copies today.
  - `docs/adr-011-byte-migration-baseline-methodology.md` — Status Accepted,
    following the ADR template in `docs/documentation-style-guide.md:418-495`.
    It records: the operational definition of a copied byte; why Valgrind DHAT
    copy mode was chosen and what it cannot see; why allocation counting is
    thread-local; why deterministic counters are normative and timings are not;
    and the correction that `wrap_payload` and `Bytes::from` are free on the
    default codec while `Serializer::serialize` is not.
  - `docs/developers-guide.md` — a new subsection after "Example and benchmark
    support" (line 332-352) covering the baseline module, the two make targets,
    the `insta` re-blessing procedure, and the rule that the baseline document
    is re-blessed alongside the snapshot. Update the statement at lines 376-379
    that `run-verus` is expected to fail.
  - `docs/users-guide.md` — a short note under the codec testing material
    stating that `wireframe_testing::codec_benchmarks` now exposes baseline
    probes and an allocation instrument for consumers who wish to measure their
    own codecs, and that these require the `test-support` feature.
  - `docs/contents.md` — register the new baseline document and ADR-011.
  - `CHANGELOG.md` — one bullet under `## Unreleased`.
  - `docs/roadmap.md` — mark `10.2.1` done using `mapsplice`.
- **Acceptance evidence**: `make check-fmt`, `make lint`, `make test`,
  `make markdownlint`, and `make nixie` all pass, delegated to `scrutineer`
  with logs under `/tmp`.
- **Conformance check**: every upstream identifier in `Conformance basis` maps
  to a milestone and an acceptance item; ADR-008's requirement at line 85-86
  is now measurable; ADR-010's risk at lines 189-192 now has a number attached.
- **Recovery**: documentation changes are independently revertible.
- **Remaining gaps**: item `10.2.2` will choose the threshold numbers; this
  plan deliberately does not.
- **Compatibility decision**: none.

## Concrete steps

Run everything from the repository root.

Establish the red state:

```bash
set -o pipefail
cargo test --all-targets --all-features 2>&1 \
  | tee /tmp/test-wireframe-10-2-1-red.out
```

Expected: compilation failures naming
`wireframe_testing::codec_benchmarks::alloc_probe`, which does not yet exist.

After EP-M1:

```bash
set -o pipefail
cargo test -p wireframe_testing --all-features 2>&1 \
  | tee /tmp/test-alloc-probe-10-2-1.out
```

Expected: `alloc_probe_scoping` passes, including the concurrent case.

After EP-M2:

```bash
set -o pipefail
make run-verus 2>&1 | tee /tmp/verus-wireframe-10-2-1.out
make kani-baseline 2>&1 | tee /tmp/kani-wireframe-10-2-1.out
```

Expected from Verus, approximately:

```plaintext
verification results:: 7 verified, 0 errors
```

Expected from Kani, approximately:

```plaintext
VERIFICATION:- SUCCESSFUL
```

After EP-M4:

```bash
make baseline 2>&1 | tee /tmp/baseline-wireframe-10-2-1.out
```

Expected: a Markdown table on standard output whose body matches
`tests/snapshots/baseline_report__deterministic_baseline.snap`.

After EP-M5:

```bash
make baseline-copy 2>&1 | tee /tmp/baseline-copy-wireframe-10-2-1.out
make baseline-copy 2>&1 | tee /tmp/baseline-copy-wireframe-10-2-1-b.out
diff /tmp/baseline-copy-wireframe-10-2-1.out \
     /tmp/baseline-copy-wireframe-10-2-1-b.out
```

Expected: `diff` reports no differences in the per-stage copied-byte figures.

Final gates, delegated to `scrutineer`:

```bash
set -o pipefail
make check-fmt 2>&1 | tee /tmp/check-fmt-wireframe-10-2-1.out
make lint      2>&1 | tee /tmp/lint-wireframe-10-2-1.out
make test      2>&1 | tee /tmp/test-wireframe-10-2-1.out
make markdownlint 2>&1 | tee /tmp/mdlint-wireframe-10-2-1.out
make nixie     2>&1 | tee /tmp/nixie-wireframe-10-2-1.out
```

## Validation and acceptance

Acceptance is behavioural, not structural.

- Running `make baseline` on any machine prints a table, and running it again
  prints the identical table. Running it on a different machine prints the
  identical table. If it does not, the instrument is broken.
- Running `make baseline-copy` twice on the same machine prints identical
  per-stage copied-byte figures.
- `docs/frame-vec-u8-baseline.md` states, for the `Large` payload class, a
  specific number of bytes copied during outbound encode, and that number is
  at least 65,536 — because `bincode` must copy the payload into the vector it
  allocates. If the recorded number is below the payload size, the measurement
  is wrong and the milestone fails.
- The middleware pass-through row records zero payload-sized allocations,
  confirming the inventory's "moves only" characterization
  (`docs/frame-vec-u8-inventory.md:118`). If it does not, the inventory is
  wrong and that is a finding worth recording.
- Deleting one line from `docs/frame-vec-u8-baseline.md`'s table and running
  `make test` fails with a clear diagnostic.
- Applying the seeded-fault probe variant and running `make test` fails,
  because the recorded allocated bytes increase by at least the payload size.

Red-Green-Refactor evidence is recorded per milestone in
`Artefacts and notes`, with the red transcript, the green transcript, and the
post-refactor transcript.

Quality criteria:

- Tests: `make test` passes with the new unit, property, behavioural, and
  snapshot tests included.
- Verification: V-1 discharged in Verus, V-2 in Kani, V-3 in `proptest`,
  V-4 through V-6 in `rstest` and `rstest-bdd`, each with its stated
  non-vacuity control demonstrated.
- Lint and format: `make check-fmt`, `make lint`, `make markdownlint`, and
  `make nixie` pass.
- Performance: no threshold is set by this item. Recording indicative figures
  is the deliverable; choosing thresholds is item `10.2.2`.
- Security: none applicable; no new runtime dependency and no network
  behaviour changes.

## Idempotence and recovery

Every step is re-runnable. `make baseline` and `make baseline-copy` are
read-only with respect to the working tree except for Valgrind output files,
which are written under `target/baseline/` and are covered by `.gitignore`.
Snapshot re-blessing via `cargo insta accept` is idempotent. The Verus proof
files live outside the Cargo build, so deleting them restores the prior state
without touching compilation. No step is destructive, and none requires a
backup.

## Interfaces and dependencies

New dependency: `insta`, `dev-dependencies` only, used solely for the rendered
baseline snapshot. No new runtime dependency.

External tool: Valgrind 3.17 or later, for DHAT copy mode. It is not a build
requirement; `make baseline-copy` is operator-invoked and fails with a clear
diagnostic when Valgrind is absent.

New modules:

- `wireframe_testing/src/codec_benchmarks/alloc_probe.rs`
- `wireframe_testing/src/codec_benchmarks/pipeline_baseline.rs`
- `wireframe_testing/src/codec_benchmarks/baseline_report.rs`
- `src/app/baseline_support.rs`, gated by `feature = "test-support"`
- `benches/pipeline_baseline.rs`
- `verus/wireframe_proofs.rs` and `verus/wireframe_proofs_alloc.rs`

Existing modules that this plan reuses rather than duplicates:
`wireframe_testing::codec_benchmarks::codec_benchmark_support` for
`PayloadClass`, `payload_for_class`, and `Measurement`; and
`wireframe_testing::helpers::drive` for in-process transport should an
end-to-end cross-check be wanted.

## Relevant documentation and skills

Documentation to read before starting:

- `AGENTS.md` — code style, quality gates, and Rust-specific rules.
- `docs/documentation-style-guide.md` — mandatory for every document written
  here, including the ADR template at lines 418-495.
- `docs/developers-guide.md` — quality gates (176-188), example and benchmark
  support (332-352), formal verification tooling (354-393), test
  infrastructure (394-511), and roadmap editing with `mapsplice` (512-540).
- `docs/frame-vec-u8-inventory.md` — the call-site inventory this baseline
  measures.
- ADRs 008, 009, and 010 — the approved design this baseline serves.
- `docs/rust-testing-with-rstest-fixtures.md` and
  `docs/rstest-bdd-users-guide.md` — fixture and scenario conventions.
- `docs/reliable-testing-in-rust-via-dependency-injection.md` — for keeping the
  probes injectable rather than hard-wired.
- `docs/rust-doctest-dry-guide.md` — doctest conventions for the new public
  items; note `make doctest-benchmark` enforces a runnable-doctest ratio.
- `docs/multi-layered-testing-strategy.md` and
  `docs/formal-verification-methods-in-wireframe.md` — where Kani and Verus sit
  relative to the existing Stateright-style model in
  `crates/wireframe-verification`.
- `docs/hardening-wireframe-a-guide-to-production-resilience.md`,
  `docs/generic-message-fragmentation-and-re-assembly-design.md`, and
  `docs/multi-packet-and-streaming-responses-design.md` — background on paths
  deliberately excluded from the default-path baseline.
- `docs/the-road-to-wireframe-1-0-feature-set-philosophy-and-capability-maturity.md`
  — why the 1.0 story needs these numbers.

Skills to load:

- `leta` first, for symbol navigation; prefer `leta show`, `leta refs`, and
  `leta calls` over reading whole files.
- `rust-router`, then `rust-performance-and-layout` for the measurement work
  and `rust-unit-testing` for the assertion and fixture shape.
- `kani` for the V-2 harness and `verus` for the V-1 proof; load
  `rust-verification` first if the choice between them is ever in doubt.
- `proptest` for V-3.
- `arch-decision-records` when writing ADR-011.
- `execplans` for keeping this document current.
- `en-gb-oxendict` for spelling in every document touched.
- `mapsplice` for the roadmap edit in EP-M6.
- `nextest` if test selection becomes awkward while iterating.

## Progress

- [x] (2026-08-23) Reconnaissance of the default codec path, existing
  benchmark infrastructure, ADRs 008 to 010, the inventory, and repository
  conventions.
- [x] (2026-08-23) Research into allocation and copy measurement tooling:
  Criterion baselines, `critcmp`, `dhat-rs`, `stats_alloc`, `iai-callgrind`,
  and Valgrind DHAT copy mode.
- [x] (2026-08-23) Draft plan written.
- [ ] Plan reviewed and approved.
- [ ] Stage A — confirm stage boundaries; answer the two open questions.
- [ ] Stage B — red tests.
- [ ] EP-M1 — byte-aware, thread-local allocation instrument.
- [ ] EP-M2 — Verus proof and Kani harness.
- [ ] EP-M3 — stage probes and `test-support` shims.
- [ ] EP-M4 — deterministic baseline report and snapshot.
- [ ] EP-M5 — copied bytes, throughput, and latency.
- [ ] EP-M6 — documentation, roadmap tick, and final gates.

## Surprises and discoveries

- Observation: the "final default-path `Vec<u8>` copy between serialization and
  `FrameCodec::wrap_payload`" named by the roadmap and ADR-010 is not located
  where the phrase suggests.
  Evidence: `LengthDelimitedFrameCodec::wrap_payload` is the identity function
  (`src/codec.rs:260-285`), and `Bytes::from(Vec<u8>)`
  (`src/app/outbound_encoding.rs:36`) takes ownership without copying. The
  allocation and copy occur inside `bincode` because `Serializer::serialize`
  returns `Vec<u8>` (`src/serializer.rs:61-113`).
  Impact: the baseline must attribute outbound cost to `Serializer::serialize`.
  ADR-011 records the correction so that item `11.1.2` targets the right code.

- Observation: `FrameCodec::frame_payload_bytes` has a copying default body
  that the default codec does not use.
  Evidence: `src/codec.rs:90` uses `Bytes::copy_from_slice`; the
  `LengthDelimitedFrameCodec` implementation overrides it with `frame.clone()`
  (`src/codec.rs:260-285`).
  Impact: a protocol-native codec that omits the override pays a
  payload-sized copy per frame. Out of scope here, but relevant to item
  `11.2.3`, which covers a protocol-native codec.

- Observation: the existing allocation counter cannot support the baseline as
  specified.
  Evidence: `benches/codec_performance_alloc.rs:34-77` never reads
  `layout.size()`, so it cannot report bytes, and it uses process-global
  atomics, which its own documentation at lines 79-84 acknowledges makes it
  noisy.
  Impact: EP-M1 replaces it rather than extending it.

- Observation: `docs/repository-layout.md` is referenced by `AGENTS.md:41-43`,
  the documentation style guide, and the roadmap, but does not exist.
  Evidence: no file matches `docs/repository-layout*`; only inbound references
  were found.
  Impact: out of scope for this item; recorded so it is not mistaken for a
  search failure.

- Observation: `src/app/codec_driver.rs:1-12` documents that the frame pipeline
  applies protocol hooks, but no `ProtocolHooks::before_send` invocation was
  located in `FramePipeline::process` (`:56-71`).
  Evidence: reconnaissance traced the default path from `process_stream`
  through `forward_response` to `flush_pipeline_output` without encountering
  the call.
  Impact: to be confirmed in Stage A. If the documentation is stale, raise a
  separate issue; do not fix it here.

- Observation: `insta` is not currently a dependency anywhere in the workspace,
  and no snapshot test exists.
  Evidence: no matches for `insta` in `Cargo.toml`, `Cargo.lock`, or any
  source file.
  Impact: EP-M4 introduces the first snapshot convention, documented in
  `docs/developers-guide.md`.

- Observation: no Verus proof and no Kani harness exists in the repository yet;
  `make run-verus` is documented as expected to fail.
  Evidence: `docs/developers-guide.md:376-379`; no `verus/` directory and no
  `#[kani::proof]` attribute in the tree.
  Impact: EP-M2 establishes both, which is why it carries its own tolerance and
  is separable from the measurement work.

## Decision log

- **D-1.** Decision: extend the existing benchmark support in
  `wireframe_testing::codec_benchmarks` rather than starting a parallel
  harness.
  Rationale: `docs/developers-guide.md:332-352` already directs bench targets,
  unit tests, and BDD fixtures to that module. A second harness would fragment
  the convention and duplicate the payload-class definitions.
  Date/Author: 2026-08-23, planning agent.

- **D-2.** Decision: define a copied byte as a byte passed to `memcpy`,
  `memmove`, `strcpy`, or `bcopy`, and measure it with
  `valgrind --tool=dhat --mode=copy`.
  Rationale: no definition existed anywhere in the repository. The copies that
  matter occur inside `bincode` and `tokio_util`, so no `wireframe`-owned
  counter can see them; only an external observer can. Valgrind's DHAT copy
  mode is documented for exactly this purpose (Valgrind manual, section 10.5)
  and its own documentation shows a Rust `extend_from_slice` stack, confirming
  that Rust slice copies are intercepted. Alternatives rejected: the `dhat`
  crate, whose documentation states it does not profile copy functions and
  which is self-described as experimental with low maintenance priority;
  `stats_alloc`, which gives allocation bytes but not copy bytes;
  `iai-callgrind`, which would add a benchmark harness dependency for a
  measurement that a direct Valgrind invocation already provides.
  Date/Author: 2026-08-23, planning agent.

- **D-3.** Decision: allocation counting is thread-local, not process-global.
  Rationale: the existing process-global counter is documented as noisy, and a
  baseline that varies between runs cannot support the thresholds that item
  `10.2.2` must set. Thread-local counting makes the deterministic counters
  genuinely deterministic and lets the measurement run inside the normal
  parallel test harness rather than requiring serialization.
  Date/Author: 2026-08-23, planning agent.

- **D-4.** Decision: "request hooks" means the client-side hooks in
  `src/client/hooks.rs`. Server-side `WireframeProtocol` hooks and client
  preamble replay are excluded.
  Rationale: ADR-008 lists `BeforeSendHook` among the surfaces it governs, and
  `docs/frame-vec-u8-inventory.md:200-204` groups the client hooks as the hook
  surface for this migration. Server-side protocol hooks are inert on the
  default path because `protocol` is `None`. Preamble leftovers are treated by
  the inventory (lines 212-222) as a separate compatibility surface and are
  scheduled for item `12.2.2`.
  Date/Author: 2026-08-23, planning agent.

- **D-5.** Decision: baseline the default codec only; do not include a
  protocol-native codec.
  Rationale: `docs/roadmap.md:472-474` says "on the default codec path". The
  precursor document
  `docs/zero-copy-frame-and-payload-migration-roadmap.md` item 1.2.1 mentions a
  protocol-native codec, but `docs/roadmap.md` is the governing artefact and
  item `11.2.3` is where a protocol-native codec is explicitly required. Where
  the existing workload matrix makes a Hotline figure free to collect, it is
  recorded as supplementary context and clearly marked non-normative.
  Date/Author: 2026-08-23, planning agent.

- **D-6.** Decision: deterministic counters are normative and asserted in
  tests; throughput and latency are indicative and never asserted.
  Rationale: allocation events, allocated bytes, and copied bytes are
  identical on every machine, so they can be committed and guarded. Wall-clock
  figures are hardware-dependent, and asserting on them produces flaky tests
  and false confidence. Item `10.2.2` must therefore express any timing
  threshold as a relative delta measured within a single session on a single
  machine, not as an absolute number compared across machines.
  Date/Author: 2026-08-23, planning agent.

- **D-7.** Decision: expose crate-private stage entry points through
  `test-support`-gated shims rather than re-implementing them in the probes.
  Rationale: a probe that re-implements the code it measures verifies only
  itself; a mutation in production code would leave the baseline unchanged. The
  repository already uses `test-support` for this class of widening, for
  example `src/connection/test_support.rs`.
  Date/Author: 2026-08-23, planning agent.

- **D-8.** Decision: both Verus and Kani are used, for different obligations.
  Rationale: the accounting property is over unbounded sequences, which only a
  prover can discharge, but the code that runs is `unsafe` and uses
  thread-local storage, which Verus cannot model. Kani closes that gap for
  bounded sequences against the real implementation. Using only one would leave
  either the long-sequence case or the real-code case unverified. The residual
  gap between them is stated in the verification plan rather than concealed.
  Date/Author: 2026-08-23, planning agent.

- **D-9.** Decision: adopt `insta` as a new development dependency.
  Rationale: the rendered baseline is multivariant output across stages and
  payload classes whose format consistency is the point of the artefact, which
  is exactly the case the project's testing policy reserves for snapshot
  testing. Re-blessing via `cargo insta accept` gives a clean workflow for the
  deliberate re-baselining that item `11.1.2` will require.
  Date/Author: 2026-08-23, planning agent.

## Outcomes and retrospective

To be completed at EP-M6. Before setting this plan to `COMPLETE`, reconcile
every discovery in `Surprises and discoveries` against the upstream artefacts
listed in `Conformance basis`. In particular, the correction about where the
default path actually copies (see the first discovery) affects the wording of
`docs/roadmap.md` items `10.2.2` and `11.1.2` and of ADR-010's known-risks
section. Either update those artefacts or record in this log why the existing
wording remains adequate. Do not mark the plan complete while that
reconciliation is outstanding.

## Artefacts and notes

To be populated during implementation with the red, green, and refactor
transcripts for each milestone, the Verus and Kani output, the two
`make baseline-copy` runs used to demonstrate reproducibility, and the seeded
fault transcripts that discharge the non-vacuity requirements in V-1, V-2,
V-4, and V-5.
