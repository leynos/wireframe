# Approve the actor and codec-driver boundary (10.1.3)

This ExecPlan (execution plan) is a living document. The sections
`Constraints`, `Tolerances`, `Risks`, `Progress`, `Surprises & Discoveries`,
`Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work
proceeds.

Status: COMPLETE

## Purpose / big picture

Roadmap item `10.1.3` is a *decision-closure* item, not a code change. Its job
is to move
[ADR 010: transport-frame boundary for zero-copy](../adr-010-transport-frame-boundary-for-zero-copy.md)
from `Proposed` to `Accepted` by resolving its three Outstanding Decisions, so
that the later runtime work in roadmap phase 11 has an approved, written
boundary to build against. After this item lands, the project has a single
agreed answer to "who owns transport-frame emission, where do protocol hooks
run, and which `Vec<u8>` bridges survive", and the migration can stop keeping
`Vec<u8>` bridges alive merely because the architecture was underspecified.

A reader can observe success in three ways once the plan is executed:

1. `docs/adr-010-transport-frame-boundary-for-zero-copy.md` reads `Accepted`,
   carries dual `Proposed`/`Accepted` dates, and its former "Outstanding
   Decisions" are each resolved in place with a link to the roadmap item that
   implements them.
2. `docs/roadmap.md` item `10.1.3` and
   `docs/zero-copy-frame-and-payload-migration-roadmap.md` item `1.1.3` (with
   its child bullets) are checked `[x]`.
3. `make check-fmt`, `make lint`, `make test`, and `make markdownlint` all pass
   on a clean tree, and a `coderabbit review --agent` pass raises no
   outstanding concerns.

This item changes **no Rust code and no public API**. It is a documentation and
decision artefact. The runtime changes it sanctions are deliberately deferred to
roadmap phase 11 (items `11.1.2`, `11.2.1`, `11.2.2`, `11.2.3`) and the release
review in `14.2.1`.

## The decision being approved (summary)

The plan approves ADR 010's **Option B**: the connection actor stays
packet-oriented, and the codec driver becomes the sole owner of transport-frame
emission. Concretely, the three Outstanding Decisions in ADR 010 are resolved as
follows. Each is justified in the `Decision Log` and `Context and orientation`
sections below.

1. **Where protocol hooks run.** Protocol hooks stay **packet-oriented** and
   continue to execute against `Envelope` (a `Packet`) inside the connection
   actor, before serialization. The codec driver hosts serialization and
   transport framing only; it introduces no new transport-frame-level hook. The
   current asymmetry — `before_send` fires for actor-driven push and
   multi-packet frames but not yet for app-router responses routed through
   `FramePipeline` — is documented as a known limitation and tracked for closure
   under roadmap `11.2.1`.
2. **Where the `Vec<u8>` bridges live.** No public, feature-gated `Vec<u8>`
   compatibility shim is introduced at the actor boundary. `Packet for Vec<u8>`
   stays test-only in `src/connection/test_support.rs`. The public
   `impl CorrelatableFrame for Vec<u8>` is sanctioned to **leave the core
   runtime surface** and move to test support; because it is a public impl, its
   removal is a breaking change and is sequenced with the epic-284 breaking
   release under roadmap `11.2.2` and reviewed in `14.2.1`.
3. **Whether a new "serializable packet" trait is needed.** **No new trait.**
   The existing `Packet` trait composed with `EncodeWith<Serializer>` already
   expresses "a packet that can be serialized"; `Envelope` satisfies both.
   Adding a bridging trait is ADR 010's rejected Option C and is corroborated as
   unnecessary by prior art (see `Artifacts and notes`).

## Constraints

Hard invariants that must hold throughout implementation. Violation requires
escalation, not a workaround.

1. **No code or public API change in this item.** `10.1.3` edits documentation
   only. Do not modify any file under `src/`, `crates/`, `tests/`, `examples/`,
   or `wireframe_testing/`. Runtime changes belong to phase 11.
2. **ADRs are append-only decisions, not rewrites of history.** ADR 010 must be
   moved to `Accepted` by *resolving* its Outstanding Decisions and expanding
   the Decision Outcome, mirroring how ADR 008 and ADR 009 were accepted. Do not
   delete the record of the options considered or the rationale.
3. **The accepted boundary must match the code as it exists today.** The
   production outbound path already routes every frame through the codec driver,
   with exactly one production `FrameCodec::wrap_payload` call site at
   `src/app/outbound_encoding.rs:36`. The ADR text must not claim a boundary the
   code does not yet have; where the boundary is aspirational (for example,
   `before_send` for app-router responses), it must be labelled as deferred and
   linked to its roadmap item.
4. **British English, Oxford spelling.** Follow the documentation style guide:
   en-GB with Oxford spelling, the Oxford comma where it improves clarity, ATX
   headings, two blank lines after each heading in this ExecPlan, and ordered
   lists written `1.`, `2.`, and so on.
5. **Cross-references stay consistent.** Every document that states ADR 010's
   status (`roadmap.md`, the zero-copy migration roadmap, the inventory,
   `contents.md`) must agree that it is accepted once the change lands.

## Tolerances (exception triggers)

1. **Scope.** If executing the plan appears to require editing any non-documentation
   file, stop and escalate. This item is documentation-only by definition.
2. **Decision drift.** If, while writing the accepted ADR, any of the three
   resolved decisions appears wrong against the actual code (for example, a
   second production `wrap_payload` caller is discovered, or `before_send` turns
   out to already fire for app-router responses), stop and escalate rather than
   quietly changing the decision.
3. **Ambiguity.** If the user's intent for a decision differs from the
   resolution recorded here and the difference is material to phase 11, stop and
   present the options with trade-offs.
4. **Document growth.** If the ADR rewrite exceeds roughly double its current
   length, stop and reconsider — an accepted decision record should be
   sharper, not longer.
5. **Gate failures.** If `make markdownlint`, `make check-fmt`, `make lint`, or
   `make test` fail after the documentation edits and the cause is not an
   obvious local formatting fix, stop and escalate.

## Risks

1. Risk: The accepted ADR overstates the boundary, claiming the codec driver is
   the sole framing owner when a non-driver `wrap_payload` caller still exists.
   Severity: medium. Likelihood: low.
   Mitigation: the inventory of `wrap_payload` call sites was taken during
   planning (one production caller, `src/app/outbound_encoding.rs:36`; all
   others are tests, examples, or testkit). The ADR records this list and the
   plan adds a requirement (executed in phase 11) for a guard test that fails if
   a new non-driver production caller appears.
2. Risk: Removing the public `impl CorrelatableFrame for Vec<u8>` is treated as
   a free internal change and slips into a non-breaking release.
   Severity: medium. Likelihood: medium.
   Mitigation: the ADR and decision log mark the removal as breaking and bind it
   to the epic-284 breaking release via roadmap `11.2.2` and the review in
   `14.2.1`, governed by the ADR 009 rollout policy.
3. Risk: The documented `before_send` asymmetry confuses a downstream protocol
   author whose hook fires for push frames but not for routed responses.
   Severity: low. Likelihood: medium.
   Mitigation: the asymmetry is stated explicitly in ADR 010 and the developers'
   guide, with a tracked closure item (`11.2.1`).
4. Risk: Cross-reference drift — one document still calls ADR 010 "proposed".
   Severity: low. Likelihood: medium.
   Mitigation: the concrete steps enumerate every file that names the ADR
   status, and validation greps for stale "proposed" references to ADR 010.

## Progress

- [x] (2026-06-24) Stage A: research and propose — gather code, prior art, and
  the sibling-item delivery pattern; draft the resolved decisions. (completed
  during planning; see `Context and orientation` and `Artifacts and notes`.)
- [x] (2026-06-24) Stage B: author the accepted ADR 010 rewrite (status, dates,
  Decision Outcome expansion, resolved Outstanding Decisions, tracked call-site
  note). Commit: `813898b`.
- [x] (2026-06-24) Stage C: propagate the acceptance across `roadmap.md`, the
  zero-copy migration roadmap, the inventory, `contents.md`, and
  `developers-guide.md`. Commit: `4123b3e`.
- [x] (2026-06-24) Stage D: run documentation and commit gates, then a
  `coderabbit review --agent` pass; clear all concerns. CodeRabbit completed
  with `findings: 0`.
- [x] (2026-06-24) Mark roadmap `10.1.3` done and finalise this ExecPlan to
  `COMPLETE`.

Each stage is gated by `make markdownlint` (and `make nixie` if a diagram is
touched) before commit. Stages B and C are committed separately so the decision
edit and its propagation can be reviewed independently.

## Surprises & discoveries

- Observation: The "sole owner" goal of ADR 010 is already substantially met in
  code on the outbound path.
  Evidence: `rg -n "\.wrap_payload\(" src/` shows exactly one non-test
  production caller, `src/app/outbound_encoding.rs:36`
  (`codec.wrap_payload(Bytes::from(bytes))`); all other matches are under
  `*/tests.rs`, `src/codec/examples.rs`, `src/test_helpers/`, or
  `src/testkit/`.
  Impact: `10.1.3` is genuinely a decision-closure item. The remaining work is
  to write the boundary down precisely and dispose of the test-only bridges, not
  to relocate production framing logic.
- Observation: The final outbound copy is a single, already-annotated bridge.
  Evidence: `src/app/outbound_encoding.rs` carries a comment citing
  `https://github.com/leynos/wireframe/issues/538`, noting the
  `Bytes::from(bytes)` conversion is behaviour-preserving until the serializer
  contract becomes `Bytes`-native.
  Impact: ADR 010 should point its "remove the final copy" consequence at
  roadmap `11.1.2` / issue #538 rather than implying `10.1.3` removes it.
