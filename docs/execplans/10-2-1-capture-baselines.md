# 10.2.1 Close the residual gaps in the zero-copy baseline

This execution plan (ExecPlan) is a living document. The sections `Constraints`,
`Tolerances`, `Risks`, `Progress`, `Surprises and discoveries`, `Decision log`,
`Outcomes and retrospective`, `Conformance basis`, and `Verification plan`
must be kept up to date as work proceeds.

Status: DRAFT

No `PLANS.md` exists in this repository as of 2026-11-20.

Revision 3 (2026-11-20) replaces the earlier design. Roadmap item `10.2.1` was
delivered on `main` by
[pull request 838](https://github.com/leynos/wireframe/pull/838) with a
different instrument from the one revisions 1 and 2 of this plan proposed. This
revision treats that implementation as the prescribed direction, drops
everything that contradicted it, and narrows the plan to the requirement gaps
and proof obligations that remain. The revision note at the end records what
changed.

## Purpose / big picture

The zero-copy migration replaces `Vec<u8>` payload hand-offs with shared
`bytes::Bytes` buffers. Roadmap items `10.2.2`, `11.1.1`, `11.1.2`, `11.2.3`,
`13.2.1`, and `13.2.2` all judge later work against the baseline that item
`10.2.1` captured. That baseline exists: sixteen timed rows, allocation counts
per stage, and a copied-byte gate that fails the build when the outbound path
copies more or less than it does today.

The baseline is nonetheless weaker than the thresholds that depend on it. An
audit on 2026-11-20 found that the copied-byte gate observes only the outbound
path, that one of its assertions measures the test harness and not production
code, that two of the proposed thresholds have no enforcing test at all, and
that the allocation window stays open if the measured operation panics. A
maintainer who inserts `payload.to_vec()` into a middleware today still sees a
green `make test`.

After this work a maintainer can observe the difference directly:

```bash
make test                # every invariant below holds
make test-feature-gate   # counters are inert without their feature
make unsafe-boundary     # the workspace still contains no unsafe code
```

and can demonstrate each gate by seeding the fault it guards against: a copying
middleware, a panicking measured operation, an inbound path that stops
recording, and a serializer that leaves spare capacity each make a named test
fail for the stated reason. No number in
`docs/zero-copy-benchmark-baselines.md` changes; what changes is that the
claims that document makes are enforced.

## Context and orientation

### What wireframe is

`wireframe` is a Rust library for building binary-protocol servers and clients.
A server is a `WireframeApp<S, C, E, F>`, where `S` is a serializer, `E` is a
packet (envelope) type, and `F` is a frame codec. The default instantiation is
`WireframeApp<BincodeSerializer, (), Envelope, LengthDelimitedFrameCodec>`.
`Envelope` (`src/app/envelope.rs:149-156`) owns its payload as a `Vec<u8>`.
"The default codec path" means that configuration.

### The instrument that exists

Pull request 838 delivered the following, and this plan builds on all of it.

- **Copied-byte counters.** `src/benchmark_counters.rs` defines the public
  struct `CopiedBytes` with fields `serialized`, `handoff`, and `framed`, three
  recorder functions (`record_serialized`, `record_handoff`, `record_framed`),
  `snapshot`, `reset`, and `bytes_from_vec`. The counters are process-global
  atomics that exist only with the `benchmark-counters` feature; without it
  every recorder is an empty inline function and `snapshot` reports zeros. The
  counters are *declarative*: a copy is counted only where production code
  calls a recorder. Production code records at four sites:
  `encode_message_frame` (`src/app/outbound_encoding.rs:36`), the client's
  `serialize_and_send` (`src/client/send_pipeline.rs:45`),
  `LengthDelimitedEncoder::encode` (`src/codec/mod.rs:271`), and the Hotline
  example codec (`src/codec/examples.rs:121`).
- **The copied-byte gate.** `wireframe_testing/tests/copied_bytes.rs` asserts,
  for both codecs and both payload classes, that the outbound path copies the
  payload once in the serializer and once in the framing encoder, and nowhere
  else. It runs under `make test`, because the root `Makefile` test recipe
  passes `--workspace --all-targets --all-features` and that enables the
  `pipeline-benchmarks` feature the file is gated on.
- **Allocation counting.** `wireframe_testing/src/benchmark_alloc_support/`
  wraps the third-party `stats_alloc` crate. A benchmark binary installs
  `INSTRUMENTED_SYSTEM` as its `#[global_allocator]` and calls
  `count_allocations`, which returns the number of allocation and reallocation
  operations in a window. The window is process-global. Its bookkeeping
  arithmetic lives in `window.rs` as the pure value `WindowState`, covered by
  six `proptest` properties.
- **Stage harnesses.** `wireframe_testing/src/codec_benchmarks/` holds
  `pipeline_middleware.rs`, `pipeline_hooks.rs`, and `pipeline_outbound.rs`,
  driven by `benches/pipeline_performance.rs` and
  `benches/pipeline_performance_alloc.rs`. Hook dispatch and outbound encoding
  reach production code through two `test-support` entry points marked
  `#[doc(hidden)]`: `wireframe::client::benchmark_run_before_send_hooks`
  (`src/client/hooks.rs:173-177`) and
  `wireframe::app::benchmark_encode_message_frame`
  (`src/app/outbound_encoding.rs:63`).
- **The published record.** `docs/zero-copy-benchmark-plan.md` defines the
  matrix and the measurement boundaries.
  `docs/zero-copy-benchmark-baselines.md` records the captures and proposes
  thresholds `T1` to `T7`, which roadmap item `10.2.2` keeps open until a
  performance reviewer accepts them.

### Where the default path copies payload bytes

These facts were verified against the code on 2026-11-20 and are the ground
truth the gaps are measured against. `P` is the payload length.

Outbound, the payload is copied twice. `Serializer::serialize`
(`src/serializer.rs:67-71`) returns `Vec<u8>`, so the serializer materializes
`P` bytes; roadmap item `11.1.2` removes this copy. The framing encoder then
copies the payload into the transport write buffer; nothing scheduled removes
that copy. Between the two, `Bytes::from(Vec<u8>)` and `wrap_payload` copy
nothing.

The default serializer leaves no spare capacity. `BincodeSerializer` reaches
`bincode::encode_to_vec`, which measures the encoded size first and allocates
exactly that much (`bincode-2.0.1/src/features/impl_alloc.rs:54-64`). With
`len == capacity`, `Bytes::from` takes the vector's buffer and allocates
nothing. A vector with spare capacity would instead cost one small shared
control block, still without copying. Revisions 1 and 2 of this plan asserted
that the control block is paid on every default-path frame; that was wrong and
is withdrawn.

Inbound, the payload is copied once more than the baseline records. The framed
reader yields a frame, and `parse_envelope`
(`src/app/inbound_handler/core.rs:115`) deserializes it into an `Envelope` whose
`payload` is a freshly allocated `Vec<u8>`. That is a `P`-byte copy, removable
by roadmap item `11.1.1`. No counter records it and no harness times it: the
decode benchmark stops at the codec.

Middleware and request hooks move the payload without copying it.
`forward_response` (`src/app/frame_handling/response.rs:87-88`) builds a
`ServiceRequest` from the envelope's vector, and the client's before-send loop
(`dispatch_before_send`, `src/client/hooks.rs:155-159`) lends each hook the
same vector. Nothing enforces either statement.

### The unsafe-code rule

`Cargo.toml:203-205` sets `unsafe_code = "forbid"` for every workspace package,
and every target root repeats `#![forbid(unsafe_code)]`.
`tests/workspace_unsafe_boundary.rs` and `make unsafe-boundary` enforce it.
`docs/adr-016-unsafe-code-boundary.md` records the decision and why the
workspace needs no audited unsafe crate today: the only code that ever needed
`unsafe`, a handwritten `GlobalAlloc`, was replaced by `stats_alloc`'s safe
wrapper.

The rule this plan works under is therefore: no unsafe code anywhere in the
workspace. Unsafe code is admissible only behind a protected safe interface, in
an isolated crate whose unsafe implementation is exercised under Miri (the Rust
interpreter that detects undefined behaviour). `stats_alloc` is the one such
crate the baseline depends on, and it is third-party. Revisions 1 and 2
proposed a handwritten `unsafe impl GlobalAlloc`; that is now a compile error
and is withdrawn.

### Size evidence used for the tolerances

Pull request 838 delivered the whole instrument in 3,369 inserted lines across
31 files. The runtime-ownership baselines plan
(`docs/execplans/issue-639-establish-runtime-ownership-and-task-churn-baselines.md`)
set a tolerance of 25 files and 2,500 net lines and recorded no increase. This
plan adds tests and small seams to an existing instrument, so its budgets are a
fraction of both. Revisions 1 and 2 of this plan were never executed, so they
supply no budget history.

### Terms defined

- **Seam**: a place where payload bytes cross from one owner or representation
  to another, such as serializer output becoming a codec frame.
- **Recorder**: a `record_*` function in `src/benchmark_counters.rs` that adds
  a byte count to one seam's counter.
- **Payload class**: `Small` (32 bytes) or `Large` (65,536 bytes), as defined
  in `wireframe_testing::codec_benchmarks`.
- **Payload-sized allocation**: allocator traffic of at least `P` bytes inside
  one measured operation.
- **Pointer identity**: the address returned by `as_ptr()` is the same before
  and after a hand-off, which proves the bytes were moved and not copied.
- **Seeded fault**: a deliberately wrong variant, compiled only for tests,
  that a verification must reject. It proves the verification can fail.
- **Inversion**: an assertion that is expected to flip when a named roadmap
  item lands, so its failure then signals success.

## Requirement gaps

Each gap is a place where the delivered baseline does not yet support a
requirement that an open roadmap item or a proposed threshold places on it.

- **G-1. Inbound copied bytes are unmeasured.** Roadmap item `10.2.1` names
  inbound decode among the stages needing a copied-byte baseline. `CopiedBytes`
  has no inbound field, and the `P`-byte copy in `parse_envelope` is invisible.
  Item `11.1.1` will remove that copy and nothing would show it.
- **G-2. Read-only stages have no enforced gate.** Threshold `T5` requires that
  a read-only stage add no payload-buffer allocation and no payload byte copy.
  No test asserts either for middleware or hooks. The baselines document says
  the copied-byte counters show the middleware payload is not written; no
  counter observes middleware at all.
- **G-3. The middleware row measures a replica.** `pipeline_middleware.rs`
  builds its own services over the public traits. The production dispatch seam
  in `forward_response` is not exercised by any baseline check.
- **G-4. The serializer assertion is circular.** The serialize-only pass calls
  a harness-owned `VectorSerializer` and then records its own copy
  (`pipeline_outbound.rs:149`). `copied_bytes.rs:51-54` therefore passes
  whatever production serialization does.
- **G-5. The exact-capacity claim is asserted only against the harness.** The
  hand-off result `handoff == 0` and the allocation count both depend on the
  serializer leaving no spare capacity. Only the harness serializer is checked,
  not `BincodeSerializer`.
- **G-6. The client hand-off bypasses the guard.** `serialize_and_send` calls
  `Bytes::from(bytes)` directly, so `bytes_from_vec`'s pointer comparison never
  runs on the client path.
- **G-7. Expected inversions are unregistered.** `copied_bytes.rs:51-77` and
  `:115` assert exact pre-migration totals. They must fail when item `11.1.2`
  lands, and nothing tells the implementer that failure is the success signal.
- **G-8. The allocation window is not unwind-safe.** `count_allocations`
  (`benchmark_alloc_support/mod.rs:163-184`) reopens no guard: a panic inside
  the measured operation leaves the gate open for the rest of the process. The
  pipeline and codec allocation benchmarks also lack the zero-and-positive
  self-check that the runtime-ownership benchmark runs.
- **G-9. The feature-off contract is untested.** The module documentation of
  `src/benchmark_counters.rs` says a test asserts that a copy is visible only
  with the feature enabled. No such test exists.
- **G-10. Two documents still name the wrong copy.** Roadmap item `10.2.2`
  describes "the final default-path `Vec<u8>` copy between serialization and
  `FrameCodec::wrap_payload`", an interval in which nothing copies. ADR-0010
  says item `11.1.2` removes "the remaining final outbound copy", while the
  gate itself shows the framing copy survives it.

Three weaknesses are accepted, not closed, because closing them would
contradict the delivered direction. The timing tables are recorded captures
with provenance, not generated output; that matches the repository's
runtime-ownership baseline convention. Threshold `T3`'s absolute allocation
counts stay a reviewer comparison for item `13.2.1`, not a test, because the
counts include harness plumbing. And the counters remain declarative, so a copy
into an already-allocated buffer at an unrecorded site is invisible; obligation
V-3 bounds that risk without removing it.

## Conformance basis

There is no Terms of Reference document in this repository. The upstream
artefacts, as at `origin/main` commit `7ddba322`, are:

- `docs/roadmap.md`: item `10.2.1` (lines 487-493, complete), item `10.2.2`
  (lines 494-499, open), items `11.1.1` and `11.1.2` (section 11.1), item
  `11.2.3`, and items `13.2.1` and `13.2.2`.
- `docs/zero-copy-benchmark-plan.md`, the measurement boundaries and the
  "Copied bytes" and "Gaps and follow-up work" sections.
- `docs/zero-copy-benchmark-baselines.md`, thresholds `T1` to `T7`, the
  "Copied bytes" table, and the "Correctness coupling" table.
- `docs/adrs/adr-0008-zero-copy-public-byte-container.md`,
  `docs/adrs/adr-0009-rollout-strategy-for-the-vec-u8-to-zero-copy-migration.md`,
  and
  `docs/adrs/adr-0010-transport-frame-boundary-for-the-zero-copy-migration.md`,
  all Accepted.
- `docs/adr-016-unsafe-code-boundary.md` (Proposed, enforcement live) and
  `docs/rust-boundary-verification.md`.

Trace links:

```plaintext
ROADMAP-10.2.1 (inbound copied bytes) -> G-1 -> EP-M2 -> copied_bytes::inbound_decode_copies_the_payload_once
BASELINES-T5 (read-only stages)       -> G-2, G-3 -> EP-M3 -> pipeline_allocation_invariants, forward_response pointer test
BASELINES-T4, T6 (outbound total)     -> G-4, G-5, G-7 -> EP-M2, EP-M4 -> copied_bytes production cases, inversion register
BENCH-PLAN copied-byte seam 2         -> G-6 -> EP-M2 -> client hand-off through bytes_from_vec
BENCH-PLAN allocation window          -> G-8 -> EP-M1 -> count_allocations unwind test, harness self-check
BENCHMARK-COUNTERS module contract    -> G-9 -> EP-M4 -> tests/feature_gate counter probe
ROADMAP-10.2.2, ADR-0010 wording      -> G-10 -> EP-M4 -> roadmap edit, ADR-0010 dated addendum
ADR-016 (no unsafe code)              -> all milestones -> make unsafe-boundary
```

## Constraints

- No unsafe code. No file in the workspace may contain `unsafe`, and every new
  target root begins with `#![forbid(unsafe_code)]`. Allocation is observed
  only through `stats_alloc`'s safe `StatsAlloc<System>` wrapper. If a gap
  cannot be closed without unsafe code, stop: the only admissible route is an
  isolated crate with a protected safe interface and Miri coverage, which
  requires revisiting ADR-016 and is outside this plan.
- Keep the delivered direction. Copied bytes are counted by recorders at
  production seams and gated by `wireframe_testing/tests/copied_bytes.rs`.
  Allocations are counted by `benchmark_alloc_support`. Do not introduce a
  second instrument, a Valgrind harness, a report generator, or a new baseline
  document.
- Do not change production behaviour. Recorder calls compile to nothing
  without `benchmark-counters`. No wire format, public signature outside the
  two items named in `Interfaces and dependencies`, or default-feature surface
  may change.
- Do not make a benchmark result a correctness gate. No test asserts a timing,
  a throughput, or an absolute allocation count that includes harness plumbing.
  Tests assert pointer identity, exact copied-byte counts at recorded seams
  (which are deterministic), and relations between allocation measurements.
- Do not re-take or edit the recorded timing captures. This plan adds no
  timed row and changes no timed region.
- Accepted ADR sections are immutable. Correct ADR-0010 with a dated addendum
  that names the sentence it supersedes, as its existing addenda do.
- `typos.toml` is generated. If `make spelling` regenerates it for reasons
  unrelated to this work, land that in a separate base pull request.
- Follow Red-Green-Refactor: each new assertion is committed failing, or shown
  failing against its seeded fault, before the change that satisfies it.
- Run gates sequentially with `set -o pipefail` and `tee` to `/tmp`, and
  delegate full gate runs to the `scrutineer` sub-agent.

## Tolerances (exception triggers)

The comparison base is `origin/main` at the commit the implementation branch
starts from. Counts are net lines (`git diff --numstat`, additions minus
deletions). ExecPlan files are excluded from every limit. Each category is
enforced independently.

- **Code** (Rust sources, tests, benches): 900 net lines across at most 16
  files. The instrument cost 3,369 lines; this work adds tests and four small
  seams to it, with a third held as contingency.
- **Configuration** (`Cargo.toml` manifests, `Cargo.lock`, CI workflows): 40
  net lines. Expected use is test-target declarations and one feature list.
- **Documentation** (everything under `docs/` except ExecPlans, plus
  `CHANGELOG.md`): 300 net lines.
- **Interface**: the only permitted public changes are a fourth field on
  `CopiedBytes`, one new recorder, and one new `#[doc(hidden)]` `test-support`
  entry point for inbound decode. Any other public change is an escalation.
- **Dependencies**: no new crate in any manifest.
- **Unsafe**: any need for unsafe code, a scoped lint expectation on
  `unsafe_code`, or a change to workspace membership is an escalation.
- **Iterations**: the same gate failing three times after three distinct
  fixes is an escalation.
- **Noise**: if the allocation invariants cannot be made deterministic under
  the test harness within the Stage A decision rule, stop and present the
  options instead of widening a bound until it passes.
- **Scope**: if closing a gap requires changing a recorded capture or a
  proposed threshold's value, stop; that is a decision for the performance
  reviewer under item `10.2.2`.

## Risks

- Risk: process-global allocator statistics pick up traffic from the test
  harness's other threads, making allocation invariants flaky. Severity: high.
  Likelihood: medium. Mitigation: Stage A measures the idle-window noise floor
  under the default harness before any invariant is written. The decision rule
  is fixed in advance: a single non-zero idle window in 1,000 trials moves the
  invariant tests to a `harness = false` target with one thread. Every bound is
  also a relation with at least a 4x margin against the 64 KiB payload class.

- Risk: recording the inbound copy at `parse_envelope` double-counts or misses
  a path, because the function has a metadata branch and a fallback branch.
  Severity: medium. Likelihood: medium. Mitigation: record once, after the
  function has produced its envelope, from the envelope's own payload length.
  The gate asserts exactly `P` for both payload classes and both branches
  reachable from the default serializer.

- Risk: `stats_alloc` attributes reallocation bytes differently from the
  assumption in axiom A-2, so the byte bounds are wrong. Severity: medium.
  Likelihood: low. Mitigation: obligation V-2 tests the wrapper at its real
  boundary with known-size allocations and a growing vector before any stage
  invariant depends on it.

- Risk: the production-dispatch pointer test (gap G-3) cannot reach
  `forward_response` without heavy fixture work, because that function now runs
  handlers under the connection actor. Severity: medium. Likelihood: medium.
  Mitigation: Stage A determines the smallest in-crate fixture. If it needs
  more than 150 lines of new fixture code, fall back to asserting pointer
  identity across `ServiceRequest::new` and `HandlerService::call` alone,
  record the narrower scope, and leave the remainder to item `13.2.2`.

- Risk: adding a field to `CopiedBytes` breaks a struct literal somewhere.
  Severity: low. Likelihood: certain. Mitigation: 14 literals exist across
  `src/` and `wireframe_testing/`; update them in the same commit. The type is
  unreleased, so no compatibility shim is owed.

- Risk: the inversion register drifts from the tests it names.
  Severity: medium. Likelihood: high over the life of the migration.
  Mitigation: obligation V-7 makes a missing test name a failing test.

## Verification plan

The work introduces one piece of new control flow, the unwind guard on the
allocation window, and otherwise consists of assertions about existing
production behaviour. The obligations below are therefore mostly about the
*verifications themselves*: each must be shown able to fail.

### Abstract model

One outbound message of payload length `P` passes four seams in order:
serializer materialization, vector-to-`Bytes` hand-off, frame construction, and
transport framing. One inbound message passes two: transport read and envelope
deserialization. Between them sit the read-only stages, middleware and hooks.
The baseline claims a copied-byte count at each seam and a no-payload-traffic
property at each read-only stage.

### Axioms

Trusted, not verified here:

- **A-1.** `bincode::encode_to_vec` returns a vector with `len == capacity`.
  Its source computes the size before allocating. V-5 checks the consequence at
  the repository's boundary.
- **A-2.** `stats_alloc::Stats::bytes_allocated` increases by the requested
  size on every allocation and by the growth on every growing reallocation. V-2
  checks this against the real wrapper.
- **A-3.** `bytes::Bytes::from(Vec<u8>)` never copies the payload.
  `bytes_from_vec` already guards it at run time by comparing pointers.
- **A-4.** The system allocator and `stats_alloc`'s own unsafe implementation
  are correct. They are third-party; the workspace contains no unsafe code for
  Miri to interpret.

### Obligation V-1: the allocation window closes on every exit

- **Invariant**: after `count_allocations` returns or unwinds,
  `is_allocation_counting_enabled()` is false.
- **Method**: `rstest` cases for success, error, and panic, the last through
  `std::panic::catch_unwind`. The transition arithmetic is already covered by
  the `WindowState` properties and is not re-verified.
- **Artefact**: the test module of
  `wireframe_testing/src/benchmark_alloc_support/mod.rs`.
- **Non-vacuity**: the panic case is written first and fails against the
  current implementation, which is the red step. It is the seeded fault.
- **Discharge**: the three cases pass, and both allocation benchmarks call a
  shared self-check that asserts an idle window reports zero and a 1,024-byte
  allocation reports a positive count.

### Obligation V-2: byte accounting matches the allocator at its boundary

- **Invariant**: for a window containing one allocation of `n` bytes, the
  reported allocated bytes lie in `[n, n + slack]`; for a vector grown from `n`
  to `2n`, at least `2n`.
- **Method**: `rstest` parameterized over sizes 32, 1,024, and 65,536, in a
  test binary that installs `INSTRUMENTED_SYSTEM`.
- **Artefact**: `wireframe_testing/tests/pipeline_allocation_invariants.rs`.
- **Non-vacuity**: an idle window must report zero bytes, proving the window
  does not merely report a large constant.
- **Residual gap**: the window is process-global. Stage A's noise measurement
  decides whether the binary may use the default harness.

### Obligation V-3: every recorded seam reports production behaviour

- **Invariant**: on the default path, for both payload classes, one inbound
  message records exactly `P` deserialized bytes; one outbound message driven
  through `benchmark_encode_message_frame` with `BincodeSerializer` and an
  `Envelope` records `serialized` equal to the encoded length, which is at least
  `P`, and `handoff == 0`; and the client send path reaches `bytes_from_vec`.
- **Method**: `rstest` cases added to
  `wireframe_testing/tests/copied_bytes.rs`, all `#[serial]`.
- **Independent oracle**: the same outbound operation, measured by V-2's byte
  accounting, shows at least `P` and fewer than `2P` allocated bytes. A
  recorder that fired without a real materialization, or a materialization with
  no recorder, breaks the agreement between the two instruments.
- **Non-vacuity**: gap G-9's probe shows the same operations report zero
  without the feature, so the assertions depend on the recorders. The existing
  harness-serializer case is kept and renamed to say what it measures.
- **Residual gap**: a copy into an existing buffer at an unrecorded site is
  seen by neither instrument. The seam inventory in
  `docs/frame-vec-u8-inventory.md` is the only control, and item `11.2.3` owns
  extending it.

### Obligation V-4: read-only stages carry no payload traffic

- **Invariant**: for middleware pass-through and read-only hook dispatch, the
  payload buffer's pointer is unchanged across the stage; the allocation count
  for a batch is equal across the two payload classes; and the allocated bytes
  for a `Large` batch are below `P`.
- **Method**: pointer-identity assertions, modelled on
  `src/codec/tests/mod.rs:72-88`, plus V-2's accounting, parameterized over
  codec, payload class, and access mode. An in-crate test asserts the same
  identity across the production dispatch seam.
- **Artefact**: `wireframe_testing/tests/pipeline_allocation_invariants.rs`
  and a test in `src/app/frame_handling/`.
- **Non-vacuity**: a test-only `CopyingPolicy` that replaces the frame with
  `frame.to_vec()` must fail both the pointer assertion and the byte bound. The
  count-equality relation alone does *not* catch that fault, because the extra
  allocation occurs in both classes; the test records that as the reason the
  byte bound exists. A mutating hook must be observed to change the buffer,
  proving the hook row runs.
- **Rationale**: this is `T5` stated as a relation between measurements, so it
  survives dependency churn that an absolute count would not.

### Obligation V-5: the default serializer leaves no spare capacity

- **Invariant**: `BincodeSerializer::serialize` of an `Envelope` returns a
  vector with `len == capacity`, and handing it to `bytes_from_vec` allocates
  nothing.
- **Method**: `rstest` over both payload classes.
- **Non-vacuity**: a control vector built with spare capacity must show one
  additional allocation with the pointer still equal. That proves the test
  distinguishes the two cases, and it documents the control block correctly.
- **Inversion**: none. A `Bytes`-native serializer under item `11.1.2` makes
  the hand-off disappear, and the test is then deleted with it.

### Obligation V-6: the pre-migration outbound state is stated once

- **Invariant**: between serialization and `wrap_payload` there is exactly one
  payload-sized allocation and the outbound total is `2P`.
- **Method**: the existing `copied_bytes.rs` assertions, with messages that
  cite thresholds `T4` and `T6` and the inversion register.
- **Discharge**: every assertion that item `11.1.1` or `11.1.2` is expected to
  flip is listed in the register with the item that flips it.

### Obligation V-7: the inversion register names real tests

- **Invariant**: every test function named in the register section of
  `docs/zero-copy-benchmark-baselines.md` exists in the file the register gives
  for it.
- **Method**: a test reads both files with `include_str!`, which avoids the
  filesystem lints and makes deletion of the document a compile error, and
  checks each name.
- **Non-vacuity**: a unit test of the name-extraction helper against a fixture
  that names a missing function must fail.

### Obligation V-8: the workspace still contains no unsafe code

- **Invariant**: the change adds no unsafe syntax and no target root without
  the prohibition.
- **Method**: the existing `tests/workspace_unsafe_boundary.rs` and
  `make unsafe-boundary`.
- **Non-vacuity**: those checks carry their own probes; this plan adds none.

### Formal verification decision

No Kani harness, Verus proof, or Miri run is planned, and each omission is a
finding, not a default.

Kani and Verus apply to invariants over all admissible inputs. The only new
logic here is a guard that restores one boolean, and the only arithmetic is
`WindowState`, which six existing properties already cover. Every other
obligation is a statement about what production code does to one buffer, which
a prover cannot establish without modelling `bincode`, `bytes`, and
`tokio-util`; those are axioms. The `make kani` and `make verus` targets are
stubs until roadmap items `15.3.1` and `15.5.2`, and nothing here justifies
bringing them forward.

Miri interprets unsafe code. The workspace has none, so there is nothing for it
to check, as ADR-016 records. If a later revision needed unsafe code, Miri
coverage of the isolated crate would become a mandatory obligation and this
section would have to change before that work began.

### Suggested BDD feature

The repository runs `rstest-bdd` scenarios from `tests/features/`. The gate's
observable behaviour is worth one feature, at
`tests/features/zero_copy_baseline.feature`, linked to G-1, G-2, and EP-M3:

```gherkin
Feature: Zero-copy baseline gate
  Scenario: A pass-through middleware leaves the payload in place
    Given a default-path envelope carrying a large payload
    When the envelope passes through a read-only middleware chain
    Then the payload buffer is the one the envelope arrived with
    And no payload-sized allocation is observed

  Scenario: A copying middleware is detected
    Given a default-path envelope carrying a large payload
    When the envelope passes through a middleware that copies its frame
    Then the payload buffer differs from the one the envelope arrived with

  Scenario: Inbound decode copies the payload exactly once
    Given a framed default-path message carrying a large payload
    When the message is decoded into an envelope
    Then the deserialized copied-byte count equals the payload length
```

## Plan of work

### Stage A: confirm and decide (no production change)

Re-verify the facts under "Where the default path copies payload bytes" against
the tree the branch starts from. Then answer three questions and record each
answer in `Surprises and discoveries`.

First, the noise floor. Write a throwaway test that installs
`INSTRUMENTED_SYSTEM`, opens 1,000 idle windows under the default test harness,
and reports how many were non-zero. Zero keeps the default harness; anything
else selects `harness = false`.

Second, the inbound recording site. Confirm that every default-path inbound
envelope leaves `parse_envelope` through one return, and note which branch the
default serializer takes.

Third, the production-dispatch fixture. Find the smallest existing in-crate
fixture that drives `forward_response` or the handler service with an
`Envelope`, and measure it against the 150-line risk threshold.

### Stage B: red tests

For each gap, add the assertion and show it failing: against the current code
where the behaviour is missing (G-1, G-6, G-8, G-9), and against its seeded
fault where the behaviour already holds (G-2, G-3, G-5).

### Stage C: implementation

Make the smallest change that turns each red test green, milestone by milestone.

### Stage D: documentation and closure

Update the published record, correct the two documents under G-10, and run
every gate.

## Milestones and plateaus

### EP-M1: a trustworthy allocation window (G-8)

- **Outcome**: `count_allocations` closes its window on unwind through a drop
  guard, and exposes allocated bytes beside the operation count. A shared
  `verify_allocation_harness` self-check, lifted from
  `benches/runtime_ownership_baselines_alloc/probes.rs:147`, runs at the start
  of `benches/pipeline_performance_alloc.rs` and
  `benches/codec_performance_alloc.rs`.
- **Interface**: a `count_allocation_traffic` helper returning a small struct
  with `operations` and `bytes`, built on the same window. `count_allocations`
  keeps its signature and delegates to it.
- **Acceptance evidence**: V-1 and V-2 pass. `make bench-codec` still emits
  the same `_allocs_<n>` labels with the same counts.
- **Conformance check**: no `wireframe` change; no unsafe code; the existing
  `WindowState` properties still pass unmodified.
- **Recovery**: additive within one module; revert the commit.
- **Remaining gaps**: G-1 to G-7, G-9, G-10.

### EP-M2: seams that report production behaviour (G-1, G-4, G-5, G-6)

- **Outcome**: `CopiedBytes` gains a `deserialized` field with a
  `record_deserialized` recorder, called once in `parse_envelope`. A
  `#[doc(hidden)]`, `test-support`-gated `benchmark_decode_envelope` entry
  point lets the harness reach it, mirroring the two existing entry points.
  `serialize_and_send` hands off through `bytes_from_vec`. `copied_bytes.rs`
  gains inbound cases and production-serializer cases, and the existing
  serialize-only case is renamed to state that it measures the harness
  serializer.
- **Acceptance evidence**: V-3 and V-5 pass, including the allocation oracle
  and the spare-capacity control.
- **Conformance check**: the wire format and every default-feature signature
  are unchanged; `CopiedBytes::total` includes the new field and its doctest is
  updated; the outbound total asserted by the existing gate is still `2P`,
  because inbound and outbound are measured separately.
- **Recovery**: the recorder and the hand-off change revert independently of
  the tests.
- **Remaining gaps**: G-2, G-3, G-7, G-9, G-10.

### EP-M3: enforced read-only stages (G-2, G-3)

- **Outcome**: `wireframe_testing/tests/pipeline_allocation_invariants.rs`
  asserts V-4 for middleware and hooks across the workload matrix. An in-crate
  test asserts pointer identity across the production dispatch seam. The BDD
  feature above is wired in. The misleading test name
  `middleware_passthrough_recycles_the_frame_buffer` is replaced by a test that
  asserts recycling by pointer.
- **Acceptance evidence**: V-4 passes, and the `CopyingPolicy` control fails
  for the two stated reasons.
- **Conformance check**: no timed region changed, so no capture is re-taken.
- **Recovery**: tests only, apart from any fixture Stage A selects.
- **Remaining gaps**: G-7, G-9, G-10.

### EP-M4: the register and the record (G-7, G-9, G-10)

- **Outcome**: `docs/zero-copy-benchmark-baselines.md` gains an "Inversion
  register" section, an inbound row in the copied-byte table, and corrected
  wording about what observes middleware; thresholds `T4` to `T6` cite the
  tests that now enforce them. `docs/zero-copy-benchmark-plan.md` describes the
  fourth seam and closes its own gap table rows. `tests/feature_gate` gains a
  probe that calls a recorder without the feature and observes zeros. Roadmap
  item `10.2.2` is reworded to name the serializer materialization, and
  ADR-0010 gains a dated addendum. `docs/developers-guide.md` and
  `CHANGELOG.md` are updated.
- **Acceptance evidence**: V-6, V-7, and V-8 pass; `make test-feature-gate`
  passes; `make markdownlint` and `make nixie` pass.
- **Conformance check**: no threshold value changed; ADR-0010's accepted
  sections are untouched.
- **Recovery**: documentation and one out-of-workspace probe.
- **Remaining gaps**: none within this plan. The three accepted weaknesses
  remain as recorded.

## Concrete steps

Run from the repository root.

```bash
git switch -c 10-2-1-baseline-gaps origin/main
set -o pipefail
make test 2>&1 | tee /tmp/test-wireframe-10-2-1-baseline-gaps.out
```

For each milestone, add the red test, confirm the expected failure, implement,
and re-run the focused target before the full gates:

```bash
cargo test -p wireframe_testing --all-features --test copied_bytes
cargo test -p wireframe_testing --all-features --test pipeline_allocation_invariants
make test-feature-gate
make unsafe-boundary
```

Then run `make check-fmt`, `make lint`, `make test`, `make markdownlint`, and
`make nixie` in sequence through `scrutineer`, and commit.

## Validation and acceptance

Acceptance is behavioural, and each line names the fault that proves the gate
is live.

- `make test` passes.
- Replacing the pass-through policy with `CopyingPolicy` fails the pointer
  assertion and the byte bound in `pipeline_allocation_invariants`.
- Removing the `record_deserialized` call fails
  `inbound_decode_copies_the_payload_once` with a count of zero.
- A measured operation that panics leaves
  `is_allocation_counting_enabled()` false.
- A serializer that returns spare capacity fails the exact-capacity case and
  shows one extra allocation in the control.
- Naming a non-existent test in the inversion register fails the register
  test.
- `make test-feature-gate` passes, showing zeros without the feature.
- `make unsafe-boundary` passes.

Quality criteria: `make check-fmt`, `make lint`, `make typecheck`, `make test`,
`make markdownlint`, and `make nixie` all pass. No performance threshold is set
or changed by this plan.

## Idempotence and recovery

Every step is additive or a local edit and can be re-run. Each milestone is one
or more self-contained commits that revert cleanly. The counters and the
allocation window are process-global, so a failed test can leave them dirty
only within its own process; no persistent state exists.

## Interfaces and dependencies

No new crate. `stats_alloc`, `serial_test`, `proptest`, and `rstest` are
already dependencies of `wireframe_testing`.

Public changes, all unreleased surfaces introduced by pull request 838, so no
compatibility machinery is owed:

- `wireframe::benchmark_counters::CopiedBytes` gains
  `pub deserialized: u64`.
- `wireframe::benchmark_counters::record_deserialized(bytes: usize)`.
- `wireframe::app::benchmark_decode_envelope`, `#[doc(hidden)]` and gated on
  `test-support`.

New files: `wireframe_testing/tests/pipeline_allocation_invariants.rs`,
`tests/features/zero_copy_baseline.feature` with its steps, and one probe
source under `tests/feature_gate/src/`.

## Relevant documentation and skills

Read before starting: `AGENTS.md`; `docs/zero-copy-benchmark-plan.md`;
`docs/zero-copy-benchmark-baselines.md`; `docs/adr-016-unsafe-code-boundary.md`
and `docs/rust-boundary-verification.md`; ADRs 0008, 0009, and 0010 under
`docs/adrs/`; `docs/frame-vec-u8-inventory.md`; and
`docs/documentation-style-guide.md`.

Skills: `execplans`, `rust-router`, `rust-unit-testing`,
`rust-performance-and-layout`, `rust-verification`, `proptest`, and `mapsplice`
for the roadmap edit.

## Progress

- [x] (2026-08-23) Revisions 1 and 2 drafted and reviewed; never implemented.
- [x] (2026-11-19) Rebased onto `main`; found item `10.2.1` delivered by pull
  request 838.
- [x] (2026-11-20) Audited the delivered implementation against this plan and
  against thresholds `T1` to `T7`; rewrote the plan as revision 3.
- [ ] Plan approved.
- [ ] Stage A: three questions answered and recorded.
- [ ] EP-M1: allocation window.
- [ ] EP-M2: production seams.
- [ ] EP-M3: read-only stages.
- [ ] EP-M4: register and record.

## Surprises and discoveries

- Observation: roadmap item `10.2.1` was delivered while this plan was a
  draft, by a different instrument. Evidence: commit `5e749c9f` (pull request
  838). Impact: revision 3 adopts that instrument and plans only its gaps.

- Observation: the workspace forbids unsafe code outright.
  Evidence: `Cargo.toml:203-205`; `docs/adr-016-unsafe-code-boundary.md`; commit
  `96399574`. Impact: the handwritten allocator in revisions 1 and 2 cannot
  compile. Allocation is observed through `stats_alloc`, whose `Stats` also
  exposes byte totals, which is what makes obligation V-4's byte bound possible
  without unsafe code.

- Observation: revisions 1 and 2 were wrong about the `Bytes` control block.
  Evidence: `bincode-2.0.1/src/features/impl_alloc.rs:54-64` sizes the output
  exactly, so `len == capacity` on the default path and `Bytes::from` allocates
  nothing. Impact: the claim is withdrawn, and obligation V-5 now asserts the
  true behaviour with the spare-capacity case as its control.

- Observation: the copied-byte gate's serializer assertion is circular.
  Evidence: `wireframe_testing/src/codec_benchmarks/pipeline_outbound.rs:149`
  records the copy that `copied_bytes.rs:51-54` then asserts. Impact: gap G-4.

- Observation: the inbound deserialization copy is absent from the baseline.
  Evidence: `CopiedBytes` has no inbound field; `parse_envelope`
  (`src/app/inbound_handler/core.rs:115`) returns an `Envelope` owning a new
  `Vec<u8>`. Impact: gap G-1, the only gap that leaves a named `10.2.1` stage
  without a copied-byte figure.

- Observation: count equality across payload classes cannot detect a copying
  middleware. Evidence: a `to_vec()` adds one allocation in both classes, so
  the counts stay equal. Impact: threshold `T5`'s wording about the count not
  growing with payload class is necessary but not sufficient; V-4 adds the byte
  bound.

## Decision log

- **D-1.** Adopt the delivered instrument and plan only its gaps. The
  implementation on `main` is treated as directionally correct; this plan
  introduces no competing design. 2026-11-20, at the user's direction.

- **D-2.** No unsafe code, and no route to it inside this plan. A need for
  unsafe code is an escalation that must go through an isolated crate with a
  safe interface and Miri coverage, and through ADR-016. 2026-11-20, at the
  user's direction.

- **D-3.** Enforce `T5` with pointer identity and relations between
  allocation measurements, not absolute counts. Absolute counts include harness
  plumbing and move with dependencies; the issue 639 plan sets the same rule
  for benchmark results. 2026-11-20.

- **D-4.** Count the inbound copy with a fourth recorder at `parse_envelope`,
  not with a separate instrument. It matches how the three outbound seams are
  counted, and it gives item `11.1.1` a figure to drive to zero. 2026-11-20.

- **D-5.** Keep the recorded timing tables hand-maintained with provenance.
  Revision 2's generated-document and snapshot obligations are dropped: they
  contradicted the repository's baseline-document convention, and the figures
  that matter for correctness are asserted by tests, not read from a table.
  2026-11-20.

- **D-6.** Drop the Valgrind copied-byte harness. The baselines document
  states that the copy-site counters are the sole copied-byte gate. The
  completeness risk it would have addressed is bounded by V-3's allocation
  oracle and recorded as a residual gap. 2026-11-20.

- **D-7.** Keep exact copied-byte equalities in tests, with a register. They
  are deterministic, so the concern that led revision 2 to forbid asserted
  numbers does not apply; what was missing was a record of which equalities are
  meant to break. 2026-11-20.

- **D-8.** No Kani, Verus, or Miri work. Reasons are in the verification
  plan's formal verification decision. 2026-11-20.

## Outcomes and retrospective

Not yet started. To be completed at each milestone and at the end.

## Artefacts and notes

To be filled during implementation: the Stage A noise-floor result; the red and
green transcripts for each seeded fault; and the final gate logs.

## Revision note

**Revision 3 (2026-11-20)** rewrites the plan around the implementation that
pull request 838 delivered.

What changed, and why:

- **The instrument is no longer this plan's to design.** The probe allocator,
  the Valgrind harness, the report renderer, the generated baseline document,
  and the proposed ADR are removed. `main` chose seam counters and
  `stats_alloc`, and that choice is treated as correct.
- **Unsafe code is excluded.** The workspace forbids it; the earlier
  allocator design is withdrawn, and the constraint states the only route by
  which unsafe code could ever be admitted.
- **One factual claim is withdrawn.** The default serializer leaves no spare
  capacity, so the `Bytes` control block is not paid on the default path.
- **The plan now targets ten requirement gaps**, found by auditing the
  delivered baseline against roadmap item `10.2.1` and thresholds `T4` to `T6`,
  and accepts three weaknesses explicitly.
- **The proof obligations were re-derived for the delivered direction.**
  Six of the earlier obligations had no subject left. The eight that replace
  them each name the seeded fault that shows the verification can fail.
- **Budgets are per category** and sized from the cost of the delivered
  instrument.

Effect on remaining work: nothing in revisions 1 or 2 carries forward as an
implementation task. The work is four small milestones that add enforcement to
an existing baseline, and it can start as soon as the plan is approved.
