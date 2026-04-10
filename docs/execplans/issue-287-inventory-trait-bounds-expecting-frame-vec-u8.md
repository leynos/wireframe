# Inventory `Frame = Vec<u8>` trait bounds and APIs

This ExecPlan is a living document. The sections `Constraints`, `Tolerances`,
`Risks`, `Progress`, `Surprises & Discoveries`, `Decision Log`, and
`Outcomes & Retrospective` must be kept up to date as work proceeds.

Status: PROPOSED

No `PLANS.md` exists in this repository as of 2026-04-10.

## Purpose / big picture

Issue 287 supports epic 284 by producing a source-of-truth inventory of code
paths, trait bounds, impls, and APIs that currently assume `Frame = Vec<u8>` or
otherwise expose `Vec<u8>` as the de facto frame representation. The goal is
not to prescribe the Bytes migration, but to bound its scope, identify likely
breakpoints, and record where the codebase already distinguishes transport
frames from payload bytes.

The implementation of this issue should publish a dedicated inventory document
in `docs/`. The recommended target is `docs/frame-vec-u8-inventory.md`, because
that title is specific, search-friendly, and narrow enough to link directly
from the epic.

## Constraints

- This issue is an inventory and planning task, not the Bytes migration
  itself.
- The resulting document must distinguish between:
  - true `Frame = Vec<u8>` constraints in traits, impls, and generic bounds;
  - public APIs that expose raw payload bytes as `Vec<u8>` without literally
    constraining `F::Frame = Vec<u8>`;
  - internal-only usages that can likely change with lower compatibility risk;
  - examples, tests, and documentation references that are illustrative but
    not binding.
- Every inventory entry must cite concrete evidence: file path, symbol, and
  the relevant signature or usage shape.
- The inventory must remain conceptual and non-prescriptive. Generalization
  paths and risks should be noted, but the document must not lock the epic into
  a single implementation sequence.
- The work must reflect the current repository layout rather than historical
  paths mentioned in issue context.
- If implementation adds Rust tests for exploratory validation, they must use
  `rstest` and must pass the full Rust quality gates before the change is
  considered complete.

## Tolerances (exception triggers)

- Scope: if the inventory expands beyond approximately 40 source files with
  runtime-significant `Vec<u8>` coupling, stop and group the findings into
  phases before continuing.
- Ambiguity: if more than 8 surfaces cannot be classified as
  `frame-bound`, `payload-bound`, `internal-only`, or `docs/tests-only`, stop
  and escalate with a proposed taxonomy.
- Roadmap alignment: if no relevant roadmap item exists for the published
  inventory work, do not mark an unrelated item as done. Add or request a
  scoped roadmap entry first.
- Epic linkage: if the epic cannot be updated from the local environment,
  record the exact document path and linking text in the issue notes so the
  GitHub update is trivial.

## Risks

- Risk: the inventory may overstate migration scope by treating all
  `Vec<u8>` payload APIs as equivalent to `Frame = Vec<u8>` generic bounds.
  Severity: high. Likelihood: medium. Mitigation: classify each finding by
  coupling type and call out false friends explicitly.

- Risk: stale issue context may send the investigation toward files or types
  that no longer exist. Severity: medium. Likelihood: high. Mitigation: drive
  the inventory from current `src/` signatures first, and record stale
  references as discoveries rather than facts.

- Risk: documentation-only findings may drown out public API breakpoints.
  Severity: medium. Likelihood: medium. Mitigation: keep separate sections for
  public API, internal implementation, tests/examples, and docs.

- Risk: a later Bytes migration may choose a more generic frame abstraction
  than `Bytes`, making some recorded paths less relevant. Severity: low.
  Likelihood: medium. Mitigation: describe findings in terms of ownership,
  mutability, and trait contracts, not only the replacement type.

## Validation and acceptance

Acceptance is based on observable artefacts:

- A new inventory document exists in `docs/` and catalogues:
  - traits and trait objects that assume `Frame = Vec<u8>` or equivalent;
  - impls and helper traits tied to `Vec<u8>` frames or payload ownership;
  - generic bounds, channels, and middleware types coupled to `Vec<u8>`;
  - documentation and examples that would need follow-up during a Bytes
    migration.
- The inventory document includes a brief, non-prescriptive note on
  generalization paths and conceptual risks.