- Observation: The plan's preflight call-site inventory still holds at
  execution time.
  Evidence: on 2026-06-24,
  `rg -n "\.wrap_payload\(" src/ | grep -v -i test` returned only
  `src/app/outbound_encoding.rs:36:    let frame =
  codec.wrap_payload(Bytes::from(bytes));`.
  Impact: the ADR can safely state the codec driver is the only production
  transport-frame emission site today, while deferring enforcement coverage to
  roadmap `11.2.3`.
- Observation: `AGENTS.md` asks readers to start with
  `docs/repository-layout.md`, but that file is absent in this worktree.
  Evidence: `sed -n '1,220p' docs/repository-layout.md` failed with
  `No such file or directory`.
  Impact: execution used `docs/contents.md`, the documentation style guide, and
  the governing ADR, roadmap, inventory, and developers' guide files listed in
  this plan instead.
- Observation: The original stale-reference grep in this plan was too broad
  after execution started because the ExecPlan itself intentionally describes
  the risk of stale "proposed" references.
  Evidence: after Stage C, `rg -ni "adr.?010|transport.frame boundary" docs/ |
  rg -i "propos"` matched only this ExecPlan's risk and validation text.
  Impact: no governing document still describes ADR 010 as proposed. The
  concrete validation step now treats ExecPlan self-references and preserved ADR
  proposal history as non-drift.
