# 10.2.2 Record benchmark acceptance thresholds that require removal of the serializer materialization

This ExecPlan (execution plan) is a living document. The sections `Constraints`,
`Tolerances`, `Risks`, `Progress`, `Surprises & discoveries`, `Decision log`,
`Outcomes & retrospective`, `Conformance basis`, and `Verification plan` must
be kept up to date as work proceeds.

Status: DRAFT (design review applied; awaiting user approval)

Branch: `10-2-2-record-benchmark-acceptance-thresholds`

Roadmap item: `10.2.2` in `docs/roadmap.md`, mirrored by item `1.2.2` in
`docs/zero-copy-frame-and-payload-migration-roadmap.md`.

## Purpose / big picture

Wireframe is migrating its outbound byte path from `Vec<u8>` to shared
`bytes::Bytes` buffers (Epic #284). Today `Serializer::serialize` must return a
freshly allocated, payload-sized `Vec<u8>`. `src/app/outbound_encoding.rs`
function `encode_message_frame` converts that vector to `Bytes` without a copy
and hands it to `FrameCodec::wrap_payload`. The vector is the *serializer
materialization*. Roadmap item `11.1.2` will change the serializer contract so
that output can be shared rather than materialized.

Roadmap item `10.2.2` must fix, before that migration starts, the measurable
lines a candidate migration has to cross. A *threshold* is a written rule, tied
to a named measurement, that a later comparison either meets or must explain.
Thresholds `T1` to `T7` are proposed in
`docs/zero-copy-benchmark-baselines.md`. A performance reviewer cannot agree to
them as written, for three reasons.

1. They do not say which materialization must disappear. The accepted ADR 010
   addendum (2026-11-20) says `11.1.2` "removes one allocation and one copy on
   the outbound default path". The draft `11.1.2` design (the issue #538 plan)
   does not deliver that: Bincode must still encode a typed message into a
   payload-sized buffer, a `BytesMut` instead of a `Vec<u8>`. Read strictly,
   `T6` (outbound total at most one payload copy) fails a correct migration.
   Read loosely, it passes one that changes nothing.
2. Nothing can observe the removal, for two reasons:
   - No test compares the address of a payload the message already owns with
     the address of the frame's payload after serialization.
   - The serializer-seam counter records the length of every serializer
     output, whether or not anything was copied. A correct migration would
     therefore still report a copy.
3. Several statements are wrong or unworkable:
   - `T1` calls +10 % latency equivalent to −10 % throughput. It is about
     −9.09 %.
   - `T7` says no stage is unmeasured, yet three copies go uncounted.
   - `T2`'s absolute floors ignore host load, which moved large-payload
     throughput by 15–18 % between two captures of identical code.
   - Four of the six "inversion register" entries would not change when
     `11.1.2` lands, so the register misleads whoever implements it.

After this plan, the thresholds are stated precisely and tied to tests by a
gate:

- A new ADR records what "removing the serializer materialization" means.
- A test-only *shared-payload witness* (a serializer whose message already
  holds its encoded bytes as `Bytes`) shows the copy today.
- A single switch constant in that witness must flip, and its assertions must
  pass, before `docs/roadmap.md` can tick `11.1.2`.
- A *threshold register* (a marker-delimited list in the baselines document)
  names the test or procedure behind every threshold, and the build fails when
  a named test disappears.
- The timing comparison protocol is rebuilt so that it can produce a verdict on
  this shared, heavily loaded host. A same-revision trial run (a baseline
  compared against itself, called an A–A probe) provides evidence that it can.

The thresholds become *accepted* when a named performance reviewer accepts the
new ADR and the review record. That closes `10.2.2`.

Observable outcome, in one example. With the `pipeline-benchmarks` feature,
take a 65,536-byte payload held in a `Bytes` and send it through
`encode_message_frame`. Today the frame's payload has a different address from
the message's buffer, and the counters report `serialized: 65536`. A named test
asserts this. The same test, with the witness switch set to `Shared`, fails
today and must pass once `11.1.2` lands.

Behaviour preserved: this plan changes no production source file, no public
signature, no wire format, and no benchmark harness, so captures C4 and C5 stay
valid.

## Constraints

- Implementation begins only after the user approves this plan. The acceptance
  milestone (`EP-M4`) also requires a named performance reviewer to accept ADR
  0018. No agent, including the design-review panel, may supply that acceptance.
- No production source changes. Files under `src/` are out of scope, and so is
  `wireframe_testing/src/`, because `wireframe_testing` 0.3.0 is a published
  crate (`lading.toml` orders it for release). The witness is test-private.
- Do not modify the benchmark harnesses: `benches/*.rs` and
  `wireframe_testing/src/codec_benchmarks/**`. A changed harness invalidates
  its capture.
- Authorized dependency changes: `googletest = "0.14.3"` and
  `pretty_assertions = "1.4.1"` as `[dev-dependencies]` of `wireframe_testing`,
  matching the root manifest. No other dependency. No `insta` (`D7`).
- Every code file stays at or below 400 lines (`AGENTS.md`). At the base
  revision, `wireframe_testing/tests/copied_bytes.rs` has 332 lines, so new
  tests go in new modules.
- Lint posture holds. No `unsafe`. Suppressions only as tightly scoped
  `#[expect(.., reason = "...")]`. `float_arithmetic`, `integer_division`, and
  `indexing_slicing` are denied in `Cargo.toml`. `std::env` is denied in
  `clippy.toml`. No `.expect()` outside test code. Whitaker's cognitive
  complexity ceiling (`dylint.toml`) applies to test helpers too.
- Accepted ADR sections are immutable. A targeted correction to an accepted ADR
  uses an `Amended by ADR-nnnn (YYYY-MM-DD)` banner in its Status section and an
  `## Amendments` section (`docs/documentation-style-guide.md`, lines 555–557).
- Do not edit completed or other items' ExecPlans
  (`docs/execplans/10-2-1-capture-baselines.md`,
  `docs/execplans/issue-538-plan-zero-copy-serializer-output-migration.md`).
  Obligations for `11.1.2` go into `docs/roadmap.md`, item `11.1.2.1`, and into
  ADR 0018.
- `docs/roadmap.md` is `mapsplice`-shaped: inline links only, no footnotes.
- Documentation uses en-GB-oxendict spelling, wraps prose at 80 columns, and
  passes `make fmt`, `make markdownlint`, and `make nixie`.

## Tolerances (exception triggers)

The comparison base is `origin/main` at `fa8ef421`. This ExecPlan file is
excluded from every count, including when it changes in the same commit. Count
with:

```sh
git diff --numstat fa8ef421...HEAD -- . ':(exclude)docs/execplans/**'
```

Classification is by whole file, by purpose:

- Code: `*.rs` files, and any `*.proptest-regressions` file.
- Configuration: `Cargo.toml` and `Cargo.lock`.
- Documentation: every other `*.md` file outside `docs/execplans/`.

Each limit measures net lines (additions minus deletions) in its own category.
Spare capacity in one category never offsets another.

- Files: stop and escalate if more than 24 non-ExecPlan files change.
- Code: stop and escalate above 800 net lines. Checkpoint: if `EP-M1` alone
  uses more than 320 (40 %), stop and re-estimate before `EP-M2`.
- Configuration: stop and escalate above 12 net lines.
- Documentation: stop and escalate above 650 net lines.
- Interface: stop and escalate if any file under `src/` or
  `wireframe_testing/src/` must change.
- Dependencies: stop and escalate if anything beyond the two authorized
  dev-dependencies is needed.
- Architecture: stop and escalate if ADR 0018 would need to change ADR 008, 009,
  or 010's decisions rather than correct a measurement claim, or if the user or
  reviewer rejects `D1`.
- Iterations: stop and escalate if a gate still fails after three fix attempts
  on the same cause.
- Time: stop and escalate if a milestone takes more than one working day of
  active effort, excluding compile-queue waits and the A–A probe's run time.
- Ambiguity: stop and present options if reviewer feedback admits two
  materially different threshold readings.

The budgets rest on a survey, recorded in `Context and orientation`. In summary:

- Estimates: about 580 net code lines, 3 configuration lines, and 500
  documentation lines, across 21 files.
- Main historical risk: the sibling plan `10-2-1-capture-baselines.md` overran
  its code budget about threefold, because it built new harness binaries and
  split modules late.
- This plan's mitigations: it forbids harness changes, adds no production
  code, and splits the new tests into modules up front.

## Risks

- Risk: the user or reviewer rejects `D1` and requires the Bincode default path
  to drop to one payload copy per message.
  - Severity: high. Likelihood: medium.
  - Why it matters: that requires serializing into the transport write buffer,
    which crosses the accepted ADR 010 frame boundary.
  - Mitigation: `D1` and its rejected alternative lead the approval decision
    packet. On rejection, set `BLOCKED` and escalate; do not improvise a
    transport redesign.
- Risk: no performance reviewer is assigned. The release plan lists the role as
  unassigned (`docs/zero-copy-release-plan.md`, "Owners").
  - Severity: medium. Likelihood: high.
  - Mitigation: ask at plan approval for the user to take the role or name
    someone. `EP-M1` to `EP-M3` merge as pull request A without the reviewer.
    Acceptance is a separate small pull request B (`D9`), so nothing sits in
    `BLOCKED`.
- Risk: the A–A probe shows host drift so large that no timing verdict is ever
  possible, even with interleaving.
  - Severity: medium. Likelihood: medium.
  - Mitigation: that result is itself evidence for the reviewer. `D5` then
    makes `T1` an "explain" rule rather than a failing rule, and the
    deterministic gates (`T3` to `T6`) carry the acceptance. The probe result
    and this fallback are recorded in the review packet.
- Risk: the witness passes vacuously, for example because its address
  comparison is broken or it bypasses `encode_message_frame`.
  - Severity: high. Likelihood: low.
  - Mitigation: one address helper shared with a control check, content
    equality, three mandatory seeded mutants, and a `Shared` switch flip that
    must fail today (see `Verification plan`).
- Risk: process-global counters and the counting allocator make tests flaky.
  - Severity: medium. Likelihood: medium.
  - Mitigation: counter tests run `#[serial]` in the `copied_bytes` binary, and
    allocator checks join the single-threaded `harness = false`
    `pipeline_allocation_invariants` binary. No counter assertion goes into the
    parallel BDD binary (`D10`).
- Risk: Bincode's allocation-operation count differs between payload classes
  today, so the class-independence guard cannot be stated.
  - Severity: low. Likelihood: low.
  - Mitigation: measure it in `EP-M2` before writing the assertion. If it
    differs, record why and escalate rather than weaken the guard.
- Risk: other agents' compile and benchmark work slows gates and perturbs the
  A–A probe.
  - Severity: low. Likelihood: high.
  - Mitigation: long timeouts. Never bypass compile admission. Record
    `/proc/loadavg` and `/proc/pressure/cpu` alongside the probe.

## Progress

- [x] (2026-11-28 13:10Z) Reconnaissance with a team of Wyvern agents
  (fast, read-only subagents). They surveyed the outbound serializer path, the
  zero-copy documents and ADRs, and the test and proof tooling.
- [x] (2026-11-28 13:40Z) External facts checked with Firecrawl (a web scraping
  tool): Criterion 0.8.2 defaults and `CRITERION_HOME`, and Bincode 2.0.1's
  `encode_to_vec` pre-sizing.
- [x] (2026-11-28 14:30Z) Initial draft committed (`584df91b`).
- [x] (2026-11-28 16:10Z) Design review by a four-member panel: performance
  methodology, contracts and structure, verification rigour, and alternatives
  and process. Verdict: revise. Revisions applied in this version; see the
  revision note.
- [ ] Plan approved by the user, together with `D1` and the reviewer's
  identity (one decision packet).
- [ ] `EP-M1` test-private shared-payload witness with its switch.
- [ ] `EP-M2` registers, switch–roadmap coupling, and Bincode guard.
- [ ] `EP-M3` ADR 0018 (proposed), threshold rewrite, A–A probe, and dependent
  documents (pull request A).
- [ ] `EP-M4` acceptance: ADR 0018 accepted, review record, and roadmap ticks
  (pull request B).

## Surprises & discoveries

- Observation: ADR 010's addendum and the #538 design disagree about what
  `11.1.2` removes.
  - Evidence: ADR 010 lines 299–301 at `fa8ef421`, against the #538 plan's
    "Purpose" ("does not promise allocation-free serialization") and its
    `BytesMut` writer design.
  - Impact: `D1` and ADR 0018.
- Observation: the serializer-seam counter cannot show a removal.
  - Evidence: `encode_message_frame` calls `record_serialized(bytes.len())`
    for every serializer output (`src/app/outbound_encoding.rs`, lines 32–36),
    and so does the client (`src/client/send_pipeline.rs`, line 44). After
    `11.1.2` a correctly shared output would still record `P`.
  - Impact: `D3` makes moving the counter an `11.1.2` obligation.
- Observation: the inversion register over-promises.
  - Evidence: of its six entries, four will not change at `11.1.2` under `D1`.
    - `outbound_path_copies_the_payload_twice_per_message` and
      `harness_serializer_copies_the_payload_once_at_each_seam` use the frozen
      harness's `PayloadMessage`, which holds a `Vec<u8>` and copies it
      (`wireframe_testing/src/codec_benchmarks/pipeline_outbound.rs`, lines
      45–62).
    - `default_serializer_and_handoff_copy_the_payload_once` and
      `production_outbound_allocates_one_payload` are Bincode, whose encode is
      inherent.
    - `seams_charge_one_payload_per_message_at_any_size` flips only its
      inbound half, at `11.1.1`.
  - Further evidence: the register gate checks only that named functions exist
    in an explicit `include_str!` list (`inversion_register.rs`, lines
    107–145). Nothing forces a flip.
  - Impact: `EP-M2`.
- Observation: no Criterion workload can show the removal. The timed outbound
  harness serializes a `Vec`-holding message, so under `D1` its copy remains.
  - Impact: the removal is evidenced only by the deterministic witness
    (`T4`, `T6`). The timing thresholds can only forbid regressions.
- Observation: three outbound copies go uncounted.
  - Evidence:
    - `WireframeApp::send_response_framed` records nothing at its serializer
      seam (`src/app/outbound_response.rs`, lines 109–133).
    - Its caller-supplied `Framed<_, tokio_util::codec::LengthDelimitedCodec>`
      copies into the write buffer outside wireframe.
    - The client sink (`src/client/runtime.rs`, line 66) does the same.
  - Impact: `T7` names them. Moving the counter (`D3`) covers the serializer
    seam from `11.1.2` onwards.
- Observation: `T1`'s equivalence is wrong.
  - Evidence: a time increase of `est` basis points lowers throughput by
    `10000·est / (10000 + est)` basis points. So +10 % time is −9.09 %
    throughput, and −10 % throughput needs +11.1 % time.
  - Impact: `T1` is restated on time only.
- Observation: measured host drift dwarfs `T1`'s 1 % noise term.
  - Evidence: C3 to C4 moved large-payload throughput by 15–18 % with no
    source change. Two C5 passes taken minutes apart reversed order by 2.8 %
    and 11.7 %. Criterion's confidence interval covers within-run noise only.
  - Impact: `D5`, and the A–A probe.
- Observation: the benchmark labels embed allocation counts, as in
  `..._allocs_18`, so a `T3` change renames the benchmark and breaks `T1`'s
  comparison.
  - Evidence: `benches/pipeline_performance_alloc.rs`, lines 57–61.
  - Impact: `D5` freezes identifiers with that suffix stripped.
- Observation: Bincode 2.0.1 `encode_to_vec` sizes its vector exactly.
  - Evidence: it runs a `SizeWriter` pass, then `Vec::with_capacity(size)`
    (`bincode-2.0.1/src/features/impl_alloc.rs`, read 2026-11-28). An unsized
    `BytesMut` writer would reallocate as it grows. The allocator window
    charges only the growth bytes, so `P ≤ bytes < 2P` can still pass; only the
    operation count reveals it.
  - Impact: the class-independence guard in `EP-M2`.
- Observation: documents are stale or inconsistent.
  - `docs/zero-copy-benchmark-plan.md` cites a "blocker" in the baselines
    document that does not exist.
  - The benchmark plan and the dedicated roadmap still use the
    "between serialization and `wrap_payload`" wording that ADR 010's addendum
    superseded.
  - `wireframe_testing` is published (version 0.3.0), which the first draft
    wrongly called unreleased.
  - Impact: `EP-M3` and `D2`.
- Observation: `10-2-1-capture-baselines.md` records overruns awaiting owner
  decisions while `docs/roadmap.md` ticks `10.2.1`.
  - Impact: budgeting evidence only. It is not this plan's to resolve.

## Decision log

- D1. Decision: the materialization that must be removed is the
  *contract-forced* copy: a payload-sized buffer allocated and filled only
  because `Serializer::serialize` must return an owned `Vec<u8>`, even when the
  message already holds its encoded bytes in a shareable buffer.
  - Required of `11.1.2`, for a serializer that returns a shared buffer: the
    frame's payload has the same address and content, no crate-owned site
    records serializer bytes, and no payload-sized allocation is made.
  - Not removed: an *encoding* serializer such as Bincode still writes `P`
    bytes once. That encode is inherent, not a materialization. It is bounded
    by a no-regression guard: at most one payload-sized buffer, an
    allocation-operation count independent of payload class, and no extra
    copied bytes.
  - Who benefits: forwarding, proxying, and pre-encoded payloads. Default
    Bincode users get no copy or allocation reduction from `11.1.2`, and ADR
    0018 says so plainly.
  - Anchor: accepted ADR 008, which requires output "without materializing a
    fresh `Vec<u8>`" (lines 85–86 and 160–162). The draft #538 plan is not the
    anchor.
  - Rejected alternative: one payload copy per message on the Bincode default
    path. That needs serialization directly into the transport write buffer,
    either by fusing serializer and encoder or by vectored writes that bypass
    `Framed`. It crosses ADR 010's accepted frame boundary and is outside
    `11.1.2`'s scope. A future item could pursue it under its own ADR.
  - Acceptance: the user at plan approval, and the reviewer through ADR 0018
    in `EP-M4`.
  - Date/Author: 2026-11-28, planning agent; revised after design review.
- D2. Decision: the witness is test-private, in
  `wireframe_testing/tests/witness/shared_payload.rs`, a directory without
  `main.rs` and therefore not a test target. Both `copied_bytes` and
  `pipeline_allocation_invariants` include it with `#[path]`.
  - Rationale: `wireframe_testing` is published, and `11.1.2` must rewrite the
    witness. A public module would be published API built to break. The
    benchmark harness's `PayloadMessage` is not reused, because changing it
    would invalidate C4 and C5.
  - Date/Author: 2026-11-28, planning agent; revised after design review.
- D3. Decision: this plan does not change production counting. Item `11.1.2`
  must move `record_serialized` from the seam (`encode_message_frame` and
  `send_pipeline`) to the sites that actually build a buffer:
  `PayloadBytes::from_vec` and the crate-private Bincode `BytesMut` writer. It
  must also delete the harness's explicit `record_serialized` call
  (`pipeline_outbound.rs`, line 149) to avoid double counting.
  - That last edit is a counter-only harness change that leaves the timed
    region unchanged. Copied-byte figures come from deterministic test
    assertions, not captures, so C4 and C5 timings remain valid.
  - Moving the counter also covers `send_response_framed`'s serializer seam.
  - The obligation is written into `docs/roadmap.md` item `11.1.2.1` and into
    ADR 0018, because the #538 plan may not be edited.
  - Rejected alternatives:
    - Counting `send_response_framed` now. That is a production edit
      superseded by the move.
    - Redefining `serialized` as "bytes produced at the seam". That would
      leave `T4` evidenced by addresses alone.
  - Date/Author: 2026-11-28, planning agent.
- D4. Decision: make the registers enforce rather than merely list.
  - One shared parser extracts marker-delimited items and their backtick
    spans, up to the first em dash. Each register does its own checks.
  - The inversion register is re-audited against `D1`. Non-flipping entries
    become guards named in the threshold register.
  - The witness's `SERIALIZER_OUTPUT` constant is coupled to the `11.1.2`
    checkbox in `docs/roadmap.md`. A ticked `11.1.2` with the switch still on
    `Materialized` fails `make test`.
  - The threshold register names every threshold's tests or procedure. Its
    identifier set must equal the set of threshold paragraphs in the prose.
  - Rationale: this closes the pre-mortem in which `11.1.2` merges green while
    the copy survives.
  - Date/Author: 2026-11-28, planning agent; revised after design review.
- D5. Decision: rebuild the timing protocol for a shared host.
  - `T1` is stated on Criterion's **median** time change. Throughput is
    derived from it.
  - The candidate is compared in interleaved rounds. In each of three or more
    rounds, the prebuilt baseline (A) and candidate (B) bench executables run
    back to back, workload by workload, sharing one `CRITERION_HOME`.
  - Per-workload noise term: `N_w = max(1 %, |A₁ − A₂|)`, where `A₁` and `A₂`
    are baseline runs from adjacent rounds.
  - A workload is inconclusive, not failed, only when `|A₁ − A₂| ≥ 5 %`
    (half of `R`). Inconclusive workloads are re-run, never waived.
  - Benchmark identifiers are frozen with any `_allocs_<n>` suffix stripped.
    Allocation counts are compared separately under `T3`.
  - `T2` becomes a tripwire. A breach triggers an interleaved re-run; it never
    fails a release on its own.
  - A harness edit that only adapts a signature, leaving the timed region
    unchanged, does not invalidate the comparison.
  - The verdict numbers (`R` = 10 %, noise floor 1 %) are unchanged. The 1 %
    floor matches Criterion 0.8.2's default `noise_threshold`.
  - Rationale: the measured drift above makes the draft's single-run,
    mean-based comparison unable to produce a verdict.
  - Date/Author: 2026-11-28, planning agent; from the performance review.
- D6. Decision: build no comparator or verdict kernel here. `13.2.1` owns it.
  This plan records the abstract model and lemma statements.
  - Recommendation for `13.2.1`: prove refinement and order-independence over
    arbitrary-length inputs with Verus, because those properties are
    unbounded. Use Kani or property tests for the decimal-to-basis-point
    conversion, following `docs/formal-verification-methods-in-wireframe.md`,
    which starts small concrete logic with Kani.
  - Rationale: `13.2.1` is the first item with two captures to compare.
  - Date/Author: 2026-11-28, planning agent.
- D7. Decision: no `insta`.
  - Rationale: there is no multivariant rendered output. The outputs are
    counts, addresses, and parse results, which named assertions state more
    precisely.
  - Date/Author: 2026-11-28, planning agent.
- D8. Decision: record `D1` in a new ADR 0018 ("Removal criteria for the
  serializer materialization"), status `Proposed` until `EP-M4`. ADR 010 gains
  an `Amended by ADR-0018` banner and an `## Amendments` section naming the
  superseded sentence.
  - Rationale: `D1` changes what Epic #284 delivers on the default path, which
    is a substantive decision (`AGENTS.md`). The style guide prescribes this
    form for targeted corrections. The decision concerns the serializer
    contract (ADR 008's scope), not the frame boundary.
  - Date/Author: 2026-11-28, planning agent; revised after design review.
- D9. Decision: acceptance follows the 10.1.x precedent of an ADR status flip.
  - Before `EP-M4`: the user takes the performance-reviewer role or names a
    reviewer, recorded in the release plan's "Owners" table.
  - Acceptance means three things:
    - ADR 0018 moves to `Accepted` with the reviewer's name and date.
    - The baselines document's Status table reads "accepted (date)".
    - A "Threshold review record" lists each threshold's disposition, with
      the reviewed document's blob identifier.
  - Delivery is two pull requests:
    - Pull request A (`EP-M1` to `EP-M3`) merges with thresholds still
      proposed.
    - Pull request B (`EP-M4`) is a small acceptance change.
  - Rationale: a comment by the owner's GitHub login proves nothing when
    agents push with that login. An ADR status flip is the repository's
    established, attributable act.
  - Date/Author: 2026-11-28, planning agent; revised after design review.
- D10. Decision: add no BDD scenario in this item.
  - The witness's subject is an internal measurement invariant.
  - Counters are process-global, and the `bdd` binary runs scenarios on
    parallel threads, so a counter step would be flaky.
  - The existing `tests/features/zero_copy_baseline.feature` already covers
    pass-through behaviour.
  - A proposed scenario for `11.1.2`'s `serializer_boundaries.feature` is
    recorded under `Suggested BDD features`.
  - Date/Author: 2026-11-28, planning agent; from the verification and process
    reviews.
- D11. Decision: do not adopt an instruction-count tier (gungraun, the
  successor of iai-callgrind) here.
  - It needs a new dependency and Valgrind in the gate.
  - It is offered to the reviewer as an open question in the decision packet
    and could become a `13.2.1` option.
  - Date/Author: 2026-11-28, planning agent.
- D12. Decision: run an A–A probe in `EP-M3`, interleaving the baseline
  revision against itself, and publish the per-workload drift.
  - Rationale: the performance reviewer will not agree to `T1` or `T2`
    without evidence that the protocol can return a verdict on this host.
  - Date/Author: 2026-11-28, planning agent.

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
`record_serialized(bytes.len())`, and calls
`codec.wrap_payload(benchmark_counters::bytes_from_vec(bytes))`. Its
`test-support` mirror, `wireframe::app::benchmark_encode_message_frame` (lines
61–74), returns the codec frame.

Callers fall into four groups:

- Server paths through the seam: `send_envelope` in
  `src/app/codec_driver/outbound.rs`, reached from
  `ActorFrameSink::send_frame`; and `send_response` and
  `send_response_framed_with_codec` in `src/app/outbound_response.rs`.
  Streaming responses reach the seam the same way.
- `send_response_framed` (lines 109–133): serializes and sends `Bytes::from`
  through a caller-supplied `Framed<W, LengthDelimitedCodec>`. It records
  nothing.
- The client, `src/client/send_pipeline.rs::serialize_and_send` (lines 20–69):
  records `record_serialized`, runs `before_send` hooks on the vector, and
  sends through a `LengthDelimitedCodec` sink without `wrap_payload`.
- Test helpers outside the threshold scope: `src/testkit/fragment_drive.rs`
  (line 45), `src/testkit/support.rs` (line 233), and `fragmenter.rs` (line 93).

`FrameCodec` implementations store the handed-in `Bytes` as a plain move
(`src/codec/mod.rs`, line 377; `src/codec/examples/mod.rs`, line 179).
`LengthDelimitedEncoder::encode` records `record_framed` (`src/codec/mod.rs`,
lines 346–350). The default maximum frame length is 1024 (`src/codec/mod.rs`,
line 140), so 64 KiB tests must raise it.

### The copy counters

`src/benchmark_counters.rs` (358 lines) compiles to no-ops unless the
`benchmark-counters` feature is on. `CopiedBytes` has the fields `serialized`,
`handoff`, `framed`, and `deserialized`. The recorders are `record_*`, plus
`snapshot` and `reset`. `bytes_from_vec` compares `as_ptr()` before and after
`Bytes::from` and records a hand-off only when the pointer moves.

Counters are process-global. `wireframe_testing`'s `pipeline-benchmarks`
feature enables `wireframe/benchmark-counters` and `test-support`, and the root
crate's dev-dependency on `wireframe_testing` enables `pipeline-benchmarks`.

### The enforcing tests

`wireframe_testing/tests/copied_bytes.rs` (332 lines) holds the per-seam
assertions, as `#[serial]` rstest tables. It includes three modules via
`#[path]`:

- `copied_bytes/properties.rs` (155 lines).
- `copied_bytes/inversion_register.rs` (227 lines). This module reads
  `docs/zero-copy-benchmark-baselines.md` with `include_str!` and parses the
  block between `<!-- inversion-register:start -->` and
  `<!-- inversion-register:end -->`. It takes the first two backtick spans of
  each `-` item as test name and file. `missing_register_entries` checks for
  `fn <name>(` only within an explicit `include_str!` list of sources (lines
  138–145). `inversion_register_names_real_tests` also asserts exactly six
  entries.

`wireframe_testing/tests/pipeline_allocation_invariants/` is a
`harness = false` binary (`wireframe_testing/Cargo.toml`, lines 53–57) with its
own counting allocator:

- Checks are `(&str, fn())` pairs in `checks/mod.rs` (131 lines). A function
  that is not listed never runs.
- `checks/handoff.rs` (131 lines) holds
  `production_outbound_allocates_one_payload`, which asserts `P ≤ bytes < 2P`
  for Bincode through `support.rs::outbound_traffic` (lines 166–175). That
  helper is length-delimited only.
- The window charges a reallocation only its growth
  (`benchmark_alloc_support/traffic.rs`, lines 10–17). The binary avoids
  absolute counts on purpose (`main.rs`, lines 22–23).

`src/codec/tests/mod.rs` holds codec-only pointer tests. Behavioural tests use
`rstest-bdd` 0.6.0. The `bdd` target runs scenarios in parallel.

### Benchmarks and Criterion

`make bench-codec` runs four bench targets with
`--features test-support,benchmark-counters` and `RUSTFLAGS="-D warnings"`. No
CI job runs benchmarks.

The published captures are C4 (`bef53308`, load 18–22) and C5 (`17b74b3f`, load
32–57). The timed outbound harness uses `VectorSerializer` over a `Vec`-holding
`PayloadMessage` (`pipeline_outbound.rs`, lines 45–83). Allocation benchmark
labels embed counts (`benches/pipeline_performance_alloc.rs`, lines 57–61).

Criterion 0.8.2 defaults (docs.rs, read 2026-11-28): `noise_threshold` 0.01,
`significance_level` 0.05, `confidence_level` 0.95. A run saves or compares a
baseline (`--save-baseline`, `--baseline`), but cannot do both in one run. The
results root follows `CRITERION_HOME` when set (`criterion-0.8.2/src/lib.rs`,
lines 142–146). Median change appears in `change/estimates.json`.

### Tooling and gates

- `make test` runs
  `cargo test --workspace --all-targets --all-features`, which includes
  `wireframe_testing`.
- `make test-wireframe-testing` runs `wireframe_testing` alone, with the same
  development flags. Use it for focused loops; raw `cargo test` omits the
  `DEV_FAST` configuration and forces a full rebuild.
- `make lint` runs rustdoc, Clippy with `-D warnings`, Whitaker, and Python
  doc lint.
- `make check-fmt` runs `cargo fmt --check` and `mdtablefix --check`.
- `make markdownlint` runs the spelling gate first.
- `make nixie` validates Mermaid diagrams.
- `make test-doc` runs doctests. It is not needed here, because nothing new
  carries rustdoc examples.
- Verus and Kani are not wired. `make verus` and `make kani` print
  `FORMAL-SKIP`.

`wireframe_testing` has `tokio` (with `macros` and `rt`), `futures`,
`tokio-util/codec`, `rstest`, `proptest` (dev), and `serial_test`. It has
neither `googletest` nor `pretty_assertions`.

### Budget survey

Code (about 580 net lines):

| Component                                                        | Net lines |
| ---------------------------------------------------------------- | --------- |
| Witness module                                                   | 110       |
| `copied_bytes/shared_payload.rs`                                 | 120       |
| `checks/shared.rs` plus registration                             | 70        |
| Parser extraction (`doc_register.rs`)                            | 40        |
| Inversion-register audit and switch coupling                     | 60        |
| `threshold_register.rs` with negative controls                   | 140       |
| Bincode guard, including a codec parameter on `outbound_traffic` | 40        |

Configuration (about 3 lines): two dev-dependency lines, plus possibly a
`Cargo.lock` edge.

Documentation (about 500 net lines):

| Component                                                     | Net lines |
| ------------------------------------------------------------- | --------- |
| ADR 0018                                                      | 160       |
| Baselines document rewrite and A–A evidence                   | 200       |
| ADR 010 banner and amendments                                 | 20        |
| Benchmark plan, dedicated roadmap, `roadmap.md`, release plan | 50        |
| Developers' guide                                             | 40        |
| Users' guide                                                  | 15        |
| `docs/contents.md`                                            | 5         |

The work touches about 21 files: 9 code, 2 configuration, and 10 documentation.

Historical evidence:

- `10-2-1-capture-baselines.md` set 900 code lines and 16 files, 40
  configuration lines, and 300 documentation lines. It finished at 2,725 code
  lines across 37 files, 14 configuration lines, and 377 documentation lines,
  with owner decisions pending. Its overrun came from new harness binaries,
  module splits for the 400-line cap, and review rounds: missed scope rather
  than approved growth.
- `10-1-2` raised its documentation budget from 500 to 750 net lines for an
  ADR rewrite.
- `9-6-1` needed no increase.

This plan avoids those drivers. The limits add about 38 % contingency for code
and 30 % for documentation, plus a 40 % checkpoint after `EP-M1`.

### Signposts

Read before implementing:

- `docs/zero-copy-benchmark-baselines.md`, the design document for thresholds.
- `docs/zero-copy-benchmark-plan.md`.
- ADRs 008, 009, and 010.
- `docs/execplans/issue-538-plan-zero-copy-serializer-output-migration.md`.
- `docs/documentation-style-guide.md`: ADR rules at lines 520–620.
- `docs/rust-testing-with-rstest-fixtures.md`.
- `docs/reliable-testing-in-rust-via-dependency-injection.md`.
- `docs/rstest-bdd-users-guide.md`.
- `docs/rust-doctest-dry-guide.md`.
- `docs/developers-guide.md`, "Example and benchmark support" (from line
  2333).
- `docs/formal-verification-methods-in-wireframe.md`.

For context:

- `docs/generic-message-fragmentation-and-re-assembly-design.md`.
  Fragmentation is informational only in these thresholds.
- `docs/multi-packet-and-streaming-responses-design.md`. Streamed responses
  cross `encode_message_frame`, so the witness covers them by construction.
- `docs/the-road-to-wireframe-1-0-feature-set-philosophy-and-capability-maturity.md`.
- `docs/hardening-wireframe-a-guide-to-production-resilience.md`.

Skills: `rust-router`, then `rust-unit-testing` (rstest, googletest, and
pretty_assertions), `proptest`, `rust-performance-and-layout` (benchmark
discipline), `rust-verification`, `kani`, and `verus` (for `13.2.1`), and
`arch-decision-records` (ADR 0018). Also `execplans`, `en-gb-oxendict-style`,
`commit-message`, and `pr-creation`.

## Conformance basis

Upstream artefacts at `fa8ef421`. No terms of reference exist for this
programme, and none are invented here.

- `docs/roadmap.md` items `10.2.2` (lines 494–500), `11.1.2` and `11.1.2.1`
  (lines 520–537), and `13.2.1` (line 598).
- `docs/zero-copy-frame-and-payload-migration-roadmap.md`: item `1.2.2` (lines
  91–109) and the success criteria (lines 38–43).
- `docs/zero-copy-benchmark-baselines.md`: "Status", "Inversion register",
  and "Acceptance thresholds" (lines 17–25, 460–500, and 524–643).
- `docs/zero-copy-benchmark-plan.md`: "Acceptance thresholds" and "Gaps" (lines
  370–400).
- ADR 008 (accepted; its serializer addendum is proposed), ADR 009 (accepted;
  its serializer amendment is proposed), and ADR 010 (accepted; addendum of
  2026-11-20).
- The #538 plan, a draft design for `11.1.2`.
- `docs/zero-copy-release-plan.md`: "Owners" and "Blockers".

Requirements:

- `REQ-define`: the thresholds state precisely which materialization `11.1.2`
  must remove, consistently with ADR 008.
- `REQ-witness`: a test states the pre-migration materialization through
  address identity, an allocation window, and copied bytes. A gate must stop
  `11.1.2` from being ticked until the witness shows sharing.
- `REQ-guard`: the default Bincode path has a no-regression guard on
  allocation operations and bytes.
- `REQ-coverage`: every outbound copy is either counted or named as a
  justified exception.
- `REQ-trace`: every threshold names its enforcing tests or procedure, and the
  gate fails on drift.
- `REQ-sound`: threshold rules are arithmetically correct, and the protocol
  can return a verdict on this host.
- `REQ-accept`: a named reviewer accepts ADR 0018 before `10.2.2` is ticked.

Gaps:

- `GAP-adr010-overclaim`: ADR 010 claims a default-path removal that `11.1.2`
  does not deliver.
- `GAP-counter`: the counter cannot observe a removal.
- `GAP-no-witness`: nothing observes the contract-forced copy.
- `GAP-register`: the inversion register over-promises and enforces nothing.
- `GAP-uncounted`: three outbound copies are uncounted.
- `GAP-t1`: `T1` misstates units and underestimates drift.
- `GAP-t2`: `T2` ignores load.
- `GAP-ids`: allocation counts in benchmark identifiers break comparisons.
- `GAP-stale`: stale wording and a phantom blocker remain in other documents.

Trace links:

```plaintext
roadmap-10.2.2 -> REQ-define   -> D1, D8 -> EP-M3 -> docs/adrs/adr-0018-... + baselines "What the migration must remove"
roadmap-10.2.2 -> REQ-witness  -> D2, D4 -> EP-M1, EP-M2 -> shared_payload::shared_payload_output_matches_the_switch
                                                       -> inversion_register::switch_tracks_roadmap_item_11_1_2
roadmap-10.2.2 -> REQ-guard    -> D1 -> EP-M2 -> checks::guards::bincode_outbound_allocation_ops_are_class_independent
roadmap-10.2.2 -> REQ-coverage -> D3 -> EP-M3 -> roadmap 11.1.2.1 obligation + baselines T7 exceptions
roadmap-10.2.2 -> REQ-trace    -> D4 -> EP-M2 -> threshold_register::threshold_register_matches_the_prose
roadmap-10.2.2 -> REQ-sound    -> D5, D12 -> EP-M3 -> baselines "Comparison protocol" + A–A probe evidence
roadmap-10.2.2 -> REQ-accept   -> D9 -> EP-M4 -> ADR 0018 Accepted + "Threshold review record"
roadmap-11.1.2 <- D3 obligations (counter move, switch flip, raw and no-hook client witnesses)
roadmap-13.2.1 <- D5 protocol, D6 verdict-kernel sketch
```

## Verification plan

### Abstract model

Model one outbound message as a pair `(m, s)`: a message `m` whose encoded form
has `P > 0` bytes, and a serializer `s`. Messages fall into two classes:

- *Shared*: `m` holds its encoded bytes as a `Bytes` value `b` at address
  `a(b)`.
- *Encoded*: `s` must produce the bytes, for example Bincode over a typed
  value.

The seam `E(s, c, m)` (`encode_message_frame` with codec `c`) yields a frame
with payload `f`. It changes the counters by
`Δ = (serialized, handoff, framed, deserialized)`, and it runs inside an
allocation window `W = (ops, bytes)`. A control `K(c, b)` wraps `b.clone()`
directly, with the clone taken outside the window; it runs in window `W_K`. The
witness switch `σ ∈ {Materialized, Shared}` names the contract in force.

- `σ = Materialized` (today, and for encoded messages after `11.1.2`): for a
  shared message, `a(f) ≠ a(b)`, `f = b` byte for byte, `Δ.serialized = P`,
  `Δ.handoff = 0`, and `P ≤ W.bytes − W_K.bytes < 2P`.
- `σ = Shared` (required for shared messages after `11.1.2`): `a(f) = a(b)`,
  `f = b`, `Δ.serialized = 0`, and `W.bytes − W_K.bytes < P`.
- Encoded messages under Bincode, before and after `11.1.2`:
  `Δ.serialized = P_enc`, `Δ.handoff = 0`, and `P ≤ W.bytes < 2P`. `W.ops` is
  equal for 32-byte and 64 KiB payloads, for each in-scope codec.

A serializer error yields `SendError::Serialize`, and the witness reports it as
a test failure.

### Invariants and lemmas

- `INV-switch` (`REQ-witness`): for every non-empty length, codec in
  {length-delimited, Hotline}, and value of `σ`, the witness's observations
  equal the model's prediction for that `σ`. Today `σ = Materialized` holds;
  setting `σ = Shared` must fail. That failure is the red evidence that the
  post-migration statement is falsifiable.
- `INV-roadmap-coupling` (`REQ-witness`): if `docs/roadmap.md` shows
  `- [x] 11.1.2.`, then `σ = Shared`.
- `INV-guard` (`REQ-guard`): the Bincode model above, for both codecs.
- `INV-trace` (`REQ-trace`): the threshold identifiers in the register equal
  the set of threshold paragraphs in the "Acceptance thresholds" prose. Each
  identifier appears once. Every `test` entry names at least one test, each
  found as `fn <name>(` in a loaded source. Entries in the `harness = false`
  binary also appear as strings in `checks/mod.rs`. Every `procedure` entry
  names an existing heading.
- `INV-inversion` (`REQ-witness`): every inversion-register entry names a
  roadmap item that will flip it. Every named test exists in a loaded source.
  The registers name no non-flipping test as inverting.
- `L-control`: `K` preserves the address (`a(K) = a(b)`), using the same
  address helper as the witness. If this fails, the address comparison itself
  is broken, and no `INV-switch` result is meaningful.
- `L-parser`: the shared parser accepts exactly the marker-delimited `-`
  items and joins two-space continuation lines. It reads spans only before the
  first em dash, and preserves item order. It rejects missing markers, empty
  blocks, and unknown kinds.

### Axioms (trusted, not verified here)

- `AXIOM-bytes`: `bytes` 1.x `From<Vec<u8>> for Bytes` does not copy payload
  bytes, and `Bytes::clone` shares the buffer. The first is checked at run time
  by `bytes_from_vec`; both are documented contracts.
- `AXIOM-window`: the counting allocator sees every allocation made by the
  current thread while the window is open, and charges a reallocation only its
  growth. The harness self-checks `harness_observes_real_traffic`,
  `idle_windows_report_no_traffic`, and `growing_reallocation_reports_growth`
  exercise this boundary.
- `AXIOM-codec-move`: both in-scope `wrap_payload` implementations store the
  handed-in `Bytes` without copying. The existing `wrap_payload_reuses_bytes`
  tests this, and `L-control` re-checks it.
- `AXIOM-criterion`: Criterion 0.8.2 reports relative change in time with the
  documented defaults, honours `CRITERION_HOME`, and writes median change to
  `change/estimates.json`. Only the procedural thresholds and the A–A probe
  depend on this.
- `AXIOM-bincode-size`: Bincode 2.0.1 `encode_to_vec` allocates once, with
  exact capacity. The existing `default_serializer_leaves_no_spare_capacity`
  exercises this.

### Methods per obligation

- Obligation: `INV-switch`.
  - Method: a parameterized `rstest` table plus a lightweight `proptest`
    property. The table covers the finite codec and length matrix; the property
    covers the length range cheaply.
  - Domain: codec in {length-delimited, Hotline}. Lengths in the table are 1,
    2, 4,096, and 65,536; the property draws from `1..=65_536`. The witness
    builds a codec with `max_frame_length` raised to at least the payload
    length.
  - Artefacts: in `wireframe_testing/tests/copied_bytes/shared_payload.rs`,
    `shared_payload_output_matches_the_switch`,
    `shared_payload_output_matches_the_switch_at_any_length`, and
    `direct_wrap_keeps_the_address` (`L-control`). In
    `pipeline_allocation_invariants/checks/shared.rs`,
    `shared_payload_allocation_matches_the_switch`.
  - Evidence: `make test-wireframe-testing` is green with
    `σ = Materialized`. The recorded red is a temporary run with
    `σ = Shared`, which must fail on the address and on `serialized` for every
    case.
  - Non-vacuity: lengths start at 1, because an empty `Bytes` has no
    meaningful address. Content equality is asserted, so a same-length wrong
    payload fails. Three seeded mutants are mandatory, and each must make the
    named check fail:
    - (a) The address helper returns the message's address instead of the
      frame's. `INV-switch` must fail.
    - (b) The control wraps `Bytes::copy_from_slice(&b)`. `L-control` must
      fail.
    - (c) The witness skips `benchmark_encode_message_frame` and wraps
      directly. The `serialized` assertion must fail.
  - Mutant handling: record each failure in `Artefacts and notes`, revert with
    `git restore`, and delete any regression file a mutant produced.
  - Proptest settings: the default 256 cases. Commit a regression file only if
    a genuine failure produced it.
- Obligation: `INV-roadmap-coupling`.
  - Method: a named unit test,
    `inversion_register::switch_tracks_roadmap_item_11_1_2`. It reads
    `docs/roadmap.md` with `include_str!`, finds the line for `11.1.2.`, and
    compares its checkbox with the witness constant.
  - Evidence and non-vacuity: green today. Fixture tests feed a ticked
    roadmap line with `Materialized`, which must fail, and an absent `11.1.2`
    line, which must also fail rather than pass silently.
- Obligation: `INV-guard`.
  - Method: an allocator-window check in the `harness = false` binary,
    `checks/guards.rs::bincode_outbound_allocation_ops_are_class_independent`.
    `outbound_traffic` gains a codec parameter.
  - Evidence: green today. Before writing the assertion, record the measured
    `ops` values for both classes and both codecs in `Artefacts and notes`.
  - Non-vacuity: a seeded mutant routes the check through a serializer that
    grows a `Vec` from zero capacity by repeated `push`. Growth scales with
    `log P`, so the operation counts differ and the check must fail. Revert
    afterwards.
- Obligation: `INV-trace`, `INV-inversion`, and `L-parser`.
  - Method: parser unit tests with fixture strings, plus live register tests
    over the real baselines document. Artefacts:
    - `wireframe_testing/tests/copied_bytes/doc_register.rs`: the shared
      extraction and its tests.
    - `inversion_register.rs`: inversion-specific checks and the coupling
      test.
    - `threshold_register.rs`: `threshold_register_matches_the_prose`.
  - Negative controls, each rejected with its own error variant:
    - a missing test;
    - a test whose file is not loaded (distinct from a missing test);
    - a duplicate `T3`;
    - a prose threshold with no register entry;
    - a register entry with no prose threshold;
    - an unknown kind;
    - a `test` entry with no names;
    - a `procedure` entry naming an absent heading;
    - backticked prose after the em dash, which must not be read as a test
      name.
  - Non-vacuity: identifiers are derived from the prose, not hard-coded, so
    adding `T8` without a register entry fails.
  - Code health: keep each checker's `match` on kind below Whitaker's
    complexity ceiling, and use iterators rather than indexing.

### Formal verification decision

This item adds no algorithm, protocol, or arithmetic kernel. Its obligations
are facts about concrete executions: addresses, byte counts, and allocation
operations. Its executable checks are register consistency. Tests check each
against the real code, with the non-vacuity controls above.

- Verus would add nothing here. The properties depend on `bytes`, the
  allocator, and serializers, which are axioms rather than proof subjects.
- Kani would add nothing either. There is no bounded pure function where every
  path matters more than the concrete witnesses.

Formal methods are warranted for `13.2.1`'s verdict kernel (`D6`). That kernel
is contractual release-gating logic over all integer inputs. Its statements
below are hypotheses about the eventual proof structure, not commitments. No
`assume`, `admit`, or external body may be used to make them compile. `13.2.1`
must add the chosen tool's Make target (`make verus` or `make kani`, once
wired) as a milestone acceptance criterion.

Abstract model for `13.2.1`:

- Changes are measured in basis points (bp; 1 % = 100 bp) of relative median
  time change.
- A frozen identifier set `I` comes from `D5`.
- A capture is a finite list of `(id, result)` pairs. A result is
  `Measured(lo, med, hi)` with integers, or `Unmeasured`.
- Each round also yields a drift `d_w = |A₁ − A₂|` per workload.
- Parameters: `R = 1000` and `Nmin = 100`.
- `N_w = max(Nmin, d_w)`.
- `regressed(w) ⇔ med > R ∧ lo > N_w`.
- `inconclusive(w) ⇔ d_w ≥ R/2`, compared cross-multiplied as `2·d_w ≥ R`,
  because `integer_division` is denied.
- Verdicts are ordered `Pass < Inconclusive < Unmeasured < Fail`.

Lemmas that can actually fail:

- `L-refine`: the executable verdict equals the specification's verdict for
  every capture. In particular, it is independent of list order, and it rejects
  duplicate identifiers.
- `L-missing`: an identifier in `I` absent from the capture is treated as
  `Unmeasured`, so a renamed benchmark can never be silently dropped.
- `L-monotone`: worsening any workload's result never improves the overall
  verdict. This includes a result going from `Measured` to `Unmeasured`, and
  any rise in `lo` or `med`.
- `L-malformed`: a result violating `lo ≤ med ≤ hi` never yields `Pass`.
- `L-precedence`: `inconclusive(w)` never masks a `regressed(w')` on another
  workload, and the rule for `regressed(w) ∧ inconclusive(w)` is stated
  explicitly. Proposal: `Fail` when `lo > N_w` holds even with the drift
  included, otherwise `Inconclusive`.
- `L-units`: for `med > −10000`, a time regression above 10 % (`med > 1000`)
  is equivalent to throughput below 10/11 of baseline,
  `10·(10000 + med) > 110000`. The draft's "10 % lower throughput" corresponds
  to `9·(10000 + med) > 100000`, that is `med > 1111`. The two rules disagree
  for `1000 < med ≤ 1111`, which is why `T1` is stated on time.
- `L-round`: converting Criterion's decimal percentage to basis points rounds
  towards not failing. For example, 10.004 % becomes 1000 bp, which is not a
  regression, so conversion can never manufacture a `Fail`.

### Suggested BDD features

None are adopted in this item (`D10`). The following scenario is proposed for
`11.1.2`, at `tests/features/serializer_boundaries.feature`, linked to
`REQ-witness` and `INV-switch` with `σ = Shared`. It asserts addresses only,
because counters are unsafe in the parallel `bdd` binary:

```gherkin
Feature: Serializer boundaries and metadata context
  Scenario: A serializer holding a shared payload hands it over without a copy
    Given a message holding a shared payload of 65536 bytes
    When the message is serialized and wrapped by the length-delimited codec
    Then the frame payload shares the message's buffer
    And the frame payload equals the message's payload
```

## Plan of work

### `EP-M1` test-private shared-payload witness with its switch

Requirements: `REQ-witness`, `GAP-no-witness`. Decisions: `D1`, `D2`.

1. Add the two authorized dev-dependencies to `wireframe_testing/Cargo.toml`.
2. Create `wireframe_testing/tests/witness/shared_payload.rs` (about 110 lines,
   with a module doc comment). It contains:
   - `pub(crate) enum SerializerOutput { Materialized, Shared }`.
   - `pub(crate) const SERIALIZER_OUTPUT: SerializerOutput =
     SerializerOutput::Materialized;`, whose doc comment names roadmap
     `11.1.2` and the coupling test.
   - `SharedPayloadSerializer` and `SharedPayloadMessage { payload: Bytes }`.
     `encode_with` returns `Ok(self.payload.to_vec())`, the only option under
     the current contract; its comment says `11.1.2` replaces it with a shared
     hand-off.
   - `shared_payload_message(len: NonZeroUsize)`, filling a pattern that does
     not repeat within 65,536 bytes, for example the little-endian bytes of
     successive `u32` indices. It does not reuse `payload_for_class`, which
     supports only two sizes and repeats every 251 bytes.
   - `encode_through_seam(codec, &message) -> Result<Bytes, String>`. It calls
     `wireframe::app::benchmark_encode_message_frame` with a codec built with
     `max_frame_length` raised to at least the payload length, and returns the
     frame's payload as `Bytes`.
   - `wrap_directly(codec, &Bytes) -> Bytes`, the control.
   - `payload_address(&Bytes) -> usize`, the one address helper. It returns
     `as_ptr() as usize` and is never dereferenced. The witness and the control
     both use it.
3. In `wireframe_testing/tests/copied_bytes.rs`, add
   `#[path = "witness/shared_payload.rs"] mod witness;` and
   `#[path = "copied_bytes/shared_payload.rs"] mod shared_payload;`. Write the
   three tests named under `INV-switch`, each `#[serial]`. Each reads
   `SERIALIZER_OUTPUT` and asserts the model's prediction for it:
   - googletest `assert_that!` with `eq` or `not(eq)` for addresses and
     counts;
   - `pretty_assertions::assert_eq` for whole `CopiedBytes` values and for
     content.

   Use no `#[gtest]` attribute (see `rust-unit-testing`).
4. In `pipeline_allocation_invariants/main.rs`, include the witness with
   `#[path]`. Add `checks/shared.rs` with
   `shared_payload_allocation_matches_the_switch`, taking the control window
   with the clone made outside it, and register the check in `checks::CHECKS`.
5. Red evidence: temporarily set `SERIALIZER_OUTPUT` to `Shared`, run
   `make test-wireframe-testing`, and record that every witness case fails on
   the address and on `serialized: 65536 != 0` (or its length). Restore the
   constant. Run the three seeded mutants (a), (b), and (c) and record the
   failures. Restore with `git restore`.
6. Green: run `make check-fmt`, `make lint`, and `make test` sequentially.
   Expect the new tests in the `copied_bytes` summary, and the new check name
   in the `pipeline_allocation_invariants` output.
7. Code-budget checkpoint: count net code lines. If above 320, stop and
   re-estimate.

### `EP-M2` registers, switch–roadmap coupling, and Bincode guard

Requirements: `REQ-trace`, `REQ-guard`, `GAP-register`. Decisions: `D1`, `D4`.

1. Refactor, with tests green before and after. Move item and span extraction
   from `copied_bytes/inversion_register.rs` into
   `copied_bytes/doc_register.rs`, parameterized by marker name. It returns
   items with spans read only before the first ` — `. The parser unit tests
   move with it.

   `inversion_register.rs` keeps its layout checks. Its source list gains:
   - `copied_bytes/shared_payload.rs`;
   - `witness/shared_payload.rs`;
   - `pipeline_allocation_invariants/checks/shared.rs`;
   - `pipeline_allocation_invariants/checks/guards.rs`.

   The "file not loaded" error becomes separate from "test missing". Commit the
   refactor on its own.
2. Re-audit the inversion register in `docs/zero-copy-benchmark-baselines.md`:
   - Keep `inbound_decode_copies_the_payload_once` (flips at `11.1.1`).
   - Retag `seams_charge_one_payload_per_message_at_any_size` to `11.1.1`
     (inbound half only).
   - Remove `outbound_path_copies_the_payload_twice_per_message`,
     `harness_serializer_copies_the_payload_once_at_each_seam`,
     `default_serializer_and_handoff_copy_the_payload_once`, and
     `production_outbound_allocates_one_payload`. They become guards under
     `T3`, `T4`, and `T6` in the threshold register.
   - Add `shared_payload_output_matches_the_switch`,
     `shared_payload_output_matches_the_switch_at_any_length`, and
     `shared_payload_allocation_matches_the_switch` (flip at `11.1.2`, by
     setting `SERIALIZER_OUTPUT` to `Shared`).

   Replace the hard-coded six-entry count with a check that every entry names a
   roadmap item.

   Red: before the document is edited, the updated live test fails on the stale
   entries. Green: after the edit.
3. Add `switch_tracks_roadmap_item_11_1_2` and its two fixture controls. It
   reads `docs/roadmap.md` with `include_str!("../../../docs/roadmap.md")`.
4. Write `copied_bytes/threshold_register.rs` with
   `threshold_register_matches_the_prose` and the negative-control fixtures
   listed under `Verification plan`. It derives the prose identifier set from
   lines matching `T<digits>.` that start a paragraph within the "Acceptance
   thresholds" section.

   Red: fails because the `<!-- threshold-register:start -->` and
   `<!-- threshold-register:end -->` markers are missing. Then add the register
   to the baselines document, listing the current `T1` to `T7` with their tests
   and procedures. Green.

   Entry format, one per threshold. The spans before the em dash are, in order:
   identifier, kind, then either the file and its test names (`test`) or a
   heading (`procedure`).

   ```markdown
   - `T4` `test` `wireframe_testing/tests/copied_bytes/shared_payload.rs`
     `shared_payload_output_matches_the_switch` `direct_wrap_keeps_the_address`
     — removal of the contract-forced copy (ADR 0018).
   - `T1` `procedure` `Comparison protocol` — interleaved median time change.
   ```

   A threshold enforced in several files gets one entry per file, sharing its
   identifier. The uniqueness check is on (identifier, file).
5. Bincode guard:
   - Give `support.rs::outbound_traffic` a `CodecUnderTest` parameter, and
     update its existing caller.
   - Run once to record the measured `ops`.
   - Add
     `checks/guards.rs::bincode_outbound_allocation_ops_are_class_independent`
     and register it.
   - Run the growth mutant and record its failure.
6. Validation: `make check-fmt`, `make lint`, `make test`.

### `EP-M3` ADR 0018, threshold rewrite, A–A probe, and documents (pull request A)

Requirements: `REQ-define`, `REQ-sound`, `REQ-coverage`. Gaps:
`GAP-adr010-overclaim`, `GAP-counter`, `GAP-uncounted`, `GAP-t1`, `GAP-t2`,
`GAP-ids`, `GAP-stale`. Decisions: `D1`, `D3`, `D5`, `D8`, `D11`, `D12`.
Thresholds stay "proposed" in this milestone.

1. Write
   `docs/adrs/adr-0018-removal-criteria-for-the-serializer-materialization.md`
   with status `Proposed`, following the style guide's template:
   - Context: the ADR 010 overclaim and the counter problem.
   - Decision drivers.
   - Options considered: `D1` against serialize-into-transport.
   - Decision outcome, with the shared and encoded classes and the
     beneficiaries.
   - The `11.1.2` obligations (`D3`): move the counter; flip the witness
     switch; add a raw-path witness for `send_response_framed` and a no-hook
     client witness, each flipping at `11.1.2`. The client hook path is a
     mutation path owned by `12.2.1`.
   - The obligation to keep the Bincode guard green. Stated as an outcome
     (operations independent of payload class, window below `2P`), not as a
     pre-sizing mechanism. Note that Bincode's own `encode_to_vec` meets it by
     running a size pass first.
   - Known risks.
   - Outstanding decisions: the reviewer's acceptance and the
     instruction-count tier (`D11`).
2. ADR 010: add `Amended by ADR-0018 (2026-11-28)` to its Status section, and
   an `## Amendments` section that names the superseded sentence ("Item
   `11.1.2` therefore removes one allocation and one copy on the outbound
   default path") and points to ADR 0018.
3. Run the A–A probe (`D12`), described in `Concrete steps`, and record its
   per-workload drift summary and host-load samples.
4. Rewrite `docs/zero-copy-benchmark-baselines.md`:
   - Add "What the migration must remove": `D1`, with per-seam tables for
     shared and encoded messages after `11.1.2`, and the statement that no
     Criterion workload evidences the removal.
   - `T1`: interleaved median time change per `D5`, with throughput derived
     and `L-units` stated.
   - `T2`: a tripwire per `D5`, keeping C4's floors and naming the C4 load
     band (18–22).
   - `T3`: list which counts are test-enforced
     (`bincode_outbound_allocation_ops_are_class_independent`,
     `read_only_middleware_counts_match`, `read_only_hook_counts_match`, and
     the hand-off checks) and which are benchmark-label procedure. Identifiers
     are frozen with the `_allocs_<n>` suffix stripped.
   - `T4`: the witness and `L-control`, and the switch coupling to `11.1.2`.
   - `T5`: unchanged apart from register linkage.
   - `T6`: the shared witness falls from `P` to zero serializer bytes. The
     Bincode and frozen-harness totals stay at `2P`.
   - `T7`: name the uncounted copies, with reasons:
     - `send_response_framed`'s serializer seam, until the `11.1.2` counter
       move;
     - the `tokio-util` length-prefix copies in `send_response_framed` and
       the client sink, which happen inside a caller-supplied or third-party
       encoder that wireframe never calls;
     - the test helpers `src/testkit/fragment_drive.rs`,
       `src/testkit/support.rs`, and `fragmenter.rs`, which are out of scope.
   - "Comparison protocol": the interleaved procedure with exact commands,
     `CRITERION_HOME`, median, `N_w`, the inconclusive rule, recorded load and
     pressure stall information (PSI) samples, the signature-only harness-edit
     rule, and the A–A probe results.
   - Add the corrected inversion register and the threshold register.
   - Update the "Status" table, and keep "proposed" wording.
   - Add an empty "Threshold review record" section.
5. `docs/zero-copy-benchmark-plan.md`: replace the "between serialization and
   `wrap_payload`" rule with ADR 0018's wording, and point the sign-off gap row
   at the review record instead of the phantom blocker.
6. `docs/zero-copy-frame-and-payload-migration-roadmap.md`: replace the same
   superseded wording in the success criteria and in `1.2.2`'s sub-bullet. Tick
   nothing.
7. `docs/roadmap.md`, item `11.1.2.1`: extend its "Success:" sentence with the
   ADR 0018 obligations (counter move, switch flip with roadmap coupling, raw
   and no-hook client witnesses, Bincode guard green), with an inline link to
   ADR 0018.
8. `docs/zero-copy-release-plan.md`: the sign-off blocker row now points to
   ADR 0018 acceptance.
9. `docs/developers-guide.md`, "Example and benchmark support": document the
   witness and its switch, the reuse policy, the two registers, and how to
   change a threshold:
   - The witness is the only sanctioned shared-payload witness. It is
     test-private, so harnesses must not grow their own.
   - A threshold changes only through an ADR 0018 amendment accepted by the
     reviewer.
   - Document the interleaved comparison procedure.
10. `docs/users-guide.md`, `CopiedBytes` section: state which outbound copies
    the counters do not see (`T7`). No behaviour changes, so no `CHANGELOG.md`
    entry is needed.
11. `docs/contents.md`: add ADR 0018 and this ExecPlan.
12. Validation, in order: `make fmt`, `make check-fmt`, `make markdownlint`,
    `make nixie`, `make lint`, `make test`. The register tests re-run because
    both documents are compiled in.
13. Open pull request A (`EP-M1` to `EP-M3`).

### `EP-M4` acceptance (pull request B)

Requirements: `REQ-accept`. Decision: `D9`. This milestone waits for the
reviewer.

1. Post the decision packet on pull request A. In order:
   - `D1` with its rejected alternative;
   - the `11.1.2` obligations;
   - each threshold's old and new text, with its enforcing tests or
     procedure;
   - the A–A probe evidence;
   - the measured guard counts;
   - the open question on an instruction-count tier (`D11`).
2. If the reviewer amends a threshold, apply the amendment on a follow-up
   commit to pull request A or B, re-run the gates, and record it in
   `Decision log`. If the reviewer rejects `D1`, set `BLOCKED` and escalate.
3. On acceptance, on a branch from `main` after pull request A merges:
   - ADR 0018 becomes `Accepted` with the reviewer's name, the date, and a
     summary.
   - Fill in the "Threshold review record" with the reviewer, the date, the
     blob identifier from
     `git rev-parse HEAD:docs/zero-copy-benchmark-baselines.md`, and the
     dispositions.
   - Change "proposed" to "accepted (date)" in the Status table and threshold
     text.
   - Record the reviewer in the release plan's "Owners" table, and remove the
     sign-off blocker row.
   - Mark the benchmark plan's sign-off gap closed.
   - Tick `10.2.2` in `docs/roadmap.md`, and `1.2.2` with its open
     sub-bullets in the dedicated roadmap.
   - Update `docs/contents.md`'s description.
   - Set this plan to `COMPLETE` after reconciling `Outcomes & retrospective`.
4. Validation: the documentation gates, then `make lint` and `make test`,
   because the registers read both documents.

## Milestones and plateaus

- `EP-M1`: the witness exists and states today's contract.
  - Requirements: `REQ-witness`.
  - Evidence: three `copied_bytes` tests, one allocation check, the recorded
    `Shared` red, and three mutant failures.
  - Conformance: no change under `src/` or `wireframe_testing/src/`; no harness
    change; dev-dependencies as authorized.
  - Recovery: revert the milestone's commits; nothing depends on them yet.
  - Remaining gaps: the register and the coupling.
  - Compatibility: none.
- `EP-M2`: the registers enforce.
  - Requirements: `REQ-trace`, `REQ-guard`.
  - Evidence: `threshold_register_matches_the_prose`,
    `switch_tracks_roadmap_item_11_1_2`, the parser negative controls, and
    `bincode_outbound_allocation_ops_are_class_independent` with its mutant.
  - Conformance: test and documentation-marker changes only.
  - Recovery: the parser refactor is its own commit and reverts independently.
  - Compatibility: none.
- `EP-M3`: the proposed thresholds are coherent and documented.
  - Requirements: `REQ-define`, `REQ-sound`, `REQ-coverage`.
  - Evidence: green gates, A–A probe results, and ADR 0018 (proposed).
  - Conformance: ADR 010 changes only through the amendment form.
  - Recovery: revert the documentation commits together with any
    register-marker edit that depends on them.
  - Compatibility: none.
- `EP-M4`: the thresholds are accepted.
  - Requirements: `REQ-accept`.
  - Evidence: ADR 0018 `Accepted` and the review record.
  - Conformance: roadmap ticks match the record.
  - Recovery: revert pull request B.
  - Compatibility: none.

## Concrete steps

Run every command from the worktree root. Use the shared Cargo cache. Never
bypass `RUSTC_WRAPPER`. Run gates sequentially.

```sh
set -o pipefail
BR=$(git branch --show-current)
make check-fmt 2>&1 | tee "/tmp/check-fmt-wireframe-$BR.out"
make lint      2>&1 | tee "/tmp/lint-wireframe-$BR.out"
make test      2>&1 | tee "/tmp/test-wireframe-$BR.out"
```

Documentation milestones first run:

```sh
make fmt          2>&1 | tee "/tmp/fmt-wireframe-$BR.out"
make markdownlint 2>&1 | tee "/tmp/markdownlint-wireframe-$BR.out"
make nixie        2>&1 | tee "/tmp/nixie-wireframe-$BR.out"
```

Focused loop for `EP-M1` and `EP-M2`:

```sh
make test-wireframe-testing 2>&1 | tee "/tmp/test-wt-wireframe-$BR.out"
```

A–A probe for `EP-M3`. It runs the same prebuilt bench executables three times,
saving the first run and comparing the later two against it, all under one
results root. Build once with the `bench-codec` flags; the bench executables
print their paths with `--message-format=json`, or can be found under
`target/release/deps/`:

```sh
export CRITERION_HOME="$PWD/target/criterion-aa-probe"
RUSTFLAGS="-D warnings" cargo bench --no-run \
  --bench codec_performance --bench pipeline_performance \
  --features test-support,benchmark-counters 2>&1 | tee "/tmp/aa-build-wireframe-$BR.out"
for round in a1 a2 a3; do
  cat /proc/loadavg /proc/pressure/cpu | tee -a "/tmp/aa-load-wireframe-$BR.out"
  if [ "$round" = a1 ]; then flag="--save-baseline a1"; else flag="--baseline a1"; fi
  for exe in <codec_performance executable> <pipeline_performance executable>; do
    "$exe" --bench $flag 2>&1 | tee -a "/tmp/aa-$round-wireframe-$BR.out"
  done
done
```

Then extract each workload's median change for rounds a2 and a3 from
`$CRITERION_HOME/**/change/estimates.json`. Report, per workload, the maximum
absolute median drift and the fraction of workloads at or above 5 %. Delete
`target/criterion-aa-probe` afterwards. The probe takes about 40–60 minutes
under contention.

Expected transcripts (to be filled in):

```plaintext
EP-M1 red (σ = Shared): shared_payload_output_matches_the_switch::case_1 ... FAILED
                        expected address equality; serialized: expected 0, actual 1
EP-M1 green:            shared_payload_output_matches_the_switch ... ok (8 cases)
EP-M2 red:              threshold register: missing `<!-- threshold-register:start -->` marker
EP-M2 green:            threshold_register_matches_the_prose ... ok
EP-M3 probe:            32 workloads; max |drift| = <x> %; <k> workloads ≥ 5 %
```

Commit after each green plateau, with gates passing, using imperative subjects.
Examples:

- "Add a test-private shared-payload witness"
- "Share one parser between the benchmark registers"
- "Couple the witness switch to roadmap item 11.1.2"
- "Guard Bincode outbound allocation operations"
- "Propose ADR 0018 for serializer materialization removal"
- "Restate the zero-copy acceptance thresholds"

## Validation and acceptance

Quality criteria:

- Tests: `make test` passes. Every new test exists, and the recorded red
  evidence and mutant failures are in `Artefacts and notes`.
- Verification:
  - Discharged by the named tests and their controls: `INV-switch` (for
    `σ = Materialized`), `INV-roadmap-coupling`, `INV-guard`, `INV-trace`,
    `INV-inversion`, `L-control`, and `L-parser`.
  - `INV-switch` for `σ = Shared` is shown falsifiable today. It is discharged
    by `11.1.2`, which cannot tick its roadmap item without it.
  - The `13.2.1` lemmas remain statements.
- Lint and format: `make check-fmt`, `make lint`, `make markdownlint`, and
  `make nixie` pass.
- Performance: the A–A probe has been run and its results recorded. No
  candidate comparison is made; that is `13.2.1`'s job.
- Review: ADR 0018 is accepted by a named reviewer, and the review record is
  complete.

Acceptance, phrased as behaviour:

- A reviewer opens `docs/zero-copy-benchmark-baselines.md` and finds every
  threshold with its tests or procedure. Deleting a named test, or adding a
  `T8` paragraph without a register entry, makes `make test` fail.
- On the current code, the witness reports a copied, relocated 64 KiB shared
  payload, while the control keeps the address. Flipping the witness switch to
  `Shared` makes `make test` fail.
- Ticking `11.1.2` in `docs/roadmap.md` while the switch says `Materialized`
  makes `make test` fail.
- `10.2.2` is ticked only after ADR 0018 is accepted by a named reviewer.

## Idempotence and recovery

All steps are re-runnable.

- Seeded mutants and the `Shared` red run are temporary edits. Make each
  uncommitted, run the focused command, record the result, restore with
  `git restore <file>`, and confirm with `git status`.
- Delete any `*.proptest-regressions` file that a mutant produced.
- The A–A probe writes only under `target/criterion-aa-probe`. Delete that
  directory afterwards.
- If the disk or `/tmp` fills, stop and report.
- If `make lint` reports missing dylint externs after a full disk, remove
  `target/dylint` and re-run.
- If compile admission is unhealthy, report the full `build-limits-status`
  output and continue documentation work.

## Artefacts and notes

Reconnaissance (2026-11-28, `fa8ef421`):

```plaintext
src/app/outbound_encoding.rs:32-36  serialize, then record_serialized(bytes.len()) for any output
src/app/outbound_encoding.rs:41     codec.wrap_payload(bytes_from_vec(bytes))
src/app/outbound_response.rs:118-132  serialize -> Bytes::from(bytes); no recorder
src/client/send_pipeline.rs:44      record_serialized(bytes.len())
wireframe_testing/tests/copied_bytes.rs:147  serialize_and_wrap.total() + framed.total() == 2 * payload_len
inversion_register.rs:138-152       fixed include_str! source list; exactly 6 entries
```

Bincode 2.0.1 `encode_to_vec`, quoted from source:

```rust
let size = { /* SizeWriter pass over the value */ };
let writer = VecWriter::with_capacity(size);
```

The design-review panel's verdict and principal findings are summarized in the
revision note below. Implementation transcripts, mutant results, measured
operation counts, and A–A probe results are to be added here.

## Interfaces and dependencies

No production interface changes.

Test-private module `wireframe_testing/tests/witness/shared_payload.rs`,
included with `#[path]` by `copied_bytes` and by
`pipeline_allocation_invariants`:

```rust
pub(crate) enum SerializerOutput { Materialized, Shared }
pub(crate) const SERIALIZER_OUTPUT: SerializerOutput = SerializerOutput::Materialized;

pub(crate) struct SharedPayloadSerializer;
pub(crate) struct SharedPayloadMessage { payload: Bytes }

pub(crate) fn shared_payload_message(len: NonZeroUsize) -> SharedPayloadMessage;
pub(crate) fn encode_through_seam(
    codec: CodecUnderTest,
    message: &SharedPayloadMessage,
) -> Result<Bytes, String>;
pub(crate) fn wrap_directly(codec: CodecUnderTest, payload: &Bytes) -> Bytes;
pub(crate) fn payload_address(payload: &Bytes) -> usize;
```

`CodecUnderTest` is reused from
`wireframe_testing::codec_benchmarks::CodecUnderTest`. The witness builds its
own codecs with raised frame limits rather than using the harness's defaults.

Reuse policy: this is the only shared-payload witness. Tests that need one
include it with `#[path]`. `11.1.2` changes `SharedPayloadMessage`'s
`encode_with` body to a shared hand-off and sets `SERIALIZER_OUTPUT` to
`Shared`. It changes nothing else in the witness.

Dependencies: add to `wireframe_testing/Cargo.toml` `[dev-dependencies]`:

```toml
googletest = "0.14.3"
pretty_assertions = "1.4.1"
```

No production dependency changes.

## Revision note

- 2026-11-28: initial draft from Wyvern reconnaissance and Firecrawl checks.
- 2026-11-28: revised after the four-member design review (verdict: revise).
  - Counting: added `D3`. The counter cannot show a removal, so moving it is
    an `11.1.2` obligation written into the roadmap and ADR 0018.
  - Witness: made test-private, because `wireframe_testing` is published.
    Added the `SERIALIZER_OUTPUT` switch coupled to the `11.1.2` checkbox,
    because a register that only checks names forces nothing.
  - Inversion register: re-audited all six entries; four do not flip.
  - Scope: dropped the production edit to `send_response_framed`, which
    `D3`'s counter move supersedes, and the BDD scenarios, which would be
    flaky on process-global counters.
  - Decision record: moved `D1` into a proposed ADR 0018, per the style
    guide's amendment rule.
  - Timing protocol: rebuilt around interleaved median comparison, a
    per-workload noise term, a `T2` tripwire, and frozen identifiers, with an
    A–A probe as evidence.
  - Corrections: fixed the `L-units` arithmetic; replaced the definitional
    `13.2.1` lemmas with falsifiable ones; replaced the vacuous seeded fault
    with three mandatory mutants and a `Shared` red run.
  - Delivery: split into two pull requests, so acceptance does not leave the
    plan blocked.
  - Effect on remaining work: milestones are renumbered to four. The budgets
    are code 800, configuration 12, documentation 650, and 24 files.
