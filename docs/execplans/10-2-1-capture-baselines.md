# 10.2.1 Capture allocation, copied-byte, throughput, and latency baselines

This execution plan (ExecPlan) is a living document. The sections
`Constraints`, `Tolerances`, `Risks`, `Progress`, `Surprises and discoveries`,
`Decision log`, `Outcomes and retrospective`, `Conformance basis`, and
`Verification plan` must be kept up to date as work proceeds.

Status: DRAFT

No `PLANS.md` exists in this repository as of 2026-08-23.

Revision 2 (2026-08-23) follows a six-lens design review. The changes are
summarized in the revision note at the end of this document; the short version
is that the plan now asserts invariants and merely records numbers, rather than
asserting numbers.

## Purpose / big picture

Roadmap item `10.2.1` is the measurement gate that the rest of the
`Frame = Vec<u8>` migration depends on. Items `10.2.2`, `11.1.2`, `11.2.3`, and
`13.2.1` all compare later work against evidence that does not exist yet.
Without it, "remove the final default-path copy" is an assertion nobody can
falsify.

This plan produces two different kinds of artefact, and keeping them apart is
the single most important design decision in it.

**Invariants** state what is true about the default codec path today, in a form
that survives a dependency bump and inverts exactly when the migration lands.
For example: "outbound encode performs at least one allocation of at least the
payload size." These are asserted by tests.

**Measurements** are the concrete numbers — allocation counts, allocated bytes,
copied bytes, nanoseconds. These are recorded with a full environment stamp and
are explicitly *not* asserted, because they are properties of `bincode`,
`bytes`, `tokio-util`, `libstd`, the toolchain, and the optimization profile as
much as of `wireframe`.

After this work a maintainer can observe success three ways:

```bash
make test            # invariants hold; format of the report is stable
make baseline        # regenerates the recorded numbers into the document
make baseline-copy   # captures copied bytes under Valgrind
```

and `docs/frame-vec-u8-baseline.md` contains a generated, environment-stamped
record covering inbound decode, middleware pass-through, request hooks, and
outbound encode, together with a named, falsifiable claim about where the
default path copies payload bytes today.

## Context and orientation

This section assumes no prior knowledge of the repository.

### What wireframe is

`wireframe` is a Rust library for building servers and clients that speak
custom binary protocols. A server application is a `WireframeApp<S, C, E, F>`
where `S` is a serializer, `E` is a packet (envelope) type, and `F` is a frame
codec. The default instantiation (`src/app/builder/core.rs:29-34`) is:

```rust
WireframeApp<BincodeSerializer, (), Envelope, LengthDelimitedFrameCodec>
```

Its `Default` implementation (`src/app/builder/core.rs:52-80`) leaves
`middleware` empty and `protocol`, `fragmentation`, and `message_assembler` all
`None`. Throughout this plan, "the default codec path" means exactly this
configuration.

### The four stages

**Inbound decode.** `process_stream` (`src/app/inbound_handler.rs:158-226`)
wraps the transport in a `tokio_util::codec::Framed` built from `CombinedCodec`
(`src/app/combined_codec.rs:8-18`). Frames reach `handle_frame` (`:228-276`),
then `build_dispatchable_envelope` (`:278-322`), then `parse_envelope`
(`:74`), which calls `BincodeSerializer::parse` (`src/serializer.rs:150-157`)
and thence `bincode`. The `Vec<u8>` that becomes `Envelope.payload` is
allocated inside `bincode`, not inside `wireframe`; it is observable only
through allocator instrumentation.

**Middleware pass-through.** `forward_response`
(`src/app/frame_handling/response.rs:29-63`) builds
`ServiceRequest::new(env.payload, env.correlation_id)` at line 41, calls
`service.call(request)` at line 42, and rebuilds the packet from
`resp.into_inner()` at line 55. The payload itself is moved, never copied. The
stage is not free, however: `Service` is declared `#[async_trait]`
(`src/middleware.rs:168-169`), so every `call` boxes a future.

**Request hooks.** The client-side hooks in `src/client/hooks.rs`:
`BeforeSendHook = Arc<dyn Fn(&mut Vec<u8>) + Send + Sync>` (line 152) and
`AfterReceiveHook` (line 176), held in the `pub(crate)` struct `RequestHooks`
(lines 184-189). They are invoked by
`WireframeClient::invoke_before_send_hooks` (`src/client/messaging.rs:335`,
inside the `impl<S, T, C> WireframeClient<S, T, C>` block opened at line 27) —
a method on the client, not on `RequestHooks`.

On the default path `RequestHooks` is two empty vectors, so the stage is a loop
over nothing. Measuring it therefore requires synthetic hooks, and the row
records a *hook-shape cost model*, not a default-path figure. The plan says so
in the document rather than pretending otherwise.

**Outbound encode.** `encode_message_frame`
(`src/app/outbound_encoding.rs:22-38`), called from `send_envelope`
(`src/app/codec_driver.rs:132`) and `flush_pipeline_output` (`:164-180`).

### Where the default path actually copies

The roadmap and ADR-010 describe "the final default-path `Vec<u8>` copy between
serialization and `FrameCodec::wrap_payload`". Investigation shows that phrase
names two operations that do not copy, and misses two costs that do. Getting
this right is the highest-value output of this item, because item `11.1.2` is
currently pointed at code that is already free.

What does **not** copy:

- `LengthDelimitedFrameCodec::wrap_payload` (`src/codec.rs:283`) is the identity
  function; `type Frame = Bytes` (`:262`).
- `Bytes::from(Vec<u8>)` (`src/app/outbound_encoding.rs:36`) takes ownership of
  the vector's buffer rather than copying it.

What does cost, in order along the outbound path:

1. **`Serializer::serialize` allocates and copies.** Its signature returns
   `Vec<u8>` (`src/serializer.rs:67-71`), so `bincode` must allocate a vector
   and copy the envelope payload into it. This is the cost item `11.1.2` can
   remove by giving the serializer a `Bytes`-native contract.
2. **`Bytes::from(Vec<u8>)` allocates, even though it does not copy.** When
   `len != capacity` it heap-allocates a 24-byte `Shared` control block
   (`bytes-1.12.1/src/bytes.rs:947-967`). Because `bincode` grows its output
   vector amortized, `len != capacity` is the normal case, so the default path
   pays this on every outbound frame. A `Bytes`-native serializer that produces
   a right-sized buffer removes it; one that still hands over an over-capacity
   `Vec` does not.
3. **The framed encoder copies the whole payload again, and item `11.1.2`
   cannot remove it.** `LengthDelimitedEncoder::encode`
   (`src/codec.rs:248-257`) delegates to `tokio_util`'s `LengthDelimitedCodec`,
   which reserves space in the `Framed` write buffer and copies the payload
   into it. This copy is outside `encode_message_frame` and therefore outside
   the boundary a naive reading of the roadmap would measure.

The consequence is concrete: a baseline scoped to `encode_message_frame` would
record roughly one payload-sized copy where the default path performs two, and
item `10.2.2` would then set thresholds against half the outbound traffic. This
plan therefore measures the framed encoder as a fifth row and states in the
baseline document that it is out of scope for `11.1.2`.

A related asymmetry inbound: `FrameCodec::frame_payload_bytes` has a default
body of `Bytes::copy_from_slice` (`src/codec.rs:89-91`), which copies, but
`LengthDelimitedFrameCodec` overrides it with `frame.clone()` (`:281`). The
default path does not pay that copy; a protocol-native codec that omits the
override would. That is item `11.2.3`'s concern, not this one's.

### What already exists, and what it cannot do