- Observation: The full gate and review pass succeeded after the documentation
  changes.
  Evidence: `make markdownlint`, `make check-fmt`, `make lint`, and `make test`
  all exited 0 on 2026-06-24 using tee logs under `/tmp`. `coderabbit
  review --agent` completed with `findings: 0`.
  Impact: no deterministic gate failure or CodeRabbit concern remains for this
  decision-closure item.

## Decision log

- Decision: Resolve ADR 010 Outstanding Decision (A) by keeping protocol hooks
  packet-oriented and documenting the app-router `before_send` gap as a tracked
  limitation.
  Rationale: `before_send` currently runs in `ConnectionActor::push_frame`
  (`src/connection/frame.rs`) against `F: Packet` (production `Envelope`), where
  it has access to packet identity and correlation. Moving hooks to the
  transport-frame layer would force them to operate on `F::Frame` (for example
  `Bytes`), discarding the packet semantics they rely on. The genuine gap — that
  `FramePipeline` does not yet fire `before_send` for app-router responses
  because `F::Frame` and `Envelope` can differ (deferred in roadmap `9.3.1`) —
  is best closed by unifying the response path onto the actor's `Envelope` hook
  stage under `11.2.1`, not by relocating hooks downward.
  Date/Author: 2026-06-22, planning agent (with Logisphere panel input).

