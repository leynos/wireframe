# Approve the `Vec<u8>` compatibility and rollout policy for downstream users

This ExecPlan is a living document. The sections `Constraints`, `Tolerances`,
`Risks`, `Progress`, `Surprises & discoveries`, `Decision log`, and
`Outcomes & retrospective` must be kept up to date as work proceeds.

Status: COMPLETE

Implementation approval: RECEIVED. The user approved the plan and its
recommended policy answers on 2026-06-15, and waived the CodeRabbit review
gate for this docs-only milestone.

## Purpose / big picture

Roadmap item `10.1.2` closes the second of three design questions that block
the zero-copy payload migration in epic 284. Roadmap item `10.1.1` already
approved [ADR 008](../adr-008-zero-copy-public-byte-container.md), which fixes
`bytes::Bytes` (or a transparent project wrapper) as the stable public byte
representation and adopts an explicit edit-on-demand workflow for mutation.
That leaves the question of *how downstream users move across the
release boundary*: which `Vec<u8>` helpers stay, for how long, behind what
visibility, and how the project will know when to remove them.

After this work is approved and implemented, maintainers can see success by
opening [ADR 009](../adr-009-vec-u8-migration-rollout.md) and finding the
status field set to `Accepted` with three previously open questions resolved:

1. Which `Vec<u8>` compatibility helpers ship in the breaking release.
2. Whether those helpers are feature-gated or ship in the default build.
3. What adoption or benchmark signal justifies removing them later.

The accepted policy must give later roadmap items (`12.1.2`, `12.2.2`,
`13.1.1`, `13.1.2`, `14.1.1`, `14.1.2`, `14.1.3`, and `14.2.1`) an unambiguous
contract to implement against. No runtime Rust code changes are authorised by
this plan.

## Constraints

- Use the `execplans` skill for this plan and keep this document
  self-contained.
- Use the `leta` skill for code navigation when confirming the current
  `Vec<u8>` surfaces that the policy will govern. Add the worktree as a
  `leta` workspace before running any `leta` command.
- Use the `rust-router` skill to direct the design discussion into more
  specific skills. The natural targets here are `rust-types-and-apis`,
  `arch-supply-chain`, `arch-decision-records`, `arch-crate-design`, and
  `rust-unused-code` for the deprecation and removal mechanics.
- Use the `firecrawl` MCP tools only for bounded prior-art checks; the
  repository documents remain the source of truth for Wireframe requirements.
- Use the `en-gb-oxendict` skill voice and the
  [documentation style guide](../documentation-style-guide.md) for every
  edited Markdown file. Oxford spelling, Oxford comma where it improves
  clarity, and en-GB usage.
- Use the `commit-message` skill for the final commit and the `pr-creation`
  skill for the draft pull request.
- Keep the scope to roadmap item `10.1.2`. Do not approve the actor and
  codec-driver boundary (`10.1.3`), benchmark thresholds (`10.2.1`, `10.2.2`),
  the migration-guide outline (`10.2.3`), or any item from phases `11`–`14`.
- Do not claim runtime behaviour has changed as part of `10.1.2`. If later
  implementation edits Rust code under this roadmap item, this plan must be
  revised and re-approved before those edits begin.
- Do not mark roadmap item `10.1.2` done until ADR 009 is accepted, aligned
  documents are updated, and all required quality gates pass on a clean run.
- Use Makefile targets for validation. Run formatting, linting, and tests
  sequentially with `tee` logs under `/tmp` using the template named in the
  user's global instructions.
- Use `coderabbit review --agent` after the decision-draft milestone and
  before opening the pull request. Clear every concern or document why it is
  out of scope.
- Preserve all unrelated formatter and lint baselines. Do not stage
  repository-wide Markdown reformat changes that this plan did not introduce.

## Tolerances

- Scope: if implementation of this decision-closure task requires Rust source
  changes, stop and ask whether `10.1.2` should be expanded beyond an
  approval and documentation item.
- Scope: if documentation changes exceed six files or 750 net lines, stop and
  split the work into a decision pull request and a follow-up documentation
  pull request. The line-count budget is wider than the `10.1.1` plan
  because ADR 009 needs a substantive rewrite of its `Status`, `Outstanding
  Decisions`, and `Known Risks` sections.
- Interface: if the accepted policy cannot name a stable disposition (retain
  as deprecated helper, or remove outright) for each of `PacketParts`,
  `Envelope`, `ServiceRequest`, `ServiceResponse`, `Serializer::serialize`,
  `BeforeSendHook`, the `CorrelatableFrame for Vec<u8>` runtime bridge, the
  test-only `Packet for Vec<u8>` bridge, and the client preamble leftover
  surface, stop and present the remaining options with trade-offs.
- Naming: if the Logisphere review or the user requires committing to
  canonical helper *names* (not only the *set*) inside ADR 009, treat that
  as in scope for this milestone rather than deferring to `12.1.1`. Naming
  ambiguity is cheap to lock now and expensive to relitigate later.
- Dependencies: if the accepted policy requires a new external crate beyond
  the existing `bytes` and standard `#[deprecated]` mechanics, stop and
  request approval before accepting that dependency.
- Coverage: if the policy depends on `cargo-semver-checks` or
  `cargo-public-api` being wired into CI before the breaking release lands,
  capture that as a follow-up roadmap item under phase `14`; do not silently
  add new CI work to `10.1.2`.
- Ambiguity: if the user disagrees with any of the recommended helper-set
  choices below, stop, record both options in the Decision log, and present
  the trade-offs without picking one.
- Performance: if the policy relies on an unmeasured performance claim, keep
  it as a hypothesis and defer thresholds to `10.2.1` and `10.2.2`.
- Iteration: if the same quality gate fails after three fix attempts, stop,
  document the failure in `Decision log`, and ask for direction.
- Time: if any milestone takes more than four hours of wall time without
  visible progress, stop and escalate.

## Risks

- Risk: the accepted helper set quietly becomes permanent because no removal
  signal is written down. Severity: high. Likelihood: medium. Mitigation:
  ADR 009 must record both a target removal release and an explicit review
  point with named criteria; roadmap item `14.2.1` is the next review hook
  and must be linked from the ADR.

- Risk: the policy preserves the `Vec<u8>` mental model so well that
  downstream users never migrate. Severity: high. Likelihood: medium.
  Mitigation: every retained helper must carry `#[deprecated(since, note)]`
  pointing at the recommended zero-copy equivalent in the breaking release,
  and the migration guide outline in `10.2.3` must show before-and-after code
  for each helper.

- Risk: feature-gating the helpers makes them invisible to users who have not
  read the changelog. Severity: medium. Likelihood: medium. Mitigation: keep
  helpers in the default build for the breaking release so the deprecation
  warnings reach all consumers; document the visibility decision in ADR 009.

