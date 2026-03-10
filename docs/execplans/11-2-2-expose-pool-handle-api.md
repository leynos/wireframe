# Expose a `PoolHandle` API with fair pooled acquisition for logical sessions (11.2.2)

This ExecPlan (execution plan) is a living document. The sections
`Constraints`, `Tolerances`, `Risks`, `Progress`, `Surprises & Discoveries`,
`Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work
proceeds.

Status: DRAFT

## Purpose / big picture

Roadmap item `11.2.2` exists because the current pooled-client surface is still
too low-level for larger deployments. Today, all contention flows through
`WireframeClientPool::acquire()`. That API is correct for manual control, but
it gives the pool no durable notion of "logical session A" versus "logical
session B", so a busy caller can repeatedly reacquire leases and crowd out
other waiters.

After this change, a library consumer will be able to create a `PoolHandle`
from a pooled client and treat that handle as the fairness identity for one
logical session. When many handles contend for a small number of warm sockets,
the pool will admit them according to an explicit fairness policy while still
respecting the existing per-socket admission permits and serialized transport
access. Observable success is:

- unit tests written with `rstest` prove that handle-aware acquisition is fair,
  preserves back-pressure, and does not regress warm reuse or idle recycle;
- behavioural tests written with `rstest-bdd` v0.5.0 prove the same behaviour
  through user-visible scenarios;
- `docs/wireframe-client-design.md` records the `11.2.2` design decisions;
- `docs/users-guide.md` explains when to use `PoolHandle` instead of direct
  `pool.acquire()`; and
- `docs/roadmap.md` marks `11.2.2` as done only after all quality gates pass.

This plan deliberately keeps `PooledClientLease` as the low-level escape hatch
for split-phase workflows (`send` followed later by `receive`). `PoolHandle`
adds fairness around lease acquisition and may add one-shot convenience methods
only where they remain safe on a shared pool.

## Constraints

- Scope is limited to roadmap item `11.2.2`.
- Existing `WireframeClientPool`, `ClientPoolConfig`, and `PooledClientLease`
  APIs must remain backward compatible.
- The `pool` Cargo feature remains the opt-in gate for all pooled client code.
- Fairness must be additive above the existing slot-permit back-pressure. The
  new API must not bypass `max_in_flight_per_socket`, socket serialization, or
  idle recycle rules introduced in `11.2.1`.
- `PoolHandle` must represent a logical-session fairness identity, not a
  promise of transport affinity to one physical socket.
- Split-phase receive semantics must remain explicit. Do not imply that a
  shared `PoolHandle` can safely route arbitrary uncorrelated responses unless
  the implementation truly adds response demultiplexing.
- No single Rust source file may exceed 400 lines; extract submodules before
  crossing roughly 390 lines.
- Public APIs must be documented with Rustdoc examples that satisfy the doctest
  guidance in `docs/rust-doctest-dry-guide.md`.
- Unit tests must use `rstest`.
- Behavioural tests must use `rstest-bdd` v0.5.0 with the repository's
  existing `feature + fixture + steps + scenarios` layout.
- BDD fixture parameter names in step definitions must match fixture function
  names exactly, and step parameters must not use underscore-prefixed fixture
  names.
- Design decisions must be recorded in `docs/wireframe-client-design.md`.
- Public interface changes must be recorded in `docs/users-guide.md`.
- `docs/roadmap.md` `11.2.2` is updated to done only after full validation.
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

- Scope: if implementation requires a full response router for uncorrelated
  `receive()` traffic across shared handles, stop and escalate. That is a
  materially larger feature than `11.2.2`.
- Interface: if delivering `PoolHandle` requires removing or breaking
  `WireframeClientPool::acquire()` or `PooledClientLease`, stop and escalate.
- Surface area: if more than 20 files or roughly 1600 net lines change before
  documentation/test additions, stop and re-evaluate the design.
- Dependencies: do not add new crates for scheduling or fairness. Reuse Tokio,
  `bb8`, and existing std/futures primitives.
- Validation: if the same quality gate fails three focused fix attempts in a
  row, stop and document the blocker before proceeding.
- Time: if the implementation cannot reach a first red-green pass in one work
  session, stop after the scheduler prototype and request review on the chosen
  API boundary.

## Risks

- Risk: the current pool selection path uses `select_all` over waiter futures,
  so under contention it is scheduler-dependent rather than demonstrably fair.
  Severity: high. Likelihood: high. Mitigation: introduce an explicit handle
  scheduler with deterministic policy tests.

- Risk: `PooledClientLease` holds a slot permit for the lease lifetime, which
  means fairness can still be defeated by callers that keep leases idle for too
  long. Severity: high. Likelihood: medium. Mitigation: document that
  `PoolHandle` is the preferred fairness surface and add tests that release
  leases promptly after work.

- Risk: adding convenience methods directly on `PoolHandle` can accidentally
  promise unsafe split-phase semantics on a shared pool. Severity: high.
  Likelihood: medium. Mitigation: keep `PoolHandle` focused on fair
  acquisition, and only add convenience methods that encapsulate the whole
  request/response cycle.

- Risk: fairness semantics are easy to confuse with the existing round-robin
  slot rotation. Severity: medium. Likelihood: high. Mitigation: document the
  distinction clearly in code docs and the users' guide: slot rotation spreads
  work across warm sockets, while handle fairness orders contending logical
  sessions.

- Risk: deterministic tests for fairness can become flaky if they depend on
  runtime wake ordering. Severity: medium. Likelihood: medium. Mitigation: use
  explicit barriers, bounded channels, and recorded grant order rather than
  timing assertions.

- Risk: BDD scenarios can fail for fixture wiring reasons unrelated to pooling.
  Severity: medium. Likelihood: medium. Mitigation: mirror the established
  `tests/bdd_pool/` layout and keep fixture names identical across feature,
  scenario, and step files.

## Progress

- [x] (2026-03-10 00:00Z) Reviewed roadmap item `11.2.2`, the completed
  `11.2.1` pool implementation, relevant design/testing docs, and project notes.
- [x] (2026-03-10 00:00Z) Drafted this ExecPlan.
- [ ] Stage A: confirm the public API boundary and internal scheduler shape.
- [ ] Stage B: add failing unit and behavioural tests that define fairness and
  back-pressure behaviour.
- [ ] Stage C: implement `PoolHandle`, fairness policies, and scheduler-backed
  lease acquisition.
- [ ] Stage D: update design docs, users' guide, roadmap, and run full gates.

## Surprises & Discoveries

- Observation: the current `WireframeClientPool::acquire()` first tries an
  immediate permit, then races all slot waiters with `futures::select_all`.
  Evidence: `src/client/pool/client_pool.rs`. Impact: current contention is
  "works eventually" rather than a documented fairness contract.

- Observation: the existing `11.2.1` design already treats the pool as a
  hybrid of `bb8` lifecycle management plus Wireframe-owned admission control.
  Evidence: `docs/wireframe-client-design.md` decision record for `11.2.1`.
  Impact: `11.2.2` should extend the Wireframe-owned layer rather than fight or
  replace `bb8`.

- Observation: pooled BDD coverage already lives in a separate
  `tests/bdd_pool/` target gated behind `advanced-tests` and `pool`. Evidence:
  `Cargo.toml`, `tests/bdd_pool/mod.rs`. Impact: add new handle scenarios
  alongside the existing pool scenarios instead of inventing a new harness.

- Observation: `make test` and `make lint` already run with `--all-features`,
  so the pooled test target and any new pool docs are covered by the standard
  gates. Evidence: `Makefile`. Impact: no bespoke validation target is needed,
  only the standard gates plus Markdown/doc gates.

## Decision Log

- Decision: treat `PoolHandle` as the fairness identity for one logical
  session. Rationale: this directly addresses the roadmap requirement to
  multiplex many logical sessions without changing the lower-level lease API.
  Date/Author: 2026-03-10 / planning.

- Decision: keep `PooledClientLease` as the explicit low-level API for
  split-phase and transport-sensitive workflows. Rationale: a shared
  `PoolHandle` does not, by itself, solve response demultiplexing for later
  `receive()` calls. Date/Author: 2026-03-10 / planning.

- Decision: implement fairness as a pool-level policy, exposed through a new
  public policy type and configured through pooled-client configuration.
  Rationale: fairness is only coherent when every contending handle is governed
  by the same scheduler rules. Date/Author: 2026-03-10 / planning.

- Decision: prefer `PoolHandle::acquire(&mut self)` over a clone-heavy or
  unbounded queueing model. Rationale: one outstanding wait per handle gives
  natural back-pressure and keeps "one handle == one logical session"
  straightforward. Date/Author: 2026-03-10 / planning.

- Decision: any convenience methods added to `PoolHandle` must encapsulate an
  entire operation (for example `call`) and must be built atop fair lease
  acquisition. Rationale: this avoids implying that a shared handle can safely
  perform arbitrary split-phase I/O. Date/Author: 2026-03-10 / planning.

## Context and orientation

Current pooled-client code lives under `src/client/pool/` and is feature-gated
by `pool`.

Relevant files today:

- `src/client/pool/client_pool.rs`: public pool type plus current slot
  selection and acquisition logic.
- `src/client/pool/slot.rs`: one physical-socket slot with a `bb8` pool of
  size one and a per-slot semaphore.
- `src/client/pool/lease.rs`: `PooledClientLease` forwarding operations through
  a checked-out managed client connection.
- `src/client/pool/config.rs`: `ClientPoolConfig`.
- `src/client/builder/pool.rs`: `WireframeClientBuilder::connect_pool(...)`.
- `src/test_helpers/pool_client.rs`: shared test server and pooled-client
  builders.
- `src/client/tests/pool.rs`: unit tests for warm reuse, in-flight admission,
  idle recycle, and broken-connection recycle.
- `tests/features/client_pool.feature`, `tests/fixtures/client_pool.rs`,
  `tests/steps/client_pool_steps.rs`, and
  `tests/scenarios/client_pool_scenarios.rs`: current behavioural coverage.
- `docs/wireframe-client-design.md`: current client and pooling design record.
- `docs/users-guide.md`: pool-facing public documentation.

Terms used in this plan:

- Logical session: one consumer identity that wants fair access to pooled
  leases over time.
- Handle fairness: the rule used to decide which waiting logical session gets
  the next lease opportunity.
- Slot rotation: the existing round-robin choice of which physical socket to
  try first. This is not the same thing as handle fairness.
- Back-pressure: the property that callers must wait instead of creating
  unbounded queued work when all permits/sockets are busy.

## Planned public API

The target public shape is:

```rust
use wireframe::client::{
    ClientPoolConfig,
    PoolFairnessPolicy,
    PoolHandle,
    WireframeClient,
};