- Decision: Resolve ADR 010 Outstanding Decision (B) with no public
  feature-gated compatibility shim; keep `Packet for Vec<u8>` in test support and
  sanction the public `CorrelatableFrame for Vec<u8>` impl to leave the core
  surface under the breaking release.
  Rationale: `Packet for Vec<u8>` is already test-only
  (`src/connection/test_support.rs`) and provides actor-test value, matching ADR
  010's non-goal of removing test helpers prematurely.
  `impl CorrelatableFrame for Vec<u8>` lives in production module
  `src/correlation.rs` but has no production caller; ADR 009 already routes
  runtime bridges to ADR 010 and removes middleware `&mut Vec<u8>` editors.
  Because the impl is public, Telefono (contracts lens) flagged its removal as
  a breaking change, so it is bound to `11.2.2` / `14.2.1` rather than treated
  as a free internal cleanup.
  Date/Author: 2026-06-22, planning agent (with Logisphere panel input).

- Decision: Resolve ADR 010 Outstanding Decision (C) as "no new trait".
  Rationale: `Packet` (`id`, `into_parts`, `from_parts`) plus
  `EncodeWith<Serializer>` already composes "serializable packet", satisfied by
  `Envelope`. A bridging trait is ADR 010's rejected Option C and adds a wrapper
  layer without removing copies. Prior art corroborates: tonic reuses the message
  type plus an `Encoder` rather than a bespoke packet trait, and keeps gRPC
  length-framing in a separate buffer layer (see `Artifacts and notes`).
  Date/Author: 2026-06-22, planning agent (with Logisphere panel input).

- Decision: Record the canonical production `wrap_payload` call site in the ADR
  and require a guard against new non-driver callers (implemented in phase 11).
  Rationale: ADR 010's "Known Risks" already asks for a tracked list of
  non-driver callers. Buzzy Bee (scaling lens) and Doggylump (ops lens) noted a
  manual list rots; a grep-style guard test keeps the "sole owner" guarantee
  enforceable. The requirement is recorded now; the test is added with the
  runtime work (`11.2.3`).
  Date/Author: 2026-06-22, planning agent (with Logisphere panel input).

- Decision: Deliver `10.1.3` as documentation only, mirroring `10.1.1` and
  `10.1.2`.
  Rationale: those sibling approval items each flipped an ADR to `Accepted`,
  expanded its Decision Outcome, ticked the roadmaps, aligned the inventory, and
  added a developers'-guide note, with no runtime change. Consistency keeps the
  decision-closure phase reviewable and reversible.
  Date/Author: 2026-06-22, planning agent.

- Decision: Begin execution from explicit user approval on 2026-06-24 while
  leaving the scope documentation-only.
  Rationale: the user asked to proceed with implementation of this plan. The
  branch already tracks `origin/10-1-3-approve-actor-and-codec-driver-boundary`,
  and the preflight inventory confirmed no decision-drift tolerance has been
  breached.
  Date/Author: 2026-06-24, implementation agent.

- Decision: Treat the broad stale-reference grep as an advisory drift check
  rather than a literal zero-output command.
  Rationale: accepted ADRs deliberately preserve proposal history such as
  proposed dates, options considered, and rejected directions. After Stage C,
  the only matches from the original command were this ExecPlan's own risk and
  validation prose. No governing status reference still described ADR 010 as
  proposed, so this did not breach the cross-reference-drift risk.
  Date/Author: 2026-06-24, implementation agent.

## Outcomes & retrospective

ADR 010 is accepted and now records the actor and codec-driver boundary needed
for phase 11. The accepted record keeps the connection actor packet-oriented,
makes the codec driver the production transport-frame emission owner, keeps
protocol hooks at the packet stage, rejects a new "serializable packet" trait,
and sequences the public `CorrelatableFrame for Vec<u8>` bridge removal with
the breaking-release runtime-boundary work.

