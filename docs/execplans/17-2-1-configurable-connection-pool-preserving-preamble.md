# Implement hybrid client connection pooling with `bb8` and custom per-socket admission/fairness controls (17.2.1)

This ExecPlan (execution plan) is a living document. The sections
`Constraints`, `Tolerances`, `Risks`, `Progress`, `Surprises & Discoveries`,
`Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work
proceeds.

Status: COMPLETE (all validation commands currently pass)

## Purpose / big picture

Roadmap item `17.2.1` requires a configurable client connection pool that keeps
already-negotiated preamble state on reused sockets, enforces a bounded number
of admitted operations per socket, and recycles idle sockets to avoid stale
connections.

This plan adopts a hybrid implementation strategy:

- use `bb8` for connection lifecycle, idle reaping, and timeout mechanics; and
- layer Wireframe-specific per-socket admission and fairness policies on top.

After this change, a library consumer can build a pooled client from the
existing `WireframeClientBuilder`, configure pool size/in-flight/idle settings,
and acquire pooled client leases for request work. Reused leases stay on warm
connections (no repeated preamble handshake), while idle connections are
re-established automatically when they exceed the configured idle threshold.

Trade-off accepted for `17.2.1`: more functionality now with battle-tested pool
operations. If deeper per-socket multiplex control is needed later, the pool
internals will be reviewed and potentially revised in a follow-up phase.

Observable success:

- unit tests (using `rstest`) verify preamble preservation, per-socket
  in-flight admission limits, and idle connection recycling;
- behavioural tests (using `rstest-bdd` v0.5.0) verify the same behaviours
  through user-facing scenarios;
- `docs/users-guide.md` documents the public pool API and migration guidance;
- design decisions are recorded in `docs/wireframe-client-design.md`; and
- `docs/roadmap.md` marks `17.2.1` as done only after all quality gates pass.

## Constraints

- Scope is limited to roadmap item `17.2.1`.
- Existing `WireframeClient` single-connection APIs must remain backward
  compatible.
- No single source file may exceed 400 lines; extract submodules before
  crossing 390 lines.
- New pool code must preserve existing preamble semantics:
  - preamble runs once when a physical socket is created;
  - reused warm sockets must not replay preamble;
  - recycled sockets must run preamble again.
- Per-socket in-flight limiting must be explicit and configurable.
- Introduce `bb8` as the pool lifecycle dependency.
- Do not introduce additional pool crates beyond `bb8` for this milestone.
- Unit tests must use `rstest` fixtures/patterns.
- Behaviour-driven development (BDD) tests must use `rstest-bdd` v0.5.0 with
  the existing `feature + fixture + steps + scenarios` layout.
- BDD fixture parameter names in step functions must match fixture function
  names exactly.
- Design decisions must be added to `docs/wireframe-client-design.md`.
- Public API changes must be documented in `docs/users-guide.md`.
- `docs/roadmap.md` `17.2.1` is updated to done only after full validation.
- Follow guidance from:
  - `docs/generic-message-fragmentation-and-re-assembly-design.md`
  - `docs/multi-packet-and-streaming-responses-design.md`
  - `docs/the-road-to-wireframe-1-0-feature-set-philosophy-and-capability-maturity.md`
  - `docs/hardening-wireframe-a-guide-to-production-resilience.md`
  - `docs/rust-testing-with-rstest-fixtures.md`
  - `docs/reliable-testing-in-rust-via-dependency-injection.md`
  - `docs/rstest-bdd-users-guide.md`
  - `docs/rust-doctest-dry-guide.md`

## Tolerances (exception triggers)

- Scope: if implementation requires more than 30 files changed, stop and
  escalate.
- Size: if net change exceeds 2200 lines before tests/docs, stop and escalate.
- Interface: if any existing public API must break, stop and escalate.
- Dependencies: if a new crate other than `bb8` is required, stop and
  escalate.
- Ambiguity: if in-flight semantics cannot be made coherent without also
  implementing `17.2.2` fairness/pool-handle work, stop and escalate with
  alternatives.
- Iterations: if the same gate fails 3 times after focused fixes, stop and
  escalate.
- Time: if any stage exceeds 4 hours elapsed, stop and escalate.

## Risks

- Risk: `src/client/runtime.rs` is already 399 lines, so direct pool logic
  additions can violate file-size policy. Severity: high. Likelihood: high.
  Mitigation: add new pool-specific modules instead of extending runtime.

- Risk: “in-flight per socket” is easy to interpret inconsistently.
  Severity: medium. Likelihood: medium. Mitigation: define it explicitly in
  docs and code as admitted concurrent operations per physical socket, enforced
  by per-socket permits.

- Risk: idle-recycle tests can be flaky if tied to wall-clock time.
  Severity: medium. Likelihood: medium. Mitigation: use deterministic time
  control (`tokio::time::pause`/advance) or injected clock boundaries for tests.

- Risk: preamble callbacks may have side effects; reconnecting idle sockets
  replays those callbacks. Severity: medium. Likelihood: medium. Mitigation:
  document this lifecycle clearly in users-guide and design doc.

- Risk: BDD fixture wiring errors (name mismatch) can break all scenarios.
  Severity: medium. Likelihood: medium. Mitigation: keep fixture and step
  parameter names identical and avoid underscore-prefixed fixture parameters.

- Risk: `bb8`'s checkout model may not directly represent Wireframe's
  per-socket admission/fairness semantics. Severity: medium. Likelihood:
  medium. Mitigation: wrap pooled connections in a Wireframe-managed lease
  layer that enforces admission limits and fairness before socket use.

## Progress

- [x] (2026-03-05 17:47Z) Drafted ExecPlan for `17.2.1`.
- [x] (2026-03-06 09:42Z) Revised strategy to the hybrid `bb8` + custom
  admission/fairness approach.
- [x] Stage A: finalize pool interface and internal module boundaries.
- [x] Stage B: add failing unit and behavioural tests for the three roadmap
  behaviours.
- [x] Stage C: implement pool config, acquisition, in-flight limits, and idle
  recycling.
- [x] Stage D: update design docs, user guide, roadmap, and run full gates.

## Surprises & Discoveries

- Observation: there is no existing connection-pool implementation or pool
  design section beyond a short “future work” bullet in
  `docs/wireframe-client-design.md`. Evidence: repository search for
  `PoolHandle`, `connection pool`, and client-side pool code only matches
  roadmap/design placeholders. Impact: this plan defines initial pool semantics
  explicitly to avoid implicit assumptions leaking into `17.2.2`.

- Observation: `src/client/runtime.rs` is at 399 lines.
  Evidence: `wc -l src/client/runtime.rs`. Impact: all pool implementation must
  live in new modules/submodules.

- Observation: deterministic idle recycling was more reliable on the slot
  checkout path than by waiting for `bb8`'s background reaper alone. Evidence:
  targeted idle-recycle test initially stayed on the warm socket after virtual
  time advance; adding lazy recycle on checkout made the behaviour
  deterministic. Impact: the final implementation still uses `bb8` for socket
  lifecycle, but Wireframe now enforces idle recycle at acquire/use boundaries
  as well.

- Observation: `make markdownlint` failed earlier in planning because of
  unrelated baseline issues elsewhere in the repository, but those baseline
  failures no longer block the current tree. Evidence: the final validation run
  for this milestone now completes with `make markdownlint` exiting `0`.
  Impact: the final status remains complete and the validation commands listed
  in this plan are internally consistent.

## Decision Log

- Decision: introduce pooling via a new `connect_pool(...)` builder method,
  rather than changing `connect(...)` behaviour. Rationale: keeps existing
  client API stable and allows opt-in adoption. Date/Author: 2026-03-05 / plan
  phase.

- Decision: use a hybrid implementation with `bb8` for lifecycle/reaping/
  timeout mechanics and a Wireframe-owned wrapper for per-socket admission and
  fairness controls. Rationale: this delivers battle-tested pooling behaviour
  quickly while preserving Wireframe-specific control points. If deeper
  multiplex control is needed later, that can be reviewed without blocking
  `17.2.1`. Date/Author: 2026-03-06 / plan revision.

- Decision: define in-flight limiting as a per-socket admission bound enforced
  by permits (not transport-level multiplexing fairness). Rationale: satisfies
  `17.2.1` while keeping `17.2.2` (`PoolHandle` fairness) as a follow-on
  feature. Date/Author: 2026-03-05 / plan phase.

- Decision: perform idle recycling on acquisition path (lazy recycle) instead
  of adding a background sweeper task. Rationale: reduces lifecycle complexity
  and keeps behaviour deterministic in tests. Date/Author: 2026-03-05 / plan
  phase.

## Outcomes & Retrospective

Implemented with a slightly narrower lease API than the initial draft:

- `WireframeClientPool<S, P, C>` is the public pool type.
- `PooledClientLease<S, P, C>` forwards pooled request methods instead of
  dereferencing to `WireframeClient`.
- Each physical socket is backed by its own `bb8` pool with `max_size = 1`;
  Wireframe owns slot selection and per-socket admission permits above that.

Validation evidence:

- targeted unit tests for warm reuse, admission limiting, and idle recycle;
- targeted rstest-bdd scenarios for the same behaviours;
- passing `make check-fmt`, `make lint`, `make test`, `make test-doc`,
  `make doctest-benchmark`, and `make nixie`; and
- passing `make markdownlint MDLINT=/root/.bun/bin/markdownlint-cli2`.

## Context and orientation

Current client architecture is single-connection and lives under `src/client/`.
`WireframeClientBuilder::connect(...)` builds exactly one physical TCP
connection, optionally performs preamble exchange, and returns a
`WireframeClient`.

Relevant current files:

- `src/client/builder/connect.rs`: socket setup, preamble exchange, and
  runtime construction.
- `src/client/preamble_exchange.rs`: handshake semantics and callback flow.
- `src/client/runtime.rs`: request lifecycle (`send`, `receive`, `call`,
  `close`).
- `src/client/hooks.rs`: lifecycle/request hooks.
- `src/client/tests/*`: existing unit tests.
- `tests/features/client_*.feature`, `tests/fixtures/client_*.rs`,
  `tests/steps/client_*.rs`, `tests/scenarios/client_*_scenarios.rs`: existing
  rstest-bdd pattern.

Terms used in this plan:

- Physical socket: one established TCP connection backing a
  `WireframeClient` instance.
- Preamble state: negotiated handshake outcome and any leftover bytes captured
  through existing preamble exchange callbacks.
- In-flight admission: the count of operations currently admitted for
  execution on a socket.
- Idle recycle: replacing a socket that has been unused for longer than
  configured timeout.

## Plan of work

### Stage A: Interface and scaffolding (no behavioural change)

Add a dedicated pool surface and keep it isolated from existing runtime files.
Wire `bb8` in this stage as the lifecycle engine.

Create a new client pool module tree under `src/client/pool/` (for example
`mod.rs`, `config.rs`, `slot.rs`, `lease.rs`, and `pool.rs`) and expose public
types via `src/client/mod.rs`.

Add a `bb8` manager for Wireframe client sockets that encapsulates:

- connect path (socket options + preamble exchange + framed client creation);
- health check/reconnect rules; and
- lifecycle hooks for close/recycle paths.

Introduce a public configuration type:

```rust
pub struct ClientPoolConfig {
    // count of physical sockets managed by the pool
    pub fn pool_size(self, size: usize) -> Self;
    // max admitted operations per socket
    pub fn max_in_flight_per_socket(self, limit: usize) -> Self;
    // idle threshold before socket recycling
    pub fn idle_timeout(self, timeout: Duration) -> Self;
}
```

Add a new builder entrypoint in a new module (for example
`src/client/builder/pool.rs`):

```rust
pub async fn connect_pool(
    self,
    addr: SocketAddr,
    pool_config: ClientPoolConfig,
) -> Result<WireframeClientPool<...>, ClientError>
```

This stage ends when code compiles and no existing tests change behaviour.

Go/no-go check: if this surface cannot be added without breaking existing
builder generics or forcing API breakage, stop and escalate.

### Stage B: Test-first coverage (red before green)

Add unit tests (`rstest`) first, then behavioural tests (`rstest-bdd`), and
confirm at least one new assertion fails before implementation.

Unit test targets (new module `src/client/tests/pool.rs`):

- pooled reuse preserves preamble state (preamble callback count does not
  increase across warm reuse);
- per-socket in-flight admission limit is enforced;
- idle timeout causes recycle and re-runs preamble callback on new socket;
- recycled sockets remain functional for `call` after reconnect.

Behavioural suite files:

- `tests/features/client_pool.feature`
- `tests/fixtures/client_pool.rs`
- `tests/steps/client_pool_steps.rs`
- `tests/scenarios/client_pool_scenarios.rs`

And register new modules in:

- `tests/fixtures/mod.rs`
- `tests/steps/mod.rs`
- `tests/scenarios/mod.rs`

Use globally unique step text prefixed with `client pool` to avoid collisions.

Go/no-go check: if reliable idle-time assertions cannot be made deterministic,
add clock-control support before proceeding.

### Stage C: Implement pool mechanics

Implement the pool internals with clear ownership and RAII cleanup semantics.
Use `bb8::Pool` as the underlying connection lifecycle primitive.

Expected internal shape:

- `WireframeClientPool` wrapping a `bb8::Pool<WireframeConnectionManager>`;
- per-slot admission control (permit/semaphore style);
- per-slot exclusive client access for actual transport operations;
- lease object (`PooledClientLease`) that updates last-used timestamps and
  releases admission permits on drop;
- idle-recycle and timeout mechanics delegated to `bb8` configuration, with
  Wireframe rules enforced at acquire/use boundaries.

Operational rules:

- warm lease reuse must not run preamble again;
- recycle must create a fresh socket and run preamble once;
- setup hooks run per physical connection creation;
- teardown hooks run when recycled/closed sockets are dropped or explicitly
  closed.

If internal configuration must be cloned for reconnect, add narrowly scoped
`Clone` implementations/bounds required for pooled usage and document those
bounds in rustdoc.

Go/no-go check: if `bb8` cannot support required preamble-preserving reuse plus
idle recycling semantics without unacceptable hacks, stop and escalate with
alternatives before implementation continues.

### Stage D: Documentation, roadmap, and hardening

Update docs after implementation passes tests:

- `docs/wireframe-client-design.md`:
  add a new section/decision record for `17.2.1` covering pool semantics,
  in-flight definition, preamble lifecycle, and idle recycle strategy.
- `docs/users-guide.md`:
  add public API guidance, config reference row(s), and usage example for
  pooled connections.
- `docs/roadmap.md`:
  mark `17.2.1` as done.

Also add/adjust rustdoc examples for new public pool API and run doctest gates.

## Concrete steps

Run from repository root (`/home/user/project`). Use logs for every gate.

```plaintext
set -o pipefail
cargo test --all-features src::client::tests::pool 2>&1 | tee /tmp/17-2-1-unit-red.log
cargo test --test bdd --all-features -- client_pool 2>&1 | tee /tmp/17-2-1-bdd-red.log
```

Expected during Stage B: at least one new test fails before implementation.

```plaintext
set -o pipefail
make fmt 2>&1 | tee /tmp/17-2-1-fmt.log
make check-fmt 2>&1 | tee /tmp/17-2-1-check-fmt.log
make lint 2>&1 | tee /tmp/17-2-1-lint.log
make test 2>&1 | tee /tmp/17-2-1-test.log
make test-doc 2>&1 | tee /tmp/17-2-1-test-doc.log
make doctest-benchmark 2>&1 | tee /tmp/17-2-1-doctest-benchmark.log
make markdownlint MDLINT=/root/.bun/bin/markdownlint-cli2 2>&1 | tee /tmp/17-2-1-markdownlint.log
make nixie 2>&1 | tee /tmp/17-2-1-nixie.log
```

Expected final result: all commands exit `0` and logs contain no new warnings
or failures.

## Validation and acceptance

Acceptance is behavioural and must be demonstrated by tests and docs.

- Tests:
  - new `rstest` unit tests for pool reuse, in-flight limit, and idle recycle;
  - new `rstest-bdd` scenarios proving the same behaviours from feature files;
  - existing suite still passes via `make test`.
- Lint/format/type/doc:
  - `make fmt`
  - `make check-fmt`
  - `make lint`
  - `make test-doc`
  - `make doctest-benchmark`
  - `make markdownlint MDLINT=/root/.bun/bin/markdownlint-cli2`
  - `make nixie`
- Documentation:
  - `docs/users-guide.md` includes new consumer-facing pool interface details;
  - `docs/wireframe-client-design.md` records design decisions;
  - `docs/roadmap.md` marks `17.2.1` done.

## Idempotence and recovery

- All commands are safe to rerun.
- If a stage fails, fix only the failing stage and rerun that stage's commands
  before progressing.
- Keep staged changes small so failed attempts can be inspected with
  `git diff --stat` and `git diff`.
- Avoid destructive reset commands; if refactoring is needed for file-size
  limits, extract to new modules instead of reverting broad edits.

## Artifacts and notes

Capture implementation evidence in temporary logs under `/tmp/17-2-1-*.log`. At
minimum retain:

- first red test run proving tests fail before implementation;
- final green unit + BDD test runs;
- final full-gate outputs.

During implementation completion, store a qdrant project-memory note with:

- exact pool API names;
- key gotchas found (for example fixture naming or idle-time control);
- verification command set used.

## Interfaces and dependencies

Public interfaces expected at end of milestone:

```rust
// src/client/pool/config.rs
pub struct ClientPoolConfig { /* fields private */ }
impl ClientPoolConfig {
    pub fn default() -> Self;
    pub fn pool_size(self, size: usize) -> Self;
    pub fn max_in_flight_per_socket(self, limit: usize) -> Self;
    pub fn idle_timeout(self, timeout: Duration) -> Self;
}

