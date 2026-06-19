# Approve the stable public byte-container model

This ExecPlan is a living document. The sections `Constraints`, `Tolerances`,
`Risks`, `Progress`, `Surprises & discoveries`, `Decision log`, and
`Outcomes & retrospective` must be kept up to date as work proceeds.

Status: COMPLETE

Implementation approval: RECEIVED. The user approved implementation on
2026-05-26.

## Purpose / big picture

Roadmap item `10.1.1` closes the design question that blocks the zero-copy
payload migration: what public byte container should Wireframe expose for
packets, envelopes, middleware, client hooks, and serializer output?

After this work is approved and implemented, maintainers can see success by
reading `docs/adr-008-zero-copy-public-byte-container.md` and finding an
accepted, concrete decision for the stable read-only byte representation and
the edit-on-demand mutation model. The decision must make the later
implementation work predictable: `PacketParts`, `Envelope`, `ServiceRequest`,
`ServiceResponse`, `BeforeSendHook`, and `Serializer::serialize` each have a
named target shape or a named follow-up decision point.

This plan covers decision closure only. It does not migrate the runtime public
API, remove compatibility helpers, set benchmark thresholds, or rework the
actor and codec-driver boundary. Those are separate roadmap items.

## Constraints

- Use the `execplans` skill for this plan and keep this document
  self-contained.
- Use the `leta` skill for code navigation when checking current Rust symbols
  and call sites.
- Use the `rust-router`, `rust-types-and-apis`, `rust-memory-and-state`, and
  `rust-performance-and-layout` skills when refining the public API decision,
  because the decision affects ownership, mutability, trait signatures, and
  allocation pressure.
- Use the `firecrawl-mcp` skill only for bounded external prior-art checks.
  External facts may inform the decision, but repository documents remain the
  source of truth for Wireframe requirements.
- Preserve en-GB Oxford spelling and the documentation style guide in all
  edited Markdown.
- Keep the scope to roadmap item `10.1.1`. Defer rollout policy to `10.1.2`,
  actor and codec-driver boundary decisions to `10.1.3`, and performance
  baseline thresholds to `10.2.1` and `10.2.2`.
- Do not claim runtime behaviour has changed as part of `10.1.1`. If later
  implementation edits Rust code, this plan must be revised and re-approved
  before those edits begin.
- Do not mark roadmap item `10.1.1` done until ADR 008 is accepted, aligned
  documents are updated, and all required quality gates pass.
- Use Makefile targets for validation. Run formatting, linting, and tests
  sequentially with `tee` logs under `/tmp`.
- Use `coderabbit review --agent` after the decision-draft milestone and clear
  all concerns before committing or opening the pull request.

## Tolerances

- Scope: if implementation of this decision-closure task requires Rust source
  changes, stop and ask whether `10.1.1` should be expanded beyond an
  approval/documentation item.
- Scope: if documentation changes exceed six files or 500 net lines, stop and
  split the work into a decision PR and follow-up documentation PR.
- Interface: if the decision cannot name a stable byte model for any of
  `PacketParts`, `Envelope`, `ServiceRequest`, `ServiceResponse`,
  `BeforeSendHook`, or `Serializer::serialize`, stop and present the remaining
  options with trade-offs.
- Dependencies: if the target model requires a new external crate beyond the
  existing `bytes` crate, stop and request approval before accepting that
  dependency.
- Compatibility: if this work needs to decide the lifetime, feature-gating, or
  removal criteria for `Vec<u8>` compatibility helpers, record the dependency
  and defer that decision to `10.1.2`.
- Performance: if the decision relies on an unmeasured performance claim, keep
  it as a hypothesis and defer thresholds to `10.2.1` and `10.2.2`.
- Validation: if the same quality gate fails after three fix attempts, stop,
  document the failure in `Decision log`, and ask for direction.

## Risks

- Risk: the edit-on-demand surface remains too vague to implement.
  Severity: high. Likelihood: medium. Mitigation: ADR 008 must accept a
  concrete direction, such as stable `bytes::Bytes` storage plus a
  project-defined editor or helper that exposes mutation and documents when a
  copy can occur.

