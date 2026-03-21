# Add ergonomic helpers for consuming streaming responses (11.3.1)

This ExecPlan (execution plan) is a living document. The sections
`Constraints`, `Tolerances`, `Risks`, `Progress`, `Surprises & Discoveries`,
`Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work
proceeds.

Status: DRAFT

## Purpose / big picture

Roadmap item `11.3.1` exists because the client can already *receive* streaming
responses, but larger protocol implementations still need to hand-roll the same
consumption loop over and over:

- call `call_streaming()` or `receive_streaming()`;
- loop with `while let Some(result) = stream.next().await`;
- match each typed frame;
- ignore control frames such as progress or notices;
- convert data-bearing frames into a more useful domain item; and
- stop cleanly on termination or surface the first protocol-level error.

That works, but it pushes repetitive stream-plumbing into every downstream
crate that implements a multiplexed protocol on top of Wireframe. The missing
ergonomic layer is a small public helper surface that adapts
`client::ResponseStream` into a protocol-specific item stream without changing
Wireframe's transport, back-pressure, or correlation guarantees.

After this change, a library consumer should be able to take a
`ResponseStream<P>` and attach a mapper that yields only the frames they care
about. Control frames can be skipped, data frames can be converted into typed
items, and transport or correlation errors still surface exactly as they do
today. The same helper should also be usable with `response::FrameStream`
obtained from `Response::into_stream()` whenever that is practical, so the
roadmap's "`Response::Stream` helper" goal is covered by the same abstraction.

Observable success:

- a public helper trait and adapter type are available for streaming-response
  consumption;
- unit tests written with `rstest` prove ordered typed delivery, frame
  skipping, and error propagation;
- behavioural tests written with `rstest-bdd` v0.5.0 prove the helper against
  the real client streaming path;
- `docs/users-guide.md` documents the new public interface and when to prefer
  it over manual `StreamExt::next` loops;
- the relevant streaming/client design documents record the decision; and
- `docs/roadmap.md` marks `11.3.1` done only after the full validation suite
  passes.

## Constraints

- Scope is limited to roadmap item `11.3.1`.
- Existing public streaming APIs must remain backward compatible:
  `WireframeClient::call_streaming`, `WireframeClient::receive_streaming`,
  `client::ResponseStream`, `response::Response`, and `response::FrameStream`
  must continue to work unchanged.
- The new helper must be additive. Manual `while let Some(result)` loops remain
  supported.
- No single Rust source file may exceed 400 lines. Create new modules instead
  of expanding near-limit files.
- Public APIs must be documented with Rustdoc examples that satisfy
  `docs/rust-doctest-dry-guide.md`.
- Every new Rust module must begin with a module-level `//!` comment.
- The helper must preserve existing transport semantics:
  - terminator handling stays inside `ResponseStream`;
  - correlation validation stays inside `ResponseStream`;
  - TCP back-pressure remains unchanged; and
  - the exclusive `&mut WireframeClient` borrow held by `ResponseStream`
    remains visible and must not be hidden behind an API that implies
    concurrent I/O is safe.
- The helper must stay protocol-agnostic. Wireframe may supply generic stream
  adaptation, but downstream protocols remain responsible for deciding which
  frames are data, which are control, and how to decode them into domain values.
- Unit tests must use `rstest`.
- Behavioural tests must use `rstest-bdd` v0.5.0 with the repository's
  existing `feature + fixture + steps + scenarios` layout.
- BDD fixture parameter names in step definitions must match fixture function
  names exactly, and step parameters must not use underscore-prefixed fixture
  names.
- Design decisions must be added to the relevant design documentation:
  `docs/wireframe-client-design.md` for the client-facing API surface and
  `docs/multi-packet-and-streaming-responses-design.md` for the streaming model.
- Public interface changes must be added to `docs/users-guide.md`.
- `docs/roadmap.md` item `11.3.1` is updated to done only after full
  validation.
