# 10.2.2 Record benchmark acceptance thresholds that require removal of the serializer materialization

This ExecPlan (execution plan) is a living document. The sections `Constraints`,
`Tolerances`, `Risks`, `Progress`, `Surprises & discoveries`, `Decision log`,
`Outcomes & retrospective`, `Conformance basis`, and `Verification plan` must
be kept up to date as work proceeds.

Status: DRAFT (awaiting design review and user approval)

Branch: `10-2-2-record-benchmark-acceptance-thresholds`

Roadmap item: `10.2.2` in `docs/roadmap.md`, mirrored by item `1.2.2` in
`docs/zero-copy-frame-and-payload-migration-roadmap.md`.

## Purpose / big picture

Wireframe is migrating its outbound byte path from `Vec<u8>` to shared
`bytes::Bytes` buffers (Epic #284). Today every outbound message passes through
`Serializer::serialize`, which must return a freshly allocated, payload-sized
`Vec<u8>`. `src/app/outbound_encoding.rs::encode_message_frame` converts that
vector to `Bytes` and hands it to `FrameCodec::wrap_payload`. The vector is the
*serializer materialization*. Roadmap item `11.1.2` will change the serializer
contract so that output can be shared rather than materialized.

Roadmap item `10.2.2` must fix, before that migration starts, the measurable
lines a candidate migration has to cross to be accepted. A *threshold* here is
a written rule, tied to a named measurement, that a later comparison either
meets or must explain. The thresholds `T1` to `T7` are already proposed in
`docs/zero-copy-benchmark-baselines.md`, but three problems stop a performance
reviewer from agreeing to them as written:

1. They do not say *which* materialization must disappear. The accepted
   ADR 010 addendum (2026-11-20) says item `11.1.2` "removes one allocation and
   one copy on the outbound default path". The only migration design, the issue
   #538 plan, deliberately does not promise that: Bincode still has to encode a
   typed message into a payload-sized buffer, only now a `BytesMut` instead of a
   `Vec<u8>`. As written, threshold `T6` (outbound total at most one payload
   copy) would fail the default Bincode path even after a correct migration.
   Read loosely instead, it would let a migration that changes nothing pass.
2. The "pointer-identity and allocation gate that states the removal
   requirement" (threshold `T4`) has no test that can observe it. No test
   compares the address of a payload the message already owns with the address
   of the frame's payload after serialization. The existing pointer tests in
   `src/codec/tests/mod.rs` cover `wrap_payload` alone.
3. Several statements are inaccurate. `T1` calls "+10 % latency" equivalent to
   "10 % lower throughput", but +10 % latency is about 9.09 % lower throughput.
   `T7` says no stage is unmeasured, yet `WireframeApp::send_response_framed`
   serializes without passing any copy counter. `T2`'s absolute floors come
   from a capture at host load 18–22, while the companion capture C5 ran at
   32–57. Nothing links each threshold to the tests that enforce it, so the
   text and the gates can drift apart.

After this work a reviewer can read one document that states each threshold and
its enforcing test, and the build fails if a named test disappears. A
shared-payload *witness* is a small test serializer whose message already holds
its encoded bytes as `Bytes`. Today the witness fails pointer identity and
records a payload-sized copy. Once `11.1.2` lands it must preserve the pointer
and record zero bytes, or the inversion register will not let `11.1.2` close.
The thresholds become *accepted* only when a named performance reviewer records
agreement in the repository; that closes `10.2.2`.

Observable outcome, in one example. Today, with
`--features test-support,benchmark-counters`, a 65,536-byte payload held in a
`Bytes` and sent through `encode_message_frame` reports
`CopiedBytes { serialized: 65536, handoff: 0, framed: 0, deserialized: 0 }`.
The frame payload's `as_ptr()` differs from the message's. After this plan, a
named test asserts exactly that pre-migration state, and the baselines document
states the post-migration requirement: `serialized: 0` and an equal pointer.

Behaviour preserved: no public signature in the `wireframe` crate changes, no
wire format changes, and no benchmark harness changes, so captures C4 and C5
remain valid. One measurement changes deliberately: `send_response_framed` will
start reporting its serializer seam through the `benchmark-counters` recorders,
which compile to nothing without that feature.

## Constraints

- Implementation begins only after the user approves this plan. The final
  acceptance milestone (`EP-M5`) also needs explicit agreement from a named
  performance reviewer. No agent, including the design-review panel, may supply
  that agreement.
- Do not change any public signature in the `wireframe` crate: `Serializer`,
  `EncodeWith`, `FrameCodec`, `WireframeApp`, client APIs, or
  `benchmark_counters` recorder signatures. Adding recorder *calls* inside
  `send_response_framed` is permitted. The serializer contract change belongs to
  `11.1.2`.
- Do not modify the benchmark harnesses: `benches/*.rs`, and the measurement
  functions in `wireframe_testing/src/codec_benchmarks/pipeline_outbound.rs`,
  `pipeline_middleware.rs`, `pipeline_hooks.rs`, and
  `codec_benchmark_support.rs`. The baselines document's rule is that a changed
  harness invalidates its capture, and this plan must not force a re-capture.
- `wireframe_testing` may gain one feature-gated public module (the witness,
  see `Interfaces and dependencies`). `wireframe_testing` is an unreleased
  test-support crate, so no compatibility layer is needed or permitted.
- Authorized dependency changes: `googletest = "0.14.3"` and
  `pretty_assertions = "1.4.1"` as `[dev-dependencies]` of `wireframe_testing`,
  matching the root manifest's versions. No other dependency, and no `insta`.
- Every code file stays at or below 400 lines (`AGENTS.md`). At the base
  revision `wireframe_testing/tests/copied_bytes.rs` has 332 lines and
  `wireframe_testing/src/codec_benchmarks/pipeline_outbound.rs` has 372, so new
  tests go in new modules.
- No `unsafe`, no lint suppression without a tightly scoped
  `#[expect(.., reason = "…")]`, no `float_arithmetic` (denied in
  `Cargo.toml`), no `std::env` access (denied in `clippy.toml`), and no
  `.expect()` outside test code.
- Accepted ADR sections are immutable. Corrections go in dated addenda that
  name each superseded sentence.
- Completed ExecPlans, including `docs/execplans/10-2-1-capture-baselines.md`
  and the draft `issue-538-plan-zero-copy-serializer-output-migration.md`, are
  historical or owned by other items. Do not edit them. Record any consequences
  for the #538 plan in this plan and in the baselines document.
- `docs/roadmap.md` uses inline links only. `mapsplice` rejects footnotes.
- Documentation uses en-GB-oxendict spelling (`-ize`, `-yse`, `-our`), wraps
  prose at 80 columns, and passes `make fmt`, `make markdownlint`, and
  `make nixie`.

## Tolerances (exception triggers)

The comparison base is `origin/main` at `fa8ef421`. This ExecPlan file is
excluded from every count below, including when it changes in the same commit.
Count with:

```sh
git diff --numstat fa8ef421...HEAD -- . ':(exclude)docs/execplans/**'
```

Classification is by whole file, by purpose. Code is `*.rs` and `*.feature`
files, since feature files are executable specifications. Configuration is
`Cargo.toml` and `Cargo.lock` files. Documentation is every other `*.md` outside
`docs/execplans/`. Each limit measures net lines (additions minus deletions)
in its own category, and spare capacity in one category never offsets another.

- Files: stop and escalate if more than 22 non-ExecPlan files change.
- Code: stop and escalate above 900 net lines.
- Configuration: stop and escalate above 12 net lines. `Cargo.lock` changes
  that only record already-resolved dev-dependency edges count here as well.
- Documentation: stop and escalate above 500 net lines.
- Interface: stop and escalate if any public `wireframe` signature must change,
  or if `wireframe_testing` needs public items beyond the witness module.
- Dependencies: stop and escalate if any dependency other than the two
  authorized dev-dependencies is needed.
- Architecture: stop and escalate if threshold wording would need to change
  ADR 008, 009, or 010's *decisions*, as opposed to correcting a measurement
  claim in a dated addendum, or if the reviewer rejects decision `D1`.
- Iterations: stop and escalate if a gate still fails after three fix attempts
  on the same cause.
- Time: stop and escalate if any milestone takes more than one working day of
  active effort, excluding compile-queue waits.
- Ambiguity: stop and present options if the reviewer's feedback admits two
  materially different threshold readings.

Budget evidence is recorded in `Context and orientation`. In short, the sibling
plan `10-2-1-capture-baselines.md` set 900 code lines and 16 files and finished
at 2,725 code lines across 37 files, its overrun still awaiting an owner
decision. That plan built new harnesses and split modules to satisfy the
400-line cap. This plan forbids harness changes and reuses the existing
allocation-window support. Its estimate is about 650 net code lines; the 900
limit leaves roughly 40 % contingency for module splits and review rounds. The
documentation estimate is about 330 net lines, mostly the threshold rewrite.
The 500 limit reflects `10-1-2`'s increase from 500 to 750 for an ADR rewrite,
which this plan avoids by using addenda.

## Risks

- Risk: the reviewer rejects decision `D1` and instead requires the Bincode
  default path to drop to one payload copy per message. That requires
  serializing directly into the transport write buffer, which changes the
  accepted ADR 010 frame boundary and the #538 design. Severity: high.
  Likelihood: medium. Mitigation: present `D1` and its rejected alternative
  first in the review packet (`EP-M5`), before any threshold wording is
  polished. If rejected, set the plan to `BLOCKED` and escalate; do not
  improvise a transport redesign.
- Risk: no performance reviewer is assigned. The release plan says the role is
  unassigned (`docs/zero-copy-release-plan.md`, "Owners"). Severity: medium.
  Likelihood: high. Mitigation: `EP-M1` to `EP-M4` deliver value without the
  reviewer. `EP-M5` blocks explicitly and asks the user to name the reviewer;
  the user may act as reviewer.
- Risk: the shared-payload witness passes vacuously. Pointer comparisons on
  empty buffers, or a witness that never reaches `encode_message_frame`, prove
  nothing. Severity: high. Likelihood: low. Mitigation: use non-empty payloads
  only, plus a control that wraps the same `Bytes` directly and must preserve
  the pointer, so a broken comparison cannot pass both checks.
- Risk: process-global counters and the counting allocator make new tests flaky
  under libtest threads. Severity: medium. Likelihood: medium. Mitigation:
  follow the existing pattern. Counter tests are `#[serial]` in the
  `copied_bytes` binary, and allocator checks join the `harness = false`
  `pipeline_allocation_invariants` binary.
- Risk: the Bincode allocation-count guard proposed in `EP-M3` cannot be
  stated as an exact count, because the window may see incidental allocations.
  Severity: low. Likelihood: medium. Mitigation: measure the count at the red
  stage before writing the assertion, record the observation here, and fall
  back to a payload-class-independence assertion if the count is not stable.
- Risk: the threshold register parser becomes a second ad hoc Markdown parser.
  Severity: low. Likelihood: medium. Mitigation: generalize the existing
  inversion-register parser into one shared module rather than writing a second
  one.
- Risk: other agents' compile work makes gates slow. Severity: low.
  Likelihood: high. Mitigation: allow long timeouts; never bypass compile
  admission.

## Progress

- [x] (2026-11-28 13:10Z) Reconnaissance with a Wyvern team: outbound
  serializer path, zero-copy documents and ADRs, test and proof tooling.
- [x] (2026-11-28 13:40Z) External facts checked with Firecrawl: Criterion
  0.8.2 defaults and Bincode 2.0.1's `encode_to_vec` pre-sizing.
- [x] (2026-11-28 14:30Z) Initial draft written.
- [ ] Design review by the community-of-experts panel, with revisions applied.
- [ ] Plan approved by the user.
- [ ] `EP-M1` shared-payload removal witness.
- [ ] `EP-M2` raw length-delimited send path counted.
- [ ] `EP-M3` registers: shared parser, corrected inversion register,
  threshold register, Bincode allocation guard.
- [ ] `EP-M4` threshold text, ADR 010 addendum, and dependent documents
  revised (status still "proposed").
- [ ] `EP-M5` performance review recorded; thresholds accepted; roadmaps
  ticked.

## Surprises & discoveries

- Observation: the accepted ADR 010 addendum and the #538 design disagree
  about what `11.1.2` removes. Evidence: ADR 010 lines 281–310 at `fa8ef421` say
  `11.1.2` "removes one allocation and one copy on the outbound default path".
  The #538 plan's "Purpose" says zero-copy "does not promise allocation-free
  serialization", and its writer design encodes into a `BytesMut`. Impact:
  resolved by decision `D1` and an ADR 010 addendum in `EP-M4`.
- Observation: Bincode 2.0.1's `encode_to_vec` sizes its vector exactly, using
  a `SizeWriter` pass before encoding. The #538 plan's `BytesMut` writer
  specifies no such pre-sizing. Evidence:
  `bincode-2.0.1/src/features/ impl_alloc.rs`, function `encode_to_vec`, read
  through Firecrawl on 2026-11-28. Impact: a naive `BytesMut` writer would grow
  by reallocation for 64 KiB payloads and could raise the default path's
  allocation count, which `T3` forbids. `EP-M3` adds a guard and `EP-M4` states
  the requirement for `11.1.2`.
- Observation: `WireframeApp::send_response_framed`
  (`src/app/outbound_response.rs`, lines 109–133) serializes and converts with
  `Bytes::from` without any copy counter, so `T7`'s claim that no stage is
  unmeasured is false. Impact: `EP-M2`.
- Observation: `T1` treats +10 % latency and −10 % throughput as equivalent.
  They are not. Impact: `EP-M4` restates `T1` on time only.
- Observation: `docs/zero-copy-benchmark-plan.md` line 396 points to a
  "blocker" recorded in the baselines document, which contains no such entry.
  The dedicated roadmap (lines 38–39 and 101–102) and the benchmark plan (lines
  379–381) still use the "between serialization and `wrap_payload`" wording
  that the ADR 010 addendum superseded. Impact: `EP-M4`.
- Observation: the inversion register lists two Bincode-path tests,
  `default_serializer_and_handoff_copy_the_payload_once` and
  `production_outbound_allocates_one_payload`, as flipping when `11.1.2` lands.
  Under the #538 design and decision `D1` they do not flip; they become
  non-regression guards. Impact: `EP-M3`.
- Observation: the sibling plan `10-2-1-capture-baselines.md` records code,
  file, and documentation overruns with owner decisions "pending", yet
  `docs/roadmap.md` ticks `10.2.1`. Impact: none on this plan's scope. It is
  recorded as budgeting evidence and is not this plan's to resolve.

## Decision log

- D1. Decision: define the materialization that must be removed as the
  *contract-forced* copy. That is the payload-sized buffer a serializer must
  allocate and fill only because `Serializer::serialize` has to return an owned
  `Vec<u8>`, even when the message already holds its encoded bytes in a
  shareable buffer. After `11.1.2`, a serializer whose message holds a `Bytes`
  must hand it to `wrap_payload` with the same data pointer, record zero bytes
  at the serializer seam, and allocate no payload-sized buffer. An *encoding*
  serializer such as Bincode still writes `P` bytes once. That encode is
  inherent, not a materialization. It is bounded by a no-regression guard
  instead: exactly one payload buffer, no growth reallocations, and no
  additional copied bytes. Rationale: this is the only reading that the
  approved design direction (#538, ADR 008) can satisfy, and it matches the
  roadmap item's own definition ("the payload-sized `Vec<u8>` that
  `Serializer::serialize` returns"). The rejected alternative is to require one
  payload copy per message on the Bincode default path. That needs
  serialization into the transport write buffer, either by fusing serializer
  and encoder or by vectored writes that bypass `Framed`. Both cross the
  accepted ADR 010 frame boundary, and both are outside `11.1.2`'s scope. A
  future item could pursue it, and it would need its own ADR. This decision
  corrects an accepted ADR's measurement claim, so it needs explicit acceptance
  by the user at plan approval and by the performance reviewer in `EP-M5`.
  Date/Author: 2026-11-28, planning agent; pending acceptance.
- D2. Decision: build the witness as a separate, feature-gated module in
  `wireframe_testing`, not by changing the benchmark harness's `PayloadMessage`
  to hold `Bytes`. Rationale: changing the harness would invalidate captures C4
  and C5 under the baselines document's own rule. A separate module is reusable
  by the `copied_bytes` binary, the `pipeline_allocation_invariants` binary,
  and the root BDD suite, none of which can share `tests/` modules.
  Date/Author: 2026-11-28, planning agent.
- D3. Decision: count `send_response_framed`'s serializer seam with the
  existing recorders (`record_serialized` and `bytes_from_vec`). Do not count
  `tokio-util`'s internal length-prefix copy. Name that copy as a justified
  exception instead. Rationale: the thresholds are about the serializer seam,
  and that seam is observable. The `tokio-util` copy happens inside a
  third-party encoder that wireframe does not own, and recording it from
  outside would claim a copy before `send` succeeds. Date/Author: 2026-11-28,
  planning agent.
- D4. Decision: add a *threshold register*, a marker-delimited list in the
  baselines document that names each threshold and its enforcing tests. Check
  it with a test that shares one parser with the inversion register. Thresholds
  enforced by procedure rather than by a test (`T1`, `T2`, and `T3`'s
  benchmark-label counts) are marked `procedure` and must name the procedure's
  document section. Rationale: it turns "the thresholds name their enforcing
  tests" from prose into a gate, so accepted text and tests cannot drift.
  Date/Author: 2026-11-28, planning agent.
- D5. Decision: restate `T1` on Criterion's time-change estimate only, and
  derive throughput. Make `T2`'s absolute floors conditional on a declared load
  band and entry point; outside the band they report "not comparable" rather
  than pass or fail. Require `13.2.1` to bracket the candidate run between two
  baseline runs on the same host in one session (an A–B–A run). Freeze
  benchmark identifiers. Rationale: this corrects the equivalence error, stops
  host load from turning `T2` into a noise generator, and makes the pre/post
  comparison reproducible on a shared host. The verdict numbers (10 % and 1 %)
  are unchanged; the 1 % noise figure matches Criterion 0.8.2's default
  `noise_threshold`. Date/Author: 2026-11-28, planning agent; for reviewer
  agreement.
- D6. Decision: do not implement a threshold comparator or verdict evaluator in
  this item. Record its abstract model and lemma statements here, and assign
  the implementation and its Verus proof to `13.2.1`. Rationale: `13.2.1` is
  the first item with two captures to compare. A comparator written now would
  be dead code checked only against invented data. Date/Author: 2026-11-28,
  planning agent.
- D7. Decision: do not use `insta`. Rationale: the change has no multivariant
  rendered output. Its outputs are counts, pointer equalities, and register
  parse results, and named assertions state those more precisely than
  snapshots. Date/Author: 2026-11-28, planning agent.
- D8. Decision: correct the ADR 010 overclaim with a dated addendum. Do not
  open a new ADR. Rationale: the boundary decision is unchanged; only a
  measurement statement is wrong, and dated addenda are the repository's
  convention for that. The threshold policy itself lives in the baselines
  document, the design document named by the roadmap item. Date/Author:
  2026-11-28, planning agent.
- D9. Decision: record reviewer agreement as a "Threshold review record"
  section in the baselines document. It names the reviewer's GitHub login, the
  date, the document revision reviewed, and a disposition per threshold
  (accepted, accepted with amendment, or rejected). Back it with a pull-request
  review or comment by that login that links the record. Rationale: the 10.1.x
  approval items used ADR status flips; thresholds have no ADR, so the record
  must be explicit and attributable. Date/Author: 2026-11-28, planning agent.

## Outcomes & retrospective

Not started. To be completed at each milestone and at completion.

## Context and orientation

Base revision: `fa8ef421` ("Cancel superseded pull-request runs in every PR
workflow (#986)"). Line numbers below refer to that revision. To relocate a
symbol, run `rg -n '<symbol>' src wireframe_testing tests docs`.

### The outbound serializer path

`src/serializer.rs` defines `Serializer`. Its method
`fn serialize<M>(&self, value: &M) -> Result<Vec<u8>, Box<dyn Error + Send +
Sync>>`
is at lines 61–70. `BincodeSerializer::serialize` (lines 121–127) delegates to
`EncodeWith::encode_with` (`src/message.rs`, lines 62–69). For `Message` types
that goes through the blanket implementation at lines 145–154, which calls
`Message::to_bytes` (Bincode's `encode_to_vec`).

`src/app/outbound_encoding.rs::encode_message_frame` (lines 22–43) is the seam
every framed outbound message crosses. It calls `serializer.serialize`, records
`benchmark_counters::record_serialized(bytes.len())`, and calls
`codec.wrap_payload(benchmark_counters::bytes_from_vec(bytes))`.
`benchmark_encode_message_frame` (lines 61–74) is its `test-support` mirror for
external harnesses. Production callers are `src/app/codec_driver/outbound.rs`
(`send_envelope`, reached from `ActorFrameSink::send_frame`) and
`src/app/outbound_response.rs` (`send_response`,
`send_response_framed_with_codec`).

`src/app/outbound_response.rs::send_response_framed` (lines 109–133) is the raw
length-delimited path. It serializes, converts with `Bytes::from`, and sends
through `Framed<W, tokio_util::codec::LengthDelimitedCodec>`, which adds the
length prefix. It deliberately skips `wrap_payload` to avoid double framing,
and records nothing.

The client path, `src/client/send_pipeline.rs::serialize_and_send` (lines
20–69), records `record_serialized`, runs `before_send` hooks on the vector,
and sends `bytes_from_vec(bytes)` to a `LengthDelimitedCodec` sink. It never
calls `wrap_payload`.

### The copy counters

`src/benchmark_counters.rs` (358 lines) is compiled to no-ops unless the
`benchmark-counters` feature is on. `CopiedBytes` has `serialized`, `handoff`,
`framed`, and `deserialized`. The recorders are `record_serialized`,
`record_handoff`, `record_framed`, and `record_deserialized`, alongside
`snapshot` and `reset`. `bytes_from_vec` compares `as_ptr()` before and after
`Bytes::from` and records a hand-off only when the pointer moves. Counters are
process-global, so tests that read them run under `serial_test::serial`.
`wireframe_testing`'s `pipeline-benchmarks` feature enables
`wireframe/benchmark-counters`, and the root crate's dev-dependency on
`wireframe_testing` enables `pipeline-benchmarks`.

### The enforcing tests

`wireframe_testing/tests/copied_bytes.rs` (332 lines) contains the per-seam
assertions, including the exact pre-migration total
`outbound_path_copies_the_payload_twice_per_message` (`== 2 * P`). It pulls in
`copied_bytes/properties.rs` (155 lines) and
`copied_bytes/inversion_register.rs` (227 lines) via `#[path]`.
`inversion_register_names_real_tests` reads
`docs/zero-copy-benchmark-baselines.md` with `include_str!`, parses the block
between `<!-- inversion-register:start -->` and
`<!-- inversion-register:end -->`, takes the first two backtick spans of each
item as test name and file, and checks that each file contains `fn <name>(`. It
also asserts exactly six entries and requires three specific names.

`wireframe_testing/tests/pipeline_allocation_invariants/` is a
`harness = false` binary (`wireframe_testing/Cargo.toml`, lines 53–57, feature
`pipeline-benchmarks`) with its own counting global allocator. Checks are
`(&str, fn())` pairs in `checks/mod.rs` (131 lines). `checks/handoff.rs` (131
lines) holds `production_outbound_allocates_one_payload`, which asserts
`P <= bytes < 2P` for Bincode through `benchmark_encode_message_frame`. It
answers nextest's `--list` with a single unit.

`src/codec/tests/mod.rs` (355 lines) holds the codec pointer-identity tests
`length_delimited_wrap_payload_reuses_bytes`, `wrap_payload_reuses_bytes`, and
`frame_payload_bytes_reuses_memory`. None crosses the serializer.

Behavioural tests use `rstest-bdd` 0.6.0. The relevant feature is
`tests/features/zero_copy_baseline.feature`, with three scenarios. Scenarios
are bound with `#[scenario(path = ..., name = ...)]` in
`tests/scenarios/zero_copy_baseline_scenarios.rs`. Steps live in
`tests/steps/zero_copy_baseline_steps.rs` and the world fixture in
`tests/fixtures/zero_copy_baseline.rs`. They compile into the `bdd` test target
(`required-features = ["advanced-tests"]`), which `make test` runs because it
passes `--all-features`.

### Benchmarks

`make bench-codec` runs `codec_performance`, `codec_performance_alloc`,
`pipeline_performance`, and `pipeline_performance_alloc` with
`--features test-support,benchmark-counters` and `RUSTFLAGS="-D warnings"`. No
CI job runs benchmarks. Captures C4 (`bef53308`) and C5 (`17b74b3f`) are the
published baselines. Criterion 0.8.2's defaults are `noise_threshold` 0.01,
`significance_level` 0.05, and `confidence_level` 0.95 (docs.rs, read
2026-11-28).

### Tooling and gates

`make test` runs `cargo test --workspace --all-targets --all-features`. The
workspace includes `wireframe_testing`, so `copied_bytes` and
`pipeline_allocation_invariants` run under `make test`. `make lint` runs
rustdoc, Clippy with `-D warnings`, Whitaker, and Python doc lint.
`make check-fmt` runs `cargo fmt --check` and `mdtablefix --check`.
`make markdownlint` runs the spelling gate first. `make nixie` validates
Mermaid. Verus and Kani are not wired. `make verus` and `make kani` are stubs
that print `FORMAL-SKIP` (`Makefile`, lines 301–314), and
`docs/formal-verification-methods-in-wireframe.md` reserves Verus for crisp
protocol invariants with proof-only spec types. Root dev-dependencies include
`rstest` 0.28.0, `rstest-bdd` 0.6.0, `proptest` 1.7.0, `googletest` 0.14.3, and
`pretty_assertions` 1.4.1. `wireframe_testing` has `rstest`, `proptest` (dev),
`serial_test`, and `stats_alloc`, but neither `googletest` nor
`pretty_assertions`.

### Budget survey

Code touched: about 10 lines in `src/app/outbound_response.rs`. New witness
module of about 120 lines in `wireframe_testing/src/codec_benchmarks/`. About
200 lines of new `copied_bytes` submodules, about 90 lines of new allocation
checks, and about 120 lines of BDD feature, steps, fixture, and scenarios. The
register parser refactor nets about 60 lines, plus about 50 lines of
threshold-register test. Total: about 650 net lines.

Documentation: the baselines document's threshold, register, and exception
sections (about 180 net lines), an ADR 010 addendum (about 40), the benchmark
plan, dedicated roadmap, and release plan corrections (about 40), the
developers' guide (about 40), the users' guide `CopiedBytes` note (about 15),
and `docs/contents.md` (about 5). Total: about 330.

Historical evidence. `10-2-1-capture-baselines.md` set 900 code lines and 16
files, 40 configuration, and 300 documentation; it recorded 2,725 code lines
across 37 files, 14 configuration, and 377 documentation, with owner decisions
pending. The causes it records are new harness binaries, module splits for the
400-line cap, and review-driven additions, which are missed scope rather than
approved growth. `10-1-2` raised its documentation budget from 500 to 750 net
lines for an ADR rewrite. `9-6-1` (16 files, 1,000 lines) needed no increase.
This plan removes the main overrun driver (no new harnesses) and pre-splits
modules. The limits above add about 40 % code contingency and 50 %
documentation contingency.

### Signposts

Read before implementing: `docs/zero-copy-benchmark-baselines.md` (the design
document for thresholds), `docs/zero-copy-benchmark-plan.md`,
`docs/adrs/adr-0010-transport-frame-boundary-for-the-zero-copy-migration.md`
(addendum of 2026-11-20),
`docs/execplans/issue-538-plan-zero-copy-serializer-output-migration.md`,
`docs/rust-testing-with-rstest-fixtures.md`,
`docs/reliable-testing-in-rust-via-dependency-injection.md`,
`docs/rstest-bdd-users-guide.md`, `docs/wireframe-rstest-bdd-users-guide.md`,
`docs/rust-doctest-dry-guide.md`, `docs/documentation-style-guide.md`, and
`docs/developers-guide.md` ("Example and benchmark support", from line 2333).
For context: `docs/generic-message-fragmentation-and-re-assembly-design.md`
(fragmentation is informational only in these thresholds),
`docs/multi-packet-and-streaming-responses-design.md` (streamed responses also
cross `encode_message_frame`, so the witness covers them by construction),
`docs/the-road-to-wireframe-1-0-feature-set-philosophy-and-capability-maturity.md`,
and `docs/hardening-wireframe-a-guide-to-production-resilience.md`.

Skills: `rust-router`, then `rust-unit-testing` (rstest, googletest,
pretty_assertions), `proptest`, `rust-performance-and-layout` (benchmark
discipline), `rust-verification` and `verus` (for the deferred `13.2.1` kernel),
`execplans`, `en-gb-oxendict-style`, `commit-message`, and `pr-creation`.

## Conformance basis

Upstream artefacts at `fa8ef421`. No terms of reference exist for this
programme, and none are invented here.

- `docs/roadmap.md` item `10.2.2` (lines 494–500) and item `13.2.1` (line 598).
- `docs/zero-copy-frame-and-payload-migration-roadmap.md` item `1.2.2`
  (lines 91–109) and its success criteria (lines 38–43).
- `docs/zero-copy-benchmark-baselines.md`, "Acceptance thresholds" (lines
  524–643) and "Inversion register" (lines 460–500).
- `docs/zero-copy-benchmark-plan.md`, "Acceptance thresholds" (lines 370–384).
- ADR 008 (accepted; serializer addendum proposed), ADR 009 (accepted;
  serializer amendment proposed), and ADR 010 (accepted; addendum of
  2026-11-20).
- `docs/execplans/issue-538-plan-zero-copy-serializer-output-migration.md`
  (draft design for `11.1.2`).
- `docs/zero-copy-release-plan.md`, "Owners" and "Blockers".

Requirements derived for this plan:

- `REQ-define`: the thresholds state precisely which materialization `11.1.2`
  must remove, consistently with the approved design direction (`D1`).
- `REQ-witness`: a test states the pre-migration materialization through
  pointer identity, an allocation window, and copied bytes, and is registered
  to flip when `11.1.2` lands.
- `REQ-guard`: the default Bincode path has a no-regression guard on
  allocations and copied bytes that `11.1.2` must keep green.
- `REQ-coverage`: every outbound serializer path is counted or explicitly
  excepted.
- `REQ-trace`: every threshold names its enforcing tests or procedure, and a
  gate fails when a named test disappears.
- `REQ-sound`: threshold rules are mathematically and statistically coherent
  and reproducible on a shared host.
- `REQ-accept`: a named performance reviewer's agreement is recorded before the
  thresholds are called accepted and before `10.2.2` is ticked.

Observed gaps:

- `GAP-adr010-overclaim`: the ADR 010 addendum claims a default-path copy
  removal that the #538 design does not deliver.
- `GAP-no-witness`: no test observes contract-forced materialization.
- `GAP-raw-path`: `send_response_framed` is uncounted.
- `GAP-register-flips`: two inversion-register entries will not flip.
- `GAP-t1-units`: `T1` misstates the latency–throughput equivalence.
- `GAP-t2-load`: `T2` floors ignore host load.
- `GAP-stale-wording`: the benchmark plan and dedicated roadmap use superseded
  wording, and the benchmark plan cites a blocker that does not exist.

Trace links:

```plaintext
roadmap-10.2.2 -> REQ-define  -> D1 -> EP-M4 -> ADR-0010 addendum + baselines "Acceptance thresholds" T4/T6
roadmap-10.2.2 -> REQ-witness -> D2 -> EP-M1 -> copied_bytes::shared_payload::shared_payload_serializer_copies_the_payload
roadmap-10.2.2 -> REQ-guard   -> D1 -> EP-M3 -> pipeline_allocation_invariants::production_outbound_allocation_count_is_stable
roadmap-10.2.2 -> REQ-coverage -> D3 -> EP-M2 -> copied_bytes::raw_response::raw_framed_send_records_the_serializer_copy
roadmap-10.2.2 -> REQ-trace   -> D4 -> EP-M3 -> copied_bytes::threshold_register::threshold_register_names_real_tests
roadmap-10.2.2 -> REQ-sound   -> D5 -> EP-M4 -> baselines "Comparison protocol", T1, T2
roadmap-10.2.2 -> REQ-accept  -> D9 -> EP-M5 -> baselines "Threshold review record"
roadmap-13.2.1 <- D6 (verdict kernel and Verus proof deferred, sketched below)
```

## Verification plan

### Abstract model

Model one outbound message as a pair `(m, s)`. `m` is a message whose encoded
form has `P > 0` bytes, and `s` is a serializer. Messages fall into two classes:

- *Shared*: `m` holds its encoded bytes as a `Bytes` value `b` with data
  pointer `p(b)`. Its encoded form is `b` itself.
- *Encoded*: `m` is a typed value that `s` must encode, for example Bincode
  over `Envelope`.

The outbound seam `E(s, c, m)` (`encode_message_frame` with codec `c`) yields a
frame `f` whose payload has pointer `p(f)` and length `P`. It changes the
counter state by `Δ = (serialized, handoff, framed, deserialized)` and opens an
allocation window `W = (ops, bytes)`. Framing by `Encoder::encode` is a
separate step `F(c, f)` that adds `Δ.framed = P`.

The pre-migration contract (`Vec<u8>` return) forces, for every class:
`Δ.serialized = P`, `Δ.handoff = 0`, and a fresh payload buffer, so for a
shared message `p(f) ≠ p(b)` and `W.bytes ≥ P`.

The post-migration requirement (`D1`) is:

- For a shared message and a serializer that returns `b` without copying:
  `p(f) = p(b)`, `Δ.serialized = 0`, and `W.bytes < P`. The total with framing
  is `P`.
- For an encoded message under Bincode: `Δ.serialized = P` (inherent),
  `Δ.handoff = 0`, `P ≤ W.bytes < 2P`, and the allocation-operation count is at
  most the pre-migration count. The total with framing stays `2P`.

Malformed inputs (serializer errors) produce `SendError::Serialize` and record
no bytes. The rejected-frame test already covers the inbound analogue.

### Invariants and lemmas

- `INV-preforced` (`REQ-witness`): before `11.1.2`, for every non-empty shared
  payload and every in-scope codec (length-delimited, Hotline), `E` copies:
  `p(f) ≠ p(b)`, `Δ.serialized = P`, and `W.bytes ≥ P`.
- `INV-postshared` (`REQ-define`; stated now, executed by `11.1.2`): after
  `11.1.2`, the same witness gives `p(f) = p(b)`, `Δ.serialized = 0`, and
  `W.bytes < P`. This is `INV-preforced` inverted. The inversion register binds
  the two.
- `INV-guard` (`REQ-guard`): for Bincode, `Δ.handoff = 0` and
  `P ≤ W.bytes < 2P` (existing), plus a stable allocation-operation count
  independent of payload class (new).
- `INV-raw` (`REQ-coverage`): `send_response_framed` records
  `Δ.serialized = encoded length` and `Δ.handoff = 0`, matching
  `encode_message_frame` for the same serializer and message.
- `INV-trace` (`REQ-trace`): every threshold `T1`..`T7` appears exactly once in
  the threshold register, identifiers are contiguous, every `test`-kind entry
  names at least one `fn <name>(` that exists in its named file, and every
  `procedure`-kind entry names a heading that exists in the baselines document.
- `L-control` (non-vacuity for `INV-preforced` and `INV-postshared`): for the
  same `b`, `c.wrap_payload(b.clone())` gives `p(f) = p(b)`. If this lemma
  fails, the pointer comparison itself is broken, and neither invariant's
  result means anything.
- `L-register-sound`: the shared parser accepts exactly the marker-delimited
  `-` items, joins two-space continuation lines, rejects a block without
  markers or with an item lacking its required spans, and preserves item order.

### Axioms (trusted, not verified here)

- `AXIOM-bytes-from-vec`: `bytes` 1.x `From<Vec<u8>> for Bytes` does not copy
  payload bytes. It is trusted because `bytes_from_vec` checks it at run time
  and `handoff_preserves_the_payload_pointer` tests it.
- `AXIOM-bytes-clone`: `Bytes::clone` shares the buffer, so the pointer is
  equal and no payload bytes are copied (documented `bytes` contract).
- `AXIOM-allocator-window`: the `stats_alloc`-based window counts every global
  allocation made by the current thread while open. The existing harness
  self-checks (`harness_observes_real_traffic`,
  `idle_windows_report_no_traffic`) exercise this boundary.
- `AXIOM-criterion`: Criterion 0.8.2 reports change as a confidence interval
  on the relative change in mean time, with the documented defaults above. Only
  the procedural thresholds `T1` and `T2` depend on this.
- `AXIOM-bincode-size`: Bincode 2.0.1 `encode_to_vec` allocates once, with
  exact capacity. The existing `default_serializer_leaves_no_spare_capacity`
  test exercises it.

### Methods per obligation

- Obligation: `INV-preforced`.
  Method: parameterized `rstest` table over codec × payload class, plus a
  `proptest` property over lengths. Rationale: the classes form a finite
  matrix, and the property covers the length range cheaply. Domain:
  `{LengthDelimited, Hotline} × {32 B, 64 KiB}`, plus generated lengths in
  `1..=65_536` with explicit edge cases 1, 250, 251, 4,096, and 65,536.
  Artefacts: `wireframe_testing/tests/copied_bytes/shared_payload.rs`
  (`shared_payload_serializer_copies_the_payload`,
  `shared_payload_is_copied_at_any_length`), and
  `wireframe_testing/tests/pipeline_allocation_invariants/checks/shared.rs`
  (`shared_payload_outbound_allocates_a_payload_copy`). Evidence: `make test`
  passes. The red stage is a compile failure on the missing witness module,
  followed by a deliberately inverted assertion (`p(f) == p(b)`) that must fail
  before the correct one is committed. Non-vacuity: `L-control` runs in the
  same module on the same `b`. Lengths start at 1, because empty `Bytes` have
  no meaningful pointer. Seeded fault: temporarily make the witness serializer
  return a `Vec` built from `b.to_vec()` *and* compare with `b` itself; the
  inverted assertion must still fail. Revert the fault before committing.
  Record the proptest case count (256 by default) and any regression file in
  `Artefacts and notes`.
- Obligation: `INV-postshared`.
  Method: specification only in this plan. It is registered in the inversion
  register so that `11.1.2` must replace `INV-preforced`'s assertions with it.
  Rationale: it cannot be executed before the contract changes. Evidence:
  `inversion_register_names_real_tests` fails if the registered tests vanish
  without the register being updated. Non-vacuity: the register entry states
  the exact post-migration values, so a migration that deletes the test instead
  of inverting it fails the register gate.
- Obligation: `INV-guard`.
  Method: allocator-window check in the `harness = false` binary. Artefact:
  `checks/handoff.rs`, new `production_outbound_allocation_count_is_stable`.
  Evidence: the red stage first records the measured operation counts for 32 B
  and 64 KiB in `Artefacts and notes`. The check asserts equality across
  payload classes and equality with the recorded count. If the count is not
  stable across ten runs, fall back to class independence only and record why.
  Non-vacuity: seeded fault. Temporarily route the check through a serializer
  that grows a `Vec` from zero capacity by repeated `push`; the check must
  fail. Revert afterwards.
- Obligation: `INV-raw`.
  Method: named `#[serial]` test through a real `Framed` over
  `tokio::io::duplex`. Artefact:
  `wireframe_testing/tests/copied_bytes/raw_response.rs`
  (`raw_framed_send_records_the_serializer_copy`). Evidence: red, the test
  fails with `serialized: 0`; green after the recorder calls are added.
  `tests/response/mod.rs::send_response_framed_sends_raw_serialized_payload`
  must stay green, proving the wire output is unchanged. Non-vacuity: the test
  also reads the frame from the peer and asserts its length equals the recorded
  count, so a counter recording the wrong length fails.
- Obligation: `INV-trace` and `L-register-sound`.
  Method: unit tests over the parser with fixture strings, plus the live
  register test over `include_str!` of the baselines document. Artefacts:
  `wireframe_testing/tests/copied_bytes/doc_register.rs` (shared parser and its
  tests), `inversion_register.rs` (slimmed), and `threshold_register.rs`.
  Evidence: `make test`. Negative controls are fixture strings with a missing
  test, a duplicate `T3`, a gap (`T1`, `T2`, `T4`), and a `procedure` entry
  naming an absent heading; each must be rejected with a specific error.
  Non-vacuity: the live test asserts the register has exactly seven entries, so
  an empty or truncated block cannot pass.
- Obligation: BDD acceptance of `INV-preforced` and `L-control`.
  Method: `rstest-bdd` scenarios (below). Artefacts:
  `tests/features/zero_copy_baseline.feature`, steps, fixture, and scenarios.
  Evidence: `make test-bdd`; the new scenarios fail before their steps exist,
  through rstest-bdd's strict compile-time validation, and pass after.

### Formal verification decision

This item introduces no algorithm, protocol, or arithmetic kernel. Its
obligations are facts about concrete executions (pointer identity, byte counts,
allocations), checked against the real code by tests whose non-vacuity is
argued above. Verus would add nothing: the properties depend on the behaviour of
`bytes`, the allocator, and serializers, which are axioms here and not proof
subjects. Kani is likewise unwarranted: there is no bounded pure function whose
every path matters more than these concrete witnesses.

Verus *is* warranted for `13.2.1`'s verdict kernel (`D6`). That kernel will be
contractual release-gating logic over all integer inputs, and Clippy's
`float_arithmetic` denial already forces it into integers, which suits Verus.
The proof sketch below is a hypothesis about its eventual proof structure, not
a commitment. No `assume`, `admit`, or external body may be used to make it
compile, and `13.2.1` must add the `make verus` milestone criterion when it
implements the kernel.

Abstract model for `13.2.1`. Work in basis points (bp; 1 % = 100 bp) of
relative change in mean time. A workload result is `r = (lo, est, hi)` with
integers `lo ≤ est ≤ hi`. Parameters are `R = 1000` (10 %) and `N = 100` (1 %).
Define `regressed(r) ⇔ est > R ∧ lo > N`. A comparison is a finite map `C` from
workload identifier to `Measured(r)` or `Unmeasured`. Its verdict is `Fail` if
some workload is `Measured(r)` with `regressed(r)`, otherwise `Unmeasured` if
some workload is unmeasured, otherwise `Pass`. A `T2` floor check takes
throughput `x`, floor `F`, and load `ℓ` with band `[ℓ₀, ℓ₁]`: `NotComparable` if
`ℓ ∉ [ℓ₀, ℓ₁]`, otherwise `Pass` if `x ≥ F`, otherwise `Fail`.

Lemmas to establish:

- `L-noise`: `lo ≤ N ⇒ ¬regressed(r)`. A result within noise never fails.
- `L-monotone`: if `r ≤ r'` componentwise, then
  `regressed(r) ⇒ regressed(r')`. A worse result never passes where a better
  one failed.
- `L-no-offset`: if `regressed(C(w))` for some `w`, then `verdict(C) = Fail`
  for every assignment of the other workloads. No gain elsewhere offsets a
  regression; this is `T1`'s "never to an average".
- `L-unmeasured-not-pass`: if any workload is `Unmeasured`, then
  `verdict(C) ≠ Pass` (`T7`).
- `L-units`: for `est > -10000`, the throughput change in bp is
  `t = -10000·est / (10000 + est)`, and `est > 1000 ⇔ t < -909.09…`. Stated
  over integers, `est > 1000 ⇔ 10000·est > 1000·(10000 + est)`. This lemma
  justifies restating `T1` on time.
- `L-floor-band`: `ℓ ∉ [ℓ₀, ℓ₁] ⇒ floor = NotComparable`. Load outside the
  band can neither pass nor fail `T2`.

These statements stay in this plan and in the baselines document's comparison
protocol until `13.2.1` reaches its proof milestone.

### Suggested BDD features

Adopted (`EP-M1`), extending `tests/features/zero_copy_baseline.feature` and
linked to `REQ-witness`, `INV-preforced`, and `L-control`:

```gherkin
  Scenario: The serializer copies a shared payload before framing
    Given a message holding a shared payload of 65536 bytes
    When the message is serialized and wrapped by the length-delimited codec
    Then the frame payload does not share the message's buffer
    And the serializer seam records 65536 copied bytes

  Scenario: A shared payload wrapped without serialization keeps its buffer
    Given a message holding a shared payload of 65536 bytes
    When the shared payload is wrapped directly by the length-delimited codec
    Then the frame payload shares the message's buffer
    And the serializer seam records 0 copied bytes
```

The first scenario's binding function,
`the_serializer_copies_a_shared_payload_before_framing`, is entered in the
inversion register. Item `11.1.2` must rewrite it to "the frame payload shares
the message's buffer" with 0 copied bytes. Run with `make test-bdd`.

## Plan of work

### `EP-M1` shared-payload removal witness

Requirements: `REQ-witness`, `GAP-no-witness`; decisions `D1` and `D2`.

Red. In `wireframe_testing/tests/copied_bytes.rs`, add
`#[path = "copied_bytes/shared_payload.rs"] mod shared_payload;`, and write
`shared_payload.rs` against a not-yet-existing
`wireframe_testing::codec_benchmarks::shared_payload` API. Run
`make test-wireframe-testing` and expect an unresolved-import error.

Green. Create `wireframe_testing/src/codec_benchmarks/shared_payload.rs` (about
120 lines, `#[cfg(feature = "pipeline-benchmarks")]`, module doc comment). It
provides:

- `SharedPayloadSerializer`, a `Serializer` whose
  `serialize` returns the message's bytes. Before `11.1.2` this is necessarily
  `Ok(message.payload.to_vec())`, and the doc comment says so and names
  `11.1.2`.
- `SharedPayloadMessage { payload: Bytes }`, with
  `EncodeWith<SharedPayloadSerializer>` and an accessor `payload()`.
- `shared_payload_message(len: NonZeroUsize) -> SharedPayloadMessage`,
  which fills a non-repeating byte pattern.
- `encode_shared_payload(codec, message)`, returning
  `Result<SharedPayloadFrame, String>`, which wraps
  `benchmark_encode_message_frame` and returns the payload pointer and length.
- `wrap_shared_payload_directly(codec, message) -> SharedPayloadFrame` for
  `L-control`.

Re-export the module from `codec_benchmarks/mod.rs`. Then make the tests pass:
`shared_payload_serializer_copies_the_payload` (rstest over `CodecUnderTest` ×
payload class), `shared_payload_control_preserves_the_pointer`, and the property
`shared_payload_is_copied_at_any_length`. Use googletest `assert_that!` with
`eq` and `not(eq)` matchers for pointer and count assertions, and
`pretty_assertions::assert_eq` for whole `CopiedBytes` values.

Add `checks/shared.rs` to `pipeline_allocation_invariants` with
`shared_payload_outbound_allocates_a_payload_copy` (window `bytes ≥ P` for both
classes) and `shared_payload_direct_wrap_allocates_no_payload` (window
`bytes < P`, the allocator-level control). Register both in `checks::CHECKS`.

Add the two BDD scenarios, their steps in
`tests/steps/zero_copy_baseline_steps.rs`, world methods in
`tests/fixtures/zero_copy_baseline.rs`, and bindings in
`tests/scenarios/zero_copy_baseline_scenarios.rs`.

Refactor. Remove duplication between the witness module and existing harness
helpers where it does not touch harness measurement functions. Run the seeded
fault described in `Verification plan`, record its output, and revert it.

Validation: `make check-fmt`, `make lint`, and `make test`, sequentially, with
logs. Expected: all pass, the new tests counted in the `copied_bytes` and `bdd`
summaries, and `pipeline_allocation_invariants` printing the two new check
names.

### `EP-M2` raw length-delimited send path counted

Requirements: `REQ-coverage`, `GAP-raw-path`; decision `D3`.

Red. Add `copied_bytes/raw_response.rs` with
`raw_framed_send_records_the_serializer_copy`. It builds a `WireframeApp` with
`SharedPayloadSerializer`, sends a witness message through
`send_response_framed` over `tokio::io::duplex`, reads the frame back with a
`LengthDelimitedCodec`, and asserts `serialized == P`, `handoff == 0`, and a
frame length of `P`. Expect a failure at `serialized`: actual 0, expected `P`.

Green. In `src/app/outbound_response.rs::send_response_framed`, call
`crate::benchmark_counters::record_serialized(bytes.len())` after
serialization, and replace `Bytes::from(bytes)` with
`crate::benchmark_counters::bytes_from_vec(bytes)`. Keep the existing comments
about double framing and `tokio-util` inference, and add one line naming the
uncounted `tokio-util` length-prefix copy (`D3`). Register the test in the
inversion register: after `11.1.2`, `serialized` must be 0 for the witness.

Validation: the gate trio. `tests/response/mod.rs` must remain green.

### `EP-M3` registers and the Bincode guard

Requirements: `REQ-trace`, `REQ-guard`, `GAP-register-flips`; decisions `D1` and
`D4`.

Refactor first, with tests green before and after. Move the generic parsing in
`copied_bytes/inversion_register.rs` (`register_items`, `register_entries`,
`missing_register_entries`, and their unit tests) into
`copied_bytes/doc_register.rs`, parameterized by marker name and by the number
of backtick spans required per item. `inversion_register.rs` keeps only its
live test.

Red. Write `copied_bytes/threshold_register.rs` with
`threshold_register_names_real_tests` and the negative-control fixture tests.
Expect failure: the markers `<!-- threshold-register:start -->` and
`<!-- threshold-register:end -->` do not yet exist in the baselines document.
Then add the register block to `docs/zero-copy-benchmark-baselines.md`, in the
"Acceptance thresholds" section, in this format:

```markdown
- `T4` `test` `wireframe_testing/tests/copied_bytes/shared_payload.rs`
  `shared_payload_serializer_copies_the_payload` — removal of the serializer
  materialization for shared payloads.
- `T1` `procedure` `Comparison protocol` — Criterion time-change rule.
```

For a `test` entry, the third span is the file and every further span is a test
name. For a `procedure` entry, the third span is a heading in the baselines
document. Green when all seven thresholds are listed and every reference
resolves.

Correct the inversion register. Remove
`default_serializer_and_handoff_copy_the_payload_once` and
`production_outbound_allocates_one_payload`; they become `T3`/`T4` guards in
the threshold register. Add `shared_payload_serializer_copies_the_payload`,
`shared_payload_is_copied_at_any_length`,
`shared_payload_outbound_allocates_a_payload_copy`,
`raw_framed_send_records_the_serializer_copy`, and
`the_serializer_copies_a_shared_payload_before_framing`. Update the live
inversion test's expected count and required names to match. The register gate
accepts entries in any of the named files because it greps for `fn <name>(`;
this includes the `checks/*.rs` files and the BDD scenarios file.

Add the guard. In `checks/handoff.rs`, red-first record the measured operation
counts (see `Verification plan`), then add
`production_outbound_allocation_count_is_stable` and register it. Check the
file stays under 400 lines; split into `checks/guards.rs` if not.

Validation: the gate trio.

### `EP-M4` threshold text and dependent documents

Requirements: `REQ-define`, `REQ-sound`, `GAP-adr010-overclaim`, `GAP-t1-units`,
`GAP-t2-load`, `GAP-stale-wording`; decisions `D1`, `D3`, `D5`, and `D8`. This
milestone is documentation only, and the thresholds stay marked "proposed".

In `docs/zero-copy-benchmark-baselines.md`:

- Add a short "What the migration must remove" subsection that states `D1`
  with the shared and encoded classes and the expected per-seam table for each
  after `11.1.2`.
- Rewrite `T1` on time change only, with throughput derived (`L-units`).
- Make `T2` conditional on the C4 load band (proposed: 1-minute load average
  within 15–25 on the 24-thread baseline host) and the `make bench-codec` entry
  point. Outside the band the result is "not comparable".
- Restate `T3` so it names which counts are test-enforced and which are
  benchmark-label procedure, and add the requirement that `11.1.2`'s `BytesMut`
  writer must pre-size, as Bincode's `encode_to_vec` does.
- Split `T4` into `T4a` (shared witness: pointer identity, zero serializer
  bytes, no payload allocation) and `T4b` (Bincode guard), each naming its
  tests. Keep the identifier `T4` for the register entry and list both tests.
- Scope `T6` to the shared witness (`P`) and the Bincode guard (`2P`,
  unchanged).
- Correct `T7`: name the `tokio-util` length-prefix copy as the one uncounted
  copy, and say why.
- Extend the "Comparison protocol" with frozen benchmark identifiers, the
  A–B–A bracketing rule for `13.2.1`, recorded load, and the rule that drift
  between the two A runs beyond the noise threshold makes the comparison
  inconclusive.
- Add the threshold register and the corrected inversion register.
- Add an empty "Threshold review record" section stating that no reviewer has
  agreed yet.

Elsewhere:

- Append a dated addendum to ADR 010 that supersedes the sentence "Item
  `11.1.2` therefore removes one allocation and one copy on the outbound
  default path", states `D1`, and links the baselines section.
- `docs/zero-copy-benchmark-plan.md`: replace the "between serialization and
  `wrap_payload`" rule with `D1`'s wording and repair the phantom blocker row
  to point at the review record.
- `docs/zero-copy-frame-and-payload-migration-roadmap.md`: replace the same
  superseded wording in the success criteria and in item `1.2.2`'s sub-bullet.
  Do not tick anything yet.
- `docs/developers-guide.md` ("Example and benchmark support"): document the
  witness module and its reuse policy (the only sanctioned shared-payload
  witness; harnesses must not grow their own), the two registers and how to
  change a threshold (only with reviewer agreement, recorded in the review
  record), and the A–B–A procedure.
- `docs/users-guide.md`: in the `CopiedBytes` section, note that
  `send_response_framed` now reports its serializer seam under
  `benchmark-counters`, and that `tokio-util`'s length-prefix copy is not
  counted.
- `docs/contents.md`: add this ExecPlan to the execution-plan list.

Validation: `make fmt`, `make check-fmt`, `make markdownlint`, `make nixie`,
`make lint`, and `make test`. The threshold register test re-runs because the
baselines document is compiled in with `include_str!`.

### `EP-M5` performance review and acceptance

Requirements: `REQ-accept`; decision `D9`. This milestone is blocked until a
named reviewer agrees.

Prepare a review packet as a pull-request comment. It summarizes `D1` and its
rejected alternative first, then each threshold's old and new text, the
enforcing tests, and the evidence from `EP-M1` to `EP-M3`. Ask the user to name
the performance reviewer if the user is not acting as reviewer. Set this plan's
status to `BLOCKED (awaiting performance reviewer)`.

On agreement, fill in the "Threshold review record" with the reviewer's login,
date, the baselines document's blob identifier
(`git rev-parse HEAD:docs/zero-copy-benchmark-baselines.md`), and the
dispositions. Change "proposed" to "accepted" throughout the thresholds section
and in `docs/contents.md`. Tick `10.2.2` in `docs/roadmap.md` and item `1.2.2`
with its two open sub-bullets in the dedicated roadmap. Remove the release
plan's "no performance-reviewer sign-off" blocker row, and mark the benchmark
plan's sign-off gap row closed. If the reviewer amends a threshold, apply the
amendment, re-run the gates, and record it in `Decision log` before filling in
the record. If the reviewer rejects `D1`, set `BLOCKED` and escalate (see
`Tolerances`).

Validation: the documentation gates plus `make test`, because the registers
read the document.

## Milestones and plateaus

- `EP-M1`: shared-payload witness present and green.
  - Requirements: `REQ-witness`.
  - Evidence: `shared_payload_serializer_copies_the_payload`,
    `shared_payload_control_preserves_the_pointer`,
    `shared_payload_is_copied_at_any_length`, two allocation checks, and two
    BDD scenarios.
  - Conformance: no `wireframe` interface change; no harness change; the
    witness is feature-gated.
  - Recovery: revert the milestone commits. Nothing else depends on them yet.
  - Remaining gaps: not yet registered (`EP-M3`).
  - Compatibility: none.
- `EP-M2`: raw path counted.
  - Requirements: `REQ-coverage`.
  - Evidence: `raw_framed_send_records_the_serializer_copy` red then green;
    `send_response_framed_sends_raw_serialized_payload` green.
  - Conformance: recorder calls only; no signature or wire change.
  - Recovery: revert the single production edit and its test together.
  - Compatibility: none.
- `EP-M3`: registers and guard.
  - Requirements: `REQ-trace`, `REQ-guard`.
  - Evidence: `threshold_register_names_real_tests`, parser negative
    controls, the corrected `inversion_register_names_real_tests`, and
    `production_outbound_allocation_count_is_stable`.
  - Conformance: tests and documentation markers only.
  - Recovery: the parser refactor is its own commit and can be reverted
    independently of the register additions.
  - Compatibility: none.
- `EP-M4`: proposed thresholds coherent and documented.
  - Requirements: `REQ-define`, `REQ-sound`.
  - Evidence: documentation gates green; the register tests green against the
    rewritten document.
  - Conformance: ADR changes are addenda only; thresholds still "proposed".
  - Recovery: revert documentation commits; the tests from `EP-M1` to `EP-M3`
    remain valid against the previous wording except the threshold register,
    which must be reverted with it.
  - Compatibility: none.
- `EP-M5`: thresholds accepted.
  - Requirements: `REQ-accept`.
  - Evidence: the review record and the reviewer's pull-request comment.
  - Conformance: roadmap ticks match the record.
  - Recovery: if acceptance is withdrawn before merge, revert this
    milestone's single commit.
  - Compatibility: none.

## Concrete steps

Run every command from the worktree root. Use the shared Cargo cache. Do not
bypass `RUSTC_WRAPPER`. Run gates sequentially, never in parallel.

```sh
set -o pipefail
BR=$(git branch --show-current)
make check-fmt 2>&1 | tee "/tmp/check-fmt-wireframe-$BR.out"
make lint      2>&1 | tee "/tmp/lint-wireframe-$BR.out"
make test      2>&1 | tee "/tmp/test-wireframe-$BR.out"
```

Documentation milestones add, before the trio:

```sh
make fmt          2>&1 | tee "/tmp/fmt-wireframe-$BR.out"
make markdownlint 2>&1 | tee "/tmp/markdownlint-wireframe-$BR.out"
make nixie        2>&1 | tee "/tmp/nixie-wireframe-$BR.out"
```

Focused commands, for faster red/green loops (these use the same flags as
`make test`):

```sh
RUSTFLAGS="-D warnings" cargo test -p wireframe_testing --all-features \
  --test copied_bytes shared_payload 2>&1 | tee /tmp/red-shared-wireframe-$BR.out
RUSTFLAGS="-D warnings" cargo test -p wireframe_testing --all-features \
  --test pipeline_allocation_invariants 2>&1 | tee /tmp/alloc-wireframe-$BR.out
make test-bdd 2>&1 | tee /tmp/bdd-wireframe-$BR.out
```

Expected transcripts, filled in during implementation:

```plaintext
EP-M1 red:   error[E0432]: unresolved import `wireframe_testing::codec_benchmarks::shared_payload`
EP-M1 green: test shared_payload::shared_payload_serializer_copies_the_payload::case_1 ... ok (4 cases)
EP-M2 red:   assertion failed: serialized  expected 65536  actual 0
EP-M2 green: test raw_response::raw_framed_send_records_the_serializer_copy ... ok
EP-M3 red:   threshold register: missing `<!-- threshold-register:start -->` marker
EP-M3 green: test threshold_register::threshold_register_names_real_tests ... ok
```

Commit after each green plateau, with gates passing, using imperative subjects.
Examples: "Add a shared-payload witness for the serializer materialization",
"Count the raw length-delimited send seam", "Share one parser between the
benchmark registers", and "Restate the zero-copy acceptance thresholds". No
benchmark capture is required by this plan, because no harness changes.

## Validation and acceptance

Quality criteria:

- Tests: `make test` passes. The new tests listed per milestone exist and
  pass. Each was observed failing for the intended reason first; record those
  transcripts in `Artefacts and notes`.
- Verification: `INV-preforced`, `INV-guard`, `INV-raw`, `INV-trace`,
  `L-control`, and `L-register-sound` are discharged by the named tests with
  their non-vacuity controls. `INV-postshared` is discharged by registration
  only; its execution belongs to `11.1.2`. The `13.2.1` lemmas remain
  statements.
- Lint and format: `make check-fmt` and `make lint` pass; `make markdownlint`
  and `make nixie` pass for documentation milestones.
- Performance: no benchmark is run. Thresholds are *recorded*, not *evaluated*;
  evaluation is `13.2.1`'s job.
- Review: `EP-M5`'s review record is filled in by a named reviewer.

Acceptance, phrased as behaviour:

- A reviewer opens `docs/zero-copy-benchmark-baselines.md` and finds each
  threshold with its enforcing tests or procedure. Deleting any named test makes
  `make test` fail in `threshold_register_names_real_tests`.
- Running the `copied_bytes` binary shows that a shared 64 KiB payload is
  copied today (`serialized: 65536`, pointers differ), while the control keeps
  the pointer.
- After `11.1.2` changes the serializer contract, the witness's pre-migration
  assertions fail, and the inversion register directs the implementer to the
  post-migration values (`serialized: 0`, equal pointers).
- `10.2.2` is ticked only after the review record names the reviewer.

## Idempotence and recovery

All test and documentation steps are re-runnable. The seeded faults are
temporary edits: make each in an uncommitted state, run the single focused
command, record the failure, and restore with `git restore <file>`; confirm with
`git status` that nothing remains. Proptest regression files
(`*.proptest-regressions`) produced by a seeded fault must be deleted, not
committed. If a gate fails because the disk or `/tmp` fills, stop and report. If
`make lint` reports missing dylint externs after a full disk, remove
`target/dylint` and re-run. If compile admission is unhealthy, report the full
`build-limits-status` output and continue documentation work.

## Artefacts and notes

Reconnaissance observations (2026-11-28, `fa8ef421`):

```plaintext
src/app/outbound_encoding.rs:36  record_serialized(bytes.len())
src/app/outbound_encoding.rs:41  codec.wrap_payload(bytes_from_vec(bytes))
src/app/outbound_response.rs:118-132  serialize -> Bytes::from(bytes), no recorder
wireframe_testing/tests/copied_bytes.rs:147  serialize_and_wrap.total() + framed.total() == 2 * payload_len
inversion_register.rs:147-152  register must have exactly 6 entries
```

Bincode 2.0.1 `encode_to_vec`, quoted from source:

```rust
let size = { /* SizeWriter pass */ };
let writer = VecWriter::with_capacity(size);
```

Implementation transcripts, seeded-fault results, measured allocation counts,
and proptest case counts are to be added here.

## Interfaces and dependencies

New module `wireframe_testing::codec_benchmarks::shared_payload`, compiled only
with the `pipeline-benchmarks` feature:

```rust
/// Serializer whose messages already hold their encoded bytes.
pub struct SharedPayloadSerializer;

/// Message holding its encoded form as a shared buffer.
pub struct SharedPayloadMessage { payload: Bytes }

impl SharedPayloadMessage {
    pub fn payload(&self) -> &Bytes { &self.payload }
}

/// Pointer and length of a frame's payload, for identity assertions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SharedPayloadFrame { pub payload_ptr: *const u8, pub payload_len: usize }

pub fn shared_payload_message(len: NonZeroUsize) -> SharedPayloadMessage;
pub fn encode_shared_payload(
    codec: CodecUnderTest,
    message: &SharedPayloadMessage,
) -> Result<SharedPayloadFrame, String>;
pub fn wrap_shared_payload_directly(
    codec: CodecUnderTest,
    message: &SharedPayloadMessage,
) -> SharedPayloadFrame;
```

The raw pointer in `SharedPayloadFrame` is used only for equality and is never
dereferenced. If Clippy or Whitaker objects to a raw-pointer field, store
`usize` from `as_ptr() as usize` (an address only) and document that. The
witness reuses `CodecUnderTest` and `payload_for_class` from
`codec_benchmark_support` rather than defining new codec selectors.

Reuse policy: this module is the only sanctioned shared-payload witness.
Harnesses and tests that need one import it. `11.1.2` edits only
`SharedPayloadSerializer`'s body (to return the shared payload) and the
registered assertions.

Dependencies: add to `wireframe_testing/Cargo.toml` `[dev-dependencies]`:

```toml
googletest = "0.14.3"
pretty_assertions = "1.4.1"
```

No production dependency changes.

## Revision note

- 2026-11-28: initial draft from Wyvern reconnaissance and Firecrawl checks.
  Awaiting the community-of-experts design review and user approval.
