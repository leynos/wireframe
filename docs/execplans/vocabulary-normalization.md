# Normalize API vocabulary by layer

This ExecPlan is a living document. The sections `Constraints`, `Tolerances`,
`Risks`, `Progress`, `Surprises & Discoveries`, `Decision Log`, and
`Outcomes & Retrospective` must be kept up to date as work proceeds.

Status: DRAFT

`PLANS.md` is not present in this repository at the time this plan was drafted.

## Purpose / big picture

Wireframe naming currently mixes layer-specific concepts in ways that can blur
mental models (for example, frame/packet/envelope/message boundaries). This
plan executes a layer-aware naming audit, normalizes terminology without
blanket renaming, and updates conceptual documentation so API language and
architectural model match.

## Constraints

- Naming normalization must be layer-specific, not global search/replace.
- Terms must remain semantically distinct across transport, routing, and domain
  payload layers.
- Public docs must explicitly define the conceptual model and term boundaries.
- `docs/users-guide.md` must be updated to match final vocabulary.
- `docs/developers-guide.md` must be created or updated to encode architectural
  vocabulary and invariants.
- Migration guidance must be updated for any renamed public items.

## Tolerances (exception triggers)

- Scope: if vocabulary normalization touches more than 70 files, stop and
  escalate.
- Interface: if term normalization forces redesign of protocol behaviour rather
  than naming alignment, stop and escalate.
- Dependencies: if a new tooling dependency is needed for term auditing, stop
  and escalate.
- Iterations: if lint/test gates fail after 3 complete correction loops, stop
  and escalate with failure clusters.
- Ambiguity: if one term is used with two incompatible meanings in the same
  layer, stop and escalate with proposed canonical definitions.

## Risks

- Risk: seemingly simple renames can break conceptual compatibility between docs
  and code. Severity: high Likelihood: high Mitigation: define canonical
  glossary first, then map every rename to that glossary before code edits.

- Risk: cross-layer types (for example, correlation metadata) may not fit one
  term cleanly. Severity: medium Likelihood: medium Mitigation: document
  cross-layer terms explicitly and constrain where each alias is permitted.

- Risk: migration churn may be large if names are changed aggressively.
  Severity: medium Likelihood: medium Mitigation: normalize only where
  ambiguity or inconsistency is user-visible.

## Progress

- [x] (2026-02-18) Drafted ExecPlan for vocabulary normalization.
- [ ] Create canonical glossary by architectural layer.
- [ ] Inventory API symbols and docs text against glossary.
- [ ] Propose targeted rename set and update map.
- [ ] Apply code and doc renames.
- [ ] Create/update developers' guide conceptual model section.
- [ ] Update migration guide and run quality gates.

## Surprises & Discoveries

- Observation: `docs/developers-guide.md` is currently absent.
  Evidence: repository file inventory under `docs/`. Impact: this plan must
  include creating the developers' guide.

## Decision Log

- Decision: Use a glossary-first approach before any renaming.
  Rationale: This avoids ad hoc renames and keeps layer semantics stable.
  Date/Author: 2026-02-18 / Codex.

- Decision: Define and document at least these layers: transport frame,
  routing packet/envelope, domain message payload, and fragmentation artifact.
  Rationale: These are the recurring conceptual boundaries in Wireframe APIs.
  Date/Author: 2026-02-18 / Codex.

## Outcomes & Retrospective

Not started. Populate after implementation milestones complete.

## Context and orientation

Primary conceptual terms currently appear across:

- `src/frame/` and frame-related exports.
- `src/app/` packet and envelope abstractions.
- `src/message.rs`, `src/message_assembler/`, and request/response surfaces.
- `src/fragment/` transport fragmentation terminology.
- `README.md`, `docs/users-guide.md`, and design docs.

The work should align term definitions with where the model actually lives:

- Frame: transport unit processed by codec boundaries.
- Packet/Envelope: routable wrapper carrying message metadata plus payload.
- Message: typed domain payload after (de)serialization.
- Fragment: transport-level subdivision of a frame/message for size limits.

Potentially touched files:

- `src/lib.rs` and affected modules exposing renamed symbols.
- `README.md`
- `docs/users-guide.md`
- `docs/developers-guide.md` (new file expected)
- `docs/v0-1-0-to-v0-2-0-migration-guide.md`

## Plan of work

Stage A defines the glossary and naming invariants. Capture canonical terms,
layer definitions, and disallowed synonyms per layer.

Stage B audits code and docs. Build an inventory of user-visible symbols and
documentation passages that diverge from canonical vocabulary.

Stage C proposes targeted renames. Limit changes to inconsistent or ambiguous
symbols; avoid blanket renaming where semantics are already clear.

Stage D applies updates with synchronized docs. Ensure code, user guide, and
new developers' guide describe the same conceptual model.

Stage E updates migration guidance and validates compile/lint/test/doc gates.

## Concrete steps

Run all commands from repository root (`/home/user/project`).

1. Build vocabulary inventory from code and docs.

   `rg -n -e Frame -e Packet -e Envelope -e Message -e Fragment \
   -e Route -e Handler -e Session -e Connection src docs README.md`

2. Define glossary and target rename map in docs.

   `make fmt`

3. Apply symbol and doc updates.

   `make check-fmt` `make lint` `make test`

4. Validate markdown and diagram quality.

   `make markdownlint` `make nixie`

Expected success indicators:

- Conceptual glossary is explicit in users' and developers' guides.
- User-visible symbol names align to layer definitions.
- Migration guide captures all renamed public items.

## Validation and acceptance

Acceptance criteria:

- `docs/users-guide.md` clearly defines Wireframe conceptual model terms and
  layer boundaries.
- `docs/developers-guide.md` exists and documents model invariants plus naming
  rules.
- API naming is consistent within each layer and across shared cross-layer
  concepts.
- `docs/v0-1-0-to-v0-2-0-migration-guide.md` includes rename mappings.
- `make check-fmt`, `make lint`, and `make test` pass.
- `make fmt`, `make markdownlint`, and `make nixie` pass.

## Idempotence and recovery

The glossary and inventory steps are re-runnable and safe. If a rename proves
semantically incorrect, revert that rename set only, update glossary mapping,
and re-run validation.

## Artifacts and notes

Implementation should retain:

- Canonical glossary table or section in documentation.
- Symbol rename inventory with before/after names.
- Cross-layer term usage notes where dual representation is intentional.

## Interfaces and dependencies

No new external dependencies are planned.

Expected interface effects:

- Public symbol names reflect one vocabulary per layer.
- Cross-layer models are documented explicitly where names are shared.
- Developers have a stable naming contract in `docs/developers-guide.md`.

Revision note: Initial draft created on 2026-02-18 to plan layer-specific
vocabulary normalization and conceptual model documentation.