- Follow guidance from:
  - `docs/generic-message-fragmentation-and-re-assembly-design.md`
  - `docs/multi-packet-and-streaming-responses-design.md`
  - `docs/the-road-to-wireframe-1-0-feature-set-philosophy-and-capability-maturity.md`
  - `docs/hardening-wireframe-a-guide-to-production-resilience.md`
  - `docs/rust-testing-with-rstest-fixtures.md`
  - `docs/reliable-testing-in-rust-via-dependency-injection.md`
  - `docs/rstest-bdd-users-guide.md`
  - `docs/rust-doctest-dry-guide.md`

## Tolerances

- Scope: if implementing the helper requires a response-router or a true
  multi-request demultiplexer on one client connection, stop and escalate. That
  is a materially larger feature than `11.3.1`.
- Surface area: if more than 18 files or roughly 1200 net lines must change
  before documentation additions, stop and re-evaluate the design.
- Dependencies: do not add a new crate. Reuse `futures`, Tokio, and existing
  Wireframe types.
- API ambiguity: if a helper that works for both `client::ResponseStream` and
  `response::FrameStream` cannot be expressed cleanly, prefer the client-side
  API first and document the limitation rather than inventing a brittle
  abstraction. Escalate if this materially weakens the roadmap promise.
- Validation: if the same quality gate fails three focused fix attempts in a
  row, stop and document the blocker before proceeding.
- Time: if the implementation cannot reach a first red-green pass in one work
  session, stop after the API skeleton and failing tests, then request review.

## Risks

- Risk: `ResponseStream` is already typed over `P`, so a shallow helper could
  look redundant. Severity: high. Likelihood: high. Mitigation: define the
  helper around the real pain point: adapting a stream of protocol frames into
  a stream of domain items while optionally skipping control frames.

- Risk: a macro-first design would hide control flow and make errors harder to
  understand in downstream crates. Severity: medium. Likelihood: medium.
  Mitigation: prefer a trait plus named adapter type first; add a macro only if
  concrete usage still shows repeated boilerplate after the trait lands.

- Risk: adding methods directly to `ResponseStream` could push the file over
  the line-count limit and over-couple the transport receiver with higher-level
  adaptation. Severity: medium. Likelihood: medium. Mitigation: place the
  helper in a new module and keep `ResponseStream` focused on transport,
  correlation, and termination semantics.

- Risk: helper code may accidentally hide or re-wrap existing transport errors
  in ways that obscure whether failure came from the wire or from the
  protocol-specific mapper. Severity: medium. Likelihood: medium. Mitigation:
  keep mapper failures and transport failures distinct in the helper's return
  type and document that split explicitly.

- Risk: BDD scenarios can pass by exercising fixture-local convenience methods
  instead of the new public helper. Severity: medium. Likelihood: medium.
  Mitigation: the fixture must call the public helper API directly, and the
  feature text must describe observable typed-consumption behaviour.

- Risk: the existing `src/client/tests/streaming.rs` file is already 367
  lines. Severity: high. Likelihood: high. Mitigation: create a dedicated
  helper test module instead of extending the current file much further.

## Progress

- [x] (2026-03-21 00:00Z) Reviewed roadmap item `11.3.1`, the execplan
  authoring rules, project instructions, and stored project notes.
- [x] (2026-03-21 00:00Z) Read the referenced design, testing, and user-guide
  documents plus the existing 11.x execplans and current client streaming
  implementation.
- [x] (2026-03-21 00:00Z) Drafted this ExecPlan.
- [ ] Stage A: finalize the public helper shape and module boundaries.
- [ ] Stage B: add failing `rstest` unit tests and `rstest-bdd` behavioural
  scenarios for typed streaming consumption.
- [ ] Stage C: implement the helper trait, adapter type, exports, and Rustdoc
  examples.
- [ ] Stage D: update design docs, users' guide, roadmap, and run the full
  validation suite.

