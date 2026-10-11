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

- Risk: a bench binary under `benches/` is auto-discovered by Cargo, so a
  half-written file breaks `--all-targets` builds for every lane at once.
  Severity: low. Likelihood: medium. Mitigation: land the two bench binaries
  only when both compile under `RUSTFLAGS="-D warnings" cargo check --benches`,
  and keep bench edits out of the tree while a commit gate is mid-run.

- Risk: the harnesses could quietly stop comparing like with like if a future
  refactor changes a workload's parameters, making an old baseline number look
  applicable when it is not. Severity: medium. Likelihood: medium. Mitigation:
  every `label()` embeds all recorded parameters — frame and send counts, queue
  capacities, pool sizes, route and layer counts, contender counts, and the
  startup path's exact factory and transform counts — so a Criterion result
  whose label no longer matches the baseline table is visibly uncomparable
  rather than silently substituted.

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
  (an incorrectly aliased constant, a wrong policy assertion, and an unwinnable
  allocator assertion) were found and fixed by running the crate's doctests
  directly.
- [x] (2026-10-11 19:30Z) Milestone EP-M3:
      `benches/runtime_ownership_baselines.rs`
  and `benches/runtime_ownership_baselines_alloc.rs` (`harness = false`,
  `required-features = ["test-support", "pool"]`), the
  `bench-runtime-ownership` Makefile target, and the rstest assertion suite in
  `tests/runtime_ownership_benchmark_helpers.rs` (34 tests).
- [x] (2026-10-11 20:10Z) Investigated the contended-pool stall: implemented a
  candidate repair, measured it, rejected it, and reverted it (see Surprises).
  Reworked the contended scope into a bounded `tracing` observation under
  `CONTENDED_BURST_TIMEOUT`, and converted the pool assertion into a documented
  known-defect tolerance rather than a claim of correctness.
- [x] (2026-10-11 22:30Z) Applied `cargo fmt` to the benchmark support: the
  previous commit's rustfmt diffs were missed, and `make check-fmt` reported
  three of them. Also reworked `runtime_ownership_baselines_alloc.rs` after its
  first real run failed on the contended pool scope (see Surprises);
  `make check-fmt` is green.
- [x] Wrote `docs/runtime-ownership-baselines.md`: scope table, environment,
  reproduction command, baseline table, the sensitivity controls, the
  pre-existing-defect notes, and the regression policy.
- [x] Cross-referenced the new document from `docs/contents.md` and
  `docs/developers-guide.md`.
- [x] (2026-10-11 20:35Z) Captured the baseline: `make bench-runtime-ownership`
  completed cleanly on the named machine, 24 timing labels, 8 allocation
  labels, 2 arc-sensitivity labels, and 2 contended observations, with no panic
  and no error. Filled the four baseline tables in
  `docs/runtime-ownership-baselines.md` from that run.
- [x] (2026-10-11 23:20Z) Cleared the whitaker `module_max_lines` gate: the
  four new support modules exceeded the 400-line limit (822, 605, 463, 418).
  Split each into `mod.rs` + a data module (`workload.rs`/`scenario.rs`) + a
  `measurement` module + `tests.rs`, with the two genuinely distinct control
  flows given their own submodules
  (`push/measurement/{uncontended,overflow}.rs`,
  `client_pool/measurement/{phase,contended}.rs`). No `#[allow]` was used; all
  21 files are now under 200 lines.
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

- Observation: `clippy::integer-division-remainder-used` is denied
  workspace-wide, so `%` cannot be used for cycling a counter even where the
  divisor is a small literal. Evidence: the gate run reported it at
  `connection_actor_benchmark_support.rs:279` and
  `push_benchmark_support.rs:292`. Impact: frame and route indices cycle with a
  power-of-two mask (`index & 0xFF`, `id & 0x7`), which preserves the only
  property the frames need — that consecutive values differ — without a
  division. The `% 251` form was not reworkable as a mask without changing the
  distribution, and no test asserts the distribution, so the mask is a strict
  simplification rather than a behaviour change.

