# Publish an in-process server and client pair test harness (12.3.2)

This ExecPlan (execution plan) is a living document. The sections
`Constraints`, `Tolerances`, `Risks`, `Progress`, `Surprises & Discoveries`,
`Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work
proceeds.

Status: DRAFT

## Purpose / big picture

Roadmap item `12.3.2` exists because client-facing tests currently repeat the
same setup in many places: bind a loopback listener, spawn a server task,
record its address, connect a `WireframeClient`, hold onto one or more join
handles, and remember to abort or shut everything down in `Drop`.

That pattern appears in unit tests, `rstest` fixtures, and `rstest-bdd` worlds.
It works, but it creates three problems for downstream protocol crates:

1. every crate has to rediscover the same server/client wiring;
2. the repeated harness code distracts from the compatibility behaviour being
   tested; and
3. subtle lifecycle bugs become easy to copy, especially around shutdown,
   address binding, and exclusive client borrows during streaming operations.

After this change, a downstream crate should be able to start a real Wireframe
server and a connected client inside one test process through a small public
testing API. "In-process" here means the server and client run in the same test
process, but they still communicate over a real loopback TCP socket. That keeps
the compatibility check honest while remaining fast and deterministic.

Observable success:

- `wireframe_testing` exposes a reusable public harness for starting a bound
  Wireframe server and a connected client pair;
- the harness is generic enough for downstream crates to supply their own app
  factory and client-builder configuration;
- `rstest` integration tests prove round-trip compatibility, custom builder
  configuration, and clean shutdown behaviour;
- `rstest-bdd` v0.5.0 scenarios prove the harness through the public API, not
  through private fixture shortcuts;
- relevant design documentation records the chosen API boundary and rationale;
- `docs/users-guide.md` explains when to use the harness and what public
  surface it adds for library consumers; and
- `docs/roadmap.md` marks `12.3.2` done only after the full validation suite
  passes.

## Constraints

- Scope is limited to roadmap item `12.3.2`.
- Existing public client APIs must remain backward compatible:
  `WireframeClient`, `WireframeClientBuilder`, `WireframeServer`,
  `wireframe::testkit`, and the existing `wireframe_testing` helpers must keep
  working unchanged.
- The new harness must be additive. Existing handwritten fixtures may remain
  where they cover behaviour outside this milestone.
- Prefer `wireframe_testing` as the primary public home for this feature. It is
  already the dev-facing companion crate for reusable testing helpers, and it
  avoids widening the `wireframe::testkit` feature surface unless Stage A
  proves that a main-crate export is required.
- Do not create a `wireframe -> wireframe_testing -> wireframe` dependency
  cycle. Any new API must preserve the current dependency direction.
- No single Rust source file may exceed 400 lines. Create new modules instead
  of extending already large client fixture files.
- Every new Rust module must begin with a module-level `//!` comment.
- Public APIs must be documented with Rustdoc examples that follow
  `docs/rust-doctest-dry-guide.md`.
- Unit coverage must use `rstest`.
- Behavioural coverage must use `rstest-bdd` v0.5.0 with the repository's
  existing `feature + fixture + steps + scenarios` layout.
- `rstest-bdd` step parameters that receive fixtures must match fixture names
  exactly, and fixture parameters must not use underscore-prefixed names.
- `wireframe_testing` is not a workspace member. Executable coverage for new
  `wireframe_testing` public APIs must therefore live in the main crate's
  `tests/` directory, not under `wireframe_testing/src/**`.
- The harness must use real loopback TCP, not `tokio::io::duplex`, because the
  purpose of the feature is client/server compatibility rather than in-memory
  app driving.
- Use `unused_listener()` plus `WireframeServer::bind_existing_listener(...)`
  when practical so the harness owns a deterministic address before spawning
  the server task.
- Shutdown must be explicit and leak-free. The harness must not leave orphaned
  server tasks or bound sockets after a test completes.
- Relevant design decisions must be recorded in the design docs most directly
  affected by this feature:
  - `docs/wireframe-client-design.md`
  - `docs/wireframe-testing-crate.md`
- `docs/users-guide.md` must describe the public harness surface for consumers.
- `docs/roadmap.md` item `12.3.2` is updated to done only after validation.
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