- Risk: the decision accidentally creates two equally primary public byte APIs.
  Severity: high. Likelihood: medium. Mitigation: keep `Bytes`-compatible
  storage as the canonical path and record `Vec<u8>` helpers as compatibility
  surfaces controlled by `10.1.2`.

- Risk: the decision leaks buffer taxonomy into ordinary middleware and hook
  authors' workflows. Severity: medium. Likelihood: medium. Mitigation: name
  the high-level editing workflow in ADR 008 and require follow-up user-guide
  examples when runtime APIs are migrated.

- Risk: external prior art is over-applied. Severity: medium. Likelihood: low.
  Mitigation: cite `bytes::Bytes` and `bytes::BytesMut` semantics only where
  they directly support Wireframe's stated goals, and keep repository ADRs as
  the authority.

- Risk: `10.1.1` absorbs benchmark, rollout, or actor-boundary work. Severity:
  medium. Likelihood: medium. Mitigation: write explicit deferrals in ADR 008,
  the roadmap note, and this plan's acceptance criteria.

## Progress

- [x] (2026-05-20 23:19Z) Created context pack `pk_44hboqal` for the Wyvern
  planning team with roadmap, ADR, inventory, and migration-roadmap excerpts.
- [x] (2026-05-20 23:20Z) Renamed the local branch to
  `10-1-1-approve-stable-public-byte-container`.
- [x] (2026-05-20 23:20Z) Reviewed `docs/roadmap.md`,
  `docs/adr-008-zero-copy-public-byte-container.md`,
  `docs/frame-vec-u8-inventory.md`, and
  `docs/zero-copy-frame-and-payload-migration-roadmap.md`.
- [x] (2026-05-20 23:20Z) Checked current external `bytes::Bytes` and
  `bytes::BytesMut` documentation with Firecrawl for zero-copy and mutation
  semantics.
- [x] (2026-05-20 23:20Z) Received Wyvern memo on decision scope and
  documentation updates.
- [x] (2026-05-20 23:21Z) Received Wyvern memo on code-facing surfaces and
  future validation scope.
- [x] (2026-05-20 23:22Z) Received Wyvern memo on prior art and validation
  script patterns.
- [x] (2026-05-20 23:26Z) Drafted this ExecPlan.
- [x] (2026-05-26 18:55Z) Ran `coderabbit review --agent` after deterministic
  gates; CodeRabbit completed with zero findings.
- [x] (2026-05-20 23:31Z) Ran Markdown and repository quality gates.
  `make fmt` currently fails on pre-existing repository-wide Markdown
  line-length findings; the new ExecPlan passes direct `markdownlint`, and the
  configured `make markdownlint` gate passes.
- [x] (2026-05-20 23:33Z) Committed the approved-for-review ExecPlan draft.
- [x] (2026-05-20 23:35Z) Pushed the branch and opened draft pull request
  #529 for plan review.
- [x] (2026-05-26 18:24Z) Began approved implementation of `10.1.1`.
- [x] (2026-05-26 18:25Z) Reconfirmed the current public byte surfaces with
  `leta` and source inspection: `PacketParts`, `ServiceRequest`,
  `BeforeSendHook`, and `Serializer::serialize` still expose `Vec<u8>`.
- [x] (2026-05-26 18:30Z) Accepted ADR 008 with the stable `bytes::Bytes` or
  transparent wrapper decision and explicit edit-on-demand workflow.
- [x] (2026-05-26 18:30Z) Marked only the corresponding `10.1.1` roadmap item
  and zero-copy migration roadmap `1.1.1` item done.
- [x] (2026-05-26 18:39Z) Ran deterministic validation gates:
  `make check-fmt`, `make markdownlint`, `make nixie`, `make lint`, and
  `make test`.
- [x] (2026-05-26 18:55Z) Updated this ExecPlan with final implementation and
  review outcomes.

## Surprises & discoveries