- Risk: the policy ships helpers that re-introduce the final-copy bottleneck
  that ADR 008 set out to remove. Severity: high. Likelihood: low.
  Mitigation: classify each candidate helper as "edit surface", "ingest
  surface", or "preamble leftover" and forbid any helper that performs
  copies on the default outbound path between `Serializer::serialize` and
  `FrameCodec::wrap_payload`.

- Risk: the policy fragments the migration story across many tiny shims
  rather than a few obvious ones. Severity: medium. Likelihood: medium.
  Mitigation: keep the accepted helper set to a single named surface per
  affected type, and prefer a `from_vec` / `into_vec` pattern on the stable
  byte wrapper over per-call-site adapters.

- Risk: client preamble leftover bytes are pulled into the deprecation cycle
  unintentionally. Severity: medium. Likelihood: low. Mitigation: state
  explicitly in ADR 009 that the preamble leftover path stays on owned
  `Vec<u8>` for at least one release after the breaking change, per the
  resolved direction in
  [`frame-vec-u8-inventory.md`](../frame-vec-u8-inventory.md) and roadmap
  item `12.2.2`.

- Risk: `10.1.2` absorbs migration-guide outline work that belongs to
  `10.2.3`. Severity: medium. Likelihood: medium. Mitigation: write explicit
  deferrals in ADR 009 and in this plan's acceptance criteria.

- Risk: external prior art is over-applied (for example, treating hyper 1.0's
  `hyper-util` split as a template for Wireframe). Severity: medium.
  Likelihood: low. Mitigation: cite prior art only where it confirms a
  pattern already independently motivated by the Wireframe inventory; keep
  the repository ADRs as the authority.

- Risk: downstream users suppress deprecation warnings (a common
  workspace-lint configuration uses `#[allow(deprecated)]` or
  `unused_imports = "allow"` blanket overrides) and are then blindsided when
  the helpers vanish in the next breaking release. Severity: medium.
  Likelihood: medium. Mitigation: every deprecation `note` must include the
  scheduled removal version, so users who suppress the warning still see the
  removal date in `cargo doc`; the migration guide outline in `10.2.3` must
  recommend against blanket deprecation suppression in workspaces that
  consume Wireframe.

- Risk: downstream crates re-export the deprecated helpers in their own
  public API and propagate the deprecation warning to *their* users, who
  cannot fix it without forking. Severity: medium. Likelihood: low.
  Mitigation: ADR 009 should state that the deprecation note for each
  helper directs downstream re-exporters to add their own wrapper rather
  than re-export the Wireframe item directly, and the `10.2.3` migration
  guide outline must include a "for downstream library authors" section.

- Risk: the `into_vec` helper itself becomes a performance footgun because
  it allocates and copies (per the cited `bytes::Bytes` semantics) even
  though the rest of the API is zero-copy. Severity: medium. Likelihood:
  medium. Mitigation: ADR 009 must specify that the deprecation `note` on
  `PayloadBytes::into_vec` steers users to slice or `freeze()` paths, not
  only to `PayloadEditor`, so the recommended replacement is a zero-copy
  one wherever possible.

## Progress

- [x] (2026-06-04) Confirm branch state, leta workspace registration, and
      roadmap context.
- [x] (2026-06-04) Inspect current `Vec<u8>` surfaces with `leta`/`Read`
      review for `PacketParts`, `Envelope`, `ServiceRequest`,
      `ServiceResponse`, `BeforeSendHook`, `Serializer::serialize`, and the
      client preamble leftover path.
- [x] (2026-06-04) Record bounded prior-art findings (`cargo-semver-checks`,
      `cargo-public-api`, Rust `#[deprecated]`, hyper 1.0 upgrade, Common
      Changelog) in `Artifacts and notes`.
- [x] (2026-06-04) Draft the answers to ADR 009's three outstanding
      decisions, including the recommended helper set, visibility, and
      removal signal.
- [x] (2026-06-04) Run a Logisphere community-of-experts review on this
      draft and update the plan in response. Review findings recorded in
      `Surprises & discoveries`; `Decision log`, `Risks`, `Stage D`, and
      `Interfaces and dependencies` revised accordingly.
- [x] (2026-06-15) Received explicit user approval of the plan and the
      recommended policy answers. CodeRabbit review waived for this
      docs-only milestone.
- [x] (2026-06-15) Edited `docs/adr-009-vec-u8-migration-rollout.md` to
      `Accepted` with the resolved helper set, visibility, and removal
      signal, and added the prior-art footnotes.
- [x] (2026-06-15) Aligned `docs/roadmap.md` (ticked `10.1.2`),
      `docs/zero-copy-frame-and-payload-migration-roadmap.md` (ticked
      `1.1.2` and its children), and reconciled the ADR status wording in
      `docs/frame-vec-u8-inventory.md`.
- [x] (2026-06-15) Decided that no `docs/users-guide.md` or
      `docs/developers-guide.md` change is needed in this milestone; the
      downstream-facing rollout pointer belongs to roadmap item `10.2.3`
      (migration-guide outline). No runtime behaviour changed.
- [x] (2026-06-15) Ran `make check-fmt`, `make markdownlint`, `make nixie`,
      `make lint`, and `make test` with `tee` logs under `/tmp`.
- [x] (2026-06-15) CodeRabbit review waived by the user for this milestone.
- [x] (2026-06-15) Committed the decision-closure changes and pushed the
      `10.1.2` branch.
- [x] (2026-06-04) Draft pull request #533 opened with the lody session
      link in the `## References` section.
- [x] (2026-06-15) Marked roadmap item `10.1.2` done.

Use timestamps when ticking items to make rate-of-progress visible.

## Surprises & discoveries

- Observation (2026-06-04): the Logisphere design review (Pandalump,
  Wafflecat, Buzzy Bee, Telefono, Doggylump, Dinolump) flagged six
  substantive corrections before this plan reached `Status: DRAFT` review.
  Evidence: review transcript captured in the implementation postmortem
  notes (in-conversation, agent run). Impact: the `Decision log`,
  `Risks`, and Stage D were rewritten to (i) include
  `ServiceRequest`/`ServiceResponse` explicitly with a "removed outright"
  disposition, (ii) clarify that `CorrelatableFrame for Vec<u8>` and
  test-only `Packet for Vec<u8>` are governed by ADR 010, not this policy,
  (iii) replace "next major + six months" with a semver-aware event-based
  removal trigger, (iv) require the deprecation `note` to include the
  scheduled removal version, (v) require the `into_vec` deprecation note to
  steer users to zero-copy paths, and (vi) commit canonical helper names in
  ADR 009 rather than deferring naming to `12.1.1`. The line-count
  tolerance was raised to 750 net lines to accommodate the ADR 009
  rewrite.