- Observation: clippy's `-D warnings` config surface for this crate also
  includes `cast_possible_wrap`, `type_complexity`, and `unnested_or_patterns`.
  Evidence: the same gate run reported `as isize` at
  `client_pool_benchmark_support.rs:241`, the three-tuple return of
  `build_queues`, and `Err(A) | Err(_)` at `push_benchmark_support.rs:361`.
  Impact: the task delta uses `cast_signed()`, the queue fixture has a
  `PushFixture` type alias, and the fill loop matches `Err(_)` once. The
  original `Err(PushError::QueueFull) | Err(_)` was provably the whole `Err`
  arm, so the catch-all is the honest form rather than a weakening.

- Observation: a scrutineer gate report characterized this worktree as being
  edited by a second live Claude session, citing that a specific PID had the
  worktree as its working directory. That PID is this session's own harness
  process — the reporting agent's shell was its direct child. Evidence:
  `ps -o pid,ppid -p $$` from this session shows `PPID` equal to that PID. The
  "mid-gate edit" it saw was this session's own `Risks` addition. Impact: no
  concurrency hazard exists and no competing writer is present; a future gate
  report must not be read as evidence of a rival session on that basis alone.
  The underlying caution still holds for this plan: do not edit tracked files
  while a gate is mid-run.