- The inventory document names open questions and scope boundaries for epic
  284.
- Alignment on scope is recorded in the document itself, including which
  surfaces are in scope for a future migration and which are only adjacent.
- The epic is updated to link to the published document, or the exact link
  text is prepared if GitHub editing happens separately.
- If a roadmap item exists for this inventory work, it is marked done. If one
  does not exist, the implementation must record that explicitly instead of
  guessing.

Validation commands for a documentation-only implementation:

```sh
set -o pipefail
timeout 300 make fmt 2>&1 | tee /tmp/wireframe-fmt.log
echo "fmt exit: $?"

set -o pipefail
timeout 300 make markdownlint 2>&1 | tee /tmp/wireframe-markdownlint.log
echo "markdownlint exit: $?"
```

If the work adds or edits Mermaid diagrams, also run:

```sh
set -o pipefail
timeout 300 make nixie 2>&1 | tee /tmp/wireframe-nixie.log
echo "nixie exit: $?"
```

If exploratory `rstest` coverage is added to Rust source files, run the full
Rust gates as well:

```sh
set -o pipefail
timeout 300 make check-fmt 2>&1 | tee /tmp/wireframe-check-fmt.log
echo "check-fmt exit: $?"

set -o pipefail
timeout 300 make lint 2>&1 | tee /tmp/wireframe-lint.log
echo "lint exit: $?"

set -o pipefail
timeout 300 make test 2>&1 | tee /tmp/wireframe-test.log
echo "test exit: $?"
```

## Context and orientation

Current, concrete evidence already visible in the repository:

- `src/middleware.rs` hard-codes `ServiceRequest` and `ServiceResponse` around
  `FrameContainer<Vec<u8>>`. This is a public middleware API and therefore a
  high-sensitivity migration surface.
- `src/app/envelope.rs` hard-codes `PacketParts` and `Envelope` payloads as
  `Vec<u8>`. This is broader than a frame-codec concern and affects routing,
  handler invocation, and packet reconstruction.
- `src/app/frame_handling/response.rs` converts `Envelope` payloads directly
  into `ServiceRequest::new(env.payload, env.correlation_id)`, which confirms
  the middleware path is payload-owned `Vec<u8>` today.
- `src/app/builder/core.rs` stores `push_dlq` as
  `Option<mpsc::Sender<Vec<u8>>>`.
  This is internal state, but it still constrains one outbound failure path to
  owned byte vectors.
- `src/client/hooks.rs` exposes `BeforeSendHook = Arc<dyn Fn(&mut Vec<u8>) +
  Send + Sync>`. This is not literally `Frame =
  Vec<u8>`, but it is a public bytes-shaped API that may need redesign or
  compatibility shims during a Bytes migration.
- `src/correlation.rs` implements `CorrelatableFrame` for `Vec<u8>`, which
  confirms that generic frame plumbing still treats `Vec<u8>` as a valid
  concrete frame type in some paths.
- `src/hooks.rs` is already generic over `type Frame: FrameLike`; its
  `Vec<u8>` references are primarily examples, not proof that the trait itself
  is still hard-coded.

Referenced design documents already point toward a more generic or `Bytes`-led
direction:

- `docs/generic-message-fragmentation-and-re-assembly-design.md`
- `docs/multi-packet-and-streaming-responses-design.md`
- `docs/the-road-to-wireframe-1-0-feature-set-philosophy-and-capability-maturity.md`
- `docs/hardening-wireframe-a-guide-to-production-resilience.md`
- `docs/adr-004-pluggable-protocol-codecs.md`
- `docs/execplans/9-1-1-instance-aware-payload-wrapper.md`
- `docs/execplans/9-1-3-enable-zero-copy-payload-extraction-for-codecs.md`

## Illustrative examples

These examples show the level of precision the inventory should provide.

