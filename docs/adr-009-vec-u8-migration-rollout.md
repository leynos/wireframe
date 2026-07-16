# Architectural decision record (ADR) 009: rollout strategy for the `Vec<u8>` to zero-copy migration

## Status

Accepted

Accepted on 2026-06-15. Wireframe will ship the zero-copy public API as a
staged breaking release (Option C) with a narrow, finite set of `Vec<u8>`
compatibility helpers. The helpers ship in the default build, carry
`#[deprecated]` attributes that name both the migration path and the scheduled
removal version, and are removed in the second breaking release after they are
introduced. This decision closes roadmap item `10.1.2` and the corresponding
item `1.1.2` in
[`zero-copy-frame-and-payload-migration-roadmap.md`](zero-copy-frame-and-payload-migration-roadmap.md).

## Date

Proposed 2026-04-12. Accepted 2026-06-15.

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
- [`zero-copy-frame-and-payload-migration-roadmap.md`](zero-copy-frame-and-payload-migration-roadmap.md),
  which tracks the dedicated migration publish and breaking-release workstream
  that this rollout policy governs, including the Phase 1 decision closure and
  the later publication milestones.
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

## Decision Outcome

Adopt Option C: ship the zero-copy API as the new primary surface in the next
breaking release, and include a narrow, finite set of compatibility helpers
plus a committed migration guide.

The accepted rollout policy is:

- The new byte model becomes the canonical API in the breaking release.
- Compatibility helpers remain only where they clearly reduce upgrade cost and
  do not reintroduce the old bottlenecks as first-class behaviour.
- Release notes, examples, and migration documentation explicitly show how to
  move middleware, hooks, serializer code, and custom codecs off `Vec<u8>`.
- Any retained compatibility surface has a documented review point for later
  removal or retention.

### Accepted helper set and dispositions

The compatibility surface is deliberately narrow. There is a single conversion
surface, not one adapter per call site:

- **Stable byte wrapper.** `PayloadBytes::from_vec` and
  `PayloadBytes::into_vec` are the only `Vec<u8>` conversion helpers for
  transport hand-offs. Packet, envelope, and serializer code routes through the
  wrapper; no per-call-site `PacketParts::from_vec` or `Envelope::from_vec`
  constructors are added. The exact internal representation is settled by
  roadmap items `11.1.1`, `12.1.1`, and `12.2.1`; this ADR commits to the names.
- **Serializer.** `Serializer::serialize` returns the stable byte wrapper.
  If a `serialize_to_vec` shim proves necessary for downstream code that cannot
  yet accept the wrapper, it ships as a thin `#[deprecated]` wrapper over
  `PayloadBytes::into_vec`.
- **Middleware editing is removed, not retained.**
  `ServiceRequest::frame_mut`, `ServiceResponse::frame_mut`, and
  `ServiceResponse::into_inner` (the `&mut Vec<u8>` / owned-`Vec<u8>`
  accessors) are removed outright in the breaking release and replaced by the
  edit-on-demand workflow accepted in
  [ADR 008](adr-008-zero-copy-public-byte-container.md). Retaining a
  `&mut Vec<u8>` accessor would resurrect the final-copy bottleneck that ADR
  008 set out to eliminate, so it is excluded from the helper set on purpose.
- **Client request hooks.** `BeforeSendHook` gains one adapter constructor
  (`BeforeSendHook::from_vec_fn`, or the free-function
  `before_send_from_vec_fn` on `RequestHooks`) so existing `Fn(&mut Vec<u8>)`
  hooks keep compiling for one release cycle. The exact bound is finalized by
  roadmap item `12.2.1`.
- **Client preamble leftovers** stay on owned `Vec<u8>` for this breaking
  release, per the resolved direction in
  [`frame-vec-u8-inventory.md`](frame-vec-u8-inventory.md). Roadmap item
  `12.2.2` is the dedicated re-evaluation hook.
