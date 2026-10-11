# Establish runtime ownership and task-churn baselines

This ExecPlan (execution plan) is a living document. The sections `Constraints`,
`Tolerances`, `Risks`, `Progress`, `Surprises & Discoveries`, `Decision log`,
`Outcomes & retrospective`, `Conformance basis`, and `Verification plan` must
be kept up to date as work proceeds.

Status: APPROVED

## Purpose / big picture

Epic [#635](https://github.com/leynos/wireframe/issues/635) proposes changing
the ownership topology of four hot paths: the connection actor's event loop,
the push-handle send path, application connection startup, and client-pool
acquisition. Those changes are justified primarily by clearer lifetime
boundaries, but they also claim measurable reductions in atomic reference-count
operations, allocations, route preparation, and scheduler task churn.

After this change, a maintainer can run one command and obtain a reproducible
baseline for each of those four paths, on a named machine, with the compiler
profile, feature set, queue capacity, pool size, and runtime flavour recorded
alongside the numbers. Factory invocations and middleware-chain transformations
are asserted by counters rather than inferred from timing. Pool scheduler task
creation is directly observable through a process-wide counter and through
Tokio's own live-task gauge. Allocation counts for each path are recorded so a
later change that removes a nested `Arc` can be checked against a number rather
than an intuition.

The harnesses stay useful after the refactor: the scenarios are named by
behaviour (`actor/drain_no_shutdown`, `pool/uncontended_size_4`,
`startup/prepared_root`), so a post-refactor run compares like with like
without rewriting the scenario list.

## Constraints

Hard invariants that must hold throughout implementation.

- Reuse the existing benchmark stack. Codec benchmarks use Criterion 0.8.2 with
  `harness = false` and explicit `fn main()`; allocation baselines use a
  counting `GlobalAlloc` gated by an `AtomicBool`. New work mirrors those two
  patterns rather than introducing a second framework.
- Do not make benchmark results a correctness gate. No CI workflow may run
  `cargo bench`, and no absolute-timing threshold may be asserted in a test.
- No transport I/O inside the actor, push, or startup measurements. The
  client-pool scope is the single documented exception, because the pool only
  behaves realistically against a real loopback peer.
- The rate limiter must not dominate the push measurements. Every push
  measurement builds its queues with `PushQueuesBuilder::unlimited()`.
- `src/test_helpers.rs` is gated `#![cfg(any(test, feature = "test-support"))]`,
  and `wireframe::test_helpers` is invisible to `wireframe_testing` unless the
  consuming build also activates `wireframe/test-support`. New support modules
  must not create a build configuration in which
  `cargo check -p wireframe_testing` fails.
- Do not hand-edit `typos.toml`. Do not "fix" Markdown that a locally newer
  `mdtablefix` flags. Do not add `cargo bench` to CI.
- Nothing in this change may alter runtime behaviour of the four measured
  paths. The only production-code addition permitted is an observability
  counter.

## Tolerances (exception triggers)

Thresholds that trigger escalation when breached.

- Scope: if implementation requires changes to more than 25 files or 2500 net
  lines, stop and escalate.
- Interface: if an existing public API signature must change, stop and
  escalate. Additive public items are permitted and are listed in the Decision
  log.
- Dependencies: if a new external crate must be added to any manifest, stop and
  escalate. Adding an optional *feature* to `wireframe_testing` is permitted.
- Iterations: if the same gate fails three times after three distinct fixes,
  stop and escalate.
- Ambiguity: if a benchmark scenario cannot be made deterministic in its
  postconditions, stop and present the options rather than shipping a scenario
  whose pass condition is timing-dependent.

## Risks

- Risk: Criterion's automatic calibration in a self-timed benchmark can make a
  single `cargo bench` run very long when an iteration is heavy (pool scope
  performs real TCP connects). Severity: medium. Likelihood: high. Mitigation:
  pin `sample_size` and `measurement_time` per group, and record the pinned
  values in the baseline document so runs are comparable.

- Risk: the counting global allocator records allocations from the Tokio
  runtime, Criterion's harness, and the benchmark closures, not just the code
  under test. Severity: medium. Likelihood: certain. Mitigation: this is
  already the documented behaviour of `benches/codec_performance_alloc.rs`;
  state the caveat again in the new benchmark module docs and treat the numbers
  as relative baselines. For the actor and push scopes, bound the window to the
  synchronous body of a single `block_on` so runtime setup is excluded.

- Risk: the pool scope's "prove task count remains stable" obligation could be
  discharged vacuously if the live-task gauge never changes in either
  direction. Severity: high. Likelihood: medium. Mitigation: pair the stability
  assertion with a witnessed increase — a contended scenario that must observe
  at least one scheduler spawn. See `V-O5` in the Verification plan.

- Risk: relocating `ConnectionStartupInstrumentation` out of
  `tests/prepared_app.rs` breaks `tests/prepared_app/tcp.rs`, which imports it
  through `super::`. Severity: low. Likelihood: high. Mitigation: re-export the
  shared items from the `prepared_app` test module root so the `super::` path
  keeps resolving; this is a one-line re-export, not a compatibility shim.

- Risk: `wireframe_testing`'s new feature could accidentally enable
  `wireframe/test-support` for the Loom lane, changing what the Loom models
  compile. Severity: low. Likelihood: low. Mitigation: the Loom lane builds only
  `wireframe-loom` (`make test-loom` runs `cargo test -p wireframe-loom`), and
  `wireframe-loom` does not depend on `wireframe_testing`. Verify with
  `make test-loom`.

## Progress

- [x] (2026-10-11 12:39Z) Reconnaissance of all four scopes, the existing
  benchmark patterns, the crate feature graph, and ADR 011/012/013 verification
  sections.
- [x] (2026-10-11 12:39Z) Empirically decided the `wireframe_testing` →
  `wireframe::test_helpers` visibility question with a throwaway probe, then
  reverted it.
- [x] (2026-10-11 13:05Z) Wrote this ExecPlan; cleared 13 markdownlint issues
  (MD013 x4, MD029 x8, MD014 x1) found on the first gate run.
- [x] (2026-10-11 13:20Z) Milestone EP-M1: spawn counter, shared startup
  instrumentation in `src/test_helpers/connection_startup.rs`, reusable
  allocation counting in `wireframe_testing`, and migration of all four
  prepared-app test crates onto the shared fixtures. Workspace compiles under
  `--all-targets --all-features`.
- [x] (2026-10-11 15:40Z) EP-M1 gate remediation, in three rounds: (1) fixed
  seven rustfmt diffs plus `clippy::panic_in_result_fn`; (2) applied
  `mdtablefix` to this ExecPlan after `make check-fmt` flagged 134 changed
  lines; (3) moved the spawn counter out of `scheduler.rs` into a new
  `scheduler_spawns.rs` after whitaker's `module_max_lines` lint reported that
  module at 403 lines against a 400-line limit. `make check-fmt` and
  `make lint` are both green, including the `whitaker --all` step that never
  ran on the first attempt.
- [x] (2026-10-11 16:50Z) Milestone EP-M2: four benchmark support modules in
  `wireframe_testing` (`connection_actor_benchmark_support`,
  `push_benchmark_support`, `connection_startup_benchmark_support`,
  `client_pool_benchmark_support`) behind the new `runtime-benchmarks` feature,
  each exposing a workload type and a single fallible `measure_*` entry point.
  28 crate unit tests pass under `RUSTFLAGS="-D warnings"`, and
  `cargo check -p wireframe_testing` still passes with default features. A
  failing push test exposed that `try_push` reports `Ok(())` for a frame a drop
  policy dead-letters, so `PushMeasurement` reports `accepted`, `drained`, and
  `dead_lettered` separately; three further doctest defects in the new modules
  (a mis-aliased constant, a wrong policy assertion, and an unwinnable
  allocator assertion) were found and fixed by running the crate's doctests
  directly.
- [ ] Milestone EP-M3: the two bench binaries, the Makefile target, the rstest
  assertion suite, and the baseline document.
- [ ] Capture the baseline run and fill in the baseline table.
- [ ] Run every commit gate, then CodeRabbit review; clear all concerns.
- [ ] Push over SSH and open a draft pull request.

## Surprises & discoveries

- Observation: `wireframe_testing` *can* name `wireframe::test_helpers` only
  when the consuming build also activates `wireframe/test-support`. A probe
  appended to `wireframe_testing/src/lib.rs` compiled under
  `cargo check --bench connection_startup` (the root dev-dependency enables
  `test-support`) but failed under `cargo check -p wireframe_testing` with
  `could not find test_helpers in wireframe ... the item is configured out`.
  Evidence: the two command transcripts above, then reverted; `git status` was
  clean afterwards. Impact: the new benchmark-support modules cannot reference
  `wireframe::test_helpers` unconditionally. They are gated behind a new
  optional feature, `wireframe_testing/runtime-benchmarks`, which explicitly
  enables `wireframe/test-support` and `wireframe/pool`.

- Observation: `tokio::runtime::Handle::metrics()` and
  `RuntimeMetrics::num_alive_tasks()` are available on Tokio 1.53.1 with no
  extra Cargo feature. A probe test asserted `before=0 during=1` for a spawned
  task. Evidence: `tokio-1.53.1/src/runtime/handle.rs:515` is inside the plain
  `impl Handle` block, and `RuntimeMetrics::new` is unconditional in
  `runtime/metrics/runtime.rs`. Impact: the pool task-count obligation can use
  the runtime's own gauge without adding `rt-metrics` to any manifest, which
  keeps the change inside the dependency tolerance.

- Observation: `cargo check --test <name>` builds only that test target, so a
  wiring mistake in one test file does not mask itself behind unrelated
  failures. Evidence: the probe compile finished in 0.73 s and reported
  `Finished dev profile`. Impact: use this for fast, decisive compile checks
  during implementation.

- Observation: `benches/connection_startup.rs` **already exists** and is
  registered as a `[[bench]]` with `harness = false`. It landed in #642/#678
  ("Prepare applications before server readiness") and already benchmarks the
  old per-connection path against a shared prepared root, with its own private
  `StartupCounters` and `CountingTransform`. It is in the issue #639 reference
  list, but the plan's Task 3 assumed a greenfield pair of bench binaries.
  Evidence: `git log --oneline -- benches/connection_startup.rs` reports
  `2892081 Prepare applications before server readiness (#642) (#678)`;
  `git ls-files benches/` confirms it is tracked and unmodified. Impact: scope
  3 is partly covered already. Rather than duplicate it,
  `benches/ runtime_ownership_baselines.rs` extends coverage to the dimensions
  the issue actually asks for — a route-count x middleware-count x
  connection-count grid, and the counting assertions — and the pre-existing
  binary stays untouched so its published baselines are not silently renumbered.
  `build_app` there also returns the bare `WireframeApp` alias rather than the
  concrete `WireframeApp<BincodeSerializer, (), Envelope>`, which is why its
  local fixtures cannot simply be deleted in favour of the shared ones.

- Observation: `make lint` runs three steps in one target — rustdoc, clippy,
  then `whitaker --all`. When clippy aborts, `whitaker` never executes, so a
  failed lint gate leaves whitaker's verdict *unknown*, not passing. Evidence:
  the first gate run exited 101 at the clippy step and the log contains no
  whitaker output at all. Impact: after fixing a clippy failure, `make lint`
  must be re-run in full rather than assumed green, and the gate report must
  distinguish "did not run" from "passed".

- Observation: `clippy::panic_in_result_fn` (workspace-denied) fires on
  `assert` macros inside a `Result`-returning `#[tokio::test]`. Evidence: the
  first lint run failed at `src/test_helpers/connection_startup.rs:372` with
  `-D clippy::panic-in-result-fn`. Impact: fallible tests in this crate use a
  `-> ()` signature with `.expect(...)`, which the workspace's
  `allow-expect-in-tests` policy permits; the fixture's two error types
  (`WireframeError` from `build_counted_app`, `PrepareError` from `prepare`) do
  not unify, so a fallible signature would have needed boxing anyway. Note this
  contradicts the blanket guidance in `AGENTS.md` that fallible tests should
  return `Result`; when the test also asserts, the two rules conflict and
  `panic_in_result_fn` wins because it is denied.

- Observation: the local `mdtablefix` (0.6.1) and the CI pin are the *same*
  version, so its complaint about this ExecPlan was genuine rather than the
  version drift seen previously. Evidence: `.github/workflows/ci.yml` passes
  `version: "0.6.1"` to the install action, and `mdtablefix --version` reports
  `0.6.1`. Impact: the earlier belief that a locally flagged Markdown file is
  usually a stale copy does not apply here; the fix was to run
  `mdtablefix --in-place` on the plan, which changed 134 lines (prose
  rewrapping) and left all 17 sections intact.

- Observation: whitaker's `module_max_lines` lint enforces the 400-line module
  limit from `AGENTS.md` as a *hard error* under `-D warnings`, counting the
  whole file. Evidence: the second lint run failed with
  `error: Module scheduler spans 403 lines, exceeding the allowed 400` at
  `src/client/pool/mod.rs:14`; the addition that pushed it over was a 28-line
  static plus accessor. Impact: the spawn counter moved into its own
  `src/client/pool/scheduler_spawns.rs`, mirroring the existing
  `src/connection/counter.rs`, taking `scheduler.rs` from 403 back to 376
  lines. New observability code in this crate should start in its own module.

- Observation: `make lint` runs three steps (rustdoc, clippy, whitaker) and
  whitaker shells out to a *different* pinned toolchain with a dylint driver.
  Evidence: the successful run shows
  `Checking with toolchain nightly-2026-05-28` followed by its own
  `cargo check` after clippy finished. Impact: whitaker needs a generous
  timeout and its own compiler cache; a lint run that appears to stall after
  clippy completes is whitaker compiling, not a deadlock.

- Observation: `PushHandle::try_push` returns `Ok(())` when a drop policy
  routes the frame to the dead-letter queue, so "the send reported success" and
  "the frame was queued" are **not** the same event. Evidence:
  `src/push/queues/handle.rs:332-343` matches `TrySendError::Full` and, for
  `DropIfFull` and `WarnAndDropIfFull`, calls `self.route_to_dlq(f)` and then
  returns `Ok(())`; only `ReturnErrorIfFull` returns
  `Err(PushError::QueueFull)`. Found by a failing test that asserted
  `delivered + dead_lettered == sends`; it reported `16` for 8 sends because
  each dropped frame incremented both counters. Impact: the measurement reports
  `accepted`, `drained`, and `dead_lettered` as three separate figures. An
  implication for the planned ADR work is that a producer cannot distinguish
  "queued" from "dropped" using the return value alone, which is worth weighing
  if #637/#638 change the policy surface.

- Observation: `ConnectionActor` retains **no** producer handle, so draining a
  preloaded queue to completion is enough to terminate the actor without any
  cancellation. Evidence: `src/connection/mod.rs` — the struct has no `handle`
  field; `with_hooks` passes it by value into
  `hooks.on_connection_setup(handle, &mut ctx)`. `ActorState::new` combines a
  `closed_sources: 1` start (the shutdown token) with `total_sources: 3` when
  there is no response and no multi-packet source, so high + low closing
  reaches the terminal count. Impact: the actor scope drops the producer handle
  to close the queues and needs no producer-side keepalive, which keeps the
  measured region to frame processing alone.

- Observation: `PushError` is `#[non_exhaustive]`, so every `match` on it in a
  downstream crate needs a catch-all arm even though only two variants exist
  today. Evidence: two `E0004 non-exhaustive patterns` errors while writing
  `push_benchmark_support.rs`. Impact: the catch-all arms are written to report
  an unexpected variant as a diagnostic rather than silently folding it into an
  existing bucket, so a future third variant surfaces as a failure instead of
  quietly changing a baseline.

- Observation: `Ord::max` is not callable in a `const fn` on this toolchain
  (`E0658:`Ord` is not stable as a `const trait``), matching the earlier
  `usize::max` finding. Evidence: `E0658` raised on
  `client_pool_benchmark_support.rs:121`. Impact: const helper functions use
  explicit `if x < y { y } else { x }` comparisons instead.

## Decision log

- Decision: place the client-pool benchmark support in
  `wireframe_testing/src/runtime_ownership_benchmarks/` behind a new optional
  feature `wireframe_testing/runtime-benchmarks`, which enables
  `wireframe/test-support` and `wireframe/pool`. Rationale: the reviewed coding
  plan places all measurement helpers in `wireframe_testing` mirroring
  `codec_benchmarks`, and the pool scope must reuse
  `wireframe::test_helpers::pool_client::PoolTestServer`. The feature makes
  that dependency explicit, keeps `cargo check -p wireframe_testing` working
  with default features, and avoids adding `async-trait` to
  `wireframe_testing`. Alternative rejected: duplicating the preamble server in
  `wireframe_testing`, which would fork test infrastructure the issue asks us
  to reuse. Date/Author: 2026-10-11, implementer.

- Decision: put `ConnectionStartupInstrumentation`,
  `ConnectionStartupCounts`, `TransformCountingMiddleware`, and
  `counted_app_factory` in the `wireframe` crate at
  `src/test_helpers/connection_startup.rs`, not in `wireframe_testing`.
  Rationale: `Transform` is declared with `#[async_trait]`, so any
  implementation requires the `async-trait` crate; it is already a dependency of
  `wireframe` and is not a dependency of `wireframe_testing`. The reviewed
  coding plan explicitly permits `src/test_helpers` as the destination. The
  benchmark-facing measurement helpers still live in `wireframe_testing` and
  re-export the instrumentation. Date/Author: 2026-10-11, implementer.

- Decision: expose the pool scheduler spawn counter as a private
  `static AtomicU64` with a public
  `#[must_use] fn pool_scheduler_task_spawns()`, living in its own
  `src/client/pool/scheduler_spawns.rs`, rather than adding a counter to
  `src/metrics.rs` or inlining it in `scheduler.rs`. Rationale:
  `src/metrics.rs` is the `metrics`-crate facade; its counters are no-ops
  unless a recorder is installed, and benchmark binaries install none, so a
  `metrics` counter would be unobservable exactly where the issue needs it. The
  issue's acceptance criterion is satisfied by "test instrumentation or a
  stable runtime counter". Inlining it in `scheduler.rs` was tried first and
  rejected by whitaker's `module_max_lines` lint at 403 lines; the separate
  module keeps `scheduler.rs` at 376 and mirrors the existing
  `connection::counter` arrangement. Precedent:
  `connection::active_connection_count` is public for the same reason. This is
  an additive public item; recorded here as required by the interface
  tolerance. Date/Author: 2026-10-11, implementer.

- Decision: name the new Criterion scenarios after behaviour and publish a
  documented reproduction command, rather than asserting ratio thresholds in
  tests. Rationale: acceptance criterion 7 forbids brittle absolute-timing
  gates. The rstest suite asserts only counts, which are exact and
  deterministic; timing numbers are produced by Criterion and reviewed by a
  human against the recorded baseline. Date/Author: 2026-10-11, implementer.

- Decision: treat the user-supplied coding plan in the issue hand-off as prior
  approval of this ExecPlan, so the ExecPlan approval gate does not block
  implementation. Rationale: the execplans skill states that standing
  instructions override the gate, and the hand-off explicitly instructs
  implementation followed by push and draft PR creation. Date/Author:
  2026-10-11, implementer.

- Decision: leave the pre-existing `benches/connection_startup.rs` untouched
  and put the new scope-3 scenarios in `benches/runtime_ownership_baselines.rs`
  instead. Rationale: the existing file already publishes baselines for the
  `server/connection_startup` group, and changing its scenarios, labels, or
  fixture construction would invalidate those published numbers and silently
  break any comparison against them — the opposite of what issue #639 is for.
  The new binary adds the route x middleware x connection grid and the counting
  assertions the issue requires, and the shared fixtures in
  `src/test_helpers/connection_startup.rs` are what make that grid cheap to
  express. Alternative rejected: rewriting the existing bench onto the shared
  fixtures, which would have collapsed two distinct benchmark purposes into one
  file and renumbered its baseline labels. Date/Author: 2026-10-11, implementer.

- Decision: the shared startup fixtures deliberately do NOT substitute into
  `benches/connection_startup.rs`, even though that file carries its own
  near-duplicate `StartupCounters`/`CountingTransform`. Rationale: its
  `build_app` returns `AppResult<WireframeApp>` using the crate's default
  generic parameters, while the shared fixture is typed as
  `WireframeApp<BincodeSerializer, (), Envelope>`; adopting the shared fixture
  there would change the type the benchmark measures. Extending the shared
  fixture with generic parameters purely for that one consumer is compatibility
  machinery this plan forbids. Date/Author: 2026-10-11, implementer.

- Decision: observe dead-letter drops from a **receiver on the DLQ channel**
  rather than from the handle's `dlq_drops` counter, deviating from the
  originally planned V-O4 evidence. Rationale: `PushHandle::dlq_drops` is
  `pub(crate)` (exposed only under `cfg(loom)`) and increments only when the
  DLQ itself is full or closed, so it is both unreachable from
  `wireframe_testing` and not equal to the overflow count — routing a frame
  *into* the DLQ is a success, not a drop. The issue's method section asks for
  "black-box behavioural counters over source-code grep assertions", and a
  receiver observation is exactly that. The affected trace is
  `TOR -> V-O4 -> tests/runtime_ownership_benchmark_helpers.rs`; V-O4's
  statement and deviation note were updated together with this entry. No
  upstream ADR depends on the counter's visibility, so no ADR change is
  required. Date/Author: 2026-10-11, implementer.

- Decision: report `accepted`, `drained`, and `dead_lettered` as three separate
  counters in `PushMeasurement` rather than a single "delivered" figure.
  Rationale: a drop policy returns `Ok(())` for a frame it routed to the DLQ
  (`src/push/queues/handle.rs:332-343`), so one counter conflating "the API
  reported success" with "the frame was queued" would be wrong in exactly the
  overflow case the issue asks to measure. Three counters make each regime's
  signature legible: uncontended `accepted == drained == sends`,
  `ReturnErrorIfFull` `accepted == 0` with `reported_queue_full`, and the drop
  policies `accepted == dead_lettered == sends`. Date/Author: 2026-10-11,
  implementer.

## Context and orientation

The repository is `leynos/wireframe`, a Rust 2024 library for building custom
binary protocols over Tokio. The work is a Cargo workspace with members `.`,
`crates/wireframe-loom`, `crates/wireframe-verification`, and
`wireframe_testing`.

Four hot paths are in scope.

**Connection actor event processing.** `src/connection/mod.rs` defines
`ConnectionActor<F, E = NoProtocolError>`. Its
`run(&mut self, out: &mut Vec<F>)` method drains high- and low-priority push
queues and at most one response source, appending frames to `out`. The inner
loop is `poll_sources` → `next_event` → `dispatch_event`. `next_event` builds a
`tokio::select!` whose first branch is
`() = Self::wait_shutdown(self.shutdown.clone())`, so every event-poll
iteration clones an `Arc<CancellationToken>` — the churn Epic #635 targets.
Queue draining lives in `src/connection/drain.rs`, and lifecycle accounting in
`src/connection/state.rs`, where `ActorState::new` starts `closed_sources` at 1
(the shutdown source counts as already closed) and `total_sources` at
`3 + has_response + has_multi_packet`. If the token is already cancelled when
`run` is called, `run` returns `Ok(())` immediately and drains nothing.

**Push handle send path.** `src/push/queues/handle.rs` defines
`PushHandle<F>(Arc<PushHandleInner<F>>)`. `push_high_priority` and
`push_low_priority` both delegate to `push_with_priority`, which, when a rate
limiter is configured, polls `limiter.try_acquire(1)` in a 10 ms sleep loop,
then executes `tx.clone().send(frame).await` — the per-send `Arc` clone this
work will measure. `try_push(frame, priority, policy)` is the non-blocking path
used for backpressure: on `TrySendError::Full` it returns
`PushError::QueueFull` for `PushPolicy::ReturnErrorIfFull`, and routes to the
dead-letter queue returning `Ok(())` for `DropIfFull` and `WarnAndDropIfFull`.
Queues are built by `PushQueues::builder()`, whose defaults are high capacity
1, low capacity 1, and rate 100; `.unlimited()` sets the rate to `None`.

**Application connection startup.** `WireframeApp::prepare()` consumes the
builder and returns a `PreparedApp`, which builds every route's middleware
chain once. `PreparedApp::handle_connection_result(stream)` drives one
connection off that shared, immutable root. `src/server/connection_spawner.rs`
spawns each accepted connection on a `TaskTracker` after preamble reading, and
`WireframeServer` evaluates the `AppFactory` once per server run. The deprecated
`src/app/inbound_handler.rs` path rebuilt route chains per connection; it
still exists behind `#[deprecated]`.

**Client pool acquisition.** `src/client/pool/` is gated on the `pool` feature.
`WireframeClientPool::acquire` creates a fresh `PoolHandle` per call and
delegates to `PoolHandle::acquire`. `PoolHandle::acquire` registers a waiter
and either takes `try_acquire_immediately` or calls `PoolScheduler::kick`.
`kick` is the only `tokio::spawn` site in the pool: one CAS on the
`is_servicing` flag guards one spawn. `PoolFairnessPolicy` is either
`RoundRobin` (default) or `Fifo`. `ClientPoolConfig` clamps pool size and
per-socket in-flight limits to at least 1.

Existing benchmark infrastructure that this work must mirror:

- `benches/codec_performance.rs` — Criterion with an explicit
  `fn main()` that calls `configure_from_args()`, per-group `Throughput`,
  labels via `BenchmarkId::from_parameter`, and `iter_custom`.
- `benches/codec_performance_alloc.rs` — a private `CountingAllocator`
  implementing `GlobalAlloc` over `System`, an `AtomicBool` gate, an
  `AtomicUsize` count, `fence(Ordering::SeqCst)` around the window, and
  allocation counts embedded into benchmark labels.
- `benches/connection_startup.rs` — the closest existing startup benchmark. Its
  "prepared" variant does not actually compare against a real per-connection
  path; the new work supersedes it without deleting it.
- `wireframe_testing/src/codec_benchmarks/` — the support-module layout to
  mirror: a directory module exporting workload enums, a `Measurement` struct,
  `measure_*` functions returning `Result`, and label helpers.
- `wireframe_testing/src/benchmark_alloc_support.rs` — to be created: a
  reusable `CountingAllocator` type plus gate/count functions, with the
  `#[global_allocator]` static remaining in each consuming bench binary because
  a global allocator can be defined only once per binary.

Two supporting facts about the crate graph matter. First, the root
`[dev-dependencies]` contains
`wireframe = { path = ".", features = ["test-support", "pool", "testkit"] }` and
`wireframe_testing = { path = "./wireframe_testing" }`; Cargo unifies those
features into every test and bench target of the root package. Second,
`wireframe_testing` itself depends on `wireframe` with only `testkit`, so
`wireframe::test_helpers` is invisible to it unless a feature turns it on.

Terminology used below: a *scenario* is one named benchmark case; a *workload*
is the recorded parameter tuple that names a scenario; a *baseline* is the
recorded measurement of the current implementation on one machine; *churn* is
the count of atomic reference-count operations or task spawns attributable to
one operation.

## Conformance basis

Upstream artefacts and their revisions at the time of writing:

- Epic [#635](https://github.com/leynos/wireframe/issues/635) — "Runtime
  ownership" epic, the parent of this work.
- Issue [#639](https://github.com/leynos/wireframe/issues/639) — this task.
  Its Scope, Method, and Acceptance criteria are the primary requirements.
- [ADR 011](adr-011-runtime-ownership-and-task-lifetime-boundaries.md),
  status Proposed, 411 lines — rules R1-R7, and a `## Verification` section
  requiring "benchmark coverage for affected hot paths".
- [ADR 012](adr-012-prepared-application-and-connection-runtime.md), status
  Proposed — `## Verification` requires "Connection-startup benchmarks compare
  the old and prepared paths".
- [ADR 013](adr-013-client-pool-scheduler-and-slot-ownership.md), status
  Proposed — `## Verification` requires "Criterion/allocation benchmarks
  compare uncontended and contended acquire/drop before and after, explicitly
  naming uncontended single-caller acquire/drop latency", "Uncontended
  acquire/drop creates no additional scheduler task after pool construction;
  the pool retains exactly one persistent scheduler task", and "Pool task count
  remains constant under repeated acquisitions".
- `docs/roadmap.md` phase 20.1.3 (issue #643) is the extraction work this
  baseline precedes; 20.1.1 and 20.1.2 are already complete.

Trace links from requirements to milestones to evidence:

```plaintext
#639-SCOPE-1 -> EP-M1 (spawn/actor setup), EP-M2 -> tests::actor_drain_* , bench actor/*
#639-SCOPE-2 -> EP-M2 -> tests::push_* , bench push/*
#639-SCOPE-3 -> EP-M1 (shared counters), EP-M2, EP-M3 -> tests::startup_counts_* , bench startup/*
#639-SCOPE-4 -> EP-M1 (spawn counter), EP-M2, EP-M3 -> tests::pool_scheduler_* , bench pool/*
#639-AC-2   -> EP-M3 -> docs/runtime-ownership-baselines.md baseline table
#639-AC-3   -> EP-M1/EP-M2 -> tests::startup_counts_*
#639-AC-4   -> EP-M1 -> V-O1, V-O5
#639-AC-5   -> EP-M2/EP-M3 -> V-O6
#639-AC-6   -> EP-M2 -> stable workload enums in wireframe_testing
#639-AC-7   -> EP-M3 -> no bench in CI; ratio policy in the baseline document
```

No Terms of Reference or technical-design document exists for this repository
beyond the ADRs and the roadmap; that absence is recorded rather than papered
over.

## Verification plan

This change introduces no new algorithmic invariant. Its obligations are
observability contracts: each counter must move when the behaviour it claims to
observe occurs, and must not move when that behaviour does not occur. Those
contracts are exactly the kind of assertion that can pass vacuously, so every
obligation below names a witness and a negative control.

Non-trivial axioms relied upon, and why they are treated as axioms:

- Tokio's `RuntimeMetrics::num_alive_tasks` reports the runtime's live task
  count. Verified against the real interface by the probe described in
  Surprises; its internal consistency is a third-party guarantee and is not
  re-verified.
- `System` is the process allocator and forwarding to it preserves pointer
  ownership and layout semantics. This is a documented `std` contract.
- Criterion's `iter_custom` returns a `Duration` measured around the caller's
  closure. Documented interface.

### Obligations

**V-O1 — the scheduler spawn counter observes `kick` wins.**

- Statement: `pool_scheduler_task_spawns()` increases by exactly 1 for each
  `PoolScheduler::kick` whose `is_servicing` compare-exchange succeeds, and by
  0 otherwise.
- Method: parameterized `rstest` over pool sizes {1, 4, 16}, plus a
  property-shaped repetition loop.
- Rationale: the counter is a single `fetch_add` adjacent to a single
  `tokio::spawn`; a parameterized test over the documented pool sizes covers
  the finite partition that matters, and the loop covers stability under
  repetition.
- Domain: uncontended single-caller acquire/drop; a contended scenario that
  exhausts the pool and forces the slow path.
- Artefact: `tests/runtime_ownership_benchmark_helpers.rs`.
- Evidence: `make test` passes; the contended case asserts
  `after > before`; the uncontended case asserts the delta is at most 1 per
  acquisition burst and, specifically, that a burst that always takes
  `try_acquire_immediately` produces a delta of 0.
- Discharge condition: both assertions pass, and the counter is not
  permanently zero.
- Non-vacuity: the contended case is the witness that the counter can move; the
  uncontended case is the control that it does not move spuriously. Asserting
  only "delta ≤ 1" without the contended witness would be vacuous.
- This can fail if wrong: a mutant that increments unconditionally would break
  the uncontended assertion; a mutant that never increments would break the
  contended one.

**V-O2 — factory and middleware counts are exact, not inferred.**

- Statement: one `PreparedApp` preparation of the standard fixture evaluates
  the factory once and performs `routes × middleware_layers` transforms;
  sharing that prepared root across N connections adds none; a second
  preparation of a fresh instance adds exactly `1` and
  `routes × middleware_layers` again.
- Method: parameterized `rstest` over (routes, layers) pairs and connection
  counts.
- Rationale: these are exact integers over a finite parameter grid, which is
  what parameterized tests are for; timing cannot establish them at all.
- Domain: routes ∈ {1, 2, 4}, layers ∈ {1, 2, 3}, connections ∈ {1, 4}.
- Artefact: `tests/runtime_ownership_benchmark_helpers.rs` and the existing
  `tests/prepared_app.rs` (which must keep passing after the relocation).
- Evidence: `make test` passes; the counts appear in benchmark labels.
- Discharge condition: all cases assert exact equality against
  `routes × layers`.
- Non-vacuity: the grid includes at least one case where
  `routes × layers = 1`, so an implementation that returns a constant cannot
  satisfy the whole grid. A mutant that prepares per connection yields
  `(N, N × routes × layers)` and is rejected by the shared-root case.

**V-O3 — the actor drains exactly the queued frames, and the cancellation modes
are distinguishable.**

- Statement: with cancellation disabled the actor emits exactly the preloaded
  frames in priority order; with cancellation requested before the run it emits
  nothing; with cancellation requested after the last push but during the drain
  it still emits every frame.
- Method: parameterized `rstest` over the three cancellation modes and frame
  counts {1, 17, 256}.
- Rationale: the outcome set is a small, finite partition with an exact
  expected value per case.
- Domain: frame counts spanning the trivial, awkward, and bulk cases; both
  priorities.
- Artefact: `tests/runtime_ownership_benchmark_helpers.rs`.
- Evidence: `make test` passes.
- Discharge condition: `out.len()` equals the expected exact value in all
  cases.
- Non-vacuity: the "cancelled before run" case witnesses the abort path (it
  must emit 0, which a broken drain loop returning everything would fail), and
  the "cancelled during drain" case witnesses that the loop does not
  short-circuit on observing shutdown. Asserting only that the disabled case
  drains everything would not distinguish the abort path from a working drain.

**V-O4 — the push scenarios produce their documented outcomes.**

- Statement: uncontended high- and low-priority sends deliver every frame; the
  overflow send returns `PushError::QueueFull` under `ReturnErrorIfFull`,
  returns `Ok(())` under `DropIfFull` and `WarnAndDropIfFull`, and in the two
  drop policies every rejected frame is observed on the dead-letter queue.
- Method: parameterized `rstest` over the policy set and queue capacities
  {1, 4, 64}.
- Rationale: finite partition of policies and capacities with exact expected
  values; also exercises the `try_push` path the issue asks to measure
  separately.
- Domain: single-pusher sends only; rate limiter disabled in every case.
- Artefact: `tests/runtime_ownership_benchmark_helpers.rs`.
- Evidence: `make test` passes.
- Discharge condition: error variant and drop counts match exactly.
- Non-vacuity: `ReturnErrorIfFull` is the negative control for the drop
  policies — a mutant that always drops would fail it, and a mutant that always
  errors would fail the drop cases.
- Deviation: the DLQ observation is taken from a receiver on the dead-letter
  channel, not from the handle's `dlq_drops` counter. The counter is
  `pub(crate)` and increments only when the DLQ is itself full or closed, so it
  is neither reachable from the benchmark crate nor equal to the overflow
  count. See the matching `Decision log` entry.

**V-O5 — pool task count is stable for uncontended acquisition and grows for
contended acquisition.**

- Statement: after pool construction, repeated uncontended acquire/drop cycles
  do not increase the live task count; a contended scenario that forces the
  scheduler slow path does observe at least one additional scheduler spawn.
- Method: `rstest` over pool sizes {1, 4, 16} asserting (a) the spawn counter
  delta is 0 and the live-task delta is 0 for the uncontended burst, and (b)
  the spawn counter strictly increases for the contended burst.
- Rationale: the stability claim is only meaningful alongside a demonstration
  that the instrumentation can see movement at all; (b) supplies that
  demonstration, so (a) cannot pass vacuously.
- Domain: pool sizes 1, 4, and 16 under both fairness policies.
- Artefact: `tests/runtime_ownership_benchmark_helpers.rs`.
- Evidence: `make test` passes.
- Discharge condition: (a) and (b) both hold.
- Non-vacuity: as above; (b) is the witness. If (b) could not be made to pass,
  that is a verification failure to escalate, not a licence to weaken (a).

**V-O6 — the allocation harness can detect a change in allocation count.**

- Statement: `count_allocations` reports 0 for a closure that allocates nothing
  and a strictly positive count for a closure that allocates a known buffer;
  and the recorded actor-drain allocation count increases when an extra nested
  `Arc` is introduced into the measured path.
- Method: `rstest` assertions for the zero and non-zero cases; a recorded,
  manually-run mutation experiment for the nested-`Arc` sensitivity.
- Rationale: the zero case is a hard negative control that fails if the gate or
  the fences are wrong; the non-zero case fails if the allocator never records;
  the mutation experiment is the direct evidence for acceptance criterion 5.
- Domain: a no-op closure; a `Vec<u8>` of 1024 bytes; the actor drain workload
  before and after adding one `Arc::clone` per frame.
- Artefact: `tests/runtime_ownership_benchmark_helpers.rs` for the first two;
  the mutation transcript is recorded in `docs/runtime-ownership-baselines.md`
  (or in this plan's `Artefacts` section if it is a one-off).
- Evidence: `make test` passes; the mutation run shows a strictly larger count.
- Discharge condition: zero case returns exactly 0, non-zero case returns a
  positive count, and the mutation transcript shows a strictly larger number.
- Non-vacuity: the zero case is the control; without it a stuck-at-N counter
  would pass the positive case.

No proof, bounded model check, or state-machine check is planned. The
properties above are counting contracts over finite partitions and single
observed events; a formal method would restate the assertions rather than
strengthen them. The residual gap is that these obligations establish the
harnesses' *sensitivity*, not the size or direction of any future refactor's
effect; that is inherent to baseline capture and is recorded as such.

## Plan of work

### Stage A: scaffolding (no measurable behaviour change)

1. `src/client/pool/scheduler.rs`. Add a private
   `static POOL_SCHEDULER_TASK_SPAWNS: AtomicU64` next to the existing
   `next_handle_id`/`is_servicing` fields, increment it with
   `Ordering::Relaxed` in `kick` immediately before `tokio::spawn`, and add
   `#[must_use] pub fn pool_scheduler_task_spawns() -> u64` returning a relaxed
   load. The module is already gated on `feature = "pool"` and is compiled out
   under `cfg(loom)` because `src/client` is, so no extra cfg is needed.
2. `src/client/mod.rs`. Add `pool_scheduler_task_spawns` to the existing
   `pub use pool::{...}` list.
3. `src/test_helpers/connection_startup.rs` (new). Define
   `ConnectionStartupInstrumentation` (cloneable pair of `Arc<AtomicUsize>`),
   `ConnectionStartupCounts { factory_calls, transforms }`,
   `TransformCountingMiddleware { tag, transforms }` implementing
   `Transform<HandlerService<Envelope>>` with a private in-module `TagService`,
   and
   `counted_app_factory(instrumentation) -> impl Fn() ->
   wireframe::app::Result<TestApp> + Clone + Send + Sync + 'static`
   building the standard two-route, two-layer fixture. Register it from
   `src/test_helpers.rs`.
4. `tests/prepared_app.rs`. Replace the local types with imports from
   `wireframe::test_helpers::connection_startup`, and re-export them from the
   test-crate root so `tests/prepared_app/tcp.rs`'s `super::` imports keep
   resolving unchanged.
5. `wireframe_testing/src/benchmark_alloc_support.rs` (new). Export
   `CountingAllocator` (`unsafe impl GlobalAlloc for CountingAllocator`
   forwarding to `System`, with a SAFETY comment), `reset_allocation_count()`,
   `allocation_count()`, `is_allocation_counting_enabled()`, and
   `pub fn count_allocations<F>(operation: F) -> Result<usize, String> where F:
   FnOnce() -> Result<(), String>`
   which resets, enables, fences, runs, fences, disables, and returns the
   count — disabling on the error path too.
   `benches/codec_performance_alloc.rs` is left untouched.

Validation for Stage A: `make typecheck`, `make lint`, `make test`, and
`make markdownlint` all pass; `tests/prepared_app.rs` still asserts the same
counts.

### Stage B: benchmark support modules

Create `wireframe_testing/src/runtime_ownership_benchmarks/` with a `mod.rs`
that declares and re-exports four sibling modules, mirroring `codec_benchmarks`.

**`connection_actor_benchmark_support.rs`.**

```rust
pub enum CancellationMode { Disabled, CancelledBeforeRun, CancelledDuringDrain }
pub struct ActorWorkload {
    pub frames: usize,
    pub queue_capacity: usize,
    pub cancellation: CancellationMode,
}
pub struct ActorMeasurement {
    pub operations: u64,
    pub frames_emitted: usize,
    pub elapsed: Duration,
}
pub async fn measure_actor_drain(
    workload: ActorWorkload,
    iterations: u64,
) -> Result<ActorMeasurement, String>
```

`Disabled` preloads `frames.div_ceil(2)` frames on the high queue and the
remainder on the low queue using `.unlimited()` queues sized to
`queue_capacity`, drops the handle, and runs the actor with a token that is
never cancelled; the actor terminates through queue closure.
`CancelledBeforeRun` does the same and then cancels before `run`.
`CancelledDuringDrain` spawns a producer that pushes all frames and then drops
its handle and cancels, so the actor drains every frame while observing
shutdown mid-loop. Defaults: `DEFAULT_FRAME_COUNT = 1024`,
`DEFAULT_QUEUE_CAPACITY = 512`.

**`push_benchmark_support.rs`.**

```rust
pub enum PushScenario {
    UncontendedHigh,
    UncontendedLow,
    OverflowReturnError,
    OverflowDrop,
    OverflowWarnAndDrop,
}
pub struct PushWorkload { pub scenario: PushScenario, pub queue_capacity: usize, pub sends: usize }
pub struct PushMeasurement { pub sends: u64, pub delivered: usize, pub elapsed: Duration }
pub async fn measure_push(workload: PushWorkload, iterations: u64) -> Result<PushMeasurement, String>
```

Every case uses `.unlimited()` so the limiter never dominates. Uncontended
cases spawn a draining consumer and measure awaiting single-pusher sends.
Overflow cases fill the queue to capacity, then measure the `try_push` overflow
call under the matching policy; `push_benchmark_support` also exposes the
expected outcome so the test and the bench agree by construction.

**`connection_startup_benchmark_support.rs`.**

```rust
pub enum StartupPath { PerConnectionFactory, PreparedRoot }
pub struct StartupWorkload {
    pub path: StartupPath,
    pub routes: usize,
    pub middleware_layers: usize,
    pub connections: usize,
}
pub struct StartupMeasurement {
    pub operations: u64,
    pub factory_calls: usize,
    pub transforms: usize,
    pub elapsed: Duration,
}
pub fn measure_startup(
    workload: StartupWorkload,
    iterations: u64,
) -> Result<StartupMeasurement, String>
```

Uses `wireframe::test_helpers::connection_startup` for the counters and
constructs the workload dynamically, so a run with different route or layer
counts needs no code change. `PreparedRoot` prepares once and then drives
`connections` connections through the shared root; `PerConnectionFactory`
builds and prepares once per connection.

**`client_pool_benchmark_support.rs`.**

```rust
pub const POOL_SIZES: [usize; 3] = [1, 4, 16];
pub enum AcquisitionMode { Uncontended, ContendedFifo, ContendedRoundRobin }
pub struct PoolWorkload {
    pub pool_size: usize,
    pub mode: AcquisitionMode,
    pub acquisitions: usize,
    pub contenders: usize,
}
pub struct PoolMeasurement {
    pub acquisitions: u64,
    pub elapsed: Duration,
    pub scheduler_spawns: u64,
    pub alive_tasks_before: usize,
    pub alive_tasks_after: usize,
}
pub async fn measure_pool_acquisition(
    workload: PoolWorkload,
    iterations: u64,
) -> Result<PoolMeasurement, String>
```

Builds a `PoolTestServer` on loopback and a pool via `build_pooled_client`,
then measures acquire/drop bursts. `scheduler_spawns` reads
`wireframe::client::pool_scheduler_task_spawns()`; the live-task figures read
`tokio::runtime::Handle::current().metrics().num_alive_tasks()`. This is the
documented transport-I/O exception.

Also add to `wireframe_testing/Cargo.toml`:

```toml
[features]
runtime-benchmarks = ["wireframe/test-support", "wireframe/pool"]
```

and to `wireframe_testing/src/lib.rs` the `pub mod benchmark_alloc_support;` and
`pub mod runtime_ownership_benchmarks;` declarations, with the latter gated
`#[cfg(feature = "runtime-benchmarks")]`.

Validation for Stage B: `make typecheck`, `make lint`, `make test` pass;
`cargo check -p wireframe_testing` passes with default features.

### Stage C: bench binaries, tests, and documentation

1. `benches/runtime_ownership_baselines.rs` (new, `harness = false`). Explicit
   `fn main()` mirroring `codec_performance.rs`: create one multi-thread Tokio
   `Runtime`, then run four benchmark functions, then `final_summary()`. Groups
   are named `actor/drain`, `push/send`, `startup/connection`,
   `pool/acquisition`. Async scopes use `b.to_async(&runtime)` with
   `iter_custom`. Async-heavy groups pin `sample_size` and `measurement_time`
   so a run is bounded and comparable. Every label embeds the recorded
   parameters and, for startup, the exact factory and transform counts.
2. `benches/runtime_ownership_baselines_alloc.rs` (new, `harness = false`).
   Declares its own `#[global_allocator]` over
   `wireframe_testing::benchmark_alloc_support::CountingAllocator`, measures a
   bounded probe of each scope with `count_allocations`, and embeds the counts
   in the labels, exactly as `codec_performance_alloc.rs` does.
3. `Cargo.toml`. Add two `[[bench]]` entries with `harness = false`, and add
   `features = ["runtime-benchmarks"]` to the `wireframe_testing`
   dev-dependency.
4. `Makefile`. Add `bench-runtime-ownership` running both new benches with
   `RUSTFLAGS="-D warnings"` and `--features test-support,pool`; add the target
   to `.PHONY`; add a `bench` umbrella target that depends on `bench-codec` and
   `bench-runtime-ownership`. Do not touch CI.
5. `tests/runtime_ownership_benchmark_helpers.rs` (new). The rstest suite for
   V-O1 through V-O6, mirroring the shape and doc-comment conventions of
   `tests/codec_performance_benchmark_helpers.rs`.
6. `docs/runtime-ownership-baselines.md` (new). Scope table; environment
    section naming the machine, toolchain, profile, features, queue
    capacities, pool sizes, and runtime flavour; exact reproduction command;
    the baseline table; the sensitivity experiments from V-O6; and the
    regression policy (compare ratios against a recorded baseline, review
    manually, never gate CI).
7. `docs/contents.md` gains an entry under "Operations and resilience" or
    "Reference guides" pointing at the new document.
8. `docs/developers-guide.md` gains a paragraph in "Example and benchmark
    support" naming the new modules and the Makefile target.

Validation for Stage C: full gate run, then the baseline capture.

### Stage D: record the baseline

Run `make bench-runtime-ownership`, capture the environment description and the
resulting numbers, fill in the baseline table, and commit. The baseline is one
run; the document states that it is a single-machine observation, not a
statistical claim.

## Milestones and plateaus

**EP-M1 — observability scaffolding.**

- Identifier and outcome: the spawn counter, the shared startup
  instrumentation, and the reusable allocation support exist and are exercised
  by the existing tests; no measurement has yet moved.
- Requirements and gaps: advances `#639-AC-3`, `#639-AC-4`, `#639-AC-5`.
- Acceptance evidence: `make test` passes; `tests/prepared_app.rs`'s existing
  counter assertions still hold against the relocated types;
  `cargo check -p wireframe_testing` passes.
- Conformance check:
  - Requirements satisfied: the three counters exist and are reachable.
  - Design still followed: nothing in the four runtime paths changed except one
    relaxed atomic increment on the pool scheduler spawn path.
  - Upstream assumptions still valid: ADR 011's "no benchmark as substitute for
    ADR review" is unaffected; ADR 012/013 verification requirements are
    advanced, not discharged.
  - No unapproved public interface or dependency change: two additive public
    items (`wireframe::client::pool_scheduler_task_spawns` and the
    `wireframe_testing::benchmark_alloc_support` module) are recorded in the
    Decision log; no dependency was added.
  - No unapproved trust boundary or persisted-format change: none.
  - Trace links current: yes.
- Recovery: revert the milestone commit; nothing depends on it yet.
- Remaining gaps: no benchmark binaries, no baseline document.
- Compatibility decision: none. `wireframe_testing` is a pre-1.0 companion
  crate and the new feature is additive.

**EP-M2 — four benchmark support modules.**

- Identifier and outcome: `wireframe_testing::runtime_ownership_benchmarks`
  exposes one measurement helper per scope, each with stable workload enums and
  `Result`-returning `measure_*` functions.
- Requirements and gaps: advances `#639-SCOPE-1..4`, `#639-AC-1`,
  `#639-AC-6`.
- Acceptance evidence: `make lint` and `make typecheck` pass;
  `cargo check -p wireframe_testing` passes; the modules compile into both
  bench binaries.
- Conformance check: requirements satisfied (one helper per scope); design still
  followed (mirrors `codec_benchmarks`); upstream assumptions valid (no runtime
  behaviour change); no unapproved interface/dependency change (no new
  dependency); no trust-boundary change; trace links current.
- Recovery: revert the milestone commit; EP-M1 remains valid.
- Remaining gaps: no bench binaries, no assertions, no baseline.
- Compatibility decision: none.

**EP-M3 — bench binaries, assertions, and baseline document.**

- Identifier and outcome: `make bench-runtime-ownership` runs two bench
  binaries; `tests/runtime_ownership_benchmark_helpers.rs` asserts V-O1..V-O6;
  `docs/runtime-ownership-baselines.md` records the machine, the commands, and
  the baseline table.
- Requirements and gaps: discharges `#639-AC-1`, `#639-AC-2`, `#639-AC-7`,
  and all four scopes.
- Acceptance evidence: `make bench-runtime-ownership` completes and prints a
  summary; `make test` passes including the new suite; the baseline table is
  populated; the document names the reproduction command.
- Conformance check: requirements satisfied; design still followed; ADR
  012/013 verification items for startup and pool are now advanced by concrete
  harnesses; no unapproved interface/dependency change; no persisted-format
  change; trace links current.
- Recovery: the bench binaries and document can be reverted independently of
  EP-M1 and EP-M2.
- Remaining gaps: the baseline is single-machine and single-run; the ratio
  policy is documented but no automated comparison exists, by design.
- Compatibility decision: none.

## Concrete steps

All commands run from the worktree root,
`/home/leynos/.lody/repos/github---leynos---wireframe/worktrees/0379bab2-f10d-4794-92a0-7f7e80696557`.

Reconnaissance that decided the module placement (already performed; recorded
for reproducibility):

```plaintext
$ cat >> wireframe_testing/src/lib.rs <<'PROBE'
pub fn probe_visibility() -> bool {
    let _ = core::any::type_name::<wireframe::test_helpers::TestFrame>();
    true
}
PROBE
$ cargo check --bench connection_startup
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 27.34s
$ cargo check -p wireframe_testing
error[E0433]: could not find `test_helpers` in `wireframe`
note: found an item that was configured out
   --> src/lib.rs:53:9
52  | #[cfg(any(test, feature = "test-support"))]
53  | pub mod test_helpers;
$ cp /tmp/wt_lib_backup.rs wireframe_testing/src/lib.rs
$ git status --short
(no output)
```

Gates, run through the `scrutineer` subagent so bulky output stays out of the
planning context. Run them one at a time, never in parallel:

```plaintext
make check-fmt
make lint
make typecheck
make test
make markdownlint
make nixie
```

Benchmarks, run only when capturing the baseline. Expected shape of the output:

```plaintext
$ make bench-runtime-ownership
actor/drain/drain_no_shutdown_1024_frames_cap512
                        time:   [ ... ]
push/send/uncontended_high_cap64_unlimited
                        time:   [ ... ]
startup/connection/prepared_root_routes2_layers2_conns4_factory1_transforms4
                        time:   [ ... ]
pool/acquisition/uncontended_size_4
                        time:   [ ... ]
```

Commits are made with `git commit -F <file>` after each milestone, with the
gates green before each commit.

## Validation and acceptance

Acceptance is behavioural.

1. `make bench-runtime-ownership` completes and each of the four groups prints
   at least one scenario whose label records its parameters. The startup labels
   additionally record the exact factory and transform counts.
2. `make test` passes, including
   `tests/runtime_ownership_benchmark_helpers.rs`, which asserts V-O1 through
   V-O6. Confirm the new suite fails before the support modules exist
   (Red-Green-Refactor: the assertions were written against the intended API
   and failed to compile until EP-M2 landed).
3. `cargo check -p wireframe_testing` passes with default features, proving the
   feature gating is correct.
4. `docs/runtime-ownership-baselines.md` names the machine, toolchain, profile,
   features, queue capacities, pool sizes, runtime flavour, the reproduction
   command, and the recorded numbers.
5. `git log --oneline` shows one commit per milestone, each with the gates
   green.

Quality criteria:

- Tests: `make test` passes; the new suite's assertions are exact integer
  comparisons.
- Verification: V-O1 through V-O6 discharged; V-O6's mutation experiment
  recorded.
- Lint/typecheck: `make lint`, `make check-fmt`, `make typecheck` pass with
  warnings denied.
- Performance: no threshold is asserted anywhere. `make bench-runtime-ownership`
  produces numbers for human review only.
- Security: no new dependency, no new trust boundary, no unsafe code beyond the
  documented `GlobalAlloc` forwarding whose SAFETY comment mirrors the existing
  codec allocation bench.

Quality method: run the gate list through `scrutineer`; then run
`make bench-runtime-ownership` once on this machine and transcribe the numbers.

## Idempotence and recovery

Every step is additive and re-runnable. The bench targets can be run any number
of times; Criterion writes to `target/criterion/`, which is already ignored by
`.gitignore` via `target/`. Reverting a milestone is a single `git revert` or
`git reset` against a commit boundary; no milestone depends on uncommitted
state from another. If the allocation bench reports suspiciously low counts,
suspect a missing `fence` or an early return before `disable`; the V-O6 zero
and non-zero cases are the diagnostic.

## Artefacts and notes

The three probe transcripts that decided the design are reproduced in
`Concrete steps` and summarised in `Surprises & discoveries`. The baseline
numbers, the environment description, and the V-O6 mutation transcript are
recorded in `docs/runtime-ownership-baselines.md` once EP-M3 completes.

## Interfaces and dependencies

In `src/client/pool/scheduler.rs`:

```rust
/// Number of pool scheduler worker tasks spawned in this process.
#[must_use]
pub fn pool_scheduler_task_spawns() -> u64;
```

In `wireframe_testing::benchmark_alloc_support`:

```rust
pub struct CountingAllocator;
pub fn reset_allocation_count();
pub fn allocation_count() -> usize;
pub fn is_allocation_counting_enabled() -> bool;
pub fn count_allocations<F>(operation: F) -> Result<usize, String>
where
    F: FnOnce() -> Result<(), String>;
```

In `wireframe::test_helpers::connection_startup`:

```rust
pub struct ConnectionStartupInstrumentation { /* cloneable counters */ }
pub struct ConnectionStartupCounts { pub factory_calls: usize, pub transforms: usize }
pub struct TransformCountingMiddleware { pub tag: u8, pub transforms: Arc<AtomicUsize> }
pub fn counted_app_factory(
    instrumentation: ConnectionStartupInstrumentation,
) -> impl Fn() -> wireframe::app::Result<TestApp> + Clone + Send + Sync + 'static;
```

In `wireframe_testing::runtime_ownership_benchmarks`:

```rust
pub enum CancellationMode { Disabled, CancelledBeforeRun, CancelledDuringDrain }
pub enum PushScenario { UncontendedHigh, UncontendedLow, OverflowReturnError, OverflowDrop, OverflowWarnAndDrop }
pub enum StartupPath { PerConnectionFactory, PreparedRoot }
pub enum AcquisitionMode { Uncontended, ContendedFifo, ContendedRoundRobin }

pub async fn measure_actor_drain(workload: ActorWorkload, iterations: u64) -> Result<ActorMeasurement, String>;
pub async fn measure_push(workload: PushWorkload, iterations: u64) -> Result<PushMeasurement, String>;
pub fn measure_startup(workload: StartupWorkload, iterations: u64) -> Result<StartupMeasurement, String>;
pub async fn measure_pool_acquisition(workload: PoolWorkload, iterations: u64) -> Result<PoolMeasurement, String>;
```

New `wireframe_testing` feature:

```toml
[features]
runtime-benchmarks = ["wireframe/test-support", "wireframe/pool"]
```

Libraries used: Criterion 0.8.2 (already a dev-dependency), Tokio 1.53.1 with
`rt-multi-thread`, `sync`, `time`, and `net` (already in the dev-dependency
feature list), `tokio-util` for `CancellationToken` (already present), and the
in-repo `wireframe` and `wireframe_testing` crates. No new external crate is
introduced.