- Observation: the current branch was `feat/byte-container-plan` and tracked
  `origin/main`, not the requested roadmap branch. Evidence:
  `git branch --show-current` and `git status --short --branch`. Impact: the
  branch was renamed before plan work continued.

- Observation: the requested remote branch name did not exist before local
  rename. Evidence:
  `git ls-remote --heads origin 10-1-1-approve-stable-public-byte-container`
  returned no refs. Impact: the first push must create the remote branch and
  set upstream tracking.

- Observation: ADR 008 currently says `Proposed` and leaves three outstanding
  decisions open. Evidence: `docs/adr-008-zero-copy-public-byte-container.md`.
  Impact: implementing this plan must resolve those outstanding decisions
  before marking `10.1.1` done.

- Observation: `bytes::Bytes` is documented as cheaply cloneable, sliceable,
  and intended for zero-copy network programming, while `bytes::BytesMut`
  provides unique mutable access and can freeze into shareable `Bytes`.
  Evidence: Firecrawl scrapes of docs.rs pages for `Bytes` and `BytesMut`.
  Impact: those semantics support ADR 008's Option B, but Wireframe still needs
  a project-level editing API, so callers do not manage buffer taxonomy
  manually.

- Observation: the public Netsuke repository exposes small validation scripts
  such as `assert-file-absent.sh`, `assert-file-exists.sh`,
  `check-kani-version.sh`, and `install-kani.sh`. Evidence: Firecrawl scrape of
  <https://github.com/leynos/netsuke/tree/main/scripts>. Impact: future
  Wireframe validation helpers should follow the same pattern: narrow scripts
  with clear preconditions, deterministic failures, and version-gated checks
  where tooling versions matter.

- Observation: `coderabbit review --agent` failed before producing review
  findings because the service reported recoverable rate limits and usage
  exhaustion. Evidence:
  `/tmp/coderabbit-wireframe-10-1-1-approve-stable-public-byte-container.out`,
  `/tmp/coderabbit-retry-wireframe-10-1-1-approve-stable-public-byte-container.out`,
  and
  `/tmp/coderabbit-third-wireframe-10-1-1-approve-stable-public-byte-container.out`.
  Impact: no CodeRabbit findings exist to clear; retry later and report the
  external service blocker if the limit persists.

- Observation: `make fmt` runs `mdformat-all`, which invokes repository-wide
  Markdown lint fixing and currently fails on many pre-existing MD013
  line-length findings outside this task. Evidence:
  `/tmp/fmt-wireframe-10-1-1-approve-stable-public-byte-container.out`. Impact:
  unrelated formatter changes were discarded, the new ExecPlan and the
  configured `make markdownlint` gate pass, and the full formatter failure
  remains a repository baseline issue to report.

- Observation: `leta show Serializer` failed with an LSP connection error
  during the approved implementation pass. Evidence: the command returned
  `Error: Connection closed unexpectedly`. Impact: `src/serializer.rs` was read
  directly for the narrow serializer surface check, confirming
  `Serializer::serialize` still returns `Vec<u8>`.

- Observation: the approved implementation milestone passed the deterministic
  gates before CodeRabbit review. Evidence:
  `/tmp/check-fmt-wireframe-10-1-1-implementation.out`,
  `/tmp/markdownlint-wireframe-10-1-1-implementation.out`,
  `/tmp/nixie-wireframe-10-1-1-implementation.out`,
  `/tmp/lint-wireframe-10-1-1-implementation.out`, and
  `/tmp/test-wireframe-10-1-1-implementation.out`. Impact: the docs-only
  decision closure does not regress formatting, Markdown linting, diagram
  validation, Rust linting, or the workspace test suite.

- Observation: CodeRabbit completed review of the approved implementation diff
  with zero findings. Evidence:
  `/tmp/coderabbit-wireframe-10-1-1-implementation.out`. Impact: there are no
  CodeRabbit concerns to clear before committing this milestone.

## Decision log

- Decision: keep this ExecPlan in DRAFT status and block implementation until
  explicit approval. Rationale: the user explicitly stated that the plan must
  be approved before it is implemented, and the public API decision has semver
  impact. Date/Author: 2026-05-20 / Codex.