- **Runtime bridges are out of scope for this policy.** The
  `CorrelatableFrame for Vec<u8>` runtime bridge and the test-only
  `Packet for Vec<u8>` bridge are not compatibility helpers under ADR 009.
  Their lifecycle is governed by
  [ADR 010](adr-010-transport-frame-boundary-for-zero-copy.md) and roadmap items
  `11.2.1`, `11.2.2`, and `14.1.3`.

### Visibility and deprecation discipline

- Helpers ship in the **default build**, not behind a feature flag, so the
  deprecation warning reaches every consumer rather than only those who read
  the changelog.
- Every helper carries a `#[deprecated]` attribute whose `note` names both the
  recommended migration path and the removal version, so users who suppress the
  warning still learn the removal version from `cargo doc`. The release-time
  form is:

  ```rust
  #[deprecated(
      since = "<release-version>",
      note = "<recommended path>; scheduled for removal in <target-removal-version>"
  )]
  ```

- The `<release-version>` and `<target-removal-version>` placeholders are
  substituted at release-cut time by roadmap item `14.1.1`. This ADR commits to
  the substitution rule, not to literal version strings.
- The `PayloadBytes::into_vec` `note` must steer users to a zero-copy path
  (`PayloadBytes::as_slice`, `bytes::Bytes::freeze`, or the edit-on-demand
  editor) rather than only to the editor, because `into_vec` itself allocates
  and copies and the recommended replacement should preserve the zero-copy
  intent.

### Removal signal

- Helpers are removed in the **second breaking release** after they are
  introduced. Under 0.x semver, where every minor bump is a breaking release,
  that is `0.N → 0.N+2`; once Wireframe has cut 1.0, it is `1.x → 2.x`. Stating
  the trigger in terms of breaking releases keeps the policy invariant under
  whatever cadence the project adopts.
- The review trigger is **event-based, not calendar-based**: retained
  helpers are reviewed at every breaking release after their introduction,
  starting with roadmap item `14.2.1`, until removal lands.

### Release guardrails (deferred wiring)

`cargo-semver-checks` and `cargo-public-api` are the suggested guardrails for
confirming that the breaking release changes the intended public surface and
nothing more. Their CI wiring is deferred to roadmap item `14.1.x`. Until that
wiring lands, this policy is **enforced by review only**.

### Deferred to other roadmap items

- Migration-guide before-and-after examples for each helper are owned by
  roadmap item `10.2.3`.
- Changelog phrasing for the removed `Vec<u8>` contracts and retained
  helpers is owned by roadmap item `14.1.2`.

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

Ship the new canonical API together with the explicitly documented helper set
above: `PayloadBytes::from_vec` / `into_vec`, the serializer returning the
stable wrapper, and one `BeforeSendHook` adapter constructor. Each helper
carries a `#[deprecated(since, note)]` attribute whose `note` names both the
migration path and the scheduled removal version.

### Phase 3: evaluate retained helpers after the release

Review which helpers are still needed at every breaking release after the
initial migration cycle, starting with roadmap item `14.2.1`. Remove the
helpers in the second breaking release after their introduction, or record an
explicit, dated decision to retain them for one more breaking release with a
fresh review point.

## Known Risks and Limitations

- Compatibility helpers can become permanent in practice if their removal
  criteria are not written down. The event-based removal trigger (second
  breaking release after introduction, reviewed at every breaking release
  starting with roadmap item `14.2.1`) is the mitigation; it must be carried
  into the changelog and release-prep work so the trigger is acted on.
- A breaking release without concrete before-and-after examples will still feel
  abrupt even if helper APIs exist. Roadmap item `10.2.3` owns those examples.
- If too many helpers survive unchanged, downstream code may continue to depend
  on the `Vec<u8>` mental model, weakening the benefits of the migration. The
  narrow single-conversion-surface helper set above is the mitigation.
- Downstream users may suppress deprecation warnings (for example, a
  workspace-wide `#[allow(deprecated)]`) and then be blindsided when the
  helpers are removed. The deprecation `note` therefore names the scheduled
  removal version, so the removal date is visible in `cargo doc` even when the
  warning is suppressed; the `10.2.3` migration guide should recommend against
  blanket deprecation suppression in workspaces that consume Wireframe.