- Scope: if the public API cannot be implemented without also introducing
  response demultiplexing, transport abstraction, or a general-purpose async
  process supervisor, stop and escalate. Those are larger features than
  `12.3.2`.
- Surface area: if implementation requires changes to more than 16 files or
  roughly 1200 net lines before documentation updates, stop and re-evaluate.
- Dependencies: do not add a new crate. Reuse Tokio, existing Wireframe types,
  and the current `wireframe_testing` support modules.
- API ambiguity: if a single "pair" type becomes too generic to read or use,
  prefer a small pair-oriented façade over one or two lower-level helper pieces
  rather than forcing an over-abstract builder. Escalate if the consumer-facing
  API still feels unclear after the Stage A spike.
- Migration: if replacing existing client fixtures would cause churn unrelated
  to the new harness, stop after proving the new harness in dedicated tests and
  leave broad fixture migration as follow-up work.
- Validation: if the same quality gate fails three focused attempts in a row,
  stop and document the blocker before proceeding.
- Documentation baseline: if repo-wide `make markdownlint` still fails only on
  unrelated historical files, capture that evidence and additionally run
  targeted `markdownlint-cli2` against the files changed for this feature.

## Risks

- Risk: a high-level pair helper could become too rigid for tests that need
  custom client builder types, especially type-changing builder hooks such as
  `on_connection_setup`. Severity: high. Likelihood: medium. Mitigation:
  validate the API with a prototype that exercises at least one non-default
  client builder path before finalizing public signatures.

- Risk: placing the API in `wireframe::testkit` would widen an optional
  production crate feature for a capability that is primarily dev-only.
  Severity: medium. Likelihood: medium. Mitigation: keep `wireframe_testing` as
  the default home unless the spike shows a strong reason to mirror it from the
  main crate.

- Risk: the harness could duplicate logic already present in ad hoc client
  fixtures rather than factoring a coherent reusable core. Severity: medium.
  Likelihood: high. Mitigation: mine the shared lifecycle pattern first and
  only then decide which existing fixtures, if any, should be partially
  migrated to the new helper.

- Risk: server readiness races could make the harness flaky if the client
  attempts to connect before the listener is bound. Severity: high. Likelihood:
  medium. Mitigation: reserve the listener before spawning the server and bind
  via `bind_existing_listener`.

- Risk: drop-based cleanup alone may hide server shutdown failures. Severity:
  medium. Likelihood: medium. Mitigation: provide an explicit shutdown/join
  path and test it; `Drop` should remain a safety net, not the only lifecycle
  mechanism.

- Risk: new doctest examples could fail because the helper is in
  `wireframe_testing`, which is not a workspace member and is only exercised
  through integration tests. Severity: medium. Likelihood: medium. Mitigation:
  keep Rustdoc examples short and `no_run` where network setup is involved,
  while proving executable behaviour through root integration tests.

- Risk: the current client fixture files are already large
  (`tests/fixtures/client_runtime.rs` is 391 lines,
  `tests/fixtures/client_preamble.rs` is 485 lines). Severity: high.
  Likelihood: high. Mitigation: add a fresh dedicated fixture file for the new
  harness rather than appending to existing ones.

## Progress

- [x] (2026-03-29 00:00Z) Reviewed the roadmap item, project instructions, and
  `execplans` skill requirements.
- [x] (2026-03-29 00:00Z) Queried the project notes store and collected current
  gotchas about `wireframe_testing`, `rstest-bdd`, and validation commands.
- [x] (2026-03-29 00:00Z) Read the referenced design, testing, and doctest
  guidance documents.
- [x] (2026-03-29 00:00Z) Inspected existing client fixtures and internal test
  infrastructure to identify duplication and likely module boundaries.
- [x] (2026-03-29 00:00Z) Drafted this ExecPlan.
- [ ] Stage A: run a small API spike and finalize the public harness shape.
- [ ] Stage B: implement the `wireframe_testing` harness module and exports.
- [ ] Stage C: add `rstest` integration coverage.
- [ ] Stage D: add `rstest-bdd` behavioural coverage.
- [ ] Stage E: update design docs, users' guide, and roadmap.
- [ ] Stage F: run formatting, lint, test, doctest, and documentation
  validation gates.