- Observation: **an uncontended acquire/drop cycle *does* reach
  `PoolScheduler::kick` and *does* spawn a scheduler task**, contradicting the
  claim previously written into the module documentation of
  `client_pool_benchmark_support.rs` ("the scheduler is never asked to spawn a
  worker, so the live task count must not move") and the planned V-O1/V-O5
  assertion that an uncontended burst reports a spawn delta of exactly 0.
  Evidence, all three read-only: (a) `scheduler.rs:203` — `acquire_for_handle`
  enqueues *its own* waiter before probing for a free slot, so on the fast path
  `take_next_waiter_or_stop` at line 213 finds that very waiter and returns
  `Some`, and the `is_servicing` flag is never touched; (b) `lease.rs:157` —
  `PooledClientLease::drop` unconditionally calls `notify_capacity_available`,
  which at `scheduler.rs:242` calls `kick`; (c) `client_pool.rs:177-185` —
  `try_acquire_immediately`, the fast path itself, passes
  `Some(Arc::clone(self))` as `release_inner`, so the lease *is* armed to
  notify on drop even when no queueing occurred. Since `is_servicing` is
  `false` at that moment (nothing set it on the fast path), `kick`'s
  `compare_exchange(false, true, ...)` at line 249 **succeeds**, records a
  spawn, and spawns a worker that immediately finds no waiters and exits.
  Impact: the observable contract is not "uncontended acquisition spawns
  nothing" but "at least one spawn per burst, at most one spawn per
  idle-to-busy transition". The baseline must therefore assert a *bounded*
  uncontended delta together with stability of the live task count, and the
  per-release task churn this reveals is itself the finding issue #639 exists
  to record — it is exactly the kind of hidden ownership cost Epic #635 seeks
  to remove. `scheduler_spawns == 0` was never a true property and is not
  reinstated by rewriting the test.

- Observation: **the live-task gauge was measuring pool construction, not
  acquisition.** A temporary probe that bracketed the whole measurement reported
  `alive_task_delta` exactly equal to the pool size at every setting (1, 4,
  and 16), which is the signature of `bb8`'s eager connections rather than any
  acquisition churn. Evidence: probe transcript printing
  `pool_size=1 alive_delta=1`, `pool_size=4 alive_delta=4`,
  `pool_size=16 alive_delta=16` for acquisition counts of 1, 4, 16, and 256. A
  second probe that built the pool *outside* the sampled region showed the
  connection tasks still settling (`alive_before` 4/9/33 vs `alive_after`
  3/8/32). Impact: `pool_once` now constructs and closes the pool outside the
  bracket and samples the gauge only around the acquisition phase, with a
  `tokio::task::yield_now()` so a just-spawned worker is not counted mid-exit.

- Observation: the V-O6 mutation experiment doubles as a non-vacuity proof for
  the construction bracket, and it was run for real rather than described.
  Moving the gauge sample to *before* `build_preamble_pool` made
  `acquisition_phase_does_not_leak_pool_construction_tasks` fail on all three
  cases with deltas of 2, 5, and 17 for 1-, 4-, and 16-slot pools — scaling
  with pool size, which is the intended failure reason. Evidence: transcript
  recorded in `docs/runtime-ownership-baselines.md`; the file was restored from
  a backup and the three cases pass again. Impact: the bracket is load-bearing
  rather than cosmetic, and the assertion that depends on it is not vacuous.

- Observation: uncontended acquisition spawns 1 worker for bursts of 1 and 16
  acquisitions, rising to 2 at 256, identically across pool sizes 1, 4, and 16.
  Evidence: probe transcript `spawns=1` for acquisitions 1 and 16, and
  `spawns=2` for 256, at every pool size. Impact: the documented bound ("at
  least one, at most one per idle-to-busy transition") is what the tests
  assert; the exact figure is recorded as a baseline rather than pinned, since
  it depends on how often the worker drains the queue before the next release
  re-enters the slow path.

- Observation: a `git`-free line-ending artefact — the corrected support module
  briefly contained a stray blank line before the closing brace of `mod tests`,
  which defeated an `Edit` whose `old_string` assumed the canonical layout.
  Evidence: `cat -A` on the tail showed an empty line between the final `}` and
  the module's closing brace. Impact: match on trailing context and re-read the
  tail when an `Edit` on a block boundary unexpectedly fails; `make fmt` would
  have silently normalized it either way.

- Observation: **the contended pool scope exposed a pre-existing library
  defect**, not a harness artefact. A contended round intermittently fails with
  `1 of 16 contenders failed; … connection closed by peer`. Instrumenting every
  `ClientError::disconnected()` origin in the pool showed the sole live site is
  `scheduler.rs`'s `waiter.send(Ok(lease)).is_err()` arm — the branch taken
  when the selected queue head has already dropped its `oneshot::Receiver`. Two
  distinct windows reach it: a contender that observes the last busy slot and
  returns (dropping its receiver) while a worker concurrently services that
  same waiter; and `close()` racing in-flight acquires, where `notify_shutdown`
  drains the queue and abandons receivers. Evidence: 400 rounds at (contenders,
  rounds, pool_size) = (8, 8, 4) failed once, at round 162; a long-lived pool
  (no rebuild) reproduced it at burst 913 of 20,000 at 16 contenders. Only one
  line of `src/client/pool/scheduler.rs` differs from `origin/main` on this
  branch, and it is the counter increment, so the defect pre-exists this work.

- Observation: **a candidate repair was implemented, measured, and rejected.**
  Keeping the discarded lease instead of reporting `disconnected` removes the
  error, but it converts the failure into an intermittent **deadlock**, which
  is strictly worse: a caller that would have received a fast error instead
  stalls forever. The rejection was driven by measurement, not by inspection.
  Under the repair, `make bench-runtime-ownership` hung during Criterion
  warm-up on
  `pool/acquisition/contended_fifo_pool_4_acquisitions_256_contenders_16`; gdb
  showed the main thread parked in `Criterion::iter_custom` →
  `Runtime::block_on` → `measure_pool_acquisition` on a `parking_lot::Condvar`,
  with every tokio worker parked and CPU time frozen across 36 s. The change
  was reverted. Impact: the defect is **recorded, not fixed** — the correct
  scope for #639, which establishes baselines *before* Epic #635 changes
  ownership topology. The pool acquire path is ADR 013's subject, and both the
  spurious error and the stall belong to #647.

- Observation: **the stall is inherent to sustained contention**, so it cannot
  be engineered away in the harness. With the repair reverted — i.e. against
  unmodified library code — the same stall still reproduces. It was observed at
  first-stall burst 272, 386, 808, 814, 2361, 2739, 4233, 4797, 7784, 10727,
  and 18069, across: both fairness policies; a pool rebuilt per iteration and
  one long-lived pool; fresh and reused `PoolHandle`s; and contender counts of
  8 and 16 with 1, 2, and 8 rounds each. The stall is therefore not caused by
  the per-iteration rebuild, nor by the fixture's task leak, nor by any edit on
  this branch. A control separates the two effects: the same
  4-tasks-per-iteration leak ran **20,000 uncontended iterations with zero
  stalls**, so the leak does not cause it either. Impact: contended acquisition
  cannot be Criterion-timed at all, because Criterion would block during
  warm-up. The harness reports it as a bounded observation under
  `CONTENDED_BURST_TIMEOUT` instead, which turns a stall into a reported error.
  Note the earlier "the fix causes the hang" reading was confounded by the
  discarded experiment design; the reverted-code probe at round 386 is what
  settled it.

- Observation: **pool build and close leak roughly one task per slot per
  iteration**, also pre-existing. With `acquisitions: 0` — no lease is ever
  taken and no `kick` runs — a repeated `build_preamble_pool` / `close()` cycle
  grew the live-task gauge by exactly 4 per iteration for a 4-slot pool, and by
  10, 20, 40, and 80 over ten iterations for pool sizes 1, 2, 4, and 8. A
  `PoolTestServer` start/drop cycle alone leaked nothing, and a scoped runtime
  per iteration showed a settled delta of 0–2 that did not accumulate.
  Evidence: the two probe transcripts above; `git status` confirmed no library
  edit was left in place. Impact: this is why the pool assertions bound the
  settled delta and hold construction outside the bracket. The bracket is what
  makes the acquisition-phase figure meaningful; it does not make the leak go
  away, and the leak is recorded here rather than silently absorbed.

- Observation: **the allocation binary hit the same defect on its first real
  run**, which confirms the spurious disconnect is not confined to the timing
  harness. `make bench-runtime-ownership` failed with
  `allocation timing pass failed: 1 of 16 contenders failed; first: transport
  error: connection closed by peer`
  at `benches/runtime_ownership_baselines_alloc.rs:121`. The counting window
  had already succeeded — it takes one window, not a Criterion sample series —
  but the timing pass in the same group runs far more iterations, so it reaches
  the failing interleaving almost every run. Evidence: the transcript above,
  plus the labels the run did record
  (`contended_fifo_pool_4_acquisitions_256_contenders_16_allocs_2925`). Impact:
  the contended scope is removed from `allocation_scopes` and reported once
  after `Criterion::final_summary`, mirroring what the timing binary already
  does. This is a *stronger* confirmation than the earlier probe, because it is
  an unmodified benchmark binary driving unmodified library code.

- Observation: **whitaker's `module_max_lines` limit applies to the new
  benchmark-support modules, and it counts the whole file, not the `mod`
  body.** All four modules written for this issue exceeded the 400-line cap
  (822, 605, 463, 418), and because `make lint` runs rustdoc then clippy then
  whitaker, the finding was invisible until both earlier steps were green.
  Evidence: the gate log reported four
  `error: Module <name> spans <N> lines, exceeding the allowed 400` diagnostics
  pointing at
  `wireframe_testing/src/runtime_ownership_benchmarks/mod.rs:44-47`, the
  `pub mod` declarations rather than the offending files. The reported line
  numbers were deterministic across two independent runs. Impact: the four
  modules were split into `mod.rs` + a data module + a `measurement` module +
  `tests.rs` (with the genuinely distinct control flows given their own
  submodules) rather than silenced with `#[allow(module_max_lines)]`, which
  `AGENTS.md` reserves as a last resort. All 21 files now sit under 240 lines,
  and the nested splits put each regime's rationale next to that regime instead
  of in one shared header. **Do not size a new module against the 400-line
  limit; size it against a split you would defend anyway.**

- Observation: the `mod.rs` files of the split modules tripped
  `unused_imports` for names used *only* in Rustdoc intra-doc links, e.g.
  `PushError` referenced as ``[`PushError::QueueFull`]`` in a doc table but
  never in code. Evidence: `warning: unused import: PushError` at
  `push_benchmark_support/scenario.rs:10` after the split. Impact: such links
  carry the full path
  (`[`PushError::QueueFull`](wireframe::push::PushError::QueueFull)`) so the
  import is not needed for the link to resolve. This is a split-specific
  hazard: the same code in one file had the import genuinely used elsewhere.

- Observation: **`make lint` cannot see broken intra-doc links inside
  feature-gated modules, because its rustdoc step does not enable the
  features.** The `lint` target runs `cargo doc --workspace --no-deps`, while
  the `clippy` step beside it runs `--all-features`. The benchmark-support
  modules are `#[cfg]`-gated behind `runtime-benchmarks`, so rustdoc never
  compiled them and never checked their links. Evidence: `make lint` exited 0
  with zero errors at commit `311192c`, while
  `RUSTDOCFLAGS="--cfg docsrs -D warnings" cargo doc -p wireframe_testing
  --features runtime-benchmarks --no-deps`
  on the *same* tree reported **sixteen** errors — private-intra-doc-link
  violations for public docs pointing at the newly-private
  `workload`/`measurement`/`scenario`/`phase`/ `contended` submodules, and
  broken-intra-doc-links for `PushError`, `PushPolicy`, and `Envelope`, which
  the split had moved out of the file's scope. Impact: the split merged a
  defect the gate is structurally unable to detect, and the first commit's
  message described the repair as complete when only one of the three affected
  files had been touched. Fixed in `9985ed1`; the same command now reports zero
  errors, in both the feature-enabled and the default regime. **Any future work
  in this module must check links in the feature-enabled regime**, because the
  standard gate will not.

- Observation: that rustdoc/clippy asymmetry is repo-wide, not specific to
  this branch, and it is closable in one word.
  `RUSTDOCFLAGS="--cfg docsrs -D warnings" cargo doc --workspace --no-deps
  --all-features`
  exits 0 on this tree, so adding `--all-features` to that command would have
  caught all sixteen errors. Impact: recommended as a follow-up in its own base
  pull request rather than folded in here, because it changes a shared gate
  that every branch and agent runs, and because issue #639's diff should stay
  about benchmark harnesses. AGENTS.md already establishes the base-PR pattern
  for unrelated gate changes.

## Decision log

- Decision: **do not fix** the pool acquire-path defects found while measuring,
  and report contended acquisition as a bounded `tracing` observation instead
  of a Criterion timing group. Rationale: #639's stated purpose is to establish
  baselines *before* Epic #635 changes ownership topology, and its Dependencies
  section says it "should precede the larger implementation work under #637 and
  #638"; the pool acquire path is ADR 013's subject, arriving with #647. A
  candidate one-line repair was implemented and measured, and it converted an
  intermittent fast error into an intermittent hang, so shipping it would have
  replaced one defect with a worse one. Criterion cannot be used for the
  contended case because a stall occurs during warm-up, before any sample is
  recorded, which would abort the whole benchmark run; a timeout-guarded
  observation keeps the counters the issue asks for while guaranteeing
  termination. Alternatives rejected: (a) shipping the repair — strictly worse
  behaviour, demonstrated by measurement; (b) omitting contended acquisition —
  would drop a scope the issue names explicitly; (c) timing it with a shortened
  Criterion budget — a stall is not a slow sample, so no budget makes it safe.
  Date/Author: 2026-10-11, implementer.

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

- Decision: bracket **only the acquisition phase** in `pool_once`, building and
  closing the pool outside the sampled region, rather than measuring the whole
  build-acquire-close cycle. Rationale: `bb8` connects eagerly, so the
  construction phase alone moves the live-task gauge by roughly the pool size
  (measured 2, 5, and 17 for 1-, 4-, and 16-slot pools). Including it would
  have reported construction cost under an "acquisition" label — precisely the
  attribution error issue #639 exists to prevent. The cost is that
  pool-construction churn is no longer captured by this scope; that is
  acceptable because scope 3 covers connection startup separately and would
  otherwise double-count it. The affected trace is
  `TOR -> V-O5 -> PoolWorkload`; V-O5's statement and non-vacuity note were
  updated together with this entry. Date/Author: 2026-10-11, implementer.

- Decision: withdraw the V-O1/V-O5 assertion that uncontended acquisition
  produces a scheduler-spawn delta of exactly `0`, replacing it with a bounded
  delta plus a contended witness. Rationale: direct measurement falsified the
  zero claim (deltas of 1, 1, and 2 for 1-, 16-, and 256-acquisition bursts).
  Rewriting the *test* to pass while leaving the surrounding prose claiming
  "the scheduler is never asked to spawn a worker" would have been a silent
  factual error in a document whose entire purpose is to be trusted as a
  baseline; the module documentation, the obligation, and this log were
  therefore corrected together. No upstream ADR depends on the fast path
  avoiding `kick`, so no ADR change is required — though the per-release
  notification is a candidate finding for #637/#638. Date/Author: 2026-10-11,
  implementer.

- Decision: leave the benchmark-support modules above the 400-line
  `AGENTS.md` guidance rather than splitting them, after establishing that
  whitaker's `module_max_lines` lint does not apply to `wireframe_testing`.
  Rationale: `push_benchmark_support.rs` (604 lines),
  `client_pool_benchmark_support.rs` (492 at HEAD, 703 after the bracket
  correction), and `connection_actor_benchmark_support.rs` (463) all exceeded
  400 at HEAD and `whitaker --all -- --all-targets --all-features` still exited
  0 with no `module_max_lines` diagnostic; the 403-line failure recorded
  earlier was in the root crate's `src/client/pool/scheduler.rs`. Splitting
  these files purely to satisfy a rule that is not enforced here would scatter
  the workload definition, the measurement, and the tests that assert its
  contract across module boundaries, which is the opposite of the colocation
  `AGENTS.md` asks for. The rationale is recorded because it is a *consistency*
  question, not a compliance one: if a later change extends the lint's scope to
  test-support crates, these files will need splitting and this entry is the
  starting point. Date/Author: 2026-10-11, implementer.

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
- Evidence: `make test` passes; the contended case asserts `after > before`;
  the uncontended case asserts `after >= before` and that the delta is bounded
  by the number of acquire/release transitions.
- Discharge condition: both assertions pass, and the counter is not
  permanently zero.
- Non-vacuity: the contended case is the witness that the counter moves under
  load; the uncontended case bounds the fast path. Asserting only a bound
  without the contended witness would be vacuous.
- This can fail if wrong: a mutant that never increments would break both
  cases; a mutant that spawns an unbounded number of workers per release would
  break the bound.
- **Correction (measured).** The originally planned assertion — that an
  uncontended burst produces a delta of exactly `0` because it always takes
  `try_acquire_immediately` — is **false** and has been withdrawn. Every lease
  release reaches `PoolScheduler::kick`, because `PooledClientLease::drop`
  notifies unconditionally and even the fast path arms the lease with a release
  handle. Measured deltas are 1 for 1- and 16-acquisition bursts and 2 for 256,
  at every pool size. See the matching `Surprises & discoveries` entry. The
  obligation now records the true contract: a **bound**, not a zero.

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

- Statement: after pool construction, the acquisition phase releases no
  construction tasks and its scheduler-spawn delta is bounded by the number of
  acquire/release transitions; a contended scenario that forces the scheduler
  slow path does observe at least one additional scheduler spawn.
- Method: `rstest` over pool sizes {1, 4, 16} asserting (a) the acquisition
  phase's live-task delta is `<= 0` once the pool is closed and its spawn delta
  is `>= 1` and `<= 2 x acquisitions`, and (b) the spawn counter is `>= 1` for
  the contended burst.
- Rationale: the bounded-delta claim is only meaningful alongside a
  demonstration that the instrumentation can see movement at all; (b) supplies
  that demonstration, so (a) cannot pass vacuously.
- Domain: pool sizes 1, 4, and 16 under both fairness policies.
- Artefact: `tests/runtime_ownership_benchmark_helpers.rs`; the corrected
  in-module cases live beside the measurement in
  `client_pool_benchmark_support.rs`.
- Evidence: `make test` passes; the mutation transcript in
  `docs/runtime-ownership-baselines.md` shows the construction bracket is
  load-bearing.
- Discharge condition: (a) and (b) both hold.
- Non-vacuity: as above; (b) is the witness. The bracket in (a) was verified by
  mutation: sampling the task gauge before pool construction fails all three
  cases with deltas of 2, 5, and 17. If (b) could not be made to pass, that is
  a verification failure to escalate, not a licence to weaken (a).
- **Correction (measured).** The originally planned "live-task delta is 0 for
  the uncontended burst" was measuring the wrong phase: `bb8` connects eagerly,
  so a size-N pool leaves N connection tasks alive during construction. The
  bracket now excludes construction. See the matching `Surprises & discoveries`
  entry.

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
- Artefact: `verify_allocation_harness` in
  `benches/runtime_ownership_baselines_alloc.rs` for the first two, since they
  need a `#[global_allocator]`; the mutation transcript is recorded in
  `docs/runtime-ownership-baselines.md`.
- **Correction (measured).** This obligation originally placed the zero and
  positive controls in `tests/runtime_ownership_benchmark_helpers.rs`. That
  cannot work: a `tests/` target links the system allocator, so a probe there
  reported `0` for a closure that allocated a 1024-byte buffer. The controls
  live in the allocation bench binary, which does install the allocator and
  runs them before any measurement.
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
`Concrete steps` and summarized in `Surprises & discoveries`. The baseline
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