- Downstream crates may re-export the deprecated helpers in their own public
  API and propagate the warning to their users, who cannot fix it without
  forking. The deprecation `note` should direct re-exporters to add their own
  wrapper rather than re-export the Wireframe item, and the `10.2.3` migration
  guide should include a "for downstream library authors" section.
- `PayloadBytes::into_vec` allocates and copies, so it can become a
  performance footgun if its `note` only points at the editor. The accepted
  policy requires the `note` to steer users to zero-copy paths first.

## Outstanding Decisions

All previously outstanding decisions are resolved by this acceptance:

- _Which compatibility helpers ship in the breaking release?_ The narrow set
  in "Accepted helper set and dispositions": `PayloadBytes::from_vec` /
  `into_vec`, the serializer returning the wrapper (with an optional
  `serialize_to_vec` shim), and one `BeforeSendHook` adapter constructor.
  Middleware `&mut Vec<u8>` editors are removed, not retained; runtime bridges
  are governed by ADR 010.
- _Feature-gated or default build?_ Default build, with `#[deprecated]`
  attributes. Feature-gating is rejected because it hides the warning from
  users who do not read the changelog.
- _What signal justifies removing retained helpers later?_ The second breaking
  release after introduction, with an event-based review at every breaking
  release starting at roadmap item `14.2.1`. Benchmark and adoption evidence
  feed the review but do not replace the semver trigger.

## Architectural Rationale

The zero-copy migration is not only a transport optimization. It changes how
middleware, hooks, serializers, and packet routing represent bytes. A staged
breaking release acknowledges that architectural reality: the new API becomes
the primary one immediately, but the release still carries enough local
documentation and narrowly scoped adapters to keep downstream adoption
manageable.

The mechanics lean on stable Rust and the existing dependency surface. The
`#[deprecated]` attribute is the canonical mechanism for a finite compatibility
window: `rustdoc` renders both the `since` version and the `note`, so the
migration path and removal version travel with the API itself.[^deprecated] The
stable byte wrapper builds on the `bytes` crate already used by the default
codec, where `Bytes` is cheaply cloneable shared storage and `BytesMut`
provides unique mutable access with a zero-copy `freeze()`.[^bytes][^bytesmut]
No new dependency is required.

For the release boundary, `cargo-semver-checks` (rustdoc-JSON-based semver
linting, run just before `cargo publish`)[^semver-checks] and
`cargo-public-api` (public API surface diffing)[^public-api] are the suggested
guardrails that confirm the breaking release changes the intended surface and
nothing more. Wiring them into CI is deferred to roadmap item `14.1.x`; until
then the policy is enforced by review.

A staged break with finite helpers is also the pattern used by comparable Rust
libraries: hyper's 1.0 upgrade shipped `backports` and `deprecated` features on
0.14 to give users a warning window, and moved helpers that did not fit the 1.0
stability bar into a separate crate rather than retaining them in core.[^hyper]
Wireframe's policy reaches the same goal with `#[deprecated]` helpers in the
default build, scoped to a single conversion surface.

[^deprecated]: The `deprecated` attribute, Rust Reference.
    <https://doc.rust-lang.org/reference/attributes/diagnostics.html>
[^bytes]: `bytes::Bytes`, docs.rs.
    <https://docs.rs/bytes/latest/bytes/struct.Bytes.html>
[^bytesmut]: `bytes::BytesMut`, docs.rs.
    <https://docs.rs/bytes/latest/bytes/struct.BytesMut.html>
[^semver-checks]: `cargo-semver-checks-action`.
    <https://github.com/obi1kenobi/cargo-semver-checks-action>
[^public-api]: `cargo-public-api`.
    <https://crates.io/crates/cargo-public-api>
[^hyper]: hyper 1.0 upgrade guide.
    <https://hyper.rs/guides/1/upgrading/>