The roadmap, zero-copy migration roadmap, inventory, developers' guide, and
documentation index now agree with the accepted decision. No Rust code or
public API changed in this item. The main lesson from execution is that the
planned stale-reference grep was useful but too broad; a human check still has
to distinguish preserved ADR history from live status drift.

## Context and orientation

This section assumes no prior knowledge of the repository.

### Where the boundary lives in code

- **The connection actor.** `src/connection/mod.rs` defines
  `ConnectionActor<F, E>` with the bound
  `F: FrameLike + CorrelatableFrame + Packet`. In production the server
  instantiates the actor with `F = Envelope`; the generic parameter exists so
  protocol-native packet types remain possible. `FrameLike` is a
  `Send + 'static` marker; `CorrelatableFrame` (`src/correlation.rs`) supplies
  `correlation_id`/`set_correlation_id`; `Packet` (`src/app/envelope.rs`)
  supplies `id`, `into_parts`, `from_parts`. The actor never emits transport
  frames directly.
- **The codec driver.** `src/app/codec_driver.rs` owns `FramePipeline`
  (fragmentation and metrics over `Envelope`) and `send_envelope`.
  `src/app/outbound_encoding.rs::encode_message_frame` performs the
  `packet -> bytes -> transport frame` transition: it calls
  `Serializer::serialize` (returning `Vec<u8>`), bridges with `Bytes::from`, and
  calls `FrameCodec::wrap_payload` — the single production framing site.
- **Production connection wiring.** `src/server/connection_spawner.rs` reads the
  preamble, wraps the stream in `RewindStream`, then calls
  `app.handle_connection_result`. The app path drives responses through the
  codec driver. The actor's hook stage is `ConnectionActor::push_frame`
  (`src/connection/frame.rs`), which calls `self.hooks.before_send(&mut frame,
  ..)` before buffering each outbound frame.
- **The `Vec<u8>` bridges.** `src/correlation.rs` implements
  `CorrelatableFrame` for `u8`, `Vec<u8>`, and (in `src/app/envelope.rs`)
  `Envelope`. The `Vec<u8>` impl is a no-op correlation carrier with no
  production caller. `src/connection/test_support.rs` implements `Packet` for
  `Vec<u8>` (and `u8`) for actor unit tests.
- **The serializer.** `src/serializer.rs` defines
  `Serializer::serialize<M>(&self, &M) -> Result<Vec<u8>, _>`;
  `BincodeSerializer` preserves that return type. Issue #538 tracks moving this
  to a `Bytes`-native container, which is what removes the final outbound copy
  under roadmap `11.1.2`.

### The governing documents

- [`docs/adr-010-transport-frame-boundary-for-zero-copy.md`](../adr-010-transport-frame-boundary-for-zero-copy.md)
  — the decision to accept; currently `Proposed`.
- [`docs/frame-vec-u8-inventory.md`](../frame-vec-u8-inventory.md) — the source
  inventory; its "Resolved direction for epic 284" and ADR list reference ADR
  010.
- [`docs/zero-copy-frame-and-payload-migration-roadmap.md`](../zero-copy-frame-and-payload-migration-roadmap.md)
  — item `1.1.3` mirrors roadmap `10.1.3`.
- [`docs/roadmap.md`](../roadmap.md) — item `10.1.3` and the phase-11 items that
  consume the decision.
- [`docs/developers-guide.md`](../developers-guide.md) — has a "Public
  byte-container model" section (added by `10.1.1`/`10.1.2`) to extend with the
  boundary note.
- [`docs/contents.md`](../contents.md) — indexes ADRs and execution plans.

### The sibling-item delivery pattern

Items `10.1.1` (ADR 008, commit `cd153de`) and `10.1.2` (ADR 009, commit
`3969c3b`) set the template this plan follows. Each:

1. flipped the ADR `Status` from `Proposed` to `Accepted`, with the date field
   becoming `Proposed <date>. Accepted <date>.` and a prose acceptance statement
   in the Status section;
2. renamed `## Decision Outcome / Proposed Direction` to `## Decision Outcome`
   and changed "The proposed direction is:" to "The accepted direction is:";