- Decision: advance this ExecPlan to IN PROGRESS after explicit user approval.
  Rationale: the user approved implementation of the planned functionality on
  2026-05-26, so documentation-only decision closure may proceed while runtime
  API migration remains out of scope. Date/Author: 2026-05-26 / Codex.

- Decision: bound `10.1.1` to ADR 008 acceptance and documentation alignment,
  not runtime migration. Rationale: roadmap items `11.1.1`, `12.1.1`, and
  `12.2.1` carry the actual public API migration work and depend on this
  decision. Date/Author: 2026-05-20 / Codex.

- Decision: use `bytes::Bytes` semantics as the external prior-art anchor for
  stable storage, with `BytesMut` semantics informing the edit-on-demand model.
  Rationale: the existing default codec already uses `Bytes`, and docs.rs
  documents cheap cloning, shared storage, and unique mutable views in the
  upstream crate. Date/Author: 2026-05-20 / Codex.

- Decision: accept ADR 008 with `bytes::Bytes`, or a transparent project
  wrapper over `bytes::Bytes`, as the stable public byte representation.
  Rationale: the existing default codec already uses `Bytes`, the inventory
  shows the remaining issue is public payload ownership rather than a
  frame-level `Vec<u8>` requirement, and this keeps read-only packet and
  routing paths zero-copy by default. Date/Author: 2026-05-26 / Codex.

- Decision: define mutation as an explicit edit-on-demand workflow rather than
  exposing raw `BytesMut` conversion as the user-facing model. Rationale:
  middleware and client hook authors need the current inspect, optionally edit,
  then forward workflow without manually managing `Vec<u8>`, `Bytes`, and
  `BytesMut` conversions. Date/Author: 2026-05-26 / Codex.

- Decision: defer exact compatibility-helper names, helper lifetime, and
  feature-gating to ADR 009 and roadmap item `10.1.2`. Rationale: `10.1.1`
  approves the stable byte and mutation model only; rollout policy is
  intentionally a separate decision. Date/Author: 2026-05-26 / Codex.

## Outcomes & retrospective

ADR 008 is accepted. The corresponding roadmap entries in `docs/roadmap.md` and
`docs/zero-copy-frame-and-payload-migration-roadmap.md` are marked done. No
runtime public API migration was made in this work; that remains assigned to
the later roadmap items referenced by ADR 008.

The implementation passed `make check-fmt`, `make markdownlint`, `make nixie`,
`make lint`, and `make test`. CodeRabbit completed after those gates with zero
findings.

## Context and orientation

`docs/roadmap.md` defines phase 10 as "Decision closure and baseline". Roadmap
item `10.1.1` asks the project to approve the stable public byte-container and
edit-on-demand model for `PacketParts`, `Envelope`, middleware, client hooks,
and serializer output. The same decision appears as item `1.1.1` in
`docs/zero-copy-frame-and-payload-migration-roadmap.md`.

`docs/frame-vec-u8-inventory.md` shows that Wireframe no longer hard-codes
`F::Frame = Vec<u8>` at the top-level codec or protocol abstraction. The main
remaining public problem is payload ownership: `PacketParts`, `Envelope`,
`ServiceRequest`, `ServiceResponse`, `BeforeSendHook`, and
`Serializer::serialize` still expose owned `Vec<u8>` bytes directly.

`docs/adr-008-zero-copy-public-byte-container.md` currently proposes Option B:
stable `Bytes`-compatible public storage plus explicit edit-on-demand mutation
helpers. The plan implementer must turn that proposed direction into an
accepted decision and resolve the open questions about the editing surface,
serializer sequencing, and migration constructors enough for later roadmap
items to proceed.

Important current code surfaces to inspect with `leta` before implementation:

- `src/app/envelope.rs`: `PacketParts`, `Envelope`, `Packet`,
  `PacketParts::new`, `PacketParts::payload_bytes`, `PacketParts::into_payload`,
  `Envelope::new`, and conversions between `PacketParts` and `Envelope`.