## Surprises & Discoveries

- Observation: `client::ResponseStream` already handles transport reads,
  deserialization, terminator detection, correlation validation, and request
  hook integration. Evidence: `src/client/response_stream.rs`. Impact: `11.3.1`
  should not add a second transport abstraction; it should layer *above* the
  existing typed frame stream.

- Observation: the real ergonomic gap is protocol-frame adaptation, not stream
  creation. The current documentation stops at manual `StreamExt::next` loops.
  Evidence: `docs/users-guide.md` "Consuming streaming responses on the client"
  section and `docs/multi-packet-and-streaming-responses-design.md` Section 12.
  Impact: the new helper must be documented as the recommended alternative to
  repeated manual loops.

- Observation: the existing streaming behavioural harness is already rich and
  covers ordered delivery, empty streams, correlation mismatch, disconnects,
  fairness, and rate limiting. Evidence:
  `tests/features/client_streaming.feature`,
  `tests/fixtures/client_streaming.rs`, and
  `tests/steps/client_streaming_steps.rs`. Impact: extend that harness with
  typed-consumption scenarios instead of creating a new BDD world.

- Observation: several current files are close to the 400-line limit:
  `src/client/tests/streaming.rs` is 367 lines, while
  `src/client/response_stream.rs` and `src/client/streaming.rs` are 196 and 168
  lines respectively. Evidence: `wc -l` inspection during planning. Impact: use
  new modules and focused test files.

## Decision Log

- Decision: prefer a trait-based helper with a named adapter type over a
  macro-first API. Rationale: traits are easier to document, test, and debug,
  and they preserve ordinary Rust control flow for downstream protocol crates.
  Date/Author: 2026-03-21 / planning.

- Decision: keep `client::ResponseStream` responsible only for transport,
  termination, and correlation guarantees. Put typed-consumption helpers in a
  new module and export them from `wireframe::client`. Rationale: this keeps
  the layering coherent and avoids bloating the receiver implementation.
  Date/Author: 2026-03-21 / planning.

- Decision: design the helper around "map frame -> `Option<Item>`" semantics,
  where `Some(item)` yields a typed item and `None` skips a control frame.
  Rationale: multiplexed protocols often interleave data frames with notices,
  progress frames, or keepalive packets that should not appear in the final
  item stream. Date/Author: 2026-03-21 / planning.

- Decision: preserve the existing client borrow semantics. The helper may wrap
  `ResponseStream`, but it must not imply that other client operations are
  usable concurrently while the stream is alive. Rationale: that behaviour is
  part of the current safety contract and is explicitly documented already.
  Date/Author: 2026-03-21 / planning.

- Decision: do not enlarge `prelude` for this milestone unless real examples
  show the helper is used pervasively enough to justify it. Rationale: the
  prelude is intentionally small and stable. Date/Author: 2026-03-21 / planning.

## Context and orientation

Current client streaming support already exists and is spread across a small
set of focused files:

- `src/client/streaming.rs` exposes `WireframeClient::call_streaming` and
  `WireframeClient::receive_streaming`.
- `src/client/response_stream.rs` implements the typed `ResponseStream` that
  owns transport polling, deserialization, correlation checks, and terminator
  handling.
- `src/client/mod.rs` re-exports the public client surface.
- `src/response.rs` defines `response::FrameStream` and
  `Response::into_stream()`, which matters if the helper is made generic enough
  to work outside the client-specific path as well.

Current tests and docs relevant to this work:

- `src/client/tests/streaming.rs`: unit coverage for the raw client streaming
  API.
- `tests/features/client_streaming.feature`: behavioural coverage for the
  current streaming path.
- `tests/fixtures/client_streaming.rs`,
  `tests/steps/client_streaming_steps.rs`, and
  `tests/scenarios/client_streaming_scenarios.rs`: the existing BDD harness.