| Surface                                | Current shape                        | Classification                          | Why it matters                                                           |
| -------------------------------------- | ------------------------------------ | --------------------------------------- | ------------------------------------------------------------------------ |
| `src/middleware.rs` `ServiceRequest`   | `inner: FrameContainer<Vec<u8>>`     | Public API, payload-bound hard coupling | Middleware signatures and mutability helpers assume owned byte vectors.  |
| `src/app/envelope.rs` `PacketParts`    | `payload: Vec<u8>`                   | Public API, payload-bound hard coupling | Routing and packet reconstruction are anchored to owned payload bytes.   |
| `src/client/hooks.rs` `BeforeSendHook` | `Fn(&mut Vec<u8>)`                   | Public API, bytes-hook coupling         | Client hook ergonomics may need a `BytesMut`, `Bytes`, or adapter story. |
| `src/app/builder/core.rs` `push_dlq`   | `mpsc::Sender<Vec<u8>>`              | Internal-only coupling                  | Lower compatibility risk, but still part of the migration blast radius.  |
| `src/correlation.rs` impl              | `impl CorrelatableFrame for Vec<u8>` | Trait impl convenience                  | Signals where generic code currently accepts `Vec<u8>` directly.         |

The inventory document should contain similar rows for every confirmed binding
surface, plus an explicit note when a file only contains examples or tests
rather than runtime constraints.

## Plan of work

Stage A creates the taxonomy and evidence set. Start from current source
signatures, not issue text, and classify each finding as `frame-bound`,
`payload-bound`, `internal-only`, or `docs/tests-only`.

Stage B builds the source inventory. Record each relevant trait, impl, generic
bound, type alias, channel, and public constructor that expects `Vec<u8>`
directly or indirectly.

Stage C performs adjacency review. Trace the main runtime paths so the
inventory can explain how payload ownership flows from codec decode to
middleware, handlers, hooks, and outbound send paths.

Stage D performs exploratory validation where static reading is insufficient.
Add focused `rstest` coverage only for behaviours whose public contract is
ambiguous and whose clarification materially improves the inventory.

Stage E publishes the inventory document and coordination notes. Add an epic
ready summary, note generalization paths and risks, and resolve roadmap
tracking.

## Concrete steps

Run all commands from the repository root (`/home/user/project`).

1. Build the initial candidate list.

   ```sh
   rg -n "Frame = Vec<u8>|Vec<u8>|Bytes|ServiceRequest|ServiceResponse|BeforeSendHook|push_dlq|CorrelatableFrame" src docs
   ```

2. Narrow the list to runtime-significant surfaces and classify each item.

   Suggested grouping:

   - public API surface;
   - internal runtime surface;
   - tests/examples;
   - design and user documentation.

3. Trace the main flow edges to avoid misclassification.

   Minimum paths to inspect:

   - codec decode -> envelope / packet parts;
   - envelope -> middleware request / response;
   - protocol hooks and correlation plumbing;
   - client request hooks;
   - DLQ / auxiliary channels.

4. Draft `docs/frame-vec-u8-inventory.md` with:

   - summary and scope;
   - inventory table;
   - findings by category;
   - generalization paths;
   - conceptual risks;
   - open questions for epic 284.

5. If a static read leaves behaviour unclear, add small `rstest` cases in the
   owning modules.

   Candidate examples:

   - `ServiceRequest` and `ServiceResponse` preserve owned `Vec<u8>` mutation
     semantics.
   - `PacketParts::into_payload()` consumes and returns owned payload bytes.
   - `CorrelatableFrame for Vec<u8>` remains a no-op correlation carrier.
   - Client `BeforeSendHook` can mutate serialized bytes before transport.

6. Update the epic link and roadmap tracking.

   - Add the published inventory document link to epic 284.
   - If a matching roadmap item exists, mark it done.
   - If no matching roadmap item exists, record that outcome explicitly.

## Exploratory testing strategy

Exploratory tests are optional and should only be added when they clarify a
public behaviour that the inventory depends on. They should not be used to
restate obvious type aliases.

Use `rstest` parameterization where it helps compare owned-byte behaviour
across similar surfaces. For example:

```rust,no_run
#[cfg(test)]
mod inventory_exploration {
    use rstest::rstest;

    use crate::{
        app::PacketParts,
        correlation::CorrelatableFrame,
        middleware::ServiceRequest,
    };

    #[rstest]
    #[case(vec![])]
    #[case(vec![1, 2, 3])]
    fn service_request_exposes_owned_mutable_bytes(#[case] payload: Vec<u8>) {
        let mut request = ServiceRequest::new(payload.clone(), Some(7));
        request.frame_mut().push(9);

        let mut expected = payload;
        expected.push(9);
        assert_eq!(request.into_inner(), expected);
    }

    #[rstest]
    #[case(Vec::<u8>::new())]
    #[case(vec![4, 5, 6])]
    fn vec_u8_correlation_impl_is_still_noop(#[case] mut frame: Vec<u8>) {
        assert_eq!(frame.correlation_id(), None);
        frame.set_correlation_id(Some(99));
        assert_eq!(frame.correlation_id(), None);
    }

    #[rstest]
    #[case(vec![7, 8])]
    fn packet_parts_returns_owned_payload(#[case] payload: Vec<u8>) {
        let parts = PacketParts::new(1, None, payload.clone());
        assert_eq!(parts.into_payload(), payload);
    }
}
```