3. expanded the outcome with concrete, named commitments and links to the
   roadmap items that implement deferred parts;
4. ticked the matching checkboxes in `roadmap.md` and the zero-copy migration
   roadmap;
5. aligned `frame-vec-u8-inventory.md` and added a `developers-guide.md` note;
6. added the execplan and a `contents.md` index line.

## Plan of work

### Stage A: research and propose (no edits) — done during planning

Completed. Findings are captured in `Context and orientation`, `Surprises &
discoveries`, and `Artifacts and notes`. The three Outstanding Decisions are
resolved in `Decision log`. The Logisphere panel returned "Proceed with
conditions"; the conditions are folded into the constraints and the ADR rewrite
below.

### Stage B: author the accepted ADR 010

Edit `docs/adr-010-transport-frame-boundary-for-zero-copy.md` only:

1. **Status.** Change `Proposed` to `Accepted` and add an acceptance paragraph
   in the form used by ADR 008/009: "Accepted on 2026-06-22. Wireframe keeps the
   connection actor packet-oriented and makes the codec driver the sole owner of
   transport-frame emission …", naming the three resolved decisions in one
   sentence each and linking roadmap items `11.1.2`, `11.2.1`, `11.2.2`,
   `11.2.3`, and `14.2.1`.
2. **Date.** Change `2026-04-12` to `Proposed 2026-04-12. Accepted 2026-06-22.`
3. **Decision Outcome.** Rename the heading to `## Decision Outcome`, change the
   lead-in to "The accepted direction is:", and keep the four existing bullets,
   making each a firm commitment rather than a proposal.
4. **Resolve Outstanding Decisions.** Replace the `## Outstanding Decisions`
   section with `## Resolved Decisions`, recording the three resolutions from
   this plan's `Decision log`, each linking to its implementing roadmap item.
   Keep the question text so the record shows what was asked and answered.
5. **Tracked call-site note.** In `## Known Risks and Limitations` (or a short
   `## Tracked transport-frame call sites` subsection), record that the sole
   production `wrap_payload` caller today is `src/app/outbound_encoding.rs`, that
   all other callers are tests/examples/testkit, and that a guard against new
   non-driver production callers is required and tracked under `11.2.3`.
6. **Goals/Non-Goals.** Adjust tense so the decision-closure goals read as
   achieved, leaving the runtime goals pointing at phase 11.

Validate with `make markdownlint` and commit Stage B on its own.

### Stage C: propagate the acceptance

1. `docs/roadmap.md`: tick item `10.1.3` `[ ]` to `[x]`.
2. `docs/zero-copy-frame-and-payload-migration-roadmap.md`: tick item `1.1.3`
   and its three child bullets `[x]`.
3. `docs/frame-vec-u8-inventory.md`: update the "Resolved direction for epic
   284" / ADR list entry so ADR 010 reads `(accepted)` with acceptance-tense
   wording, matching how ADR 008/009 are described there.
4. `docs/developers-guide.md`: add a short "Actor and codec-driver boundary"
   subsection after "Public byte-container model", stating that the actor stays
   packet-oriented, the codec driver owns transport-frame emission, protocol
   hooks run at the packet stage, and the app-router `before_send` gap is tracked
   under `11.2.1`; cross-reference ADR 010.
5. `docs/contents.md`: update the ADR 010 line to acceptance wording and add an
   "Execution plans" index line for this plan.

Validate with `make markdownlint` and commit Stage C on its own.

### Stage D: gates and review

Run the full commit gateway and a CodeRabbit pass; clear every concern before
finalising. Then tick this plan's `Progress` items, set `Status: COMPLETE`, and
complete `Outcomes & retrospective`.

## Concrete steps

Run all commands from the repository root
(`/home/leynos/.lody/repos/github---leynos---wireframe/worktrees/9f3f8a4e-aca9-4624-b9c5-92de91c9e0bf`).

1. Confirm the call-site inventory still holds before writing the ADR claim:

   ```bash
   rg -n "\.wrap_payload\(" src/ | grep -v -i test
   ```

   Expected: a single line, `src/app/outbound_encoding.rs:36: ... codec.wrap_payload(Bytes::from(bytes));`.

