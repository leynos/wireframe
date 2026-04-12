# Inventory `Frame = Vec<u8>` trait bounds and APIs

This ExecPlan is a living document. The sections `Constraints`, `Tolerances`,
`Risks`, `Progress`, `Surprises & Discoveries`, `Decision Log`, and
`Outcomes & Retrospective` must be kept up to date as work proceeds.

Status: COMPLETE

No `PLANS.md` exists in this repository as of 2026-04-11.

## Purpose / big picture

Issue 287 supports epic 284 by producing a source-of-truth inventory of code
paths, trait bounds, impls, and APIs that currently assume `Frame = Vec<u8>` or
otherwise expose `Vec<u8>` as the de facto frame or payload representation.

The goal is to bound migration scope without prescribing the migration itself.
The published artefact for this work is `docs/frame-vec-u8-inventory.md`.

## Constraints

- This issue is an inventory and planning task, not the `Bytes` migration
  itself.
- The resulting document must distinguish between:
  - true `Frame = Vec<u8>` constraints in traits, impls, and generic bounds;
  - public APIs that expose raw payload bytes as `Vec<u8>` without literally
    constraining `F::Frame = Vec<u8>`;
  - internal-only usages that can likely change with lower compatibility risk;
  - examples, tests, and documentation references that are illustrative but
    not binding.
- Every inventory entry must cite concrete evidence: file path, symbol, and
  current signature or usage shape.
- The work must reflect the current repository layout rather than historical
  issue context.
- The implementation must not invent roadmap completion where no local roadmap
  item exists.

## Tolerances (exception triggers)

- Scope: if the inventory expands beyond roughly 40 runtime-significant files,
  stop and group the findings before continuing.
- Ambiguity: if more than a handful of surfaces cannot be classified as
  `frame-bound`, `payload-bound`, `internal-only`, or `docs/tests-only`, stop
  and define the taxonomy explicitly before proceeding.
- Epic linkage: if GitHub cannot be edited from the local environment, record
  the exact document path and link text so the follow-up is trivial.

## Risks

- Risk: the inventory may overstate migration scope by treating all
  `Vec<u8>` payload APIs as equivalent to `Frame = Vec<u8>` generic bounds.
  Severity: high. Likelihood: medium. Mitigation: classify each finding by
  coupling type and call out false friends explicitly.

- Risk: stale issue context may send the investigation toward files or types
  that no longer exist. Severity: medium. Likelihood: high. Mitigation: drive
  the inventory from the current repository state first, and record stale
  references as discoveries rather than facts.

- Risk: documentation-only findings may drown out public API breakpoints.
  Severity: medium. Likelihood: medium. Mitigation: keep separate sections for
  runtime surfaces, internal-only paths, and docs/tests-only follow-up.

## Validation and acceptance

Acceptance is based on observable artefacts:

- `docs/frame-vec-u8-inventory.md` exists and inventories:
  - true `Frame = Vec<u8>` surfaces;
  - payload-bound public APIs;
  - internal-only runtime coupling;
  - tests, examples, and docs that would need follow-up during epic 284.
- The inventory document includes non-prescriptive notes on generalization
  paths, conceptual risks, and open questions.
- The inventory records roadmap status explicitly rather than guessing.
- Documentation quality gates pass.

Validation commands for this documentation-only implementation:

```sh
set -o pipefail
timeout 300 make fmt 2>&1 | tee /tmp/wireframe-fmt.log
echo "fmt exit: $?"

set -o pipefail
timeout 300 make markdownlint 2>&1 | tee /tmp/wireframe-markdownlint.log
echo "markdownlint exit: $?"
```

## Context and orientation

The current repository already shows a split between generic transport frames
and owned payload-byte APIs:

- `src/codec.rs` defines `FrameCodec` generically over `Self::Frame`.
- `src/hooks.rs` defines `WireframeProtocol` generically over
  `type Frame: FrameLike`.
- `src/app/envelope.rs`, `src/middleware.rs`, `src/client/hooks.rs`, and
  `src/serializer.rs` still expose owned `Vec<u8>` payloads directly.
- `src/connection/mod.rs` uses generic frame bounds
  `FrameLike + CorrelatableFrame + Packet`, which is adjacent to the migration
  but is not itself a literal `Vec<u8>` constraint.

## Plan of work

Stage A builds a taxonomy and evidence set from current source signatures.