## Surprises & Discoveries

- Observation: the repository already has strong in-memory app-driving helpers
  in `wireframe::testkit` and `wireframe_testing`, but client tests do not
  currently share a comparable public loopback TCP harness. Evidence:
  `src/testkit/support.rs`, `wireframe_testing/src/helpers/runtime.rs`, and the
  many ad hoc listener-based client fixtures under
  `tests/fixtures/client_*.rs`. Impact: `12.3.2` should focus on loopback
  server/client orchestration rather than inventing another in-memory driver.

- Observation: `wireframe_testing` already exposes exactly the supporting
  pieces needed for deterministic loopback startup: `TestResult`, test envelope
  types, and `unused_listener()`. Evidence:
  `wireframe_testing/src/integration_helpers.rs`. Impact: the new harness can
  compose with existing helpers instead of reintroducing them.

- Observation: `WireframeServer::bind_existing_listener(...)` solves the main
  address-race concern cleanly. Evidence: `src/server/config/binding.rs`.
  Impact: the harness should reserve the listener first, record `local_addr`,
  then spawn the server task.

- Observation: many client fixtures duplicate the same loopback setup while
  differing only in server behaviour or client builder customization. Evidence:
  `tests/fixtures/client_runtime.rs`, `tests/fixtures/client_messaging.rs`,
  `tests/fixtures/client_tracing.rs`, `tests/fixtures/client_request_hooks.rs`,
  and `tests/fixtures/client_send_streaming.rs`. Impact: the reusable harness
  should isolate lifecycle and connection concerns, leaving per-test behaviour
  to the app factory and optional configuration closures.

- Observation: `wireframe_testing` is not a workspace member, so internal
  `#[cfg(test)]` tests there would not be enough to validate the public API.
  Evidence: project notes and prior harness work for observability and
  reassembly helpers. Impact: plan integration tests in `tests/` from the start.

- Observation: the request referenced
  `reliable-testing-in-rust-via-dependency-injection.md` without the `docs/`
  prefix, but the file lives at
  `docs/reliable-testing-in-rust-via-dependency-injection.md`. Impact: use the
  correct path in implementation notes and future documentation updates.

## Decision Log

- Decision: treat `wireframe_testing` as the intended public home for the
  first version of this harness. Rationale: the feature is test-only, matches
  the companion crate's existing remit, and avoids expanding `wireframe`'s
  feature-gated `testkit` surface without proof that consumers need the API
  there. Date/Author: 2026-03-29 / planning.

- Decision: define "in-process pair" as a real loopback TCP connection between
  a spawned `WireframeServer` and a connected `WireframeClient`, both running
  inside one test process. Rationale: this verifies actual client/server
  compatibility while remaining deterministic and fast. Date/Author: 2026-03-29
  / planning.

- Decision: prototype the public API before settling exact type signatures.
  Rationale: the client builder has type-changing configuration methods, so a
  quick spike is the safest way to avoid an unreadable or overly rigid public
  signature. Date/Author: 2026-03-29 / planning.

- Decision: keep this milestone focused on lifecycle orchestration, not on
  replacing every existing client fixture. Rationale: broad migration would add
  churn and could obscure whether the new public harness itself is good enough.
  Date/Author: 2026-03-29 / planning.

## Repository orientation

The implementation will touch four main areas.

1. `wireframe_testing/src/**`

   This is the likely home for the new public harness. `src/lib.rs` re-exports
   the public API. `src/integration_helpers.rs` already provides `TestResult`
   and `unused_listener()`. Existing modules such as `echo_server.rs` show how
   small, focused public test helpers are structured.

2. `tests/**`

   Because `wireframe_testing` is not a workspace member, executable validation
   of new public helpers must be added as root integration tests. Follow the
   existing patterns:

   - direct `rstest` integration tests in `tests/*.rs`;
   - BDD worlds in `tests/fixtures/*.rs`;
   - step definitions in `tests/steps/*.rs`;
   - scenario bindings in `tests/scenarios/*.rs`; and
   - `.feature` files in `tests/features/*.feature`.