If such tests are added and they describe supported behaviour rather than
temporary investigation scaffolding, keep them as regression tests.

## Interfaces and dependencies

The future inventory document should explicitly cover these interface classes:

- traits with associated `Frame` types or frame-shaped bounds;
- trait objects and builder fields that fix `Frame` or payloads to `Vec<u8>`;
- middleware request / response wrappers;
- packet and envelope abstractions;
- hook APIs that expose mutable serialized bytes;
- trait impls that make `Vec<u8>` participate in generic frame plumbing;
- internal channels carrying `Vec<u8>`;
- user-facing documentation that still teaches `Vec<u8>` as the dominant
  frame or payload representation.

No new runtime dependencies are expected for the inventory itself.

## Idempotence and recovery

The source scan, classification, and documentation steps are re-runnable. If
the first pass over-collects findings, keep the raw candidate list, then
reclassify it rather than deleting evidence. If exploratory tests are added and
later deemed unnecessary, remove them in a separate clean-up step before
closing the issue.

## Progress

- [x] (2026-04-10) Reviewed repository instructions, existing execplan style,
  and the referenced design and testing documentation.
- [x] (2026-04-10) Confirmed current runtime evidence in
  `src/middleware.rs`, `src/app/envelope.rs`,
  `src/app/frame_handling/response.rs`, `src/app/builder/core.rs`,
  `src/client/hooks.rs`, and `src/correlation.rs`.
- [ ] Publish `docs/frame-vec-u8-inventory.md`.
- [ ] Record epic link and roadmap status.
- [ ] Add exploratory `rstest` coverage if classification needs behavioural
  confirmation.
- [ ] Run the applicable quality gates.

## Surprises & Discoveries

- Observation: the issue context mentions `BoxedFrameProcessor` in `app.rs`,
  but no `src/app.rs` or `BoxedFrameProcessor` exists in the current
  repository. Evidence: repository-wide search on 2026-04-10 returned no
  matches. Impact: the implementation must inventory the current builder and
  runtime modules rather than relying on historical file references.

- Observation: `WireframeApp` is already generic over `F: FrameCodec`.
  Evidence: `src/app/builder/core.rs`. Impact: the remaining migration surface
  appears to be concentrated in payload ownership APIs, middleware, channels,
  and hook signatures rather than the top-level app generic itself.

- Observation: several prominent `Vec<u8>` surfaces are payload-oriented
  rather than frame-oriented. Evidence: `PacketParts`, `Envelope`,
  `ServiceRequest`, and `ServiceResponse`. Impact: the inventory must separate
  payload generalization work from true codec-frame work to avoid overstating
  the blast radius.

## Decision Log

- Decision: recommend `docs/frame-vec-u8-inventory.md` as the final inventory
  artefact. Rationale: the path is explicit, stable, and easy to link from the
  epic. Date/Author: 2026-04-10 / Codex.

- Decision: classify findings by coupling type instead of maintaining a flat
  grep dump. Rationale: a migration plan needs risk-ranked impact, not only a
  list of matches. Date/Author: 2026-04-10 / Codex.

- Decision: treat exploratory `rstest` additions as optional and
  behaviour-driven. Rationale: the issue is primarily documentary; tests should
  only be introduced where they improve confidence in public semantics.
  Date/Author: 2026-04-10 / Codex.

## Outcomes & Retrospective

Not started. This section should summarize:

- what the published inventory found;
- which surfaces are true blockers versus adjacent follow-up work;
- whether epic 284 scope changed after the inventory;
- whether roadmap tracking required a new entry.

## Revision note

Initial draft created on 2026-04-10 for issue 287 to plan the inventory of
`Frame = Vec<u8>` assumptions ahead of a potential Bytes migration.