- `src/middleware.rs`: `ServiceRequest`, `ServiceResponse`,
  `FrameContainer`, `frame`, `frame_mut`, `into_inner`, and
  `HandlerService::call`.
- `src/client/hooks.rs`: `BeforeSendHook`, `AfterReceiveHook`, and
  `RequestHooks`.
- `src/client/builder/request_hooks.rs`: builder methods for request hooks.
- `src/client/messaging.rs` and `src/client/runtime.rs`: client send paths
  that currently convert serialized bytes into `bytes::Bytes`.
- `src/serializer.rs`: `Serializer::serialize` and `BincodeSerializer`.
- `src/app/codec_driver.rs`, `src/app/inbound_handler.rs`, and
  `src/app/frame_handling/response.rs`: server-side serialization and
  middleware boundary paths.

The later runtime migration must validate these surfaces with `rstest`,
`rstest-bdd`, and, where invariants span many payload values or hook orders,
`proptest`. Kani and Verus are not expected for this decision-closure item, but
a future implementation must revisit that judgement if it introduces a new
invariant over state transitions or a proof-worthy business contract.

## Plan of work

### Stage A: confirm the decision boundary

Read `docs/roadmap.md`, `docs/frame-vec-u8-inventory.md`,
`docs/zero-copy-frame-and-payload-migration-roadmap.md`, and ADRs 008 through
010. Confirm that `10.1.1` only approves the public byte-container and editing
model. Write any new findings into this plan before editing ADR 008.

Go/no-go point: if the decision cannot be separated from compatibility helper
lifetimes or actor/codec-driver ownership, stop and ask whether to fold in
`10.1.2` or `10.1.3`.

### Stage B: inspect current public surfaces

Use `leta show` and `leta refs` for the symbols listed in "Context and
orientation". The purpose is to ensure ADR 008 names every public surface that
will be affected later, not to change the code in this stage.

Expected result: the implementer can list the exact signatures or workflows
that will eventually move from owned `Vec<u8>` to stable shared bytes and
edit-on-demand mutation.

### Stage C: accept ADR 008

Edit `docs/adr-008-zero-copy-public-byte-container.md` so it becomes an
accepted ADR. The accepted decision must state:

- `bytes::Bytes` or a transparent project wrapper over `bytes::Bytes` is the
  stable public storage model for packet, envelope, serializer, middleware, and
  hook payload hand-offs.
- Mutation is explicit and edit-on-demand. Read-only inspection must not force
  a copy. Mutation may copy when the underlying storage is shared or otherwise
  cannot be mutated uniquely.
- Middleware and client hooks get a named editing workflow rather than raw
  caller-managed conversions between `Vec<u8>`, `Bytes`, and `BytesMut`.
- `Vec<u8>` remains a compatibility path only. The policy for which helpers
  survive the breaking release belongs to `10.1.2`.
- Serializer sequencing and exact constructor names may be refined during
  `11.1.1`, `12.1.1`, and `12.2.1`, but those items must not reopen the
  stable-container decision without a new ADR update.

Resolve or rewrite ADR 008's outstanding decisions so they point to concrete
follow-up roadmap items rather than remaining open-ended.

### Stage D: align roadmap and inventory documents

Update `docs/roadmap.md` to mark item `10.1.1` done only after ADR 008 is
accepted and gates pass. Do not mark `10.1.2`, `10.1.3`, or any `10.2.x` item
done.

Update `docs/zero-copy-frame-and-payload-migration-roadmap.md` item `1.1.1` and
its child bullets to show the same decision closure.

Update `docs/frame-vec-u8-inventory.md` only if its resolved-direction section
would otherwise conflict with the accepted ADR. Keep the inventory factual and
avoid turning it into a second ADR.

Update `docs/users-guide.md`, `docs/developers-guide.md`, or a component
architecture document only if the implementation changes consumer behaviour or
internal practice. For the expected documentation-only decision closure, record
that no runtime behaviour changed and defer user-facing examples to the later
API migration tasks.

### Stage E: validate and review