3. `docs/**`

   The feature should update:

   - `docs/wireframe-client-design.md` for the client/testing design record;
   - `docs/wireframe-testing-crate.md` for the companion crate's public design;
   - `docs/users-guide.md` for consumer-facing usage;
   - `docs/roadmap.md` to mark `12.3.2` done after validation.

4. Existing client fixture examples

   These files are the best source of real requirements and edge cases:

   - `tests/fixtures/client_runtime.rs`
   - `tests/fixtures/client_messaging.rs`
   - `tests/fixtures/client_tracing.rs`
   - `tests/fixtures/client_request_hooks.rs`
   - `tests/fixtures/client_streaming.rs`
   - `tests/fixtures/client_send_streaming.rs`

Terms used in this plan:

- App factory: a closure that builds a `WireframeApp` for the server.
- Pair harness: a helper that starts a bound Wireframe server, connects a
  client, and owns their shared lifecycle for the duration of a test.
- Downstream crate: a crate outside the main `wireframe` package that depends
  on `wireframe` and `wireframe_testing` to verify its protocol integration.

## Proposed public shape

Stage A exists to confirm the exact names, but the intended public shape is a
small, readable API rather than a large builder hierarchy.

The preferred direction is:

```rust,no_run
use wireframe_testing::client_pair::spawn_wireframe_pair;

# async fn example() -> wireframe_testing::TestResult<()> {
let mut pair = spawn_wireframe_pair(
    || build_app(),
    |builder| builder.max_frame_length(2048),
)
.await?;

let response: MyReply = pair.client_mut().call(&MyRequest::new()).await?;
pair.shutdown().await?;
# let _ = response;
# Ok(())
# }
```

If the spike proves that the builder closure above cannot express enough
server-side or client-side variation cleanly, fall back to a two-layer API:

1. a small, ergonomic `spawn_wireframe_pair(...)` convenience function for the
   common case; and
2. one lower-level helper that accepts explicit server and client startup
   closures and returns the same pair-handle type.

Either way, the pair handle should expose only the minimum needed behaviour:

- `client_mut()` (or an equivalent access method) so tests can call the public
  client API directly;
- `local_addr()` for diagnostics when useful;
- `shutdown().await` to stop the server and await its task; and
- a defensive `Drop` implementation that aborts only if explicit shutdown was
  skipped.

The harness should not hide the fact that streaming responses borrow the client
mutably. If a test begins a `ResponseStream`, that exclusive borrow should
remain visible in ordinary Rust code.

## Implementation stages

## Stage A: API spike and acceptance test design

Create a short-lived implementation spike on a fresh module to answer one
question: what is the smallest public API that can support both a default
round-trip test and a non-default client-builder configuration?

Use one focused integration test as the spike driver. The spike is successful
only if it proves all of the following:

- a server can be bound through `unused_listener()` and
  `bind_existing_listener(...)`;
- the client can be connected through a caller-supplied builder closure;
- the resulting handle owns server shutdown cleanly; and
- the public type signature remains readable in docs and examples.

If the spike shows that a single helper function is too rigid, promote the
two-layer API described above and record that decision in the `Decision Log`
before proceeding.

## Stage B: Implement the public harness in `wireframe_testing`

Create a new focused module, likely `wireframe_testing/src/client_pair.rs`.

The module should:

- define the public pair-handle type;
- implement the spawn helper(s);
- reserve a loopback listener up front via `unused_listener()`;
- start the server with explicit shutdown coordination;
- connect the client only after the listener is owned by the harness; and
- translate startup and shutdown failures into `TestError` / `TestResult`.

Keep the implementation small and explicit. Reuse existing helpers instead of
copying them:

- `unused_listener()` from `wireframe_testing::integration_helpers`;
- `WireframeServer::bind_existing_listener(...)` from the main crate; and
- the existing `TestResult` / `TestError` aliases already re-exported by
  `wireframe_testing`.

Update `wireframe_testing/src/lib.rs` to expose the new module and the intended
re-exports.

Add Rustdoc examples to the new public API. Networked examples should use
`no_run` or hidden scaffolding so they remain readable and doctest-safe.

## Stage C: Add `rstest` integration coverage

Add a dedicated integration test file in `tests/`, for example
`tests/client_pair_harness.rs`.