Roadmap item `9.6.1` (`docs/execplans/9-6-1-codec-performance-benchmarks.md`,
COMPLETE) built codec-level benchmarks: `benches/codec_performance.rs`
(Criterion groups `codec/encode`, `codec/decode`,
`codec/fragmentation_overhead`), `benches/codec_performance_alloc.rs` (group
`codec/allocations`, with a private `CountingAllocator` at lines 34-77 and a
`#[global_allocator]` at lines 38-39), and the shared support module
`wireframe_testing/src/codec_benchmarks/` defining `SMALL_PAYLOAD_BYTES = 32`,
`LARGE_PAYLOAD_BYTES = 64 * 1024`, `PayloadClass`, `payload_for_class`,
`Measurement`, and the workload matrix. `make bench-codec`
(`Makefile:66-67`) runs both bench binaries by name.

Three gaps matter. The existing coverage stops at the codec boundary, with
nothing for middleware, hooks, or the serializer bridge. `CountingAllocator`
never reads `layout.size()`, so it cannot report bytes, and it uses
process-global atomics, which its own documentation (lines 79-84) concedes
makes it "a noisy relative baseline". And nothing anywhere measures copied
bytes.

The repository does, however, already have the right idiom for the invariant
half of this work. `src/codec/tests.rs` asserts zero-copy behaviour by pointer
identity: `length_delimited_wrap_payload_reuses_bytes` (lines 63-71), the
parameterized `wrap_payload_reuses_bytes` (lines 195-212) and
`frame_payload_bytes_reuses_memory` (lines 214-232) across length-delimited,
Hotline, and MySQL codecs, and the helper `assert_decode_zero_copy` (lines
236-245). This plan extends that idiom rather than inventing a new one.

### Terms defined

- **Allocation event**: one call into the global allocator's `alloc`,
  `alloc_zeroed`, or `realloc` entry point.
- **Allocated bytes**: the sum of `Layout::size()` over allocation events; for
  `realloc`, the new size.
- **Payload-sized allocation**: an allocation event whose size is at least the
  payload length for the class under test. Counting these separately from raw
  totals is what makes the invariants immune to incidental allocation churn
  inside dependencies.
- **Copied byte**: a byte passed to `memcpy`, `memmove`, `strcpy`, or `bcopy`,
  as counted by `valgrind --tool=dhat --mode=copy` (Valgrind manual, section
  10.5). See `Decision log` entry D-2 and ADR-011.
- **Payload class**: `Small` (32 bytes) or `Large` (65,536 bytes), as already
  defined in `wireframe_testing::codec_benchmarks`.
- **Probe**: a synchronous, in-process function that drives one production
  stage once for one payload class, with no transport. The middleware stage is
  the exception: `Service::call` is async, so its probe uses a
  `current_thread` runtime and blocks on the future. The plan says so rather
  than pretending the stage is synchronous.
- **Reproducible measurement**: a measurement that yields an identical value
  for a fixed toolchain, a fixed `Cargo.lock`, a fixed optimization profile,
  and a 64-bit target. No measurement in this plan is claimed to be
  machine-independent; see `Decision log` entry D-6.

## Conformance basis

There is no Terms of Reference document in this repository. The upstream
artefacts are:

- `docs/roadmap.md` section 10.2, item `10.2.1` (lines 470-481), as at commit
  `d77dd76`. Item `10.1.1` is complete (lines 460-462), so this item is
  unblocked.
- `docs/adr-008-zero-copy-public-byte-container.md` (Accepted), technical
  requirement at lines 85-86 and the non-goal at line 181 declining to
  guarantee zero allocation on mutation paths.
- `docs/adr-009-vec-u8-migration-rollout.md` (Accepted), lines 270-272 on
  `PayloadBytes::into_vec` as a known allocating escape hatch.