- Observation (2026-06-15): `make nixie` fails on
  `docs/v0-2-0-to-v0-3-0-migration-guide.md` with a merman-cli classDiagram
  `LexError` ("Unexpected character"). Evidence:
  `/tmp/nixie-wireframe-10-1-2.out`; the same failure reproduces on the
  clean committed `HEAD` with the working changes stashed, so it is not
  caused by this milestone. Impact: this is a pre-existing repository
  baseline issue in a file this plan does not touch. None of the Markdown
  files edited by this milestone contain Mermaid diagrams, and all were
  processed by nixie without error. Reported as a baseline; not blocking
  `10.1.2`.

- Observation (2026-06-15): the deterministic Rust gates pass on the
  docs-only diff. Evidence: `make check-fmt` clean
  (`/tmp/check-fmt-wireframe-10-1-2.out`), `make lint` exit 0
  (`/tmp/lint-wireframe-10-1-2.out`), `make test` exit 0 with no failing
  test results (`/tmp/test-wireframe-10-1-2.out`), and `markdownlint-cli2`
  clean across all five changed Markdown files
  (`/tmp/markdownlint-wireframe-10-1-2.out`). Impact: the decision-closure
  edits do not regress formatting, Markdown linting, Rust linting, or the
  workspace test suite.

- Observation (2026-06-15): markdownlint initially flagged MD049
  (emphasis-style) on the ADR 009 "Outstanding Decisions" list because the
  rhetorical questions used `*asterisk*` italics. Evidence:
  `/tmp/markdownlint-wireframe-10-1-2.out` first run. Impact: switched to
  `_underscore_` emphasis to match the repository Markdown style; re-run is
  clean.

Record further unexpected findings with evidence and impact as work
proceeds.

## Decision log

- Decision: keep this ExecPlan in DRAFT status and block ADR 009 edits until
  explicit user approval. Rationale: the user explicitly stated that the plan
  must be approved before it is implemented, and the rollout policy has
  semver and downstream-adoption consequences. Date/Author: 2026-06-04 / Codex.

- Decision: advance this ExecPlan to COMPLETE and execute the implementation
  (accept ADR 009, align the roadmap documents, mark `10.1.2` done) after
  the user accepted the recommended policy answers and waived the CodeRabbit
  gate for this docs-only milestone. Rationale: the user replied "Accept your
  recommendations" with an explicit instruction to update the execplan and no
  CodeRabbit review, which satisfies the approval gate. Date/Author:
  2026-06-15 / Codex.

- Decision: bound `10.1.2` to ADR 009 acceptance and aligned-document
  updates, not runtime migration. Rationale: roadmap items `12.1.2`,
  `12.2.2`, `13.1.1`, `13.1.2`, and `14.1.x` carry the actual public API
  migration and release work and depend on this policy decision.
  Date/Author: 2026-06-04 / Codex.