Use `rstest` fixtures to cover at least these cases:

1. default compatibility round trip

   Start a simple routed `WireframeApp`, connect through the harness, call the
   server with a concrete request type, and assert the typed response.

2. client-builder customization

   Prove the public API can configure at least one non-default client setting
   through the harness. A frame-length override is the minimum useful proof;
   request hooks or tracing config are acceptable if they make the integration
   more convincing without adding noise.

3. explicit shutdown

   Assert that calling `shutdown().await` completes without leaking the server
   task or leaving the listener bound.

4. drop safety net

   If practical without flakiness, verify that dropping the harness without an
   explicit shutdown does not hang the test suite.

Do not place these tests under `wireframe_testing/src/**`; they would not give
enough confidence for this companion-crate API.

## Stage D: Add `rstest-bdd` behavioural coverage

Add a small BDD suite that exercises the new public harness from the
perspective of a downstream crate author.

Recommended layout:

```plaintext
tests/features/client_pair_harness.feature
tests/fixtures/client_pair_harness.rs
tests/steps/client_pair_harness_steps.rs
tests/scenarios/client_pair_harness_scenarios.rs
```

Keep the scenarios focused on observable behaviour, not internal state.

Required behavioural coverage:

1. a downstream crate can start a real server/client pair and verify a
   request/response contract; and
2. a downstream crate can supply non-default client configuration through the
   harness and still complete the compatibility check successfully.

The BDD world must call the new public helper directly. Do not hide the new API
behind fixture-only helper functions that never exercise the exported surface.

## Stage E: Update design docs, users' guide, and roadmap

Update `docs/wireframe-client-design.md` with a decision record describing:

- why the pair harness lives in `wireframe_testing`;
- what "in-process" means for this feature; and
- how the harness relates to the existing client runtime and fixture patterns.

Update `docs/wireframe-testing-crate.md` with:

- the new module in the crate layout;
- the new public API and a short usage example; and
- the rationale for using real loopback TCP instead of `duplex`.

Update `docs/users-guide.md` with a consumer-facing section that explains:

- when to prefer the pair harness over handwritten loopback setup;
- the basic spawn/shutdown flow;
- any important lifecycle caveats, especially streaming's `&mut` client borrow;
  and
- the crate or feature dependency shape required to use it.

Only after all code and tests are complete, mark `docs/roadmap.md` item
`12.3.2` as done.

## Stage F: Validation and evidence capture

Run targeted tests first so failures are easier to diagnose, then the broader
quality gates. Use `set -o pipefail` and `tee` for every command so output is
preserved even when the terminal truncates.

Suggested targeted commands:

```sh
set -o pipefail && cargo test --test client_pair_harness 2>&1 | tee /tmp/12-3-2-client-pair-unit.log
```

```sh
set -o pipefail && cargo test --test bdd --all-features -- client_pair_harness 2>&1 | tee /tmp/12-3-2-client-pair-bdd.log
```

Required repo gates after the targeted tests pass:

```sh
set -o pipefail && make fmt 2>&1 | tee /tmp/12-3-2-fmt.log
```

```sh
set -o pipefail && make markdownlint MDLINT=/root/.bun/bin/markdownlint-cli2 2>&1 | tee /tmp/12-3-2-markdownlint.log
```

```sh
set -o pipefail && make check-fmt 2>&1 | tee /tmp/12-3-2-check-fmt.log
```

```sh
set -o pipefail && make lint 2>&1 | tee /tmp/12-3-2-lint.log
```

```sh
set -o pipefail && make test 2>&1 | tee /tmp/12-3-2-test.log
```

```sh
set -o pipefail && make test-doc 2>&1 | tee /tmp/12-3-2-test-doc.log
```

```sh
set -o pipefail && make doctest-benchmark 2>&1 | tee /tmp/12-3-2-doctest-benchmark.log
```

```sh
set -o pipefail && make nixie 2>&1 | tee /tmp/12-3-2-nixie.log
```

If repo-wide markdown linting still fails on unrelated historical files,
capture that in the plan's living sections and additionally run targeted
`markdownlint-cli2` against the files changed for this feature.

## Outcomes & Retrospective

Not started yet.