- `docs/adr-010-transport-frame-boundary-for-zero-copy.md` (Accepted), lines
  189-192 naming the serializer bridge and linking issue
  [leynos/wireframe#538](https://github.com/leynos/wireframe/issues/538), and
  lines 196-206 pinning the sole production `wrap_payload` call site.
- `docs/frame-vec-u8-inventory.md`, in particular the middleware data-flow
  discussion at lines 116-119 and the explicit payload-flow statement at lines
  240-244, plus lines 200-204 on client hooks and 226-228 on the serializer.
- `docs/roadmap.md` section 15 (Formal verification, lines 583-718), which owns
  the prover work this plan deliberately does not do. See `Decision log` D-8.

Trace links:

```plaintext
ADR-008-REQ-default-path-no-fresh-vec -> EP-M3
  -> tests::baseline_invariants::outbound_encode_allocates_payload_sized_buffer
ADR-010-RISK-serializer-bridge -> EP-M1 (correction) and EP-M5 (measurement)
  -> docs/adr-011-byte-migration-baseline-methodology.md
ROADMAP-10.2.1-alloc -> EP-M1, EP-M2, EP-M4
  -> docs/frame-vec-u8-baseline.md#allocation-baselines
ROADMAP-10.2.1-copied-bytes -> EP-M5
  -> docs/frame-vec-u8-baseline.md#copied-bytes
ROADMAP-10.2.1-throughput-latency -> EP-M5 -> benches/pipeline_baseline.rs
INVENTORY-middleware-moves-payload -> EP-M3
  -> tests::baseline_invariants::middleware_moves_payload_without_copying
```

## Constraints

- The four stages, the metric set, and "the default codec path" are fixed by
  `docs/roadmap.md:472-474`. Do not narrow them. The framed-encoder row is an
  addition, justified above, not a substitution.
- A probe must drive production code, never re-implement it. Where a production
  step is not reachable, extract it into a named production function that both
  the production caller and the probe use. Four such extractions are specified
  in EP-M2; each is a refactor of existing code, not new logic.
- No test may assert an absolute allocation count, byte total, copied-byte
  total, throughput, or latency. Tests assert invariants; the numbers are
  recorded. This is the constraint that keeps a daily Dependabot cadence from
  turning the baseline into a rubber stamp.
- Nothing new may become part of the default-feature public surface of either
  `wireframe` or `wireframe_testing`. New surfaces are feature-gated and
  off by default.
- `#[global_allocator]` must never be declared inside a library. The
  `wireframe_testing` crate provides the allocator *type*; each consuming
  binary installs it. Every measurement entry point must verify the allocator
  is installed before reporting, because an uninstalled allocator reports zeros
  and zero is also what success looks like.
- Copied-byte measurement must not modify production code.
- Use `rstest` for unit tests, `rstest-bdd` v0.5.0 for behavioural tests,
  `googletest` matchers and `pretty_assertions` for assertion clarity,
  `proptest` for generated orderings, and `insta` for the report *renderer's*
  format only, driven by fixed synthetic rows rather than live measurements.
- Documentation follows `docs/documentation-style-guide.md`: en-GB Oxford
  spelling, sentence-case headings, prose at 80 columns, code at 120, language
  identifiers on fences, captions on tables, no first- or second-person
  pronouns.
- Register new documents in `docs/contents.md`; append to `CHANGELOG.md` under
  `## Unreleased`; edit `docs/roadmap.md` with `mapsplice`
  (`docs/developers-guide.md:512-540`) using inline links, not footnotes.
- Run gates with `set -o pipefail` and `tee` to `/tmp`. Delegate full gate runs
  to the `scrutineer` sub-agent.
- Commit after each milestone; every commit passes `make check-fmt`,
  `make lint`, and `make test`.

## Tolerances (exception triggers)

- **Scope**: more than 20 files or 1,400 net changed lines — stop and re-scope.
  This matches the completed `9.6.1` plan's scale.
- **Dependencies**: `insta` (dev-dependency) and the `cargo-insta` binary tool
  are the only additions authorized. Anything else — `dhat`, `stats_alloc`,
  `iai-callgrind`, `critcmp` — stop and escalate.
- **Production edits**: the four seam extractions in EP-M2 are authorized
  because they are behaviour-preserving refactors. Any further production
  change, or any change to a function's observable behaviour, stops and
  escalates.
- **Prover tooling**: this plan introduces no Verus proof and no Kani harness.
  If implementation appears to need one, stop and escalate rather than
  starting; roadmap section 15 owns that work.
- **Instrument instability**: if a recorded counter differs between two runs on
  the same machine with the same lockfile, profile, and toolchain, stop. First
  confirm it is not first-touch initialization (see EP-M1's warm-up
  requirement) before declaring an instrument defect.
- **Valgrind runtime**: if `make baseline-copy` exceeds ten minutes, reduce the
  iteration count and record the reduction.
- **Criterion runtime**: if a single target exceeds six minutes locally, reduce
  the sample size and record it.
- **Ambiguity**: if the correct attribution of a measurement to a stage is
  genuinely unclear, stop and present options rather than choosing silently.

## Risks

- Risk: the thread-local instrument silently under-reports when work escapes to
  another thread, and an under-report is indistinguishable from the zero that
  means success.
  Severity: high. Likelihood: medium.
  Mitigation: the instrument carries an escape detector. A process-global
  counter records allocations seen on any thread while a measurement scope is
  open on another; a measurement whose `escaped` flag is set is an error, not a
  number. This is a hard requirement of EP-M1, not a nicety.

- Risk: thread-local access inside `GlobalAlloc` recurses or fails during
  thread-local storage setup and teardown, because initializing thread-local
  storage may itself allocate.
  Severity: high. Likelihood: medium.
  Mitigation: declare the counters with `thread_local!` using
  `const { Cell::new(0) }` so first access performs no lazy allocation, reach
  them with `try_with`, and treat failure as "not measuring". Add a test that
  allocates during thread teardown and asserts the process does not abort.

- Risk: recorded numbers drift because `bincode`, `bytes`, or `tokio-util`
  changes. Cargo Dependabot runs daily (`.github/dependabot.yml`) with
  auto-merge configured.
  Severity: high without mitigation, low with. Likelihood: high.
  Mitigation: no test asserts a number. The generated document carries an
  environment stamp naming the toolchain and the locked versions of the three
  crates, so a changed number is self-explaining rather than alarming.

- Risk: an uninstalled `#[global_allocator]` in one of the six consuming
  binaries yields all-zero counters that read as success.
  Severity: high. Likelihood: medium.
  Mitigation: `assert_installed()` allocates a canary and errors if the
  counters do not move. Called at the entry of every measurement binary. The
  report renderer refuses to emit an all-zero row without an explicit marker.

- Risk: first-touch lazy initialization inside `bincode`, `tokio-util`, or
  `tracing` is attributed to whichever probe runs first on a thread, making
  counters order-dependent.
  Severity: medium. Likelihood: high.
  Mitigation: every probe performs one unmeasured warm-up iteration before the
  measured run, and the tolerance above distinguishes first-touch variance from
  an instrument defect.

- Risk: Valgrind does not run on Apple Silicon, stranding contributors on the
  copied-byte metric.
  Severity: medium. Likelihood: medium.
  Mitigation: `make baseline-copy` runs inside a pinned rootless-Podman image,
  which also makes figures comparable across Linux distributions by fixing
  libc and the Valgrind version. `BASELINE_COPY_NATIVE=1` selects the faster
  native path. The image digest goes into the provenance stamp.

- Risk: the copied-byte table is the one artefact no test guards, so it goes
  stale silently.
  Severity: medium. Likelihood: medium.
  Mitigation: the provenance stamp records the git commit at capture time, and
  a test fails when that commit predates the most recent change to
  `src/serializer.rs`, `src/codec.rs`, or `src/app/outbound_encoding.rs`.
  Staleness becomes visible without needing Valgrind to detect it.

- Risk: `make test` does not build `wireframe_testing`'s tests, because
  `Cargo.toml:15` sets `default-members = ["."]`, and CI does not run
  `make test` at all — `.github/workflows/ci.yml` runs `check-fmt`, `lint`,
  `markdownlint`, `nixie`, `test-workflow-contracts`, and a coverage action.
  Severity: high. Likelihood: certain if unaddressed.
  Mitigation: all invariant tests live in the root crate's `tests/`, which the
  gates do build. EP-M1 additionally verifies that the counters agree between
  the dev profile and the coverage build before anything depends on them.

- Risk: the seam extractions in EP-M2 drag on items 11.x and 12.x.
  Severity: low. Likelihood: medium.
  Mitigation: each extraction moves an existing statement sequence into a named
  function called from its original site. Later items change the function's
  body along with everything else; the probe follows.

## Verification plan

The implementation introduces one non-trivial piece of logic — the allocation
instrument's scoping and escape detection — and one set of claims about the
production code. Everything else is straight-line calls into production code
and a string renderer.

### Axioms

Assumed, not verified: the system allocator satisfies the `GlobalAlloc`
contract; `bincode`, `bytes`, and `tokio-util` behave as documented and their
internals are measured rather than proven; Valgrind's DHAT copy mode counts
bytes passed to the `memcpy` family as documented; `thread_local!` with a
`const` initializer performs no heap allocation on first access on the
supported targets; Criterion's timing methodology is sound.

### Obligation V-1: measurement scoping is exact, and escape is loud

- **Obligation**: for a measured scope `S` on thread `t`, the reported counters
  count each allocator event on `t` between entering and leaving `S` exactly
  once per enclosing scope and zero times for any non-enclosing scope; and if
  any allocation occurs on a thread other than `t` while `S` is open, the
  reported result is an error rather than a number.
- **Method**: `proptest` over generated scope schedules, plus `rstest` cases
  for concurrent threads, panic unwinding, and thread teardown.
- **Rationale**: this is a property over orderings and interleavings. It is
  also the only place where a wrong answer is silently plausible, which is why
  it gets the strongest instrument in the plan.
- **Domain**: generated sequences of `Enter`, `Allocate(size)`, `Leave`, and
  `SpawnAllocatingThread`, nesting-depth-bounded to 3.
- **Artefact**: `tests/baseline_alloc_scoping.rs` in the root crate, so the
  gates actually run it.
- **Evidence**: `make test` passes. Record `proptest` classification output
  showing that generated cases reach depth 3 and do spawn threads; an
  unreached class means the generator is wrong and the run does not count.
- **Non-vacuity**: three negative controls, each of which must fail the
  property — a variant sharing the counter across threads, a variant whose
  scope guard does not restore state on unwind, and a variant with the escape
  detector disabled.

### Obligation V-2: the probes drive the stages they name

- **Obligation**: each probe performs a non-zero, stage-specific amount of
  work, and its counters change when the production code it drives changes.
- **Method**: `rstest` parameterized tests with `googletest` matchers, plus a
  seeded-fault probe variant.
- **Rationale**: a baseline of zero is indistinguishable from a probe that
  never ran. This obligation makes that failure mode impossible.
- **Domain**: the four stages plus the framed-encoder row, crossed with the two
  payload classes.
- **Artefact**: `tests/baseline_invariants.rs`.
- **Non-vacuity**: the seeded-fault control. A `#[cfg(test)]` probe variant
  inserts one additional `payload.to_vec()` into the middleware stage; the test
  asserts the variant reports strictly more payload-sized allocated bytes than
  the faithful variant, by at least `payload_len`. This proves the instrument
  sees exactly the class of copy item `11.1.2` will remove. If the fault
  variant reports the same numbers, the milestone fails.

### Obligation V-3: the default-path claims hold

These are the invariants the baseline exists to state. Each is written to
invert when the migration lands, and each is listed in the inversion register
(EP-M3) so that a future failure is read as success rather than as a defect.

- **V-3a**: `wrap_payload` on the default codec returns a `Bytes` whose pointer
  equals the input's. Method: pointer-identity assertion, extending the
  existing idiom at `src/codec/tests.rs:195-212`. Expected to keep holding.
- **V-3b**: outbound encode performs at least one payload-sized allocation.
  Expected to **invert at 11.1.2**.
- **V-3c**: outbound encode performs at least one non-payload-sized allocation
  when the serializer's output vector has `len != capacity`, corresponding to
  the `Bytes::from` control block. Expected to invert at 11.1.2 only if the
  replacement produces a right-sized buffer, which is the point of stating it.
- **V-3d**: middleware pass-through performs zero payload-sized allocations —
  the payload is moved — while performing at least one non-payload-sized
  allocation for the boxed future. Expected to keep holding.
- **V-3e**: the framed encoder performs at least one payload-sized copy.
  Expected to keep holding through 11.1.2; stated so that nobody mistakes its
  survival for a failed migration.
- **Method**: `rstest` parameterized tests over both payload classes, in
  `tests/baseline_invariants.rs`.
- **Non-vacuity**: each assertion is paired with a control demonstrating it can
  fail — for V-3b, a variant driving a handwritten right-sized serializer that
  must be reported as *not* satisfying the invariant.

### Obligation V-4: the report renderer's format is stable

- **Obligation**: `render_report` produces a stable, sorted,
  column-aligned Markdown table for a given set of rows.
- **Method**: `insta` snapshot over **fixed synthetic rows**, never over live
  measurements.
- **Rationale**: this is genuinely multivariant output whose format consistency
  matters, which is what snapshot testing is for. Driving it from synthetic
  rows keeps it immune to dependency churn, so the snapshot only ever changes
  when a human changes the format.
- **Artefact**: `tests/baseline_report_format.rs` and
  `tests/snapshots/baseline_report_format__table.snap`.
- **Non-vacuity**: a case with rows supplied in reverse order must produce
  identical output, proving the sort is real.

### Obligation V-5: the recorded document is generated, not transcribed

- **Obligation**: the table region of `docs/frame-vec-u8-baseline.md`, delimited
  by `<!-- baseline:begin -->` and `<!-- baseline:end -->`, equals what the
  renderer produces for the current tree.
- **Method**: `rstest-bdd` scenario driving `make baseline --check`, plus a
  compile-time `include_str!` comparison in the unit suite. `include_str!`
  avoids Whitaker's `no_std_fs_operations` deny-level lint and makes deletion
  of the document a compile error rather than a runtime failure.
- **Rationale**: a generated document cannot lie. This replaces the
  hand-transcription step that a review identified as the plan's most likely
  source of long-term rot.
- **Artefact**: `tests/features/baseline_document.feature`,
  `tests/steps/baseline_document_steps.rs`, and
  `tests/baseline_document_sync.rs`.
- **Non-vacuity**: a fixture with one digit altered must be rejected, and the
  rejection message must name the marker region.

### Obligation V-6: copied-byte capture is reproducible

- **Obligation**: two consecutive `make baseline-copy` runs in the same
  environment report identical per-stage figures.
- **Method**: the target runs itself twice and diffs; a mismatch is a non-zero
  exit. No separate behavioural fixture, because a test over handwritten
  profile files cannot fail when the Valgrind harness is wrong.
- **Evidence**: the transcript in `Artefacts and notes`.
- **Non-vacuity**: Valgrind is deterministic by construction, so this check
  detects harness non-determinism specifically — a fixture that varies per run,
  a stray timestamp, an unpinned iteration count.

### Why no Verus proof and no Kani harness

`docs/roadmap.md` section 15 (lines 583-718) is a dedicated formal-verification
phase that owns this work: item `15.1.4` (line 611) adds the `make kani`,
`make kani-full`, and `make verus` targets; item `15.3.1` (line 651) adds the
first Kani smoke harnesses; item `15.5.2` (line 704) adds proof-only modules
under `verus/` for accounting invariants. None is done, and
`docs/developers-guide.md:376-379` records that `make run-verus` is expected to
fail until they are.

An earlier revision of this plan proposed discharging the instrument's counter
arithmetic in Verus and its `GlobalAlloc` implementation in Kani. That was the
wrong call twice over. It would have written the repository's first proof into
the entry point item `15.5.2` reserves, about a test instrument's addition
rather than about a protocol invariant. And it aimed formal machinery at the
part of the design least likely to be wrong: the way this instrument produces a
false baseline is misattribution — wrong scope, wrong thread, wrong profile,
uninstalled allocator — none of which a proof about arithmetic touches. V-1's
property tests and V-2's seeded fault attack misattribution directly.

The counter arithmetic is instead covered by an `rstest` table of eight known
event sequences, including the `realloc` case and the counting-disabled case,
against hand-computed expected values. If the estate later wants a proof of it,
it belongs as a sub-item under roadmap section 15.3, once `15.1.4` exists.

## Plan of work

### Stage A — confirm, correct, and measure one thing (minimal code)

Confirm the four stage boundaries and the copy analysis above against the
current tree. Then do three things.

First, ship the roadmap and ADR-010 wording correction as its own docs-only
commit. `docs/roadmap.md:475-477` (item 10.2.2) and `:491-492` (item 11.1.2)
currently point implementers at `wrap_payload`, which is free. This correction
is independently valuable and must not wait for the measurement work.

Second, record answers to two open questions in `Surprises and discoveries`:
whether `src/app/codec_driver.rs:1-12`'s claim that the frame pipeline applies
protocol `before_send` hooks is accurate — no such invocation was located in
`FramePipeline::process` (`:56-71`) — and the absence of
`docs/repository-layout.md`, which `AGENTS.md:41-43` and the style guide both
reference. Neither is fixed here; if the first is a genuine defect, raise an
issue.

Third, run one fifteen-minute experiment that decides the shape of EP-M5: a
single `valgrind --tool=dhat --mode=copy` over a throwaway 32-byte outbound
encode. An earlier revision assumed LLVM inlines small copies so DHAT would
undercount the `Small` class. That assumption is untested and probably wrong,
because the copies at issue take runtime lengths across crate boundaries and
so emit real `memcpy` calls. If `Small` copied bytes scale with payload size,
the `Small` class is measurable and stays in scope.

Validation: the correction is committed and gates pass; the two answers and the
experiment result are written into this plan.

### Stage B — red tests

Write the failing tests before the implementation, all in the root crate's
`tests/`: `baseline_alloc_scoping.rs` (V-1), `baseline_invariants.rs` (V-2 and
V-3), `baseline_report_format.rs` (V-4), then the document tests (V-5) once
EP-M4 creates the document stub.

Each test fails for its intended reason, recorded as a transcript. To keep the
commit-gate constraint intact, tests are committed in the same milestone as the
code that makes them pass, with the red transcript captured beforehand and
recorded in `Artefacts and notes`.

### Stage C — implementation

Milestones EP-M1 through EP-M4.

### Stage D — measurement, publication, and wider validation

Milestone EP-M5.

## Milestones and plateaus

### EP-M1 — the allocation instrument

- **Outcome**: `wireframe_testing::codec_benchmarks::alloc_probe` exists, is
  verified against V-1, and `benches/codec_performance_alloc.rs` uses it
  instead of its private `CountingAllocator`.
- **Interfaces**, in
  `wireframe_testing/src/codec_benchmarks/alloc_probe.rs`:

  ```rust
  /// Allocator traffic observed on one thread within a measured scope.
  #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
  #[non_exhaustive]
  pub struct AllocatorTraffic {
      allocation_events: u64,
      allocated_bytes: u64,
      payload_sized_events: u64,
      payload_sized_bytes: u64,
      deallocation_events: u64,
      deallocated_bytes: u64,
  }

  /// A `GlobalAlloc` that counts traffic on the measuring thread.
  ///
  /// Binaries install this; libraries must not. See `install_probe_allocator!`.
  pub struct ProbeAllocator;

  /// Errors that prevent a measurement from being trustworthy.
  #[derive(Debug, thiserror::Error)]
  #[non_exhaustive]
  pub enum MeasureError {
      /// The binary did not install `ProbeAllocator`.
      #[error("ProbeAllocator is not the installed global allocator")]
      AllocatorNotInstalled,
      /// Allocation occurred on another thread while the scope was open.
      #[error("allocation escaped the measuring thread; result is not usable")]
      Escaped,
  }

  /// Confirms the probe allocator is installed, by allocating a canary.
  pub fn assert_installed() -> Result<(), MeasureError>;

  /// Runs `operation` with counting enabled on the current thread.
  ///
  /// Nested scopes attribute each event once to every enclosing scope. A
  /// panic inside `operation` restores the prior state via the scope guard.
  pub fn measure_in_place<F: FnOnce()>(
      threshold_bytes: usize,
      operation: F,
  ) -> Result<AllocatorTraffic, MeasureError>;
  ```

  `AllocatorTraffic`'s fields are private with accessor methods, so a seventh
  counter is not a breaking change. Counters are `thread_local!` `Cell`s with
  `const { Cell::new(0) }` initializers, reached via `try_with`. The escape
  detector is a process-global `AtomicU64` of open scopes plus a per-thread
  owner check. `alloc_zeroed` is overridden explicitly, matching the existing
  bench allocator (`benches/codec_performance_alloc.rs:66-70`); omitting it
  would route through `alloc` and shift the counts. An
  `install_probe_allocator!()` macro makes each binary's registration one
  greppable line.
- **Acceptance evidence**: `tests/baseline_alloc_scoping.rs` passes with all
  three V-1 negative controls rejected. Counters are shown to agree between the
  dev profile and a coverage-instrumented build; if they do not, that fact is
  recorded and the document's environment stamp gains a profile field before
  anything else proceeds. `make bench-codec` still emits the same
  `wrap_allocs_<n>` label shape, with old and new values recorded side by side
  in the commit message, because the thread-local change will move them and
  every saved Criterion baseline under `target/criterion` is orphaned by it.
- **Conformance check**: no `wireframe` API changed. `wireframe_testing` gains
  a module and a `[lints]` section inheriting the workspace lint policy, so the
  most correctness-critical component in this item is held to the same standard
  as the rest of the tree rather than the least-linted crate in it.
- **Recovery**: additive plus a one-file bench substitution, committed
  separately so it reverts alone.
- **Remaining gaps**: no probes yet.

### EP-M2 — production seams and probes

- **Outcome**: four production seam extractions, feature-gated shims, and five
  probes that drive production code.
- **Production seams** (each a behaviour-preserving extraction, called from its
  original site):
  1. `src/app/inbound_handler.rs` — widen `parse_envelope` (line 74, currently
     a bare private `fn`) to `pub(crate)`, and add the shim inside this module
     rather than a sibling, because a sibling module cannot see a private item.
  2. `src/app/frame_handling/response.rs` — extract
     `pub(crate) async fn middleware_round_trip(...)` covering lines 41-55, and
     call it from `forward_response`. Without this the middleware probe would
     re-implement the very lines it measures.
  3. `src/client/hooks.rs` — extract a free function
     `invoke_before_send(hooks: &[BeforeSendHook], bytes: &mut Vec<u8>)`
     and have `WireframeClient::invoke_before_send_hooks`
     (`src/client/messaging.rs:335`) delegate to it. The method is on the
     client and reaching it otherwise requires a live `Framed` transport,
     contradicting the definition of a probe.
  4. `src/app/outbound_encoding.rs` — the shim returns
     `Result<F::Frame, SendError>`, not `EncodedFrame<F::Frame>`, because
     `EncodedFrame` is `pub(crate)` (line 18) and returning it from a `pub`
     function trips `private_interfaces`, which `-D warnings` makes an error.
- **Feature plumbing**, which is a hard compile break if omitted:
  `wireframe_testing/Cargo.toml` currently declares
  `wireframe = { path = "..", features = ["testkit"] }` and has no `[features]`
  section, so it cannot see anything gated on `test-support`. Add
  `[features] baseline = ["wireframe/test-support"]`, default off, gate the new
  modules behind it, and add `features = ["baseline"]` to the root crate's
  dev-dependency on `wireframe_testing`. Enabling `test-support`
  unconditionally would ship the shims to every downstream consumer of a
  published crate.
- **Interfaces**, in
  `wireframe_testing/src/codec_benchmarks/default_path_baseline.rs`:

  ```rust
  /// A measurable stage of the default codec path.
  #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
  #[non_exhaustive]
  pub enum Stage {
      /// Declaration order fixes the recorded row order; do not reorder.
      InboundDecode,
      MiddlewarePassThrough,
      RequestHooks,
      OutboundEncode,
      FramedEncoderWrite,
  }

  /// Whether a request hook reads or mutates the payload.
  #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
  pub enum HookShape { ReadOnly, Mutating }

  pub fn fixture(stage: Stage, class: PayloadClass, hook: Option<HookShape>)
      -> Result<StageFixture, BaselineError>;

  /// Drives the production stage once. Infallible by construction, so no
  /// error path lies inside the measured window.
  #[must_use]
  pub fn run_stage(fixture: &StageFixture) -> StageOutput;
  ```

  `run_stage` returns `StageOutput` so the caller can `black_box` it; a
  `-> ()` signature would let the optimizer elide the work being measured.
  `PayloadClass` gains `PartialOrd, Ord` upstream so the report can sort.
- **Acceptance evidence**: `tests/baseline_invariants.rs` passes V-2 including
  the seeded-fault control, and V-3a through V-3e hold with their controls
  rejected. `cargo build -p wireframe_testing` succeeds standalone, not merely
  inside the workspace.
- **Conformance check**: every new `wireframe` item is behind `test-support`;
  every new `wireframe_testing` item is behind `baseline`. Verify by diffing
  `cargo public-api` output with and without the features, since a default-run
  would never see a gated item.
- **Recovery**: the seams are extractions; inlining them restores the prior
  shape. Shims and probes are additive.

### EP-M3 — invariants and the inversion register

- **Outcome**: the five default-path claims are asserted, and a register
  records what is expected to change and when.
- **Deliverable**: a section of `docs/frame-vec-u8-baseline.md` listing, for
  each assertion, the test name, the claim, and the roadmap item at which it is
  expected to invert (V-3b and V-3c at `11.1.2`) or its signature to change
  (`BeforeSendHook` at `12.2.1`, which flips the hook surface alongside
  `Serializer::serialize`). Cross-reference this register from the roadmap
  items themselves.
- **Rationale**: when item `11.1.2` succeeds, three assertions fail. Without
  the register the implementer reads "outbound encode performs at least one
  payload-sized allocation — FAILED" and goes hunting for an instrument bug.
- **Acceptance evidence**: `make test` passes; the register names every
  assertion that appears in `tests/baseline_invariants.rs`, checked by a test
  that compares the register's list against the test names.

### EP-M4 — the generated record

- **Outcome**: `make baseline` regenerates the recorded numbers into
  `docs/frame-vec-u8-baseline.md`; `make baseline --check` verifies it.
- **Interfaces**: `collect_baseline()`, returning
  `Result<Vec<BaselineRow>, BaselineError>` so item `10.2.2` can set thresholds
  without re-parsing Markdown, and `render_report(&[BaselineRow]) -> String`
  producing a captioned
  Markdown table sorted by `(stage, payload_class, hook_shape)`. Both live in
  `wireframe_testing`, which declares no `print_stdout` deny, so the printing
  binary does not need an `#[expect]` attribute.
- **The environment stamp** is part of the rendered output: `rustc` version,
  the locked versions of `bincode`, `bytes`, and `tokio-util`,
  `target_pointer_width`, the cargo profile, and the git commit. A changed
  number then explains itself.
- **Regeneration**: `make baseline` rewrites only the region between
  `<!-- baseline:begin -->` and `<!-- baseline:end -->`. There is no
  hand-transcription step, so V-5 verifies a generated artefact.
- **Acceptance evidence**: `make baseline` then `make baseline --check` exits
  zero; altering one digit and re-running `--check` exits non-zero naming the
  region; `tests/baseline_report_format.rs` passes V-4 with the
  reverse-order control.
- **Conformance check**: `insta` appears only in `[dev-dependencies]`, and the
  snapshot is driven by synthetic rows, so no dependency bump can touch it.

### EP-M5 — copied bytes, timings, and publication

- **Outcome**: copied-byte and timing figures are captured and published, the
  methodology is recorded as an ADR, and the roadmap is ticked.
- **Copied bytes**: a `[[bin]]` target `baseline-copy-probe` in
  `wireframe_testing`, gated on the `baseline` feature, accepting
  `--stage <name>` and `--iterations <n>`. `make baseline-copy` runs each stage
  at `N` and at `2N` under `valgrind --tool=dhat --mode=copy` and reports the
  slope `(total_2N - total_N) / N`. The slope cancels process startup, dynamic
  linking, fixture construction, and Valgrind's own overhead exactly, needs no
  separate calibration stage, and gives a free linearity check: figures that do
  not scale mean the harness is measuring setup. Division uses `checked_div`,
  because `integer_division` and `integer_division_remainder_used` are both
  `deny` (`Cargo.toml:129-130`), and the two raw totals and `N` are published
  alongside the quotient so truncation is recoverable. Recorded defaults:
  `N = 10_000` for `Large`, `N = 100_000` for `Small`. A named script under
  `scripts/` parses DHAT's `dhat.out.<pid>` output; its input format is stated
  in the script's header comment.
- **Timings**: `benches/pipeline_baseline.rs`, registered in `Cargo.toml` as
  `[[bench]] name = "pipeline_baseline", harness = false`, and added by name to
  `make bench-codec`, which lists its benches explicitly (`Makefile:66-67`).
  Reuse the existing `iter_custom` shape from
  `wireframe_testing/src/codec_benchmarks/codec_benchmark_support.rs`, which
  amortizes clock resolution across a batch. Use `Throughput::Bytes` for
  decode, encode, and the framed write; `Throughput::Elements(1)` for
  middleware and read-only hooks, where a bytes figure would report a pointer
  move in TiB/s and be quoted out of context. `black_box` the loop body, not
  just the aggregate, or the no-work stages are deleted by the optimizer.
- **Documentation**:
  - `docs/frame-vec-u8-baseline.md` — the generated tables, the inversion
    register from EP-M3, the provenance stamp, and the falsifiable claim about
    where the default path copies today, including the framed-encoder copy that
    `11.1.2` cannot remove.
  - `docs/adr-011-byte-migration-baseline-methodology.md` — Status Proposed
    until the final commit, then Accepted. Records the operational definition
    of a copied byte and why Valgrind was chosen; why the instrument is
    thread-local and why escape is an error; why invariants are asserted and
    numbers are not; the correction about where the default path copies,
    including the `Bytes::from` control-block allocation; and that prover
    targets are deliberately operator-invoked and must not be added to CI,
    because `rust-toolchain.toml` and `tools/verus/VERSION` are pinned
    independently and a CI-wired prover would turn a pin gap into a red build.
  - `docs/developers-guide.md` — a subsection after "Example and benchmark
    support" (line 331 onward) covering the instrument, the probes, the two
    make targets, the regeneration workflow, and the rule that a dependency
    bump legitimately changes the recorded numbers.
  - `docs/contents.md`, `CHANGELOG.md`, and `docs/roadmap.md` (via `mapsplice`,
    in a commit that does nothing else).
- **Acceptance evidence**: `make baseline-copy` self-diffs clean; the recorded
  outbound copied-byte figure for the `Large` class is at least 131,072,
  reflecting both the serializer copy and the framed-encoder copy — a figure
  near 65,536 means the stage boundary is wrong. All gates pass via
  `scrutineer`.
- **Recovery**: documentation changes revert independently; ADR-011 stays
  Proposed until the final commit; the roadmap tick is its own commit.

## Concrete steps

Run everything from the repository root, with `set -o pipefail`.

```bash
cargo test --all-targets --all-features 2>&1 \
  | tee /tmp/test-wireframe-10-2-1-red.out          # Stage B: expect red
cargo test -p wireframe_testing --features baseline 2>&1 \
  | tee /tmp/test-wt-10-2-1.out                     # after EP-M1/EP-M2
make baseline        2>&1 | tee /tmp/baseline-10-2-1.out
make baseline --check 2>&1 | tee /tmp/baseline-check-10-2-1.out
make baseline-copy   2>&1 | tee /tmp/baseline-copy-10-2-1.out
```

`make baseline --check` on an unmodified tree is expected to print:

```plaintext
docs/frame-vec-u8-baseline.md is up to date
```

Final gates, delegated to `scrutineer`: `make check-fmt`, `make lint`,
`make test`, `make markdownlint`, `make nixie`, each teed to
`/tmp/<action>-wireframe-10-2-1.out`.

## Validation and acceptance

Acceptance is behavioural.

- `make test` passes, and every V-1 through V-5 negative control is rejected
  for its intended reason.
- Inserting one `payload.to_vec()` into the middleware stage makes
  `tests/baseline_invariants.rs` fail with a payload-sized-bytes increase of at
  least `payload_len`.
- Deleting `docs/frame-vec-u8-baseline.md` makes the test suite fail to
  compile, because the document is read with `include_str!`.
- Editing one digit inside the generated region and running
  `make baseline --check` exits non-zero and names the region.
- `make baseline-copy` self-diffs clean, and the recorded `Large` outbound
  copied-byte figure is at least 131,072.
- `cargo build -p wireframe_testing` succeeds outside the workspace.

Quality criteria: `make check-fmt`, `make lint`, `make test`,
`make markdownlint`, and `make nixie` all pass. No performance threshold is set
by this item; choosing thresholds is item `10.2.2`. No security surface
changes.

## Idempotence and recovery

Every step is re-runnable. `make baseline` is idempotent by construction —
running it twice produces no diff. `make baseline-copy` writes Valgrind output
under `target/baseline/`, which is git-ignored. No step is destructive. The
commit order is chosen so that the bench-allocator substitution, the ADR status
flip, and the roadmap tick are each their own revertible commit.

## Interfaces and dependencies

New dependency: `insta` (`[dev-dependencies]`) plus the `cargo-insta` binary
tool, used only for the renderer's format snapshot over synthetic rows.

External tools: Valgrind 3.17 or later for DHAT copy mode, invoked through a
pinned rootless-Podman image so the figures are portable and comparable;
`BASELINE_COPY_NATIVE=1` selects the native path. Neither is a build
requirement, and `make baseline-copy` fails with a clear diagnostic when the
tool is absent rather than recording zeros.

New modules: `wireframe_testing/src/codec_benchmarks/{alloc_probe,
default_path_baseline, baseline_report}.rs`; a `baseline-copy-probe` binary and
`benches/pipeline_baseline.rs`; `scripts/parse-dhat-copy.<ext>`; root-crate
tests `baseline_alloc_scoping.rs`, `baseline_invariants.rs`,
`baseline_report_format.rs`, `baseline_document_sync.rs`, plus the BDD feature
and steps.

Reused rather than duplicated: `PayloadClass`, `payload_for_class`,
`Measurement`, and the `iter_custom` shape from
`wireframe_testing::codec_benchmarks::codec_benchmark_support`; the
pointer-identity idiom from `src/codec/tests.rs`.

## Relevant documentation and skills

Read before starting: `AGENTS.md`; `docs/documentation-style-guide.md`
(including the ADR template at lines 418-495);
`docs/developers-guide.md` (quality gates 176-188, benchmark support 331-352,
formal tooling 354-393, test infrastructure 394-511, `mapsplice` 512-540);
`docs/frame-vec-u8-inventory.md`; ADRs 008, 009, and 010;
`docs/rust-testing-with-rstest-fixtures.md`;
`docs/rstest-bdd-users-guide.md`;
`docs/reliable-testing-in-rust-via-dependency-injection.md`;
`docs/rust-doctest-dry-guide.md` (note `make doctest-benchmark` enforces a
runnable-doctest ratio, and `missing_docs` is denied, so every new public item
needs a doc comment);
`docs/multi-layered-testing-strategy.md`;
`docs/formal-verification-methods-in-wireframe.md` (for why the prover work
belongs to roadmap section 15);
`docs/generic-message-fragmentation-and-re-assembly-design.md`,
`docs/multi-packet-and-streaming-responses-design.md`, and
`docs/hardening-wireframe-a-guide-to-production-resilience.md` for the paths
deliberately excluded; and, in `docs/`,
`the-road-to-wireframe-1-0-feature-set-philosophy-and-capability-maturity.md`.

Skills: `leta` first, for symbol navigation. Then `rust-router`, routing to
`rust-performance-and-layout` for the instrument, `rust-unit-testing` for
fixtures and assertions, `rust-unsafe-and-ffi` for the `GlobalAlloc` safety
comments, and `arch-crate-design` for the feature gating. `proptest` for V-1.
`arch-decision-records` for ADR-011. `en-gb-oxendict` for every document.
`mapsplice` for the roadmap edit. `execplans` for keeping this current.
`nextest` if test selection becomes awkward. Do **not** load `kani` or `verus`
for this item; see the verification plan.

## Progress

- [x] (2026-08-23) Reconnaissance: default codec path, benchmark
  infrastructure, ADRs 008-010, inventory, repository conventions.
- [x] (2026-08-23) Research: Criterion baselines, `critcmp`, `dhat-rs`,
  `stats_alloc`, `iai-callgrind`, Valgrind DHAT copy mode.
- [x] (2026-08-23) Draft plan (revision 1).
- [x] (2026-08-23) Six-lens design review; plan revised (revision 2).
- [ ] Plan reviewed and approved.
- [ ] Stage A — confirm boundaries; ship the roadmap/ADR-010 correction; answer
  the two open questions; run the Small-class Valgrind experiment.
- [ ] EP-M1 — the allocation instrument.
- [ ] EP-M2 — production seams and probes.
- [ ] EP-M3 — invariants and the inversion register.
- [ ] EP-M4 — the generated record.
- [ ] EP-M5 — copied bytes, timings, and publication.

## Surprises and discoveries

- Observation: the "final default-path `Vec<u8>` copy between serialization and
  `FrameCodec::wrap_payload`" is not where the phrase suggests, and there is a
  second payload-sized copy the roadmap does not mention.
  Evidence: `src/codec.rs:283` (`wrap_payload` is the identity),
  `src/app/outbound_encoding.rs:36` (`Bytes::from` takes ownership),
  `src/serializer.rs:67-71` (the serializer must materialize a `Vec<u8>`), and
  `src/codec.rs:248-257` (`LengthDelimitedEncoder::encode` delegates to
  `tokio_util`, which copies the payload into the `Framed` write buffer).
  Impact: `docs/roadmap.md:475-477` and `:491-492` and ADR-010's known-risks
  section point item `11.1.2` at free operations. Stage A ships the correction.

- Observation: `Bytes::from(Vec<u8>)` does not copy, but it is not free.
  Evidence: `bytes-1.12.1/src/bytes.rs:947-967` heap-allocates a 24-byte
  `Shared` control block whenever `len != capacity`, which is the normal case
  for `bincode`'s amortized output vector.
  Impact: the outbound row's expected allocation count is at least two, and
  invariant V-3c states the control-block allocation explicitly so that
  `11.1.2` is judged on whether it produces a right-sized buffer.

- Observation: middleware pass-through is not allocation-free.
  Evidence: `Service` is `#[async_trait]` (`src/middleware.rs:168-169`), so
  every `call` boxes a future.
  Impact: the claim is "zero *payload-sized* allocations", not "zero
  allocations". Stated in V-3d and in ADR-011.

- Observation: CI does not run `make test`.
  Evidence: `.github/workflows/ci.yml` runs `make check-fmt`, `make lint`,
  `make markdownlint`, `make nixie`, and `make test-workflow-contracts`; Rust
  tests reach CI only through a pinned coverage action.
  Impact: revision 1's anti-rot argument was false. Compounding it,
  `Cargo.toml:15` sets `default-members = ["."]`, so `make test` does not build
  `wireframe_testing`'s tests either. All invariant tests therefore live in the
  root crate's `tests/`.

- Observation: `wireframe_testing` cannot see anything gated on `test-support`.
  Evidence: `wireframe_testing/Cargo.toml:16` declares
  `features = ["testkit"]` and the crate has no `[features]` section. It
  compiles inside the workspace only because feature unification papers over
  the gap; `cargo build -p wireframe_testing` would break, as would docs.rs.
  Impact: EP-M2 adds an opt-in `baseline` feature rather than enabling
  `test-support` unconditionally, which would ship the shims to every consumer
  of a published crate.

- Observation: roadmap section 15 already owns the prover work.
  Evidence: `docs/roadmap.md:583-718`; item `15.1.4` (line 611) adds the make
  targets, `15.3.1` (line 651) the first Kani harnesses, `15.5.2` (line 704)
  the `verus/` proof modules — the exact entry point revision 1 proposed to
  occupy.
  Impact: EP-M2 of revision 1 was cut. See `Decision log` D-8.

- Observation: Cargo Dependabot runs daily with auto-merge.
  Evidence: `.github/dependabot.yml` (`package-ecosystem: "cargo"`,
  `interval: "daily"`) and `.github/workflows/dependabot-automerge.yml`.
  Impact: any test asserting an absolute allocation figure would go red on bot
  PRs until re-blessing became reflex. This is the direct cause of the
  invariants-versus-measurements split.

- Observation: the repository already has the right idiom for the invariant
  half.
  Evidence: `src/codec/tests.rs:63-71`, `:195-212`, `:214-232`, `:236-245`
  assert zero-copy by pointer identity, parameterized across three codecs.
  Impact: V-3a extends an existing convention instead of inventing one.

- Observation: `docs/repository-layout.md` is referenced by `AGENTS.md:41-43`,
  the style guide, and the roadmap, but does not exist.
  Impact: out of scope; recorded so it is not mistaken for a search failure.

- Observation: `src/app/codec_driver.rs:1-12` documents that the frame pipeline
  applies protocol `before_send` hooks, but no such invocation was located in
  `FramePipeline::process` (`:56-71`).
  Impact: to be confirmed in Stage A; if the documentation is stale, raise a
  separate issue rather than fixing it here.

## Decision log

- **D-1.** Extend `wireframe_testing::codec_benchmarks` rather than starting a
  parallel harness. `docs/developers-guide.md:331-352` already directs bench
  targets, unit tests, and BDD fixtures there. 2026-08-23.

- **D-2.** Define a copied byte as a byte passed to `memcpy`, `memmove`,
  `strcpy`, or `bcopy`, measured with `valgrind --tool=dhat --mode=copy`. No
  definition existed in the repository. The copies that matter occur inside
  `bincode` and `tokio_util`, so no `wireframe`-owned counter can see them.
  Rejected: the `dhat` crate, whose documentation states it does not profile
  copy functions and which is self-described as experimental; `stats_alloc`,
  which gives allocation bytes but not copies; `iai-callgrind`, which adds a
  harness dependency for something a direct Valgrind invocation provides.
  2026-08-23.

- **D-3.** Allocation counting is thread-local, and escape is an error rather
  than a silent under-count. A thread-local counter alone would report zero
  when work escapes, and zero is also what a successful migration looks like;
  an instrument whose failure mode is indistinguishable from its success signal
  cannot support a threshold. 2026-08-23, revised after design review.

- **D-4.** "Request hooks" means the client-side hooks in `src/client/hooks.rs`.
  Server-side `WireframeProtocol` hooks are inert on the default path
  (`protocol` is `None`) and client preamble replay is scheduled for item
  `12.2.2` (`docs/frame-vec-u8-inventory.md:212-222`). Because the default
  path registers no hooks, the row is documented as a hook-shape cost model
  with a stated synthetic hook body, not as a default-path figure. 2026-08-23.

- **D-5.** Baseline the default codec only. `docs/roadmap.md:472-474` says "on
  the default codec path"; item `11.2.3` is where a protocol-native codec is
  required. Hotline figures are recorded as supplementary and marked
  non-normative where the existing matrix makes them free. 2026-08-23.

- **D-6.** Assert invariants; record measurements. No test asserts an absolute
  number. Allocation counts are properties of `bincode`, `bytes`,
  `tokio-util`, `libstd`, the toolchain, and the optimization profile as much
  as of `wireframe`; `Vec`'s growth strategy is an unspecified implementation
  detail, and Cargo Dependabot runs daily. A committed number would therefore
  produce red bot PRs whose only remedy is re-blessing, and a re-blessing
  reflex destroys exactly the before-and-after comparability items `10.2.2`,
  `11.2.3`, and `13.2.1` need. Invariants such as "at least one payload-sized
  allocation" survive dependency churn and invert precisely when the migration
  lands. 2026-08-23, revised after design review.

- **D-7.** Extract production seams rather than re-implementing steps in
  probes. A probe that re-implements what it measures verifies only itself; a
  mutation in production code would leave the baseline unchanged. Four
  extractions are authorized, each moving an existing statement sequence into a
  named function called from its original site. 2026-08-23.

- **D-8.** No Verus proof and no Kani harness in this item. Roadmap section 15
  owns prover bring-up: `15.1.4` adds the targets, `15.3.1` the first harnesses,
  `15.5.2` the `verus/` modules that revision 1 proposed to pre-empt. Beyond
  the scheduling conflict, the formal machinery was aimed at counter
  arithmetic, whereas this instrument's realistic failure mode is
  misattribution — wrong scope, wrong thread, wrong profile, uninstalled
  allocator — which V-1's property tests and V-2's seeded fault attack
  directly. The arithmetic is covered by an eight-case `rstest` table.
  2026-08-23, decided at design review.

- **D-9.** `insta` snapshots the renderer's format over fixed synthetic rows,
  never over live measurements. The rendered report is genuinely multivariant
  output whose format consistency matters, which is what snapshot testing is
  for; driving it from synthetic rows means the snapshot changes only when a
  human changes the format. 2026-08-23, revised after design review.

- **D-10.** The recorded document is generated between marker comments, not
  transcribed. A generated document cannot drift from the renderer, which
  removes both the byte-identity hand-paste step and one of the two mechanisms
  revision 1 used to guard the same property. 2026-08-23, decided at design
  review.

- **D-11.** Measure the framed-encoder copy as a fifth row. A baseline scoped
  to `encode_message_frame` would record roughly half the outbound payload
  traffic, because `LengthDelimitedEncoder::encode` (`src/codec.rs:248-257`)
  copies the payload into the `Framed` write buffer outside that boundary.
  Item `10.2.2` would then set thresholds against half the traffic, and item
  `11.1.2` would appear to remove "the final copy" while a payload-sized copy
  per frame remained. 2026-08-23, decided at design review.

## Outcomes and retrospective

To be completed at EP-M5. Before setting this plan to `COMPLETE`, reconcile
every entry in `Surprises and discoveries` against the upstream artefacts in
`Conformance basis`. The copy analysis affects the wording of
`docs/roadmap.md` items `10.2.2` and `11.1.2` and ADR-010's known-risks
section; Stage A ships that correction, and the retrospective must confirm it
landed. Do not mark the plan complete while an upstream change remains
unrecorded.

## Artefacts and notes

To be populated during implementation with the red, green, and refactor
transcripts per milestone; the Stage A Valgrind experiment result that decides
the `Small` class's scope; the dev-versus-coverage profile comparison from
EP-M1; the old and new `wrap_allocs_<n>` values recorded when the bench
allocator is substituted; the `proptest` classification output discharging
V-1's non-vacuity requirement; and the two `make baseline-copy` runs
demonstrating reproducibility.

## Revision note

**Revision 2 (2026-08-23)** follows a six-lens design review that examined
structural integrity, alternatives, measurement validity, interface contracts,
operational failure modes, and long-term viability.

What changed, and why:

- **Invariants replace committed numbers as the asserted artefact.** Cargo
  Dependabot runs daily with auto-merge, and allocation counts are properties
  of three third-party crates and the toolchain. Committed numbers would have
  become a rubber stamp within a fortnight. The numbers are still captured;
  they are recorded with an environment stamp rather than asserted.
- **The Verus proof and Kani harness were cut.** Roadmap section 15 already
  owns prover bring-up, including the exact `verus/` entry point revision 1
  proposed to occupy, and the machinery was aimed at the part of the design
  least likely to be wrong.
- **Six hard defects were fixed**: the decode shim could not compile across a
  module boundary; `invoke_before_send_hooks` was attributed to the wrong type
  and could not be reached without a transport; the outbound shim would have
  leaked a `pub(crate)` type through a public signature; the middleware stage
  had no seam and would have been re-implemented; `wireframe_testing` could not
  see `test-support` at all; and `Stage` and `PayloadClass` lacked the `Ord`
  the sort required.
- **Three factual corrections**: `Bytes::from(Vec<u8>)` allocates a control
  block even though it does not copy; `LengthDelimitedEncoder::encode` performs
  a second payload-sized copy that item `11.1.2` cannot remove; and middleware
  pass-through boxes a future per call, so it is not allocation-free.
- **The instrument gained an escape detector and an installation check**,
  because both of its silent failure modes report zero, and zero is what
  success looks like.
- **The `noop` calibration was replaced by an N-versus-2N slope**, which
  cancels fixed costs exactly and gives a linearity check for free.
- **The `Small`-class undercount caveat became a Stage A experiment**, since
  the inlining hypothesis behind it was never tested and is probably wrong.
- **Scope tightened** from six milestones to five, and the tolerance from
  32 files and 2,500 lines to 20 and 1,400, matching the completed `9.6.1`
  plan.

Effect on remaining work: Stage A now ships an independently valuable
docs-only correction before any instrument exists, and the measurement work
that follows is smaller, introduces one new tool rather than three, and
produces an artefact that answers "did the copy go away?" with a test that
inverts rather than a number that needs interpreting.