2. Apply Stage B edits, then:

   ```bash
   make markdownlint 2>&1 | tee /tmp/markdownlint-wireframe-10-1-3.out
   git add docs/adr-010-transport-frame-boundary-for-zero-copy.md
   ```

   Commit with a message recording acceptance of ADR 010 and resolution of its
   Outstanding Decisions.

3. Apply Stage C edits, then re-run `make markdownlint`, confirm no document
   still calls ADR 010 proposed:

   ```bash
   rg -ni "adr.?010|transport.frame boundary" docs/ | rg -i "propos"
   ```

   Expected: no governing document still describes ADR 010 as proposed. Matches
   inside this ExecPlan's risk/validation text, or inside preserved ADR proposal
   history such as dual dates and rejected options, are not cross-reference
   drift. Stage and commit the propagation edits.

4. Run the commit gateway (documentation-only change, but the full suite must
   stay green):

   ```bash
   make check-fmt 2>&1 | tee /tmp/check-fmt-wireframe-10-1-3.out
   make lint      2>&1 | tee /tmp/lint-wireframe-10-1-3.out
   make test      2>&1 | tee /tmp/test-wireframe-10-1-3.out
   ```

   Run sequentially (the environment caches builds; do not parallelise).

5. Request review:

   ```bash
   coderabbit review --agent 2>&1 | tee /tmp/coderabbit-wireframe-10-1-3.out
   ```

   Resolve every concern, re-running the relevant gate, before finalising.

## Validation and acceptance

Because this item changes only documentation, there is no Red-Green-Refactor
code cycle; the nearest observable substitutes are the documentation gates and
the cross-reference checks below (recorded under the execplans guidance for when
test-first delivery is genuinely unavailable).

Acceptance is behaviour a reviewer can verify:

1. Opening `docs/adr-010-transport-frame-boundary-for-zero-copy.md` shows
   `Status: Accepted`, dual dates, a `## Decision Outcome` heading, and a
   `## Resolved Decisions` section answering all three former Outstanding
   Decisions, each linking a phase-11 roadmap item.
2. `docs/roadmap.md` line for `10.1.3` and the zero-copy migration roadmap line
   for `1.1.3` (and children) render checked.
3. `rg -ni "adr.?010" docs/ | rg -i "propos"` returns nothing.
4. `make markdownlint`, `make check-fmt`, `make lint`, and `make test` each exit
   0; their `tee` logs in `/tmp` show success.
5. `coderabbit review --agent` reports no unresolved concerns.

Quality criteria ("done"):

- Tests: `make test` passes unchanged (no new or altered tests; documentation
  only).
- Lint/format: `make lint`, `make check-fmt`, and `make markdownlint` pass.
- Documentation: all five governing documents agree ADR 010 is accepted; the
  developers' guide records the boundary and the tracked `before_send` gap.

Quality method: the commands in `Concrete steps`, then CodeRabbit.

## Idempotence and recovery

Every step is a documentation edit and is safe to re-run; re-running
`make markdownlint` or the grep checks has no side effects. If a stage commit is
wrong, `git revert` it — there is no runtime state to roll back. If the
call-site inventory in step 1 returns more than one production caller, that
breaches Tolerance 2: stop and escalate rather than weakening the ADR's "sole
owner" claim.

## Artifacts and notes

### Prior art (firecrawl research)

The "no new trait" resolution of Outstanding Decision (C) is corroborated by two
mature Rust networking ecosystems that separate message/packet semantics from
transport framing:

- **tonic** (`tonic::codec`) defines a `Codec` that yields an `Encoder` and
  `Decoder` over *message types*, while gRPC length-framing is handled by a
  separate buffer layer (`EncodeBuf`/`DecodeBuf` over `bytes::Bytes`). The
  decoder receives "exactly the bytes of a full message"; framing is not the
  message trait's concern. tonic introduces no bespoke "serializable packet"
  trait — it reuses the message type plus an encoder.
  Source: <https://docs.rs/tonic/latest/tonic/codec/index.html>.