Run documentation formatting and validation first, then the repository gates.
Use `tee` logs under `/tmp` and run the gates sequentially:

```sh
set -o pipefail
make fmt 2>&1 | tee /tmp/fmt-wireframe-10-1-1-approve-stable-public-byte-container.out
make check-fmt 2>&1 | tee /tmp/check-fmt-wireframe-10-1-1-approve-stable-public-byte-container.out
make markdownlint 2>&1 | tee /tmp/markdownlint-wireframe-10-1-1-approve-stable-public-byte-container.out
make nixie 2>&1 | tee /tmp/nixie-wireframe-10-1-1-approve-stable-public-byte-container.out
make lint 2>&1 | tee /tmp/lint-wireframe-10-1-1-approve-stable-public-byte-container.out
make test 2>&1 | tee /tmp/test-wireframe-10-1-1-approve-stable-public-byte-container.out
```

Run `coderabbit review --agent` after the ADR/roadmap draft is ready and before
the commit. Address every concern or record why the concern is out of scope
before proceeding.

### Stage F: commit and publish for approval

Commit the plan and any decision-document changes only after all required gates
pass. Push the branch to `origin/10-1-1-approve-stable-public-byte-container`
and set upstream tracking. Open a draft pull request for the approved plan and
decision review.

The pull request title must include `(10.1.1)`. The pull request description
must link this ExecPlan and include a `## References` section with the Lody
session URL from:

```sh
echo "${LODY_SESSION_ID}"
```

## Concrete steps

From the repository root:

1. Confirm branch and status:

   ```sh
   git branch --show-current
   git status --short --branch
   ```

   Expected branch:

   ```plaintext
   10-1-1-approve-stable-public-byte-container
   ```

2. Confirm the current decision documents:

   ```sh
   sed -n '447,473p' docs/roadmap.md
   sed -n '1,220p' docs/adr-008-zero-copy-public-byte-container.md
   sed -n '32,232p' docs/frame-vec-u8-inventory.md
   sed -n '1,90p' docs/zero-copy-frame-and-payload-migration-roadmap.md
   ```

3. Use `leta` for symbol inspection before any runtime-facing design text is
   tightened:

   ```sh
   leta show PacketParts
   leta show ServiceRequest
   leta show BeforeSendHook
   leta show Serializer
   ```

4. Edit ADR 008 and aligned roadmap documents as described in "Plan of work".

5. Run `coderabbit review --agent` and resolve concerns.

6. Run validation gates sequentially with `tee` logs as described in Stage E.

7. Commit using a file-based commit message:

   ```sh
   COMMIT_MSG_DIR="$(mktemp -d)"
   cat > "$COMMIT_MSG_DIR/COMMIT_MSG.md" << 'ENDOFMSG'
   Approve public byte-container plan

   Add the execution plan for roadmap item `10.1.1`, which defines
   how ADR 008 should be accepted before the zero-copy public API
   migration begins.
   ENDOFMSG
   git commit -F "$COMMIT_MSG_DIR/COMMIT_MSG.md"
   rm -rf "$COMMIT_MSG_DIR"
   ```

8. Push and open the draft pull request after validation:

   ```sh
   git push -u origin 10-1-1-approve-stable-public-byte-container
   echo "${LODY_SESSION_ID}"
   ```

## Validation and acceptance

Acceptance for the ExecPlan draft:

- This file exists at
  `docs/execplans/10-1-1-approve-stable-public-byte-container.md`.
- The plan is self-contained, names the approval gate, names the affected
  public API surfaces, and defers implementation until explicit approval.
- The plan signposts the relevant documents and skills.
- `coderabbit review --agent` has no unresolved concerns, or any remaining
  concern is documented as out of scope with a concrete rationale.
- `make check-fmt`, `make lint`, and `make test` pass.
- Documentation gates `make markdownlint` and `make nixie` pass when Markdown
  documents change.
- The branch is pushed to
  `origin/10-1-1-approve-stable-public-byte-container`.
- A draft pull request exists and links this ExecPlan.

Acceptance for later implementation of `10.1.1`:

- ADR 008 status is `Accepted` and records the stable public byte-container
  and edit-on-demand model.
- ADR 008 names each affected surface:
  `PacketParts`, `Envelope`, `ServiceRequest`, `ServiceResponse`,
  `BeforeSendHook`, and `Serializer::serialize`.
- `docs/roadmap.md` marks only `10.1.1` done in phase 10.
- `docs/zero-copy-frame-and-payload-migration-roadmap.md` marks only the
  corresponding `1.1.1` decision item done.
- Any user-facing or internal documentation affected by the accepted decision
  is updated, or the implementation records why no behaviour changed yet.
- No Rust runtime API migration begins without a separately approved plan
  revision.

## Idempotence and recovery

The planning and documentation steps are safe to repeat. If Markdown formatting
changes wrap prose differently, review the diff and keep only changes related
to this plan or ADR 008 alignment. If a quality gate fails, fix the smallest
relevant issue and rerun only the failed gate before continuing to the next
gate.

If the branch push fails because the remote branch already exists, fetch the
remote branch and compare histories before force-pushing. Do not overwrite
remote work without explicit approval.

If `coderabbit review --agent` is unavailable in the local environment, record
the command failure in `Surprises & discoveries`, proceed only with the local
quality gates, and note the missing review in the pull request.

## Artifacts and notes

External prior-art checks used while drafting this plan:

- `bytes::Bytes` documentation on docs.rs describes `Bytes` as cheaply
  cloneable, sliceable, and intended for zero-copy network programming. Source:
  <https://docs.rs/bytes/latest/bytes/struct.Bytes.html>
- `bytes::BytesMut` documentation on docs.rs describes unique mutable access
  and `freeze()` conversion into shareable `Bytes`. Source:
  <https://docs.rs/bytes/latest/bytes/struct.BytesMut.html>
- Netsuke's public `scripts/` directory shows compact validation helpers such
  as file-existence assertions and Kani version checks. Source:
  <https://github.com/leynos/netsuke/tree/main/scripts>

Existing Wireframe source documents to keep aligned:

- `docs/adr-008-zero-copy-public-byte-container.md`
- `docs/adr-009-vec-u8-migration-rollout.md`
- `docs/adr-010-transport-frame-boundary-for-zero-copy.md`
- `docs/frame-vec-u8-inventory.md`
- `docs/zero-copy-frame-and-payload-migration-roadmap.md`
- `docs/users-guide.md`
- `docs/developers-guide.md`
- `docs/documentation-style-guide.md`
- `docs/generic-message-fragmentation-and-re-assembly-design.md`
- `docs/multi-packet-and-streaming-responses-design.md`
- `docs/the-road-to-wireframe-1-0-feature-set-philosophy-and-capability-maturity.md`
- `docs/hardening-wireframe-a-guide-to-production-resilience.md`
- `docs/rust-testing-with-rstest-fixtures.md`
- `docs/reliable-testing-in-rust-via-dependency-injection.md`
- `docs/rstest-bdd-users-guide.md`
- `docs/rust-doctest-dry-guide.md`

## Interfaces and dependencies

The intended public API direction to approve is:

```rust
// Illustrative shape only. Final names are decided during implementation.
pub struct PayloadBytes(bytes::Bytes);

pub struct PayloadEditor {
    // Private representation. Mutating operations copy only when required.
}
```

`PayloadBytes` in this sketch means "the stable shared byte representation". It
may be `bytes::Bytes` directly or a project wrapper over it. `PayloadEditor`
means "the explicit edit-on-demand workflow". It may use `bytes::BytesMut`
internally, but callers should not need to manually shuttle between `Vec<u8>`,
`Bytes`, and `BytesMut` for ordinary middleware or hook edits.

Do not add a new dependency for the stable byte model unless ADR 008 is
reopened. The existing `bytes` crate is the expected dependency foundation.

## Revision note

Initial draft created on 2026-05-20. It turns the user request and roadmap item
`10.1.1` into a reviewable, approval-gated ExecPlan. No runtime implementation
is authorised by this draft.
