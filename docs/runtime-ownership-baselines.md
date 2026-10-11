# Runtime-ownership baselines

This document records reproducible baselines for the four hot paths whose
ownership topology Epic [#635](https://github.com/leynos/wireframe/issues/635)
proposes to change. It exists so that a later change can be compared against
measurement rather than intuition, and so that the comparison needs no rewrite
of the scenario.

The baselines were captured for issue
[#639](https://github.com/leynos/wireframe/issues/639), which is deliberately
scoped to *establishing* them. It lands before the ownership work under
proposals [#637](https://github.com/leynos/wireframe/issues/637) and
[#638](https://github.com/leynos/wireframe/issues/638), so the numbers below
describe the implementation as it stands, not a target.

## Scope

Four scopes, one measurement module each, all under
`wireframe_testing::runtime_ownership_benchmarks`:

| Scope                             | Module                                 | Entry point                |
| --------------------------------- | -------------------------------------- | -------------------------- |
| Connection actor event processing | `connection_actor_benchmark_support`   | `measure_actor_drain`      |
| Push handle send path             | `push_benchmark_support`               | `measure_push`             |
| Application connection startup    | `connection_startup_benchmark_support` | `measure_startup`          |
| Client pool acquisition           | `client_pool_benchmark_support`        | `measure_pool_acquisition` |

Each module keeps its scenarios as data — a workload enum or struct carrying
every parameter a baseline must state — with a single `measure_*` entry point
that returns a `Result` rather than panicking. A Criterion binary, an `rstest`
suite, and a one-off experiment all drive the same code, which is what lets the
harnesses survive the refactor: the scenario is named by behaviour, so pointing
it at a new implementation changes nothing but the number.

The arrangement mirrors `wireframe_testing::codec_benchmarks`, the repository's
existing benchmark-support module. No second benchmark framework is introduced;
the helpers report raw `Duration` and counter values, and Criterion does the
statistics.

## Environment

A baseline is only reproducible alongside the machine and build that produced
it. The named baseline machine is:

| Property  | Value                                                        |
| --------- | ------------------------------------------------------------ |
| CPU       | AMD Ryzen 9 3900 12-Core Processor, 24 hardware threads      |
| Memory    | 125 GiB                                                      |
| OS        | Rocky Linux 10.2 (Red Quartz), kernel 6.12.0-211.62.1.el10_2 |
| Toolchain | `nightly-2026-03-26` (`rustc 1.96.0-nightly 80d0e4be6`)      |
| Cargo     | `1.96.0-nightly (e84cb639e 2026-03-21)`                      |
| Profile   | Cargo `release` — the default for `cargo bench`; no          |
|           | `[profile.*]` override exists in any manifest                |
| Linker    | System default; `make bench-runtime-ownership` assigns       |
|           | `RUSTFLAGS`, which displaces the `mold` flag from            |
|           | `.cargo/config.toml`                                         |
| Features  | `test-support`, `pool`                                       |
| Runtime   | Tokio multi-thread, default worker count                     |

Two of those rows are easy to get wrong and are stated explicitly. There is no
`[profile.bench]` section anywhere in the workspace, so the benchmarks run
under Cargo's built-in `release` profile. And because the
`bench-runtime-ownership` target sets `RUSTFLAGS="-D warnings"`, Cargo takes
that as the single `rustflags` source and drops both `-Zthreads=8` and
`-Clink-arg=-fuse-ld=mold` from `.cargo/config.toml`; a run of this target is
therefore not linked with `mold`, unlike the development targets.

Contended pool acquisition cannot be timed at all — see
[Known defects](#known-defects-in-contended-pool-acquisition) — so it is
reported as a bounded observation rather than in the table below.

## Reproducing

One command captures both benchmark binaries:

```sh
make bench-runtime-ownership
```

It runs
`cargo bench --bench runtime_ownership_baselines --bench
runtime_ownership_baselines_alloc --features test-support,pool`.
The two binaries are separate because `#[global_allocator]` is per-process:
installing the counting allocator in the timing binary would tax every sample.

Criterion writes its own baseline under `target/criterion` and reports a
subsequent run as a percentage change against it. Nothing in continuous
integration runs these benchmarks, and they are not a gate.

Individual scenarios can be selected by name:

```sh
cargo bench --bench runtime_ownership_baselines --features test-support,pool -- \
  pool/acquisition
```

Note that `cargo test --all-targets` also executes both binaries, because
`harness = false` benches are compiled and run in Criterion's *test* mode. They
must therefore stay fast and must not block indefinitely, which is why the
contended burst carries a timeout.

## Baseline

Recorded on the named machine on 2026-10-11, one run, at the sample budget the
binaries pin (20 samples for timings, 10 for allocations). Criterion reports an
interval, not a point; the figure quoted is the middle estimate, with the
interval beside it so the run-to-run spread is visible.

### Timings

Actor drain. `capacity_512` is the per-queue capacity, so a workload of
`frames` preloads half that count per lane and both queues stay under
back-pressure. The three cancellation regimes are different control flows, not
three timings of one scenario, so their columns are not comparable with each
other.

| Workload                                       | Lower     | Estimate  | Upper     |
| ---------------------------------------------- | --------- | --------- | --------- |
| `frames_256_capacity_512_cancel_disabled`      | 125.23 µs | 135.69 µs | 143.77 µs |
| `frames_512_capacity_512_cancel_disabled`      | 235.02 µs | 241.73 µs | 247.14 µs |
| `frames_1024_capacity_512_cancel_disabled`     | 587.89 µs | 623.54 µs | 662.99 µs |
| `frames_256_capacity_512_cancel_before_run`    | 68.576 µs | 73.614 µs | 80.626 µs |
| `frames_512_capacity_512_cancel_before_run`    | 134.06 µs | 146.45 µs | 160.35 µs |
| `frames_1024_capacity_512_cancel_before_run`   | 264.97 µs | 279.31 µs | 294.64 µs |
| `frames_256_capacity_512_cancel_during_drain`  | 102.53 µs | 111.18 µs | 117.92 µs |
| `frames_512_capacity_512_cancel_during_drain`  | 211.54 µs | 220.89 µs | 232.74 µs |
| `frames_1024_capacity_512_cancel_during_drain` | 431.84 µs | 449.52 µs | 463.59 µs |

`cancel_before_run` is the cheapest regime at every frame count because the
actor aborts before draining anything, so its rate is per frame *declined*, not
per frame moved. `cancel_during_drain` sits between the other two: it drains
the same frames as `cancel_disabled` but observes shutdown once, inside the
output hook.

Push send. The two uncontended rows measure an awaited send into a draining
queue; the three overflow rows measure a non-blocking send against a full
queue, one per `PushPolicy` branch. Rate limiting is disabled throughout, so
the cost is queue ownership rather than leaky-bucket bookkeeping.

| Workload                                      | Lower     | Estimate  | Upper     |
| --------------------------------------------- | --------- | --------- | --------- |
| `uncontended_high_capacity_64_sends_1024`     | 482.90 µs | 521.96 µs | 557.50 µs |
| `uncontended_low_capacity_64_sends_1024`      | 429.40 µs | 486.65 µs | 539.26 µs |
| `overflow_return_error_capacity_8_sends_256`  | 4.3252 µs | 4.4581 µs | 4.6347 µs |
| `overflow_drop_capacity_8_sends_256`          | 16.699 µs | 17.284 µs | 17.740 µs |
| `overflow_warn_and_drop_capacity_8_sends_256` | 29.669 µs | 30.474 µs | 31.676 µs |

The two uncontended rows are statistically indistinguishable at this sample
budget, which is the expected result: the actor polls the high-priority queue
first, but the queues are drained as fast as they fill, so neither lane parks.
The overflow rows differ by policy as their implementations do —
`ReturnErrorIfFull` unwinds immediately, `DropIfFull` routes to the dead-letter
queue, and `WarnAndDropIfFull` additionally logs.

Connection startup. `factory_N_transforms_M` in each label is the count the
workload is expected to produce, so a timing that improves while the label
still claims per-connection preparation is visibly inconsistent rather than a
quiet false win. Transport is an in-memory duplex pair, not a socket.

| Workload                                                                         | Lower     | Estimate  | Upper     |
| -------------------------------------------------------------------------------- | --------- | --------- | --------- |
| `per_connection_factory_routes_1_layers_2_connections_1_factory_1_transforms_2`  | 4.2426 µs | 4.4880 µs | 4.8109 µs |
| `per_connection_factory_routes_1_layers_2_connections_4_factory_4_transforms_8`  | 15.860 µs | 16.265 µs | 16.769 µs |
| `per_connection_factory_routes_2_layers_2_connections_1_factory_1_transforms_4`  | 4.7448 µs | 5.0334 µs | 5.3113 µs |
| `per_connection_factory_routes_2_layers_2_connections_4_factory_4_transforms_16` | 19.823 µs | 21.881 µs | 24.148 µs |
| `prepared_root_routes_1_layers_2_connections_1_factory_1_transforms_2`           | 4.6393 µs | 5.1409 µs | 5.5676 µs |
| `prepared_root_routes_1_layers_2_connections_4_factory_1_transforms_2`           | 8.0339 µs | 8.2822 µs | 8.5397 µs |
| `prepared_root_routes_2_layers_2_connections_1_factory_1_transforms_4`           | 4.5398 µs | 5.1854 µs | 5.6862 µs |
| `prepared_root_routes_2_layers_2_connections_4_factory_1_transforms_4`           | 9.0655 µs | 10.578 µs | 11.899 µs |

The contrast is clearest where the work differs most. At four connections and
two routes, the per-connection path spends 21.881 µs against the prepared
root's 10.578 µs, and does four factory evaluations and sixteen middleware
transformations rather than one and four. At a single connection the two paths
cost about the same, which is the point: preparation is a fixed cost the
prepared root pays once and amortizes, so the paths only diverge as connections
multiply. The growth from one to four connections is roughly linear in the
per-connection path (4.4880 → 16.265 µs) and sub-linear in the prepared root
(5.1409 → 8.2822 µs) for the same reason.

Pool acquisition, uncontended. One iteration is one acquire/release burst of
256 acquisitions from a single contender, so the numbers are per burst.

| Workload                                            | Lower     | Estimate  | Upper     |
| --------------------------------------------------- | --------- | --------- | --------- |
| `uncontended_pool_1_acquisitions_256_contenders_1`  | 147.99 µs | 154.17 µs | 162.44 µs |
| `uncontended_pool_4_acquisitions_256_contenders_1`  | 186.84 µs | 191.06 µs | 194.64 µs |
| `uncontended_pool_16_acquisitions_256_contenders_1` | 322.18 µs | 331.58 µs | 340.31 µs |

Cost rises with pool size rather than falling, which is worth recording: a
larger pool buys concurrency, not cheaper single-threaded acquisition. The
16-slot pool is 2.15× the 1-slot pool at the same acquisition count. `bb8`
connects eagerly, so a size-N pool also leaves N connection tasks alive from
construction.

### Allocations

Counts are **process-global deltas across the measurement window**, taken over
four iterations. They include the async runtime's own allocations, task setup,
and closure captures — not only the code under test. Compare counts across runs
of this same binary; do not read them as a per-operation cost.

| Scope                                                                            | Iterations | Allocations |
| -------------------------------------------------------------------------------- | ---------- | ----------- |
| `frames_256_capacity_128_cancel_disabled`                                        | 4          | 2134        |
| `uncontended_high_capacity_64_sends_256`                                         | 4          | 40          |
| `overflow_drop_capacity_8_sends_256`                                             | 4          | 56          |
| `prepared_root_routes_2_layers_2_connections_4_factory_1_transforms_4`           | 4          | 160         |
| `per_connection_factory_routes_2_layers_2_connections_4_factory_4_transforms_16` | 4          | 408         |
| `uncontended_pool_4_acquisitions_256_contenders_1`                               | 4          | 2284        |

The startup pair is the sharpest allocation contrast in the table, and it
parallelizes the timing contrast: 160 allocations for the shared prepared root
against 408 for the per-connection path over the same four connections. The
difference of 248 is not exactly `3 × (one preparation)` because preparation is
only part of what a per-connection start allocates; the connection and its
transport buffer are allocated either way.

The pool figure varies between runs — 2284 and 2496 were observed across two
captures of the same binary — which is expected for a scope that spawns tasks
and opens connections. A change smaller than that spread is not evidence of
anything at this scope; prefer the scheduler spawn count below, which was
stable.

### Contended acquisition counters

Reported as `tracing` events after `Criterion::final_summary`, not as timings.
`granted` is the last iteration's completed acquisitions (128 of 256, because
the burst splits across the two fairness policies); `alive_task_delta` is
measured across the acquisition phase only, with pool construction and close
held outside the bracket. `Elapsed` is the wall time of that last burst and is
a single observation, not a sample — the interval Criterion would report is
unavailable for a scenario that can fail rather than merely vary, so this
column is not comparable with the tables above at the same confidence.

| Workload                                                      | Granted | Scheduler spawns | Alive-task delta | Elapsed    |
| ------------------------------------------------------------- | ------- | ---------------- | ---------------- | ---------- |
| `contended_fifo_pool_4_acquisitions_256_contenders_16`        | 128     | 16               | 0                | 183.953 µs |
| `contended_round_robin_pool_4_acquisitions_256_contenders_16` | 128     | 16               | 0                | 118.552 µs |

The spawn counts are the task-churn evidence the issue asks for at this scope,
and they are what makes the uncontended bound meaningful: the counter
demonstrably moves under demand.

The same counter under *no* contention isolates what the acquire path costs
when it is not fighting itself. An uncontended burst records 1 spawn for a
burst of 1 or 16 acquisitions and 2 for 256, identically at pool size 1, 4, and
16. So the hidden ownership cost is neither per acquisition nor per release: it
is one scheduler task per idle-to-busy transition, and a burst only re-enters
the slow path when its worker wins the drain race and exits before the next
release arrives. `kick` spawns only on that compare-exchange, which is what
bounds the count — a burst cannot spawn more workers than it performs
transitions, and the suite asserts both the lower and upper bound so neither
half can rot silently.

The contended observation also carries an allocation count, taken once from a
counted window after Criterion finishes: 2176 allocations over four iterations
of `contended_fifo_pool_4_acquisitions_256_contenders_16`. Read it against the
uncontended figure of 2284 for the same shape — the two are within the
run-to-run spread noted above, so contention did not measurably change the
allocation count at this scope even though it changed the spawn count
sixteenfold.

## Known defects in contended pool acquisition

Capturing the pool baseline surfaced two pre-existing defects in the acquire
path. Neither was introduced by this work, neither is fixed by it, and both are
recorded here because they are exactly the kind of hidden ownership cost Epic
[#635](https://github.com/leynos/wireframe/issues/635) exists to uncover.

### Spurious `disconnected` errors

`PoolScheduler::acquire_for_handle` reports `disconnected` whenever it dequeues
a waiter whose session has already stopped listening. A contended round reaches
that state naturally: a caller that abandons an acquisition leaves its sender
queued, and `close()` racing an in-flight acquire abandons receivers outright.
The lease being discarded is healthy, so a caller can be told the pool has gone
away while the pool is still serving traffic.

Two distinct windows reach it:

1. A contender observes the last busy slot and returns, dropping its receiver,
   while a worker concurrently services that same waiter.
2. `close()` races in-flight acquires, so `notify_shutdown` drains the queue
   and abandons receivers.

Measured on the named machine: the first failure lands at burst 913 of 20,000
at 16 contenders, and at roughly 1 round in 400 for the lighter 8-contender
workload used by the regression tolerance in
`tests/runtime_ownership_benchmark_helpers.rs`. Reproducing it on a
*long-lived* pool, with no per-iteration rebuild and no task leak, is what
proves it is an acquire-path defect rather than an artefact of how the
benchmark constructs its pool.

It is also what aborted the allocation binary's first capture. That binary ran
the contended scope through Criterion like any other, and panicked at
`benches/runtime_ownership_baselines_alloc.rs` with
`1 of 16 contenders failed; first: transport error: connection closed by peer` —
the defect reached from unmodified benchmark code driving unmodified library
code, with no probe scaffolding in between. Criterion's timing pass runs
several orders of magnitude more iterations than an allocation window, so it
hits the interleaving nearly every run. The scope was moved out of the timed
group and reported once as an observation instead; the count itself was never
in doubt, because it comes from a single counted window that completed before
the failure.

### Sustained contention stalls the acquire path

Under sustained contention the acquire path intermittently stops making
progress altogether. Every contender stalls with waiters still queued, and the
burst never completes. A `gdb` sample of the stalled process showed all 18
Tokio workers parked in `parking_lot::Condvar::wait` or `mio::Poll::poll`, with
CPU time frozen at `79s 8cs` across three samples 36 seconds apart — a true
deadlock, not slowness.

The stall is a property of the contention, not of the harness. First-stall
burst numbers observed on the named machine were 272, 386, 808, 814, 2361,
2739, 4233, 4797, 7784, 10727, and 18069 — across both fairness policies, a
pool rebuilt per iteration and one long-lived pool, fresh and reused handles,
and contender counts of 8 and 16 with 1, 2, and 8 rounds each.

A control separates the stall from a second, unrelated defect. Building and
closing a pool leaks roughly one live task per slot per iteration —
`acquisitions: 0` is enough to grow the gauge by exactly 4 per iteration for a
4-slot pool. That leak is real, but it is not the cause: the same
4-tasks-per-iteration leak ran **20,000 uncontended iterations with zero
stalls**.

### Consequences for the harness

Criterion cannot time contended acquisition. A stall occurs during warm-up,
before any sample is recorded, which would abort the whole run. The benchmark
therefore reports the contended counters as `tracing` events after
`Criterion::final_summary`, guarded by `CONTENDED_BURST_TIMEOUT` (30 seconds),
so that a stall surfaces as a reported error instead of a process that never
returns.

The counters are the evidence the issue asks for at this scope — whether a
contended burst genuinely enters the scheduler's slow path, and what task churn
it costs — and they are recorded. They are recorded as observations rather than
timings because the scenario can fail rather than merely vary.

A candidate one-line repair was implemented and measured during this work: keep
the discarded lease instead of reporting `disconnected`. It removed the error
but converted it into an intermittent deadlock, which is strictly worse — a
caller that would have received a fast error instead stalls forever. Under the
repair, `make bench-runtime-ownership` hung during Criterion warm-up with the
main thread parked in `Criterion::iter_custom` → `Runtime::block_on` →
`measure_pool_acquisition`. The repair was reverted. An earlier reading that
"the fix causes the hang" was confounded by a discarded experiment design; the
probe against the reverted code is what settled it, because the unmodified
library stalls too.

The acquire path is the subject of
[ADR 013](adr-013-client-pool-scheduler-and-slot-ownership.md) and issue
[#647](https://github.com/leynos/wireframe/issues/647). Fixing it is out of
scope for #639, whose purpose is to record baselines before the ownership
topology changes.

## Instrumentation sensitivity

A baseline is worthless if the counter behind it cannot move. Three controls
establish that these can.

### Allocation harness

`benches/runtime_ownership_baselines_alloc.rs` runs three checks before any
measurement, because the counter is process-global and a background thread
allocating during a window would make the zero case untrustworthy:

1. an allocation-free closure reports exactly zero, catching a gate stuck open;
2. a closure allocating a 1024-byte buffer reports a positive count, catching a
   gate that never opens — which would otherwise record a silent baseline of
   zeros for every scope; and
3. a per-item `Arc` construction reports one allocation per item more than
   cloning a shared `Arc`.

The third check is the direct answer to acceptance criterion 5, which asks
whether removing a nested `Arc` changes the allocation count. The question has
two possible answers and the harness must distinguish them. Cloning a shared
`Arc` costs an atomic increment and no allocation, so removing such a clone
changes nothing an allocation counter can see. Constructing a fresh `Arc` per
item costs one allocation per item, so removing it moves the count by at least
the item count. The two probe shapes are recorded as a label pair so the
contrast is visible in the report, and the assertion demands a delta of
`items - 1` rather than `items`: the shared probe still allocates once for its
single `Arc::new`, and demanding the full item count would fail a correct
counter for an off-by-one reason.

Recorded, over 256 items:

| Probe                                     | Allocations | Lower     | Estimate  | Upper     |
| ----------------------------------------- | ----------- | --------- | --------- | --------- |
| `arc_shared_clone_items_256_allocs_1`     | 1           | 2.3559 µs | 2.3892 µs | 2.4167 µs |
| `arc_fresh_per_item_items_256_allocs_256` | 256         | 3.6216 µs | 3.7454 µs | 3.9512 µs |

The delta is exactly 255, which is `ARC_PROBE_ITEMS - 1`. The counter therefore
resolves a per-item `Arc` allocation and sees nothing in a clone, which is the
resolution the criterion requires: a refactor that removes a *clone* from a
shared `Arc` will not move these numbers, and one that removes a per-item
construction will move them by at least the item count. The two timings differ
by only ~57 %, so a reader who ignored the counts could mistake these shapes
for equivalent — the label pair is what makes the distinction legible in the
Criterion report.

### Construction bracket

The pool task-count assertion is only meaningful if the gauge can see movement,
so the suite also asserts that a contended burst records a non-zero scheduler
spawn count. That witness is what makes the uncontended bound meaningful: the
counter demonstrably moves under demand, so its behaviour above is a measured
property rather than a counter wired to nothing.

The bracket itself was verified by mutation rather than argued. Sampling the
task gauge *before* pool construction made
`acquisition_phase_does_not_leak_pool_construction_tasks` fail on all three
cases, with deltas of 2, 5, and 17 for 1-, 4-, and 16-slot pools — scaling with
pool size, which is the intended failure reason. The bracket is load-bearing,
not cosmetic.

### Live-task gauge

`RuntimeMetrics::num_alive_tasks` counts every task on the runtime, not just
this workload's. Two consequences follow, and both were established by
measurement rather than assumed.

The gauge is **only meaningful on a current-thread runtime** when the pool is
running alongside other work. Sampling an idle→busy acquisition burst on a
multi-thread runtime produced deltas from −12 to +10, with a long tail above +2
in roughly a third of runs, at every pool size — on a workload whose own task
churn is a fixed small constant. Those readings move with the runtime's
scheduler, not with the pool, so they cannot discharge an assertion about pool
ownership. On a current-thread runtime the same workload is deterministic: 180
samples across pool sizes 1, 4, and 16 recorded only −1 and 0, with no outlier.

Settlement therefore has to be established, not assumed. A worker woken by the
final release observes an empty queue and exits on its first poll, so a gauge
read taken immediately afterwards can still count a task that is already on its
way out. Waiting for the gauge to hold steady across several consecutive polls
— rather than merely to stop falling once — is what turns the reading into a
settled state. A single flat pair of reads is not evidence of quiescence: an
exiting worker that has not yet been scheduled produces exactly that.

The asserted bound is consequently a non-scaling ceiling, not a tight fit to
the observed range. It exists to catch a delta that grows with `pool_size` —
the failure the mutation above demonstrates — and the property under test is
"does not scale with the pool", not "happens to settle at or below zero".

## Regression policy

Benchmark results are **not** a correctness gate. No absolute timing threshold
appears in any test, and nothing in continuous integration runs these binaries.

Regression handling is by ratio and review:

- Compare a post-refactor run against this baseline on the same machine class
  and the same feature set, and treat a change as worth investigating when it
  exceeds the noise the run-to-run interval already shows rather than at a
  fixed percentage.
- Prefer the *counts* — factory invocations, middleware transformations,
  scheduler spawns, live-task deltas, allocations — over timings when judging
  an ownership change. They are exact integers, they do not vary with machine
  load, and they are the quantities the ADRs actually propose to change.
- A scenario whose parameters changed cannot be compared against this table.
  Every workload label embeds its parameters for exactly this reason, so a
  mismatch is visible rather than silent.

The one place a count *is* asserted is
`tests/runtime_ownership_benchmark_helpers.rs`, where the factory and
middleware counts are exact equalities over a finite parameter grid. Those are
correctness properties of the prepared-application path, not performance
thresholds, and they belong in the suite.

## References

- Epic [#635](https://github.com/leynos/wireframe/issues/635)
- [#639](https://github.com/leynos/wireframe/issues/639)
- [#647](https://github.com/leynos/wireframe/issues/647)
- [ADR 011: runtime ownership and task-lifetime boundaries](adr-011-runtime-ownership-and-task-lifetime-boundaries.md)
- [ADR 012: prepared application and connection runtime](adr-012-prepared-application-and-connection-runtime.md)
- [ADR 013: client pool scheduler and slot ownership](adr-013-client-pool-scheduler-and-slot-ownership.md)