// src/client/builder/pool.rs
impl<S, P, C> WireframeClientBuilder<S, P, C> {
    pub async fn connect_pool(
        self,
        addr: SocketAddr,
        pool_config: ClientPoolConfig,
    ) -> Result<WireframeClientPool<S, P, C>, ClientError>;
}

// src/client/pool/pool.rs
pub struct WireframeClientPool<S, P, C> { /* private */ }
impl<S, P, C> WireframeClientPool<S, P, C> {
    pub async fn acquire(&self) -> Result<PooledClientLease<S, P, C>, ClientError>;
    pub async fn close(self);
}

// src/client/pool/lease.rs
pub struct PooledClientLease<S, P, C> { /* private */ }
impl<S, P, C> PooledClientLease<S, P, C> {
    pub async fn send<M: EncodeWith<S>>(&self, message: &M)
        -> Result<(), ClientError>;
    pub async fn receive<M: DecodeWith<S>>(&self) -> Result<M, ClientError>;
    pub async fn call<Req, Resp>(&self, request: &Req)
        -> Result<Resp, ClientError>
    where
        Req: EncodeWith<S>,
        Resp: DecodeWith<S>;
    pub async fn send_envelope<M>(&self, envelope: M) -> Result<u64, ClientError>
    where
        M: Packet + EncodeWith<S>;
    pub async fn receive_envelope<M>(&self) -> Result<M, ClientError>
    where
        M: Packet + DecodeWith<S>;
    pub async fn call_correlated<M>(&self, request: M)
        -> Result<M, ClientError>
    where
        M: Packet + EncodeWith<S> + DecodeWith<S>;
}
```

`PooledClientLease` no longer dereferences to `WireframeClient`; it forwards
the pooled request methods directly so the pool retains ownership of per-socket
checkout and recycle behaviour.

Dependencies for this milestone:

- `bb8` for pool lifecycle/reaping/timeout behaviour.
- existing Tokio/Futures/Wireframe crates for admission/fairness wrapper.

## Revision note

- 2026-03-05: Initial draft created for roadmap item `17.2.1`, with staged
  implementation, test strategy (`rstest` + `rstest-bdd`), explicit tolerance
  gates, and required documentation/roadmap updates.
- 2026-03-06: Revised to a hybrid pool strategy: `bb8` for lifecycle/reaping/
  timeout mechanics plus Wireframe-specific per-socket admission and fairness
  controls. Captured trade-off: faster, battle-tested pooling now; revisit
  deeper per-socket multiplex control later if needed.