Stage B inventories runtime-significant public and internal surfaces, keeping
true frame coupling separate from payload ownership APIs.

Stage C records adjacent constraints, especially the actor/codec boundary,
without collapsing them into false `Vec<u8>` findings.

Stage D publishes the inventory document and records epic-link and roadmap
status.

## Concrete steps

1. Locate the original issue-287 plan context and confirm the current
   repository state.
2. Scan source files for `Vec<u8>`-shaped frame and payload surfaces.
3. Classify findings as `frame-bound`, `payload-bound`, `internal-only`, or
   `docs/tests-only`.
4. Publish `docs/frame-vec-u8-inventory.md`.
5. Update the docs index and record roadmap and epic-link follow-up notes.
6. Run documentation gates.

## Idempotence and recovery

The source scan, classification, and documentation steps are all re-runnable.
If a first pass over-collects findings, keep the evidence and reclassify it
rather than deleting it. If follow-up work later changes the coupling shape,
publish a superseding inventory rather than silently mutating historical
conclusions.

## Progress

- [x] (2026-04-10) Original issue-287 ExecPlan drafted in commit `c0cbc75`.
- [x] (2026-04-11) Recovered the missing ExecPlan from repository history
  because the file was absent from the current worktree.
- [x] (2026-04-11) Re-scanned current runtime code and classified findings by
  coupling type.
- [x] (2026-04-11) Published `docs/frame-vec-u8-inventory.md`.
- [x] (2026-04-11) Added the inventory to `docs/contents.md`.
- [x] (2026-04-11) Recorded roadmap status and prepared epic-link text in the
  inventory document.
- [x] (2026-04-11) Ran documentation quality gates:
  `make fmt` and `make markdownlint`.

## Surprises & Discoveries

- Observation: the exact issue-287 ExecPlan file was not present in the
  current worktree. Evidence: direct file lookup failed, but `git log --all`
  showed commit `c0cbc75` creating
  `docs/execplans/issue-287-inventory-trait-bounds-expecting-frame-vec-u8.md`.
  Impact: the implementation had to recover planning context from history
  before proceeding.

- Observation: the runtime no longer hard-codes `F::Frame = Vec<u8>` at the
  codec or protocol trait layer. Evidence: `src/codec.rs` and `src/hooks.rs`.
  Impact: the main migration scope is payload ownership and mutability, not a
  blanket frame-type constraint.

- Observation: the strongest remaining coupling is public payload-oriented API
  shape. Evidence: `src/app/envelope.rs`, `src/middleware.rs`,
  `src/client/hooks.rs`, and `src/serializer.rs`. Impact: epic 284 should
  separate transport-frame work from payload-API work.

- Observation: no matching roadmap item for this inventory work exists in the
  local docs set. Evidence: repository-wide roadmap search on 2026-04-11.
  Impact: nothing was marked done.

## Decision Log

- Decision: publish the final inventory as
  `docs/frame-vec-u8-inventory.md`. Rationale: the path is explicit, stable,
  and easy to link from epic 284. Date/Author: 2026-04-11 / Codex.

- Decision: restore the missing issue-287 ExecPlan path in the worktree.
  Rationale: the user explicitly referenced this path, and the historical plan
  existed in commit history. Restoring it keeps the documentation trail intact.
  Date/Author: 2026-04-11 / Codex.

- Decision: classify findings by coupling type instead of flattening them into
  a grep dump. Rationale: migration planning needs scope-ranked evidence, not
  only a list of matches. Date/Author: 2026-04-11 / Codex.

## Outcomes & Retrospective

Completed implementation outcomes:

- Published `docs/frame-vec-u8-inventory.md` as the issue-287 source of truth.
- Distinguished true `Frame = Vec<u8>` surfaces from payload-bound APIs,
  internal-only runtime coupling, and docs/tests-only follow-up.
- Recorded the actor/codec boundary as adjacent context rather than a false
  `Vec<u8>` finding.
- Added the inventory to `docs/contents.md` for discoverability.
- Recorded that no matching roadmap item was found locally and prepared
  epic-link text for external follow-up.

Validation outcomes:

- `make fmt` passed.
- `make markdownlint` passed.

## Revision note

This file was restored on 2026-04-11 from commit `c0cbc75` and updated to
reflect completion of the documentation work in the current worktree.
