# Architectural decision record (ADR) 009: rollout strategy for the `Vec<u8>` to zero-copy migration

## Status

Proposed

## Date

2026-04-12

## Context and Problem Statement

Epic 284 is a breaking change. The inventory in
[`frame-vec-u8-inventory.md`](frame-vec-u8-inventory.md) shows that downstream
users are exposed to `Vec<u8>` not only through one public `Frame` alias, but
through packet construction, middleware wrappers, client hooks, serializer
output, tests, and documentation. A zero-copy internal implementation is not
enough; the project also needs a release and compatibility strategy that keeps
the change reviewable and predictable for downstream users.

The key rollout question is whether the project should:

- switch to the new byte model in one hard break,
- support both `Vec<u8>` and the new representation indefinitely, or
- ship a staged major release with finite compatibility helpers and explicit
  migration guidance.

## Traceability

This ADR governs the Epic 284 compatibility and rollout work tracked in:

- [`frame-vec-u8-inventory.md`](frame-vec-u8-inventory.md), especially the
  "Generalization paths and conceptual risks", "Resolved direction for epic
  284", and "Coordination notes" sections.
- [`roadmap.md`](roadmap.md), specifically:
  - roadmap item `10.1.2`, which approves the compatibility and rollout
    policy;
  - roadmap item `10.2.3`, which publishes the migration-guide outline and
    exact public surfaces affected by the change;
  - roadmap item `12.1.2`, which preserves explicit edit-on-demand ergonomics
    as part of the public migration story;
  - roadmap item `12.2.2`, which documents how client preamble leftovers fit
    into the compatibility plan;
  - roadmap items `13.1.1` and `13.1.2`, which remove obsolete `Vec<u8>`
    compatibility surfaces and publish the migration guide section;
  - roadmap items `14.1.1`, `14.1.2`, `14.1.3`, and `14.2.1`, which prepare
    and review the breaking release and any retained helpers.

## Decision Drivers

- Minimize downstream boilerplate during the migration.
- Keep the long-term public API coherent rather than permanently dual-shaped.
- Make the breaking change easy to communicate in release notes and examples.
- Avoid indefinite maintenance of compatibility shims that preserve the old
  bottlenecks.
- Leave room for downstream users to migrate in bounded, observable steps.

## Options Considered

### Option A: one-shot hard break with no compatibility helpers

Ship the new zero-copy API and require all downstream users to update in one
step. This keeps the final API clean, but it also maximizes upgrade pain and
forces every consumer to solve migration details independently.

### Option B: permanent dual support for `Vec<u8>` and the zero-copy type

Expose both old and new constructors, accessors, and hook signatures as first-
class supported APIs. This reduces short-term friction, but it risks turning a
breaking release into a permanent maintenance burden and keeps the old
allocation-heavy path alive indefinitely.

### Option C: staged breaking release with finite compatibility helpers (preferred)

Adopt the zero-copy API as the long-term default, but ship the breaking release
with bounded helper conversions, migration examples, and clear removal criteria
for any retained `Vec<u8>` adapters.

| Topic                     | Option A: hard break | Option B: permanent dual support | Option C: staged release |
| ------------------------- | -------------------- | -------------------------------- | ------------------------ |
| Long-term API coherence   | Strong               | Weak                             | Strong                   |
| Short-term upgrade pain   | High                 | Low                              | Medium                   |
| Maintenance burden        | Low                  | High                             | Medium                   |
| Migration guidance needed | High                 | High                             | High                     |
| Risk of preserving copies | Low                  | High                             | Low                      |

_Table 1: Trade-offs for the migration rollout policy._

## Decision Outcome / Proposed Direction

Adopt Option C: ship the zero-copy API as the new primary surface in the next
breaking release, and include finite compatibility helpers plus a committed
migration guide.

The proposed rollout policy is:

- The new byte model becomes the canonical API in the breaking release.
- Compatibility helpers remain only where they clearly reduce upgrade cost and
  do not reintroduce the old bottlenecks as first-class behaviour.
- Release notes, examples, and migration documentation explicitly show how to
  move middleware, hooks, serializer code, and custom codecs off `Vec<u8>`.
- Any retained compatibility surface has a documented review point for later
  removal or retention.

## Goals and Non-Goals

### Goals

- Make the breaking release adoptable without forcing users to reverse-engineer
  internal design intent.
- Prevent the project from carrying two equally primary byte APIs
  indefinitely.
- Bound the lifetime of compatibility helpers.

### Non-Goals

- Preserve full source compatibility.
- Hide the fact that this is a semver-significant release.
- Guarantee that every downstream crate can upgrade without any code changes.

## Migration Plan

### Phase 1: announce the target API and migration path

Publish the roadmap, proposed ADRs, and a migration-guide outline before the
public API flip lands.

### Phase 2: land the breaking release with bounded helpers

Ship the new canonical API together with explicitly documented adapters such as
`from_vec`, `into_vec`, or equivalent helper wrappers where they materially
reduce migration churn.

### Phase 3: evaluate retained helpers after the release

Review which helpers are still needed after the initial migration cycle, and
either accept them as narrow compatibility tools or schedule their removal in a
follow-up release plan.

## Known Risks and Limitations

- Compatibility helpers can easily become de facto permanent if their removal
  criteria are not written down.
- A breaking release without concrete before-and-after examples will still feel
  abrupt even if helper APIs exist.
- If too many helpers survive unchanged, downstream code may continue to depend
  on the `Vec<u8>` mental model, weakening the benefits of the migration.

## Outstanding Decisions

- Which compatibility helpers are important enough to ship in the breaking
  release?
- Should any helpers be feature-gated, or should they ship in the default
  build for one release cycle?
- What benchmark or adoption signals justify removing retained helpers later?

## Architectural Rationale

The zero-copy migration is not only a transport optimization. It changes how
middleware, hooks, serializers, and packet routing represent bytes. A staged
breaking release acknowledges that architectural reality: the new API becomes
the primary one immediately, but the release still carries enough local
documentation and narrowly-scoped adapters to keep downstream adoption
manageable.