- Decision: recommend Option C from ADR 009 ("staged breaking release with
  finite compatibility helpers") as the accepted direction. Rationale:
  Option A (hard break) maximises upgrade pain on a project that has not yet
  cut 1.0, Option B (permanent dual support) keeps the bottleneck ADR 008 set
  out to remove, and Option C is already the proposed direction in ADR 009.
  Date/Author: 2026-06-04 / Codex.

- Decision: recommend a narrow, single-surface helper set. The canonical
  conversion surface is `PayloadBytes::from_vec` / `PayloadBytes::into_vec`
  on the stable byte wrapper, used by every transport hand-off
  (`PacketParts`, `Envelope`, `Serializer::serialize`). Per-call-site
  `PacketParts::from_vec` / `Envelope::from_vec` constructors are *not*
  added; callers go through the wrapper. Middleware editing
  (`ServiceRequest::frame_mut`, `ServiceResponse::frame_mut`,
  `ServiceResponse::into_inner`) is *removed outright* in the breaking
  release and replaced by the edit-on-demand workflow from ADR 008, because
  retaining `&mut Vec<u8>` accessors would re-introduce the bottleneck
  ADR 008 set out to eliminate. The one `Vec<u8>` adapter that survives the
  middleware redesign is `before_send_from_vec_fn`, an adapter constructor
  on the new `BeforeSendHook` surface so existing `Fn(&mut Vec<u8>)` hooks
  keep working through one release cycle. Rationale: a single conversion
  surface avoids the fragmented-shim risk the plan's own `Risks` section
  flags, keeps the helper count countable in a release note, and leaves the
  edit path on the new edit-on-demand workflow rather than the legacy
  mutable-vec model. Date/Author: 2026-06-04 / Codex.

- Decision: state explicitly in ADR 009 that the runtime
  `CorrelatableFrame for Vec<u8>` bridge and the test-only
  `Packet for Vec<u8>` bridge are *not* compatibility helpers under this
  policy. Their lifecycle is governed by ADR 010 and roadmap items `11.2.1`,
  `11.2.2`, and `14.1.3`. Rationale: those bridges are runtime-internal,
  not part of the user-facing migration surface, and conflating them with
  the documented helper set would silently extend the policy beyond the
  rollout question. Date/Author: 2026-06-04 / Codex.

- Decision: commit canonical helper names in ADR 009 itself, not only in
  the later runtime ExecPlans. The recommended names are
  `PayloadBytes::from_vec`, `PayloadBytes::into_vec`,
  `Serializer::serialize_to_vec` (deprecated alias if a separate method
  proves necessary; otherwise the wrapper conversion suffices), and
  `BeforeSendHook::from_vec_fn` (or `before_send_from_vec_fn` as a
  free-function constructor on `RequestHooks`). Rationale: naming
  ambiguity is cheap to lock now and Telefono's review showed that
  deferring naming to `12.1.1` invites churn during implementation.
  Date/Author: 2026-06-04 / Codex.

- Decision: recommend that helpers ship in the default build for the
  breaking release, not behind a feature flag, and carry
  `#[deprecated(since = "<release-version>", note = "<recommended path>; \
  scheduled for removal in <target-removal-version>")]` attributes. The
  `<release-version>` placeholder is substituted at release-cut time by
  roadmap item `14.1.1`; ADR 009 commits to that substitution rule rather
  than the literal version string. The `<target-removal-version>` field is
  required so users who suppress deprecation warnings still learn the
  removal version from `cargo doc` and rustdoc rendering. Rationale:
  feature-gating hides the deprecation warning from users who do not read
  the changelog, while `#[deprecated]` is the canonical Rust mechanism for
  a finite compatibility window. The rejected alternative is "default-on
  with `#[deprecated]` plus a `vec-u8-compat` feature gating the legacy
  `Fn(&mut Vec<u8>)` hook for a pre-break minor"; we reject it because
  Wireframe has not yet cut 1.0, so the minor-cycle warning window the
  feature flag is meant to buy is collapsed into the release boundary
  anyway. Date/Author: 2026-06-04 / Codex.

- Decision: state the removal signal in semver-aware language for a pre-1.0
  project. Helpers are removed in the *second breaking release* after they
  are introduced (under 0.x semver, that is `0.N → 0.N+2`; once Wireframe
  has cut 1.0, it is `1.x → 2.x`). The review trigger is event-based, not
  calendar-based: review retained helpers at the first breaking release
  after the introduction (roadmap item `14.2.1`) and at every subsequent
  breaking release until removal lands. Rationale: "next major + six
  months" assumes a release cadence Wireframe has not committed to and
  Telefono flagged it as unsafe wording under 0.x semver where every minor
  bump is a breaking release. Tying removal and review to *breaking
  releases* keeps the policy invariant under whatever cadence the project
  eventually adopts. Date/Author: 2026-06-04 / Codex.

- Decision: ADR 009 must explicitly defer `cargo-semver-checks` and
  `cargo-public-api` CI wiring to roadmap item `14.1.x` and state that,
  until that wiring exists, the policy is enforced by review only.
  Rationale: Telefono's review flagged that the prior-art citations imply
  guardrails that no one is yet committed to building; calling out the
  deferral makes the dependency explicit. Date/Author: 2026-06-04 / Codex.

- Decision: recommend that client preamble leftovers stay on owned `Vec<u8>`
  for the breaking release. Rationale: the inventory already resolved this
  direction, the preamble path is one-shot and replay-only, and roadmap item
  `12.2.2` is the dedicated re-evaluation hook. Date/Author: 2026-06-04 / Codex.

- Decision: defer migration-guide before-and-after examples to `10.2.3`.
  Rationale: ADR 009 acceptance and the migration-guide outline are separate
  roadmap items; folding them into `10.1.2` would breach this plan's scope
  tolerance. Date/Author: 2026-06-04 / Codex.

Any further design decision taken during implementation must be appended here
before commit.

## Outcomes & retrospective

ADR 009 is `Accepted` (2026-06-15). The accepted policy is a staged breaking
release (Option C) with a narrow, finite `Vec<u8>` helper set:
`PayloadBytes::from_vec` / `into_vec` as the single transport conversion
surface, the serializer returning the stable wrapper (with an optional
`serialize_to_vec` shim), and one `BeforeSendHook` adapter constructor.
Middleware `&mut Vec<u8>` editors are removed outright in favour of the
ADR 008 edit-on-demand workflow; client preamble leftovers stay on `Vec<u8>`
for this release; runtime bridges (`CorrelatableFrame for Vec<u8>`, test-only
`Packet for Vec<u8>`) are governed by ADR 010, not this policy. Helpers ship
in the default build with `#[deprecated]` attributes whose `note` names the
removal version, and are removed in the second breaking release after
introduction, reviewed at every breaking release starting with roadmap item
`14.2.1`.

`docs/roadmap.md` item `10.1.2` and the zero-copy migration roadmap item
`1.1.2` are marked done. The inventory's ADR-status wording was reconciled.
No runtime public API migration was made in this work; that remains assigned
to phases `11`–`14`.

Validation gate results (2026-06-15) are recorded in `Surprises &
discoveries`. CodeRabbit review was waived by the user for this docs-only
milestone.

Lessons learnt:

- Scoping the milestone to ADR acceptance plus document alignment kept the
  diff reviewable and avoided pulling forward the helper-naming, benchmark,
  and migration-guide work that belongs to later roadmap items.
- The Logisphere review paid for itself: committing canonical helper names
  and a semver-aware removal trigger inside ADR 009 removes ambiguity that
  would otherwise have surfaced during the `12.1.x` implementation.

Feed-forward for later phases:

- Roadmap item `10.2.3` (migration-guide outline) must add a "for downstream
  library authors" section and a recommendation against blanket deprecation
  suppression, per the risks recorded here.
- Roadmap item `14.1.1` must substitute the `<release-version>` and
  `<target-removal-version>` placeholders in the helper `#[deprecated]`
  attributes at release-cut time.
- Roadmap item `14.1.x` owns the `cargo-semver-checks` and `cargo-public-api`
  CI wiring; until then the policy is enforced by review only.

## Context and orientation

`docs/roadmap.md` defines phase 10 as "Decision closure and baseline".
Roadmap item `10.1.2` asks the project to approve the compatibility and
rollout policy for downstream users, including which `Vec<u8>` helpers
survive the breaking release. The same decision appears as item `1.1.2` in
[`zero-copy-frame-and-payload-migration-roadmap.md`](../zero-copy-frame-and-payload-migration-roadmap.md).

The current state of the related design documents is:

- [`adr-008-zero-copy-public-byte-container.md`](../adr-008-zero-copy-public-byte-container.md):
  `Accepted` on 2026-05-26. Names `bytes::Bytes` (or a transparent project
  wrapper) as the stable public byte representation and adopts an explicit
  edit-on-demand workflow for mutation. Section "Follow-up Decisions" calls
  out roadmap item `10.1.2` and ADR 009 as the venue for deciding which
  `Vec<u8>` compatibility helpers survive the breaking release, whether any
  helpers are feature-gated, and when retained helpers are reviewed again.
- [`adr-009-vec-u8-migration-rollout.md`](../adr-009-vec-u8-migration-rollout.md):
  status `Proposed`. Proposes Option C ("staged breaking release with finite
  compatibility helpers"). Section "Outstanding Decisions" lists the three
  open questions this plan must resolve.
- [`adr-010-transport-frame-boundary-for-zero-copy.md`](../adr-010-transport-frame-boundary-for-zero-copy.md):
  status `Proposed`. Out of scope for `10.1.2`. Closure belongs to `10.1.3`.
- [`frame-vec-u8-inventory.md`](../frame-vec-u8-inventory.md): records the
  public payload surfaces still owned by `Vec<u8>` and resolves the direction
  for epic 284. Section "Generalization paths and conceptual risks" and
  "Coordination notes" already point at ADR 009 as the rollout venue.

The current public surfaces whose `Vec<u8>` contracts the rollout policy must
govern are listed in `frame-vec-u8-inventory.md` and are confirmed by
`leta show` and `leta refs` at draft time:

- `src/app/envelope.rs`: `PacketParts::new(..., payload: Vec<u8>)`,
  `PacketParts::into_payload(self) -> Vec<u8>`,
  `Envelope::new(id, correlation_id, payload: Vec<u8>)`, and the
  `Packet::into_parts` / `Packet::from_parts` contract.
- `src/middleware.rs`: `ServiceRequest::new(frame: Vec<u8>, correlation_id)`,
  `ServiceRequest::frame_mut(&mut self) -> &mut Vec<u8>`,
  `ServiceResponse::new(frame: Vec<u8>, ...)`,
  `ServiceResponse::frame_mut`, and `ServiceResponse::into_inner(self) -> Vec<u8>`.
- `src/client/hooks.rs`: `BeforeSendHook = Arc<dyn Fn(&mut Vec<u8>) + Send + Sync>`,
  and `src/client/builder/request_hooks.rs::before_send`.
- `src/client/messaging.rs`: `invoke_before_send_hooks(&self, bytes: &mut Vec<u8>)`.
- `src/serializer.rs`:
  `Serializer::serialize<M>(&self, value: &M) -> Result<Vec<u8>, ...>` and
  the `BincodeSerializer` impl.
- `src/client/mod.rs::ClientPreambleSuccessHandler<T>`,
  `src/client/preamble_exchange.rs::perform_preamble_exchange` and
  `run_preamble_exchange`, and `src/rewind_stream.rs::RewindStream::new`.
- `src/correlation.rs::CorrelatableFrame for Vec<u8>` and the test-only
  `Packet for Vec<u8>` in `src/connection/test_support.rs`. ADR 010 will
  govern these specifically; the rollout policy only needs to state which of
  these bridges remain as compatibility surfaces during the breaking release.

This plan does not change any of these symbols. It only decides which of
their `Vec<u8>` shapes are retained as deprecated helpers when the public API
migration in phases `12`–`14` lands.

Skills to load while implementing this plan:

- `execplans` (already loaded by this plan).
- `leta` for symbol inspection. Add the worktree as a workspace with
  `leta workspace add <path>` if it is not yet present.
- `rust-router` to choose the relevant Rust skills. The expected targets are
  `rust-types-and-apis`, `arch-decision-records`, `arch-supply-chain` (for
  semver tooling references), `arch-crate-design`, and `rust-unused-code`
  (for `#[cfg(...)]` and deprecation discipline).
- `en-gb-oxendict` for prose review.
- `logisphere-design-review` for the pre-implementation review.
- `coderabbit review --agent` (CLI, not a skill) before commit.
- `commit-message` and `pr-creation` for the final commit and the draft pull
  request.

Adjacent documentation to keep aligned with this milestone:

- [`docs/roadmap.md`](../roadmap.md)
- [`docs/zero-copy-frame-and-payload-migration-roadmap.md`](../zero-copy-frame-and-payload-migration-roadmap.md)
- [`docs/frame-vec-u8-inventory.md`](../frame-vec-u8-inventory.md)
- [`docs/adr-008-zero-copy-public-byte-container.md`](../adr-008-zero-copy-public-byte-container.md)
- [`docs/adr-009-vec-u8-migration-rollout.md`](../adr-009-vec-u8-migration-rollout.md)
- [`docs/adr-010-transport-frame-boundary-for-zero-copy.md`](../adr-010-transport-frame-boundary-for-zero-copy.md)
- [`docs/documentation-style-guide.md`](../documentation-style-guide.md)
- [`docs/users-guide.md`](../users-guide.md)
- [`docs/developers-guide.md`](../developers-guide.md)
- [`docs/the-road-to-wireframe-1-0-feature-set-philosophy-and-capability-maturity.md`](../the-road-to-wireframe-1-0-feature-set-philosophy-and-capability-maturity.md)
- [`docs/hardening-wireframe-a-guide-to-production-resilience.md`](../hardening-wireframe-a-guide-to-production-resilience.md)
- [`docs/generic-message-fragmentation-and-re-assembly-design.md`](../generic-message-fragmentation-and-re-assembly-design.md)
- [`docs/multi-packet-and-streaming-responses-design.md`](../multi-packet-and-streaming-responses-design.md)
- [`docs/rust-testing-with-rstest-fixtures.md`](../rust-testing-with-rstest-fixtures.md)
- [`reliable-testing-in-rust-via-dependency-injection.md`](../../reliable-testing-in-rust-via-dependency-injection.md)
- [`docs/rstest-bdd-users-guide.md`](../rstest-bdd-users-guide.md)
- [`docs/rust-doctest-dry-guide.md`](../rust-doctest-dry-guide.md)

Later runtime work for phases `12`–`14` must validate any retained helper
with `rstest` and `rstest-bdd` (covering happy and unhappy paths plus a
deprecation-warning regression), and add `proptest` coverage where invariants
span ranges of payload values or hook orderings. Kani and Verus are not
required for this docs-only decision-closure item, but later implementation
must revisit that judgement if a retained helper introduces a new invariant
over state transitions or a proof-worthy business contract.

## Plan of work

### Stage A: confirm the decision boundary

Read `docs/roadmap.md` (phase 10), `docs/frame-vec-u8-inventory.md` (sections
"Generalization paths and conceptual risks", "Resolved direction for epic
284", and "Coordination notes"),
`docs/zero-copy-frame-and-payload-migration-roadmap.md` (section
"1.1.2 Approve the compatibility and rollout policy"), and ADR 009 in full.
Confirm that `10.1.2` only approves the compatibility and rollout policy.

Go/no-go point: if the policy decision cannot be separated from the
actor/codec-driver boundary (`10.1.3`), the benchmark thresholds (`10.2.1`,
`10.2.2`), or the migration-guide outline (`10.2.3`), stop and ask whether
the user wants `10.1.2` folded into a wider milestone.

### Stage B: inspect current `Vec<u8>` surfaces

Add the worktree as a `leta` workspace if it is not yet present:

```sh
leta workspace add "$PWD"
```

Confirm the current shapes of the symbols the policy will govern. The purpose
is to make sure ADR 009 names every public surface that the helper set must
either retain or remove, not to change any code.

```sh
leta show PacketParts
leta show Envelope
leta show ServiceRequest
leta show ServiceResponse
leta show BeforeSendHook
leta show Serializer
leta show RewindStream
leta refs PacketParts
leta refs BeforeSendHook
leta refs invoke_before_send_hooks
```

If `leta show` returns ambiguous symbol errors, qualify with the file path
(for example, `leta show src/serializer.rs:Serializer`). If the LSP
connection is lost, fall back to direct `Read` of the file but record the
failure in `Surprises & discoveries`.

Expected result: the implementer can list every `Vec<u8>`-typed signature
that the accepted policy must either retain as a deprecated helper or
explicitly remove.

### Stage C: re-run the community-of-experts review if the plan is revised after approval

The `logisphere-design-review` skill was already run against this draft on
2026-06-04, and its findings are recorded in `Surprises & discoveries`. If
the user requests material revisions before approval, re-run the crew
against the updated draft and update `Decision log` for any new trade-offs.

Stop and escalate if a re-run surfaces a structural objection (for example,
that Option C is no longer the right rollout pattern). Do not edit ADR 009
to a position the review has cast serious doubt on without user direction.

### Stage D: accept ADR 009

Edit `docs/adr-009-vec-u8-migration-rollout.md` so that:

1. The `Status` field becomes `Accepted` and records the acceptance date.
2. The "Outstanding Decisions" section is resolved into accepted answers
   that match this plan's `Decision log`:
   - **Helper set and dispositions.** Single conversion surface on the
     stable byte wrapper: `PayloadBytes::from_vec` and
     `PayloadBytes::into_vec`. Per-call-site `PacketParts::from_vec` and
     `Envelope::from_vec` constructors are explicitly *not* added; transport
     hand-offs route through the wrapper. `Serializer::serialize` returns
     the stable byte wrapper; if a `serialize_to_vec` shim proves necessary
     it ships as a thin deprecated wrapper over `into_vec`. Middleware
     editing (`ServiceRequest::frame_mut`, `ServiceResponse::frame_mut`,
     `ServiceResponse::into_inner`) is **removed outright** and replaced by
     the ADR 008 edit-on-demand workflow; no `&mut Vec<u8>` accessor is
     retained, because retaining one would resurrect the bottleneck ADR 008
     eliminated. `BeforeSendHook` gains one adapter constructor
     (`BeforeSendHook::from_vec_fn` or the free-function
     `before_send_from_vec_fn` on `RequestHooks`) so existing
     `Fn(&mut Vec<u8>)` hooks survive one release cycle. Client preamble
     leftovers stay on owned `Vec<u8>` for the breaking release per the
     inventory's resolved direction; roadmap item `12.2.2` is the
     re-evaluation hook. The runtime `CorrelatableFrame for Vec<u8>` bridge
     and the test-only `Packet for Vec<u8>` bridge are explicitly *not*
     governed by this policy; ADR 010 and roadmap items `11.2.1`, `11.2.2`,
     and `14.1.3` own their lifecycle.
   - **Visibility.** Helpers ship in the default build, not behind a feature
     flag. Every helper carries
     `#[deprecated(since = "<release-version>", note = "<recommended path>; \
     scheduled for removal in <target-removal-version>")]`. ADR 009 commits
     to the substitution rule for `<release-version>` (filled in at
     release-cut time by roadmap item `14.1.1`) rather than to a literal
     version string. `<target-removal-version>` must be filled in at the
     same step.
   - **Removal signal.** Helpers are removed in the *second breaking
     release* after introduction (`0.N → 0.N+2` under 0.x semver;
     `1.x → 2.x` once Wireframe has cut 1.0). The review trigger is
     event-based, not calendar-based: review retained helpers at every
     breaking release after the introduction, starting with roadmap item
     `14.2.1`, until removal lands.
3. The `Decision Drivers`, `Goals`, and `Non-Goals` sections are checked
   against the resolved answers and updated only where they would otherwise
   contradict the acceptance.
4. The `Known Risks and Limitations` section is updated to reflect the
   chosen visibility and removal signals, and the named risks from this
   plan's `Risks` section are folded in where they belong in the ADR. In
   particular, the "downstream re-export propagation" and "deprecation
   warning suppression" risks are added; the existing risk about
   compatibility helpers becoming permanent in practice is rephrased
   against the new event-based removal trigger.
5. The `Architectural Rationale` section is updated to call out
   `cargo-semver-checks` and `cargo-public-api` as the suggested release
   guardrails. It must explicitly defer CI wiring to roadmap item `14.1.x`
   and state that, until that wiring lands, the policy is enforced by
   review only. It must also cite the `#[deprecated]` attribute as the
   canonical compatibility-window mechanism in Rust.
6. The ADR explicitly defers the migration guide before-and-after examples
   to roadmap item `10.2.3` and the changelog phrasing to roadmap item
   `14.1.2`.
7. Any inline `Vec<u8>` recommendation that conflicts with ADR 008's
   accepted edit-on-demand workflow is reconciled. In particular,
   `PayloadBytes::into_vec`'s deprecation `note` must steer users to a
   zero-copy path (`PayloadBytes::slice`, `Bytes::freeze`, or the
   edit-on-demand editor) rather than only to `PayloadEditor`, because
   `into_vec` itself allocates and copies and the recommended replacement
   should preserve the zero-copy intent.

### Stage E: align roadmap and inventory documents

Update `docs/roadmap.md` to tick item `10.1.2` only after ADR 009 acceptance
and gate runs are complete. Do not tick `10.1.3`, `10.2.x`, or any item in
phases `11`–`14`.

Update `docs/zero-copy-frame-and-payload-migration-roadmap.md` item `1.1.2`
and its child bullets to reflect the resolved policy. Do not edit other
items in that file.

Update `docs/frame-vec-u8-inventory.md` only if its "Generalization paths and
conceptual risks", "Resolved direction for epic 284", or "Coordination notes"
sections would otherwise contradict the accepted ADR. Keep the inventory
factual; do not turn it into a second ADR.

If the user-facing or developer-facing guides need a pointer that downstream
users should expect a finite compatibility window in the breaking release,
add a one-paragraph note. Otherwise, defer those edits to roadmap item
`10.2.3` (migration-guide outline) and record the deferral in
`Decision log`.

### Stage F: validate

Run the gates sequentially with `tee` logs under `/tmp`, using the user's
preferred filename template
`/tmp/$ACTION-wireframe-$(git branch --show-current).out`. The expected
formatter behaviour is documented in the `10.1.1` plan: `make fmt` may fail
on pre-existing repository-wide Markdown line-length findings outside this
task; if so, run `make check-fmt` and `make markdownlint` instead and record
the formatter baseline in `Surprises & discoveries`.

```sh
set -o pipefail
make check-fmt   2>&1 | tee /tmp/check-fmt-wireframe-$(git branch --show-current).out
make markdownlint 2>&1 | tee /tmp/markdownlint-wireframe-$(git branch --show-current).out
make nixie       2>&1 | tee /tmp/nixie-wireframe-$(git branch --show-current).out
make lint        2>&1 | tee /tmp/lint-wireframe-$(git branch --show-current).out
make test        2>&1 | tee /tmp/test-wireframe-$(git branch --show-current).out
```

For a docs-only decision-closure milestone, `make lint` and `make test` must
still pass. Recording their successful runs guards against accidental
runtime regressions sneaking in alongside the documentation edits.

### Stage G: external review and commit

Run `coderabbit review --agent` after the deterministic gates pass and
before the commit. Address every concern or record why it is out of scope.

Use the `commit-message` skill for the final commit message. Stage the
intended files individually rather than `git add -A`. The expected staged
set is:

- `docs/adr-009-vec-u8-migration-rollout.md`
- `docs/roadmap.md`
- `docs/zero-copy-frame-and-payload-migration-roadmap.md`
- `docs/frame-vec-u8-inventory.md` (only if a "Coordination notes" or
  "Resolved direction for epic 284" reconciliation is required)
- `docs/execplans/10-1-2-approve-vec-u8-compatibility-and-rollout-policy-for-downstream-users.md`

Push the renamed branch and open the draft pull request through the
`pr-creation` skill. The pull request title must include `(10.1.2)` and the
description must mention this execplan, link adjacent ADRs and roadmap
items, and end with a `## References` section that contains the lody
session link emitted by:

```sh
echo "https://lody.ai/leynos/sessions/${LODY_SESSION_ID}"
```

## Concrete steps

From the repository root:

1. Confirm branch and status:

   ```sh
   git branch --show-current
   git status --short --branch
   ```

   Expected branch (after the rename in step 12):

   ```plaintext
   10-1-2-approve-vec-u8-compatibility-and-rollout-policy-for-downstream-users
   ```

2. Register the worktree as a `leta` workspace if not already present:

   ```sh
   leta workspace add "$PWD"
   ```

3. Re-read the bounded scope:

   ```sh
   sed -n '444,473p' docs/roadmap.md
   sed -n '1,170p' docs/adr-009-vec-u8-migration-rollout.md
   sed -n '47,98p' docs/zero-copy-frame-and-payload-migration-roadmap.md
   sed -n '340,407p' docs/frame-vec-u8-inventory.md
   ```

   The line ranges are illustrative anchors at draft time; if a file has
   been edited since this plan was written, re-locate the relevant sections
   by heading.

4. Confirm the current public byte surfaces with `leta`:

   ```sh
   leta show PacketParts
   leta show Envelope
   leta show ServiceRequest
   leta show ServiceResponse
   leta show BeforeSendHook
   leta show Serializer
   leta show RewindStream
   leta refs invoke_before_send_hooks
   ```

5. Run the `logisphere-design-review` skill against this plan and the
   "Outstanding Decisions" section of ADR 009. Record the crew's feedback in
   `Surprises & discoveries` and update `Decision log` for any new
   trade-offs.

6. Edit `docs/adr-009-vec-u8-migration-rollout.md` as described in Stage D.

7. Edit `docs/roadmap.md` and
   `docs/zero-copy-frame-and-payload-migration-roadmap.md` to reflect the
   acceptance. Tick only `10.1.2` (or `1.1.2` in the migration roadmap).

8. Decide whether `docs/frame-vec-u8-inventory.md` needs a reconciling edit;
   skip if not.

9. Run validation gates as described in Stage F:

   ```sh
   set -o pipefail
   make check-fmt   2>&1 | tee /tmp/check-fmt-wireframe-$(git branch --show-current).out
   make markdownlint 2>&1 | tee /tmp/markdownlint-wireframe-$(git branch --show-current).out
   make nixie       2>&1 | tee /tmp/nixie-wireframe-$(git branch --show-current).out
   make lint        2>&1 | tee /tmp/lint-wireframe-$(git branch --show-current).out
   make test        2>&1 | tee /tmp/test-wireframe-$(git branch --show-current).out
   ```

10. Run `coderabbit review --agent`. Re-run after each remediation. If the
    service reports a recoverable rate limit, retry once and record the
    blocker in `Surprises & discoveries` rather than skipping the gate
    silently.

11. Commit with the `commit-message` skill (file-based commit message). Do
    not use `git commit -m`.

12. Rename the branch only once, using GitHub's branch rename flow after the
    draft pull request exists, so the pull request follows the rename. Do
    not rename locally and push directly. The target name is
    `10-1-2-approve-vec-u8-compatibility-and-rollout-policy-for-downstream-users`.

13. Push the branch (the first push must create the remote branch and set
    upstream tracking) and open the draft pull request:

    ```sh
    git push -u origin 10-1-2-approve-vec-u8-compatibility-and-rollout-policy-for-downstream-users
    echo "${LODY_SESSION_ID}"
    ```

    Use the `pr-creation` skill to draft the pull request body. The title
    must include `(10.1.2)`.

## Validation and acceptance

Acceptance for the ExecPlan draft (this commit):

- This file exists at
  `docs/execplans/10-1-2-approve-vec-u8-compatibility-and-rollout-policy-for-downstream-users.md`.
- The plan is self-contained, names the approval gate, names the affected
  public API surfaces, and defers implementation until explicit approval.
- The plan signposts the relevant documents and skills.
- The plan includes a Logisphere community-of-experts review step before
  ADR 009 edits.
- The relevant deterministic gates pass on the docs-only diff:
  - `make check-fmt`,
  - `make markdownlint`,
  - `make nixie`,
  - `make lint`,
  - `make test`.
- `coderabbit review --agent` has no unresolved concerns, or any remaining
  concern is documented as out of scope with a concrete rationale.
- The branch is pushed to the renamed remote.
- A draft pull request exists, links this ExecPlan, mentions it in the
  summary, includes `(10.1.2)` in the title, and ends with a `## References`
  section that links the lody session.

Acceptance for later implementation of `10.1.2` (a separate, post-approval
commit):

- ADR 009 status is `Accepted`, dated, and resolves all three previously
  open "Outstanding Decisions".
- ADR 009 names each affected surface that the policy governs:
  `PacketParts`, `Envelope`, `ServiceRequest`, `ServiceResponse`,
  `BeforeSendHook`, `Serializer::serialize`, and the client preamble
  leftover path.
- ADR 009 records the visibility decision (default build, not feature
  gated), the deprecation discipline (`#[deprecated(since, note)]`), and the
  removal signal (next major release; six-month review hook in `14.2.1`).
- ADR 009 explicitly defers migration-guide examples to `10.2.3` and the
  changelog phrasing to `14.1.2`.
- `docs/roadmap.md` ticks only `10.1.2` in phase 10.
- `docs/zero-copy-frame-and-payload-migration-roadmap.md` ticks only the
  corresponding `1.1.2` decision item.
- Any aligned document is updated to remove contradictions with the accepted
  ADR; otherwise the inventory remains unchanged.
- No Rust runtime API migration begins under this milestone.
- All Makefile gates and CodeRabbit review pass on the final commit.

Quality criteria summary:

- Tests: `make test` passes with no new failures attributable to this plan.
- Lint: `make lint` passes; clippy and whitaker remain clean.
- Format: `make check-fmt` passes; `make markdownlint` passes for the
  changed Markdown.
- Diagrams: `make nixie` passes if any Mermaid is edited; otherwise it is
  still run as a regression check.
- Performance: no benchmarks are introduced or required by this plan.
- Security: no security-sensitive code is touched; the policy itself does
  not affect transport correctness.

## Idempotence and recovery

The planning and documentation steps are safe to repeat. If Markdown
formatting changes wrap prose differently between runs, review the diff and
keep only changes related to this plan or ADR 009 alignment.

If a quality gate fails, fix the smallest relevant issue and rerun only the
failed gate before continuing to the next. Three consecutive failures of the
same gate triggers the iteration tolerance: stop, document, and escalate.

If the branch push fails because the remote branch already exists, fetch
the remote branch and compare histories before any force operation. Do not
overwrite remote work without explicit approval.

If `coderabbit review --agent` is unavailable in the local environment,
record the command failure in `Surprises & discoveries`, proceed only with
the local quality gates, and note the missing review in the pull request.

The branch rename is one-way once the pull request exists. Use GitHub's
rename flow so the pull request follows the rename; do not rename locally
and force-push.

## Artifacts and notes

External prior-art used while drafting this plan. Each entry is a bounded
citation, not a model to copy in full.

- `cargo-semver-checks` partially detects Rust semver violations using
  rustdoc JSON. The action runs "right before `cargo publish`" and compares
  the candidate against the latest non-prerelease version on crates.io, so
  intentional major bumps produce reviewable expected failures rather than
  silent regressions. Source:
  <https://github.com/obi1kenobi/cargo-semver-checks-action>.
- `cargo-public-api` inventories and diffs the public API surface of a
  Rust library between releases or commits. Useful as a release guardrail
  to confirm that the intended breaking surface, and only that surface,
  changes. Source: <https://crates.io/crates/cargo-public-api>.
- Rust's `#[deprecated(since = "...", note = "...")]` attribute is the
  canonical mechanism for a finite compatibility window. The Rust Reference
  documents that `rustdoc` renders both the version and the note on the
  item. Source:
  <https://doc.rust-lang.org/reference/attributes/diagnostics.html>.
- `bytes::Bytes` is documented as "a cheaply cloneable and sliceable chunk
  of contiguous memory", with reference-counted shared storage. Source:
  <https://docs.rs/bytes/latest/bytes/struct.Bytes.html>.
- `bytes::BytesMut` is a "unique view into a potentially shared memory
  region" with in-place mutation; `freeze()` converts it into a shareable
  `Bytes` without copying. Source:
  <https://docs.rs/bytes/latest/bytes/struct.BytesMut.html>.
- Hyper 1.0's upgrade guide is one precedent for "staged breaking release
  with finite compatibility helpers": users first enable
  `features = ["backports", "deprecated"]` on hyper 0.14, where `backports`
  ports several 1.0 types into 0.14 and `deprecated` emits warnings on any
  0.14 item with a direct backport. Helpers that did not fit 1.0's
  stability bar moved into `hyper-util` rather than staying in core, so the
  effective sunset criterion was "1.0 ships only what we will commit to
  under SemVer". Source: <https://hyper.rs/guides/1/upgrading/>.
- Common Changelog mandates that breaking changes are prefixed
  `**Breaking:**` and listed before other changes per category;
  subsystem-scoped breaks use `**<subsystem> (breaking):**`. Common
  Changelog also recommends an `UPGRADING.md` alongside the release entry
  for semver-major releases. Source: <https://common-changelog.org/>.

Repository documents to keep aligned with this plan (listed under "Context
and orientation").

## Interfaces and dependencies

This plan does not introduce a new Rust interface, but it does specify the
shape and canonical names of the compatibility helpers that the later
runtime work in roadmap items `12.1.x` and `12.2.x` must implement.
ADR 009 commits to *both* the helper set and the canonical names. The
version strings shown as `<release-version>` and `<target-removal-version>`
are filled in at release-cut time by roadmap item `14.1.1`; ADR 009 commits
to the substitution rule, not the literal values.

```rust
// Illustrative shape only. Internal representation and exact signatures
// are decided during roadmap items `12.1.1`, `12.1.2`, and `12.2.1`.

pub struct PayloadBytes(bytes::Bytes);

impl PayloadBytes {
    #[deprecated(
        since = "<release-version>",
        note = "construct from `&[u8]` or the edit-on-demand editor; \
                scheduled for removal in <target-removal-version>",
    )]
    pub fn from_vec(value: Vec<u8>) -> Self {
        Self(bytes::Bytes::from(value))
    }

    #[deprecated(
        since = "<release-version>",
        note = "use `PayloadBytes::as_slice`, `bytes::Bytes::freeze`, or \
                the edit-on-demand editor to stay zero-copy; \
                scheduled for removal in <target-removal-version>",
    )]
    pub fn into_vec(self) -> Vec<u8> {
        self.0.to_vec()
    }
}

// Adapter constructor on the new edit-on-demand `BeforeSendHook` surface
// to satisfy existing `Fn(&mut Vec<u8>)` hooks during the migration
// window. The exact bound on `RequestHooks::before_send_from_vec_fn` is
// finalised by roadmap item `12.2.1`.
//
// #[deprecated(
//     since = "<release-version>",
//     note = "rewrite the hook against `BeforeSendEditor`; \
//             scheduled for removal in <target-removal-version>",
// )]
// pub fn before_send_from_vec_fn<F>(f: F) -> Arc<dyn BeforeSendHookTrait>
// where
//     F: Fn(&mut Vec<u8>) + Send + Sync + 'static,
// { /* … */ }
```

`ServiceRequest::frame_mut`, `ServiceResponse::frame_mut`, and
`ServiceResponse::into_inner` are *not* in the helper set. They are
removed outright in the breaking release and replaced by the ADR 008
edit-on-demand workflow. Retaining a `&mut Vec<u8>` accessor would
resurrect the bottleneck ADR 008 eliminated.

`CorrelatableFrame for Vec<u8>` (runtime) and `Packet for Vec<u8>`
(test support) are *not* governed by this policy. ADR 010 and roadmap
items `11.2.1`, `11.2.2`, and `14.1.3` own their lifecycle.

These sketches are reference shapes for ADR 009 only. Do not implement them
in Rust under `10.1.2`.

Dependencies summary:

- Existing `bytes` crate. No new crate dependency is required.
- `#[deprecated]` is part of stable Rust and needs no extra tooling.
- `cargo-semver-checks` and `cargo-public-api` are suggested release
  guardrails that ADR 009 should cite as deferred CI work for roadmap items
  `14.1.x`.

## Revision note

Initial draft created on 2026-06-04. It turns the user request and roadmap
item `10.1.2` into a reviewable, approval-gated ExecPlan. The plan resolves
nothing in ADR 009 until the user explicitly approves the proposed answers
in `Decision log`. No runtime implementation is authorised by this draft.