let pool = WireframeClient::builder()
    .connect_pool(
        addr,
        ClientPoolConfig::default().fairness_policy(PoolFairnessPolicy::RoundRobin),
    )
    .await?;

let mut session_a: PoolHandle<_, _, _> = pool.handle();
let mut session_b = pool.handle();

let lease_a = session_a.acquire().await?;
let lease_b = session_b.acquire().await?;
```

The exact policy names may change to match repository naming conventions, but
the semantics must stay:

1. A consumer can create a stable handle for one logical session.
2. That handle can acquire leases under an explicit fairness policy.
3. The fairness policy never bypasses existing slot permits or transport
   serialization.

Preferred policy set for this milestone:

- `RoundRobin`: give waiting handles turns in rotation, one successful grant
  per turn.
- `Fifo`: grant the next lease to the earliest waiting handle.

If implementation evidence shows one of these policies is redundant or cannot
be explained clearly to users, keep `RoundRobin` as the required default and
document the change in the decision log before simplifying.

## Plan of work

### Stage A: define the handle/scheduler boundary

Refactor the pooled client around an explicit shared inner state. The simplest
shape is for `WireframeClientPool<S, P, C>` to become a thin public wrapper
around `Arc<ClientPoolInner<S, P, C>>`, because both the pool and its handles
need access to shared slots and shared fairness state.

Add new pool submodules, likely:

- `src/client/pool/handle.rs`
- `src/client/pool/policy.rs` or `src/client/pool/fairness.rs`
- `src/client/pool/scheduler.rs`

Keep `client_pool.rs`, `lease.rs`, and `slot.rs` focused by extracting helpers
instead of growing any one file past the 400-line cap.

Implement a scheduler that owns:

- the fairness policy;
- a stable handle identifier for each `PoolHandle`;
- a wait queue or rotation structure of blocked handles; and
- a wake-up path that runs when a lease is dropped or a slot becomes newly
  available.

The scheduler must grant real leases, not merely "permission to try". A handle
that wins fairness but then races all other tasks for the actual slot permit
would not be observably fair. The intended flow is:

1. A handle registers itself as waiting.
2. The scheduler chooses the next waiting handle according to policy.
3. The scheduler acquires an actual slot permit and constructs a
   `PooledClientLease`.
4. The scheduler delivers that lease to the chosen handle.
5. Lease drop notifies the scheduler to service the next waiter.

Wire direct `pool.acquire()` through the same shared machinery where practical.
If direct acquire remains a thin low-level path, document clearly that strong
fairness guarantees apply to stable `PoolHandle` usage, not to repeated
stateless calls.

### Stage B: write the failing tests first

Add unit tests before implementing the scheduler. Keep them in a new focused
test module if `src/client/tests/pool.rs` would otherwise grow too large.

Required unit tests with `rstest`:

1. `round_robin_handles_share_one_socket_fairly`
   Two handles contend on a pool with `pool_size(1)` and
   `max_in_flight_per_socket(1)`. Recorded grant order must alternate by
   handle, not allow one handle to monopolize reacquisition.

2. `fifo_policy_preserves_wait_order`
   Three handles wait in a known order. Grants must follow that order under the
   FIFO policy.

3. `handle_acquire_respects_back_pressure`
   Holding a lease from one handle must keep later handles waiting rather than
   over-admitting extra work. The assertion is "still waiting", not a sleepy
   throughput benchmark.

4. `handle_path_preserves_warm_reuse_and_preamble`
   Repeated `handle.acquire()` on one logical session with one socket should
   reuse the warm connection and keep the preamble count at one.

5. `handle_path_recycles_after_idle_timeout`
   After paused-time advancement beyond the configured idle timeout, the next
   handle acquisition should reconnect and rerun the preamble exactly once.

Add behavioural tests under the existing pooled-client BDD harness. Create a
new feature and supporting files unless the existing feature remains concise:

- `tests/features/client_pool_handle.feature`
- `tests/fixtures/client_pool_handle.rs`
- `tests/steps/client_pool_handle_steps.rs`
- `tests/scenarios/client_pool_handle_scenarios.rs`
- include them from `tests/bdd_pool/fixtures.rs` and
  `tests/bdd_pool/scenarios.rs`

Required behavioural scenarios:

1. Two logical sessions alternate access fairly on one pooled socket.
2. FIFO fairness serves waiting sessions in arrival order.
3. A waiting session remains blocked until another session releases its lease.
4. Warm reuse and idle recycle still behave the same through the handle API.

Use the shared `PoolTestServer` fixture utilities where possible. Extend
`src/test_helpers/pool_client.rs` only with deterministic coordination helpers,
such as barriers or ordered event recording, and keep those helpers generic
enough for both unit and BDD coverage.

### Stage C: implement `PoolHandle` and fairness policies

Add the new public policy type to the pool module and thread it through
`ClientPoolConfig`. The config must stay ergonomic and builder-style, for
example:

```rust
let config = ClientPoolConfig::default()
    .pool_size(2)
    .max_in_flight_per_socket(1)
    .fairness_policy(PoolFairnessPolicy::RoundRobin);