- `docs/users-guide.md`: the user-facing streaming-consumption guide.
- `docs/wireframe-client-design.md`: the client API design record.
- `docs/multi-packet-and-streaming-responses-design.md`: the streaming
  lifecycle design record.

Terms used in this plan:

- Protocol frame: the typed `Packet` value already yielded by
  `client::ResponseStream`.
- Domain item: the higher-level value a downstream crate actually wants to
  consume, such as a row, event, or record.
- Control frame: a protocol frame that influences stream state or informs the
  caller, but should not itself be yielded as a domain item.
- Typed-consumption helper: the new public abstraction that converts a stream
  of protocol frames into a stream of domain items.

## Planned public API

The preferred shape is a new client-side extension trait plus a named adapter
stream. The exact names can be tuned during implementation, but the contract
should look like this:

```plaintext
trait StreamingResponseExt<P>: Stream<Item = Result<P, ClientError>> + Sized {
    fn typed_with<Item, Mapper>(
        self,
        mapper: Mapper,
    ) -> TypedResponseStream<Self, Mapper, P, Item>
    where
        Mapper: FnMut(P) -> Result<Option<Item>, ClientError>;
}

struct TypedResponseStream<S, Mapper, P, Item> { ... }
```

Intended usage:

```plaintext
let rows = client
    .call_streaming::<MyEnvelope>(request)
    .await?
    .typed_with(|frame| match frame.kind() {
        FrameKind::Row => Ok(Some(MyRow::try_from(frame)?)),
        FrameKind::Progress | FrameKind::Notice => Ok(None),
        FrameKind::Error => Err(convert_protocol_error(frame)),
    });

let collected = rows.try_collect::<Vec<_>>().await?;
```

Non-negotiable behavioural points for the helper:

1. It must preserve item ordering for yielded `Some(item)` values.
2. It must silently skip `Ok(None)` mapper results.
3. It must stop on the first mapper error and surface that error immediately.
4. It must propagate underlying stream errors unchanged.
5. It must not alter termination behaviour; once the wrapped stream returns
   `None`, the helper also returns `None`.

If a generic implementation over any `Stream<Item = Result<P, ClientError>>`
proves clean, use that. If not, implement the trait specifically for
`client::ResponseStream` first and document why the broader generic was
deferred.

## Plan of work

### Stage A: lock the helper boundary and file layout

Create a new client submodule dedicated to streaming-response helpers instead
of expanding `src/client/response_stream.rs`. Candidate filenames are
`src/client/streaming_helpers.rs` or `src/client/typed_stream.rs`; choose one
and keep the name aligned with the exported API.

Add the new module to `src/client/mod.rs` and re-export the public helper trait
and adapter type there. Do not add the helper to `src/prelude.rs` in this
milestone unless implementation evidence shows it is clearly high-frequency
enough to justify the extra surface.

Write the module-level `//!` comment first. It should explain that the helper
adapts typed protocol frames into domain items and does not change transport or
borrow semantics.

### Stage B: write the failing tests first

Add a new focused unit-test module instead of extending
`src/client/tests/streaming.rs`. A good target is
`src/client/tests/streaming_helpers.rs`, registered from
`src/client/tests/mod.rs`.

Unit tests should cover at least these cases:

1. A mapped data-only stream yields typed items in order.
2. A mapper can skip control frames by returning `Ok(None)`.
3. A mapper error terminates the helper stream and surfaces the exact error.
4. Underlying correlation mismatch, disconnect, and decode failures still
   propagate unchanged from the wrapped `ResponseStream`.
5. An empty stream remains empty after adaptation.

Use `rstest` fixtures and the existing `streaming_infra` helpers where that
reduces duplication. Do not move helper tests into `wireframe_testing`; that
crate is not a workspace member and its internal unit tests do not run through
the normal workspace gates.

Add behavioural coverage by extending the existing client streaming BDD suite:

- extend `tests/features/client_streaming.feature` with one scenario proving
  typed row consumption and one scenario proving control-frame skipping; and