- **tokio_util::codec** parameterises `Encoder<Item>`/`Decoder` on the item
  type and owns framing in `Framed`, keeping the *item* distinct from the byte
  buffer. The tonic maintainers note the two trait families differ mainly in
  tokio-util's `Encoder` taking the item as a type parameter.
  Source: <https://github.com/grpc/grpc-rust/discussions/761>.

Both build the byte plane on `bytes::Bytes`, matching ADR 008's accepted public
byte container and reinforcing Option B: keep packet semantics in `Packet` +
`EncodeWith`, and keep transport framing in the codec driver.

### Logisphere design review (conditions folded in)

Verdict: ⚠️ Proceed with conditions. Conditions, each addressed above:

- ☎️ Telefono: removing public `CorrelatableFrame for Vec<u8>` is breaking —
  sequence with the epic-284 breaking release (`11.2.2`/`14.2.1`); `10.1.3`
  changes no public API. (Constraint 1; Risk 2; Decision (B).)
- 🐶 Doggylump: document the `before_send` asymmetry, do not hide it; track to
  `11.2.1`. (Decision (A); Risk 3; Stage C step 4.)
- 🐝 Buzzy Bee: guard the "sole owner" guarantee with a check against new
  non-driver `wrap_payload` callers; record now, implement in `11.2.3`.
  (Decision; ADR tracked-call-site note.)
- 🐼 Pandalump: state the actor stays generic over `Packet` while `Vec<u8>`
  ceases to be a sanctioned frame type. (Context; ADR Decision Outcome.)
- 🐈🧇 Wafflecat: the strongest alternative — move hooks to the transport-frame
  layer — is rejected because it strips the packet semantics hooks rely on.
  (Decision (A).)

## Interfaces and dependencies

This item defines no new Rust interfaces. It records the boundary the phase-11
runtime work must honour. The interfaces named below already exist and must
remain stable through `10.1.3`:

- `crate::connection::ConnectionActor<F, E>` with
  `F: FrameLike + CorrelatableFrame + Packet` (`src/connection/mod.rs`) — stays
  packet-oriented; `Envelope` in production.
- `crate::app::outbound_encoding::encode_message_frame` and
  `crate::app::codec_driver` (`FramePipeline`, `send_envelope`) — the sole
  owner of `packet -> bytes -> transport frame`.
- `crate::codec::FrameCodec::wrap_payload` — the canonical transport-frame
  emission point; production callers must stay confined to the codec driver.
- `crate::serializer::Serializer::serialize` — remains `Vec<u8>`-returning until
  issue #538 / roadmap `11.1.2`.
- `crate::correlation::CorrelatableFrame` and `crate::app::Packet` — the
  composition that expresses "serializable packet"; no new trait is added.

Downstream roadmap items that depend on this decision: `11.1.2` (remove the
final outbound copy), `11.2.1` (implement the boundary and close the
`before_send` gap), `11.2.2` (move `CorrelatableFrame for Vec<u8>` out of
production), `11.2.3` (allocation and pointer-reuse regressions plus the
non-driver-caller guard), and `14.2.1` (review retained bridges after the
breaking release).

## Revision note

Initial draft (2026-06-22): created the decision-closure plan for roadmap
`10.1.3`. Resolved ADR 010's three Outstanding Decisions, captured the
sibling-item delivery pattern from `10.1.1`/`10.1.2`, recorded prior-art
corroboration (tonic, tokio_util) and a Logisphere "proceed with conditions"
review, and scoped the work to documentation only with implementation deferred
to roadmap phase 11. Status: DRAFT, awaiting approval before execution.

Execution update (2026-06-24): user approval received, branch tracking confirmed
against `origin/10-1-3-approve-actor-and-codec-driver-boundary`, preflight
call-site inventory rechecked, and status moved to IN PROGRESS before Stage B
ADR edits.

Completion update (2026-06-24): accepted ADR 010, propagated the acceptance
through the governing documentation, passed `make markdownlint`,
`make check-fmt`, `make lint`, and `make test`, and completed
`coderabbit review --agent` with `findings: 0`. Status: COMPLETE.