```

Implement `WireframeClientPool::handle()` to create a new logical-session
handle registered with the scheduler. The handle itself should be lightweight
and should borrow or clone only the shared inner state plus its own stable
identifier.

Implement `PoolHandle::acquire(&mut self)` to:

1. register the handle as waiting if no immediate lease is available;
2. await a scheduler-delivered lease; and
3. return the same `PooledClientLease` type that existing callers already use.

Keep the release path explicit. Lease drop must:

1. release the per-slot semaphore permit as it does today;
2. preserve the existing last-used / broken-connection bookkeeping; and
3. notify the scheduler that another waiting handle may now be serviceable.

If convenience methods are added on `PoolHandle`, restrict them to whole
operations that are safe atop fair acquisition, such as `call`. Do not add
`receive()` or `receive_envelope()` on `PoolHandle` unless the implementation
also adds principled response routing, which is outside this milestone's
intended scope.

### Stage D: documentation, doctests, and roadmap updates

Update `docs/wireframe-client-design.md` with a new `11.2.2` decision record
covering:

- why `PoolHandle` represents logical-session fairness instead of socket
  affinity;
- how fairness differs from the existing slot-rotation logic;
- which policy becomes the default and why; and
- why split-phase APIs remain on `PooledClientLease`.

Update `docs/users-guide.md` so library consumers know:

- when to use `pool.handle()` instead of repeated `pool.acquire()`;
- what fairness policy means operationally;
- that `PoolHandle` preserves existing back-pressure rather than creating a
  separate queue outside the pool; and
- that `PooledClientLease` is still the correct surface for explicit
  split-phase workflows.

Add or update Rustdoc examples on every new public type and method. Keep the
examples realistic and runnable so `make test-doc` and `make doctest-benchmark`
stay green.

Finally, mark roadmap item `11.2.2` done in `docs/roadmap.md` only after all
quality gates pass.

## Validation and evidence

Run every gate through `tee` with `set -o pipefail` so failures are visible and
logs remain inspectable:

```bash
set -o pipefail && make check-fmt | tee /tmp/wireframe-11-2-2-check-fmt.log
set -o pipefail && make lint | tee /tmp/wireframe-11-2-2-lint.log
set -o pipefail && make test | tee /tmp/wireframe-11-2-2-test.log
set -o pipefail && make test-doc | tee /tmp/wireframe-11-2-2-test-doc.log
set -o pipefail && make doctest-benchmark | tee /tmp/wireframe-11-2-2-doctest-benchmark.log
set -o pipefail && make markdownlint MDLINT=/root/.bun/bin/markdownlint-cli2 | tee /tmp/wireframe-11-2-2-markdownlint.log
set -o pipefail && make nixie | tee /tmp/wireframe-11-2-2-nixie.log
```

Observable completion criteria:

- the new unit tests fail before the scheduler work and pass afterward;
- the new BDD scenarios fail before the scheduler work and pass afterward;
- no existing pooled-client tests regress;
- doctests for the new public API compile and run; and
- `docs/roadmap.md` shows `11.2.2` as done.

## Outcomes & Retrospective

Pending implementation. Complete this section at the end of the feature with:

- the final public API surface;
- any deviations from this draft plan;
- validation results with the exact commands that passed; and
- lessons learned about fairness, pool ergonomics, and follow-up work.