- update `tests/fixtures/client_streaming.rs`,
  `tests/steps/client_streaming_steps.rs`, and
  `tests/scenarios/client_streaming_scenarios.rs` so the world calls the new
  public helper directly.

The BDD world should continue using the runtime-backed server/client fixture
pattern already established there. Do not create a new world unless the
existing fixture becomes incoherent.

### Stage C: implement the helper trait and adapter type

Implement the adapter as a normal `Stream` wrapper. Its `poll_next` should:

1. poll the wrapped stream;
2. forward `Pending` unchanged;
3. forward underlying `Err(ClientError)` unchanged;
4. repeatedly poll-and-map until either:
   - `Some(Ok(Some(item)))` is available, which should be yielded;
   - `Some(Ok(None))` is produced, which should be skipped; or
   - `Some(Err(...))` / `None` occurs, which should be returned; and
5. never swallow mapper errors.

Keep the implementation small and explicit. Avoid over-generalized trait
machinery if a straightforward wrapper is readable and sufficient.

Add Rustdoc examples to the public trait and adapter type. At least one example
must show a mapper that filters out control frames. Keep the example on the
public surface only so it participates cleanly in `make test-doc` and
`make doctest-benchmark`.

If the generic helper naturally works with `response::FrameStream` too, add a
small integration test under `tests/` proving that path. If not, do not force
it in `11.3.1`; note the narrower scope in the design docs and leave the
generic `Response::Stream` support as a follow-up.

### Stage D: update documentation and roadmap

Update `docs/wireframe-client-design.md` with:

- the rationale for choosing a trait + adapter type over a macro-first API;
- the explicit "map frame -> optional item" contract; and
- the reminder that the helper does not change the exclusive borrow semantics
  of `ResponseStream`.

Update `docs/multi-packet-and-streaming-responses-design.md` in the client-side
streaming section so the design doc reflects the new recommended consumption
pattern instead of only the manual `next()` loop.

Update `docs/users-guide.md` in the "Consuming streaming responses on the
client" section. Add:

- a short explanation of when to keep manual frame iteration;
- a helper-based example for multiplexed protocols; and
- a note that control frames can be skipped without losing transport error
  fidelity.

Only after implementation, docs, and validation are complete, mark roadmap item
`11.3.1` as done in `docs/roadmap.md`.

## Validation and evidence

Run the full validation flow with `set -o pipefail` and `tee`, keeping the logs
for review:

```sh
set -o pipefail && make fmt | tee /tmp/11-3-1-fmt.log
set -o pipefail && make markdownlint MDLINT=/root/.bun/bin/markdownlint-cli2 | tee /tmp/11-3-1-markdownlint.log
set -o pipefail && make check-fmt | tee /tmp/11-3-1-check-fmt.log
set -o pipefail && make lint | tee /tmp/11-3-1-lint.log
set -o pipefail && make test | tee /tmp/11-3-1-test.log
set -o pipefail && make test-doc | tee /tmp/11-3-1-test-doc.log
set -o pipefail && make doctest-benchmark | tee /tmp/11-3-1-doctest-benchmark.log
set -o pipefail && make nixie | tee /tmp/11-3-1-nixie.log
```

Before declaring success, confirm all of the following:

1. The new unit tests fail before implementation and pass after it.
2. The new BDD scenarios fail before implementation and pass after it.
3. Rustdoc examples for the helper compile under `make test-doc`.
4. `docs/users-guide.md`, the relevant design docs, and `docs/roadmap.md`
   all reflect the shipped API.

## Outcomes & Retrospective

To be completed during implementation. At minimum, record:

- the final public API names and whether the helper remained client-only or was
  generalized to `response::FrameStream`;
- whether a trait-only design was sufficient or a macro was also added;
- the validation commands that passed; and
- any follow-up work intentionally deferred to `11.3.2` or later.
