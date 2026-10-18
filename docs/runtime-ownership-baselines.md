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

Contended pool acquisition cannot be timed by Criterion — see
[Known defects](#known-defects-in-contended-pool-acquisition) — because a
failure during warm-up would abort the whole run. It is instead sampled by the
harness itself, which records each burst separately and keeps failures out of
the samples rather than letting one end the run. The result is a latency
distribution reported after `Criterion::final_summary`, beside the counters,
rather than a row in the timed tables.

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

Every figure below is a single run on the named machine at the sample budget
the binaries pin (20 samples for timings, 10 for allocations). Criterion
reports an interval, not a point; the figure quoted is the middle estimate,
with the interval beside it so the run-to-run spread is visible.

A baseline without its revision is not reproducible, so each capture is named
with the commit that produced it. Two captures are recorded here, and a table
belongs to whichever one its section names.

### Captures

| Capture   | Revision                                                | Date       | Covers                          |
| --------- | ------------------------------------------------------- | ---------- | ------------------------------- |
| Original  | `d28b54e`                                               | 2026-10-11 | Every table except those noted  |
| Corrected | PR [#756](https://github.com/leynos/wireframe/pull/756) | 2026-10-12 | Push, startup, allocation, pool |

The original capture is the first published set, taken before the review of
[#756](https://github.com/leynos/wireframe/pull/756). The corrected capture
answered four defects that review found in the measurement boundaries, so the
two are not interchangeable: several scenarios measure a changed workload, and
the pool allocation window measures a different region. Where a figure changed
for a reason other than run-to-run noise this is stated with the table, and the
original is kept beside it so a reader can see what moved and why.

The corrected capture is named by its pull request rather than a commit hash
because it is the change *in* that pull request; the hash does not exist until
it merges. The original capture's hash does exist and is given directly.

The corrections were:

1. **Pool construction moved outside the acquisition timing and allocation
   windows.** It was inside both. `bb8` connects eagerly, so the counted region
   included one loopback connection per slot — a term that grows with
   `pool_size` and had been recorded as acquire-path cost.
2. **The scheduler spawn counter became per-pool.** It was a process-global
   `static`, so a delta taken while any other pool was running included that
   pool's churn. It is now [`PoolTaskChurn`](../src/client/pool/churn.rs), read
   through `WireframeClientPool::task_churn_observer`, and a delta means what
   it says.
3. **Contention is now proven rather than inferred.** A start barrier does not
   guarantee overlapping lease ownership; a task can finish several
   immediately-ready acquisitions before another runs. The contended regimes
   now hold leases and assert an observed capacity wait before releasing
   capacity.
4. **Awaited back-pressure became its own scenario.** The uncontended send
   path awaits capacity when the queue fills, and `reported_queue_full` does
   not observe that wait — it is always false there. `backpressure_high`
   exercises the awaited path deliberately; the uncontended scenarios assert
   they never parked.

None of the corrections changes the implementation being measured. They change
what the windows contain and what the counters mean, which is why the corrected
capture supersedes the original for those scopes rather than merely updating it.

### Timings

Actor drain. `capacity_512` is the per-queue capacity, so a workload of
`frames` preloads half that count per lane and neither queue reaches
back-pressure. The three cancellation regimes are different control flows, not
three timings of one scenario, so their columns are not comparable with each
other.

Corrected capture (PR [#756](https://github.com/leynos/wireframe/pull/756)):

| Workload                                       | Lower     | Estimate  | Upper     |
| ---------------------------------------------- | --------- | --------- | --------- |
| `frames_256_capacity_512_cancel_disabled`      | 81.790 µs | 82.901 µs | 84.125 µs |
| `frames_512_capacity_512_cancel_disabled`      | 172.54 µs | 178.67 µs | 183.65 µs |
| `frames_1024_capacity_512_cancel_disabled`     | 339.09 µs | 353.44 µs | 369.83 µs |
| `frames_256_capacity_512_cancel_before_run`    | 34.668 µs | 35.064 µs | 35.378 µs |
| `frames_512_capacity_512_cancel_before_run`    | 69.102 µs | 69.813 µs | 70.568 µs |
| `frames_1024_capacity_512_cancel_before_run`   | 136.88 µs | 138.37 µs | 139.84 µs |
| `frames_256_capacity_512_cancel_during_drain`  | 67.794 µs | 68.436 µs | 68.917 µs |
| `frames_512_capacity_512_cancel_during_drain`  | 133.21 µs | 136.07 µs | 139.31 µs |
| `frames_1024_capacity_512_cancel_during_drain` | 266.40 µs | 293.32 µs | 321.88 µs |

Original capture (`d28b54e`), same workloads and same benchmark code:

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

**This table fell by 1.35–2.10× between captures without its benchmark code
changing at all**, which is the clearest evidence here that the two captures
are not a controlled comparison. `measure_actor_drain` and `drain_once` are
byte-identical before and after, so the fall cannot be read off the benchmark.
Part of it does have a code-level cause: `preload` pushes every frame through
the awaited path, and at `d28b54e` that path ran `tx.clone().send(frame).await`
on every send, whereas it now probes with `tx.try_reserve()` first and clones
only when the queue is full. `cancel_before_run` — pure preload, with no drain
to dilute it — moved the most uniformly (2.02–2.10×). Both regimes that pay a
drain moved less than that, `cancel_disabled` reaching the smallest ratio of
all at 1.35× and `cancel_during_drain` staying in the narrowest band
(1.53–1.62×), which is the pattern that change would produce. That attribution
is consistent with the figures, not established by them: the machine was
carrying a load average of 10–15 on 24 threads from other work during the
corrected run, and the [startup](#timings) rows moved by a comparable factor
with no code change on their path at all. Treat the corrected column as this
baseline's figures and the original as historical, and do not read the ratio
between them as a speed-up this work delivered.

Push send. The uncontended rows measure an awaited send into a draining queue;
the overflow rows measure a non-blocking send against a full queue, one per
`PushPolicy` branch. Rate limiting is disabled throughout, so the cost is queue
ownership rather than leaky-bucket bookkeeping.

Corrected capture (PR [#756](https://github.com/leynos/wireframe/pull/756)):

| Workload                                      | Lower     | Estimate  | Upper     |
| --------------------------------------------- | --------- | --------- | --------- |
| `uncontended_high_capacity_64_sends_64`       | 16.414 µs | 17.089 µs | 17.823 µs |
| `backpressure_high_capacity_8_sends_64`       | 43.430 µs | 45.494 µs | 47.741 µs |
| `drained_high_capacity_64_sends_1024`         | 172.31 µs | 184.75 µs | 196.43 µs |
| `drained_low_capacity_64_sends_1024`          | 193.87 µs | 215.84 µs | 234.48 µs |
| `overflow_return_error_capacity_8_sends_256`  | 2.4162 µs | 2.6053 µs | 2.8774 µs |
| `overflow_drop_capacity_8_sends_256`          | 13.067 µs | 13.158 µs | 13.252 µs |
| `overflow_warn_and_drop_capacity_8_sends_256` | 19.773 µs | 20.234 µs | 20.733 µs |

Original capture (`d28b54e`), for the scenarios whose workload changed:

| Workload                                      | Lower     | Estimate  | Upper     |
| --------------------------------------------- | --------- | --------- | --------- |
| `uncontended_high_capacity_64_sends_1024`     | 482.90 µs | 521.96 µs | 557.50 µs |
| `uncontended_low_capacity_64_sends_1024`      | 429.40 µs | 486.65 µs | 539.26 µs |
| `overflow_return_error_capacity_8_sends_256`  | 4.3252 µs | 4.4581 µs | 4.6347 µs |
| `overflow_drop_capacity_8_sends_256`          | 16.699 µs | 17.284 µs | 17.740 µs |
| `overflow_warn_and_drop_capacity_8_sends_256` | 29.669 µs | 30.474 µs | 31.676 µs |

**The send-count change is the reason the row names differ, and the reason
those two captures are not comparable.** The original uncontended rows sent
1024 frames; the corrected ones send 64, so the ~30× fall from 521.96 µs to
17.089 µs is the workload change and nothing else. 64 and 1024 are not
interchangeable samples of one quantity — they are different
`PUSH_CONTROL_SENDS` and `PUSH_DRAINED_SENDS` — so the two rows must not be
diffed.

The uncontended rows are now separated from the drained ones deliberately.
`uncontended_*` sends 64 frames into a queue of capacity 64, and the scenario
asserts that it *never* had to await capacity. `drained_*` sends 1024 frames,
which does drive the queue to full and does exercise the awaited path — that is
what a draining consumer makes possible, and it is why the two are no longer
folded together. What the uncontended rows measure is therefore the awaiter's
fast path, and the assertion is what makes that a claim rather than an
assumption: `push_with_priority` awaits capacity when the queue fills, so a
scenario that only *intends* never to park had no way to show it. The count the
API returns cannot show it either, because `reported_queue_full` is `false` on
the awaited path whether or not it parked.

`backpressure_high` is the deliberate contrast: same capacity 8 as the overflow
rows, but an awaited send rather than a non-blocking one, so the producer parks
while a consumer drains. At 45.494 µs against the uncontended 17.089 µs for the
same 64 sends, it is roughly 2.7× the cost — that gap *is* the back-pressure,
and it is now recorded rather than inferred from a scenario that never entered
it.

The overflow rows differ by policy as their implementations do —
`ReturnErrorIfFull` unwinds immediately, `DropIfFull` routes to the dead-letter
queue, and `WarnAndDropIfFull` additionally logs. All three also fell
(1.31–1.71×) between the two captures, and unlike the uncontended pair their
scenario did not change: capacity 8, 256 sends, and the same drain of the DLQ
receiver after each iteration, which was already present at `d28b54e`. The
`DropIfFull` and `WarnAndDropIfFull` arms were rewritten in this branch — the
dead-letter accounting moved into its own module — but the rewrite is
behaviour-preserving and its cost is on the *drop* path only, a branch
`ReturnErrorIfFull` never enters, so it cannot explain all three rows moving
together. They are the control that shows the whole capture shifted: these rows
are dominated by a non-blocking `try_send` whose code path and workload are
both effectively fixed, and they moved by a factor comparable to the actor and
startup scopes, which are byte-identical. The fall is therefore recorded as
machine state, not as an improvement attributable to the harness, and anyone
comparing against these rows should prefer the corrected capture.

Connection startup. `factory_N_transforms_M` in each label is the count the
workload is expected to produce, so a timing that improves while the label
still claims per-connection preparation is visibly inconsistent rather than a
quiet false win. Transport is an in-memory duplex pair, not a socket.

Corrected capture (PR [#756](https://github.com/leynos/wireframe/pull/756)),
extended to 16 connections:

| Workload                                                                           | Lower     | Estimate  | Upper     |
| ---------------------------------------------------------------------------------- | --------- | --------- | --------- |
| `per_connection_factory_routes_1_layers_2_connections_1_factory_1_transforms_2`    | 2.9082 µs | 2.9501 µs | 3.0150 µs |
| `per_connection_factory_routes_1_layers_2_connections_4_factory_4_transforms_8`    | 11.844 µs | 12.311 µs | 12.769 µs |
| `per_connection_factory_routes_1_layers_2_connections_16_factory_16_transforms_32` | 45.139 µs | 45.618 µs | 46.295 µs |
| `per_connection_factory_routes_2_layers_2_connections_1_factory_1_transforms_4`    | 3.3141 µs | 3.3927 µs | 3.4578 µs |
| `per_connection_factory_routes_2_layers_2_connections_4_factory_4_transforms_16`   | 12.368 µs | 12.645 µs | 12.888 µs |
| `per_connection_factory_routes_2_layers_2_connections_16_factory_16_transforms_64` | 49.694 µs | 51.003 µs | 52.703 µs |
| `prepared_root_routes_1_layers_2_connections_1_factory_1_transforms_2`             | 2.9858 µs | 3.1333 µs | 3.3845 µs |
| `prepared_root_routes_1_layers_2_connections_4_factory_1_transforms_2`             | 5.4332 µs | 5.5683 µs | 5.7301 µs |
| `prepared_root_routes_1_layers_2_connections_16_factory_1_transforms_2`            | 16.597 µs | 17.246 µs | 18.155 µs |
| `prepared_root_routes_2_layers_2_connections_1_factory_1_transforms_4`             | 3.5082 µs | 3.5997 µs | 3.6834 µs |
| `prepared_root_routes_2_layers_2_connections_4_factory_1_transforms_4`             | 5.9337 µs | 6.0488 µs | 6.1781 µs |
| `prepared_root_routes_2_layers_2_connections_16_factory_1_transforms_4`            | 15.656 µs | 15.937 µs | 16.226 µs |

Original capture (`d28b54e`), 1 and 4 connections only:

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
two routes in the corrected capture, the per-connection path spends 12.645 µs
against the prepared root's 6.0488 µs, and does four factory evaluations and
sixteen middleware transformations rather than one and four. At a single
connection the two paths cost about the same, which is the point: preparation
is a fixed cost the prepared root pays once and amortizes, so the paths only
diverge as connections multiply.

The 16-connection rows are why the axis was extended. At 16 connections and two
routes the per-connection path costs 51.003 µs against the prepared root's
15.937 µs — a ratio of 3.2×, against 2.1× at four connections and 0.94× at one.
The gap widens monotonically across the three sizes, which is the property a
single pair of sizes could not show: an earlier concern was that 4 connections
is too small to be representative of an application, and at 4 the two paths are
still within a factor of two, so the amortization argument rested on a
difference that a reader could reasonably call marginal. It is not marginal at
16.

The per-connection path is close to linear in connections: 2.9501 → 12.311 →
45.618 µs across 1, 4, and 16, which is 4.2× and then 3.7× for a 4× and 4×
step. The prepared root is sub-linear even at the extended size (3.1333 →
5.5683 → 17.246 µs, or 1.78× and 3.10× for the same steps) because only the
connection-and-transport part of its cost scales; preparation is paid once.
Note the sub-linearity is weaker going from 4 to 16 than from 1 to 4, which
fits the same explanation: at larger connection counts the per-connection part
dominates the fixed preparation term.

**Every startup row is below its counterpart in the original capture, by
1.32–1.75×, and none of the code on this path changed.** `measure_startup` is
byte-identical, and so are `src/app`, `src/connection`, and `src/hooks` — the
whole library path a connection start runs through — both across
`d28b54e..HEAD` and in this branch's working tree. The startup scope never
touches the push handle or the client pool, so neither `try_reserve` nor the
pool bracket can reach it. The shift is therefore machine state, and it
corroborates the same caution recorded for the actor table: the corrected
capture's absolute figures are this baseline, but the *ratio* between the two
captures is not a measurement of anything this work changed. The within-capture
relationships — the per-connection-to-prepared ratio, and its growth with
connection count — are what the table is for, and those hold in either column.

The route axis is deliberately left at 1 and 2. Routes multiply the middleware
count a prepared root is built with, so they change *what* is prepared rather
than *how many times*, and the timing contrast the section exists to show is
driven by the connection count. Extending the route axis would multiply the
table without adding a regime that tests the amortization claim. Should a later
change make per-route preparation the question, the workload struct already
carries the parameter and the axis can be widened in the same way this one was.

Pool acquisition, uncontended. One iteration is one acquire/release burst of
256 acquisitions from a single contender, so the numbers are per burst.

Corrected capture (PR [#756](https://github.com/leynos/wireframe/pull/756)),
construction outside the clock:

| Workload                                            | Lower     | Estimate  | Upper     |
| --------------------------------------------------- | --------- | --------- | --------- |
| `uncontended_pool_1_acquisitions_256_contenders_1`  | 114.72 µs | 116.65 µs | 118.57 µs |
| `uncontended_pool_4_acquisitions_256_contenders_1`  | 120.45 µs | 123.88 µs | 126.81 µs |
| `uncontended_pool_16_acquisitions_256_contenders_1` | 170.24 µs | 185.31 µs | 197.45 µs |

Original capture (`d28b54e`), construction inside the clock:

| Workload                                            | Lower     | Estimate  | Upper     |
| --------------------------------------------------- | --------- | --------- | --------- |
| `uncontended_pool_1_acquisitions_256_contenders_1`  | 147.99 µs | 154.17 µs | 162.44 µs |
| `uncontended_pool_4_acquisitions_256_contenders_1`  | 186.84 µs | 191.06 µs | 194.64 µs |
| `uncontended_pool_16_acquisitions_256_contenders_1` | 322.18 µs | 331.58 µs | 340.31 µs |

Cost rises with pool size rather than falling, which is worth recording: a
larger pool buys concurrency, not cheaper single-threaded acquisition. The
corrected 16-slot pool is 1.59× the 1-slot pool at the same acquisition count,
against 2.15× before. `bb8` connects eagerly, so a size-N pool also leaves N
connection tasks alive from construction.

**This is the one table where the correction has a mechanism that the figures
themselves support.** At `d28b54e` the timed region was a loop whose every
iteration called `pool_once`, which built a pool, ran one acquire/release
burst, and closed the pool — all inside the clock
(`wireframe_testing/.../client_pool_benchmark_support.rs`, lines 554–564). Each
timed iteration therefore paid for `pool_size` eager loopback connections.
`PoolHarness` now builds and warms the pool once, outside the clock, so an
iteration times the acquire/release burst alone. The three workload labels are
unchanged, so the two tables differ only in what the clock enclosed.

The removed term scales with pool size, and it is visible in the deltas: 154.17
− 116.65 = 37.52 µs at 1 slot, 191.06 − 123.88 = 67.18 µs at 4, and 331.58 −
185.31 = 146.27 µs at 16. A per-connection construction cost would produce
exactly that progression, which is what makes construction the explanation
rather than merely a candidate.

The confound recorded for the actor and startup tables applies here too — the
load average during this run was 10–15 on 24 threads — so the *individual*
figures are not a clean measure of the construction term alone; the
37.52/67.18/146.27 progression is the part that carries the argument, and it
survives the confound because it is a within-capture relationship between three
rows measured minutes apart under the same load. What the bracket's *existence*
rests on is not this table at all: it is the zero-iteration equality asserted by
`verify_pool_bracket_excludes_construction`, which is deterministic and does
not depend on a cross-capture comparison.

### Allocations

Counts are **process-global deltas across the measurement window**, taken over
four iterations. They include the async runtime's own allocations, task setup,
and closure captures — not only the code under test. Compare counts across runs
of this same binary; do not read them as a per-operation cost.

The benchmark ID is the workload's label alone, with no count appended:
Criterion keys its saved results, `--baseline` comparisons, and change reports
on that ID, and an allocation count is not stable enough to be an ID — the
figures below move by tens between runs. Each run therefore reports its counts
as structured `tracing` events, one per scope, beside the label they belong to.
The `Allocations` column repeats those counts for the table.

| Scope                                                                            | Iterations | Allocations |
| -------------------------------------------------------------------------------- | ---------- | ----------- |
| `frames_256_capacity_128_cancel_disabled`                                        | 4          | 2133        |
| `drained_high_capacity_64_sends_256`                                             | 4          | 44          |
| `overflow_drop_capacity_8_sends_256`                                             | 4          | 56          |
| `prepared_root_routes_2_layers_2_connections_4_factory_1_transforms_4`           | 4          | 160         |
| `per_connection_factory_routes_2_layers_2_connections_4_factory_4_transforms_16` | 4          | 408         |
| `uncontended_pool_4_acquisitions_256_contenders_1`                               | 4          | 2052        |

The startup and actor rows are unchanged from `d28b54e` apart from one
allocation on the actor scope and four on the push scope, which is run-to-run
jitter at this scale rather than a change. The push row was renamed:
`uncontended_high_...` named a scenario that no longer measures what the
allocation scope needs, because the uncontended scenario now asserts it never
parked. The scope measures the pushed *workload*, and
`drained_high_capacity_64_sends_256` is the scenario that actually sends 256
frames through a queue, so the label follows the scenario. The count is
recorded under both names for continuity: 42 in one earlier capture, 40 in the
previous one, 44 here.

Note that the allocation counts are *not* subject to the load confound the
timing tables carry. Two runs under different load can report the same count
exactly, because the counter is an integer and the workloads are deterministic;
the 2133/160/408 rows reproduce across captures, and the pool row's movement
has its own explanation below.

The startup pair is the sharpest allocation contrast in the table, and it
parallelizes the timing contrast: 160 allocations for the shared prepared root
against 408 for the per-connection path over the same four connections. The
difference of 248 is not exactly `3 × (one preparation)` because preparation is
only part of what a per-connection start allocates; the connection and its
transport buffer are allocated either way.

The pool figure is the one this capture changed, and it is the reason the row
exists in a section about review corrections. **The measurement boundary moved,
not the pool.** The original 2284 was counted with the pool's construction and
teardown inside the window; the corrected 2052 counts the acquire path alone,
with construction built and closed around the counted call. The two numbers are
not a before-and-after of one quantity and must not be diffed.

What the corrected boundary buys is not a smaller number, it is a number that
*means* something: previously the count mixed a `pool_size`-scaling term into a
figure attributed to acquisition, so a future pool-size comparison would have
been reading connection setup.

The magnitude of this row should not be leaned on either way. The construction
term is on the order of 50 allocations for a 4-slot pool, and the row's own
run-to-run spread is several times that: under the corrected boundary the same
binary has read 2052, 2381, 2580, and 2738 across four captures, a spread of
~33%. The move from 2580 to 2052 sits well inside that, so it is jitter, not
evidence that excluding construction reduced the count. That the term is
invisible here is a fact about this pool's size, not a fact about the
correction being cosmetic — which is exactly why the bracket needed its own
deterministic check rather than a figure comparison. Prefer the scheduler spawn
count below, which was stable.

The boundary is now asserted rather than trusted, by a check that runs before
any measurement; see [Allocation bracket](#allocation-bracket) for how it
separates the two terms and why it does so at zero iterations.

### Contended acquisition counters

Reported as `tracing` events after `Criterion::final_summary`, not as timings.
A contended burst is driven by rounds per contender, not by an acquisition
target: each of the 16 contenders runs 8 acquire/release rounds
(`CONTENDED_ROUNDS_PER_CONTENDER`), so a fully successful burst grants 128
leases. `alive_task_delta` is measured across the acquisition phase only, with
pool construction and close held outside the bracket.

**Contention is now proven, not inferred from task creation.** The regime holds
leases long enough to force competing acquisitions to wait, then asserts an
observed capacity wait *before* releasing capacity. This matters because a
start barrier alone does not guarantee overlapping ownership: a task can
complete several immediately-ready acquisitions before another task is
scheduled, in which case a burst that looks contended is in practice a sequence
of uncontended ones. The barrier makes the contenders start together; the
capacity-wait assertion is what makes them *compete*. Scheduler spawn counts
remain task-churn evidence and are still reported — they are no longer asked to
double as proof that contention occurred.

The previous `Elapsed` column was a single wall-clock observation of one burst.
A single value is not a distribution, so the corrected capture samples the
burst 20 times (`CONTENDED_LATENCY_SAMPLES`, matching the timing groups'
budget) and reports order statistics. Attempts, successes, and failures are
reported separately, and **a failed attempt is never retried into a timing
sample** — the failure count is one of the things this reports, and a retry
would replace it with a freshly timed success. Failed attempts contribute zero
samples and the retained failure messages are printed beside the table.

Corrected capture (PR [#756](https://github.com/leynos/wireframe/pull/756)):

| Workload                                              | Attempts | Successes | Failures | Min        | Median     | p90        | Max         | Mean       |
| ----------------------------------------------------- | -------- | --------- | -------- | ---------- | ---------- | ---------- | ----------- | ---------- |
| `contended_fifo_pool_4_contenders_16_rounds_8`        | 20       | 20        | 0        | 146.954 µs | 247.636 µs | 467.762 µs | 2.546235 ms | 389.304 µs |
| `contended_round_robin_pool_4_contenders_16_rounds_8` | 20       | 20        | 0        | 143.064 µs | 311.048 µs | 515.133 µs | 1.009155 ms | 347.647 µs |

Both policies recorded **zero failures** in this run, where the same tables
carried 1 and 0 in the previous one. The spurious-`disconnected` defect
recorded below is still live — the re-test figures in
[What re-testing found](#what-re-testing-found) put it at roughly 1 in 110
bursts for FIFO at 16 contenders — so a 20-attempt sample expects about 0.2
failures and observing none is unremarkable. The failure count is reported per
run precisely so that a reader can see when it moves, and a comparison across
runs at this sample size should treat the count as the noisy quantity it is.

The spread is wide in both directions and much wider than the previous
capture's, which is why order statistics are quoted rather than a mean and
standard deviation. FIFO runs from 146.954 µs to 2.546235 ms, a factor of 17;
round-robin from 143.064 µs to 1.009155 ms, a factor of 7. The minima of the
two policies agree closely (146.954 and 143.064 µs), which suggests both are
measuring the same uncontended-burst floor, and they diverge only in their
tails. In FIFO the mean sits *above* the median (389.304 µs against 247.636
µs), which is the usual right-skewed timing shape and the reverse of what the
previous capture showed; round-robin's mean is also above its median (347.647
against 311.048 µs), though by less.

The tails are the finding. FIFO's max is 2.5 ms, roughly five times its p90 and
ten times its median, and round-robin's is 1.0 ms, roughly twice its p90. A
single max at five times the p90 and ten times the median is the shape a
straggler produces rather than a distribution's ordinary tail, and it is the
same path the stall defect below lives on. It is recorded as an observation,
not as a rate: twenty samples cannot distinguish one slow burst in twenty from
a heavier tail that this run happened to sample once, and the machine was under
a load average of 10–15 for this run. The 30-second burst timeout bounds the
failure mode; nothing here bounds the tail, and a reader comparing a future run
should look at the max before the median.

The two policies are not clearly separated at this sample size. Round-robin has
the higher median (311.048 µs against 247.636 µs) and the higher p90 (515.133
µs against 467.762 µs) but the *lower* max (1.009155 ms against 2.546235 ms)
and the lower mean. Twenty samples cannot resolve whether the policies differ
in their tails or whether this is sampling variation, and it is recorded as
unresolved rather than as a fairness result. A comparison that matters should
be taken with more attempts.

The previous capture's rows are kept here because the comparison above refers
to them, and because they show what a quieter run of the same workload looked
like:

| Workload                                              | Attempts | Successes | Failures | Min        | Median     | p90        | Max        | Mean       |
| ----------------------------------------------------- | -------- | --------- | -------- | ---------- | ---------- | ---------- | ---------- | ---------- |
| `contended_fifo_pool_4_contenders_16_rounds_8`        | 20       | 19        | 1        | 137.123 µs | 233.814 µs | 325.405 µs | 347.916 µs | 226.795 µs |
| `contended_round_robin_pool_4_contenders_16_rounds_8` | 20       | 20        | 0        | 164.643 µs | 197.093 µs | 352.745 µs | 371.216 µs | 249.589 µs |

The FIFO failure there was the live spurious-`disconnected` defect recorded
below, reported rather than hidden:
`1 of 16 contenders failed; first: transport error: connection closed by peer`.
The successes column was 19 rather than 20 because of it, and no sample was
taken from the failed burst.

The `Granted` and `Alive-task delta` columns are absent because the corrected
burst proves contention differently and no longer reports them per burst: the
held-permit phase asserts that the exhaustion granted at least one contended
lease (a burst that held every permit and granted none is an error, not a
zero), and the churn counters fold across the whole sampling run rather than
per burst. A writer comparing the two tables should read the corrected one as
"the contention is asserted inside the burst" and the original as "the leased
total was reported alongside it".

Original capture (`d28b54e`), one observation per burst:

| Workload                                              | Granted | Scheduler spawns | Alive-task delta | Elapsed    |
| ----------------------------------------------------- | ------- | ---------------- | ---------------- | ---------- |
| `contended_fifo_pool_4_contenders_16_rounds_8`        | 128     | 16               | 0                | 183.953 µs |
| `contended_round_robin_pool_4_contenders_16_rounds_8` | 128     | 16               | 0                | 118.552 µs |

The original `Elapsed` figures sit inside the corrected distributions — 183.953
µs is between the corrected median and p90 — so the single observation was not
misleading here, only uninformative about spread.

The labels were corrected before the original capture above: the contended
workload used to render `acquisitions_256` even though the contended path never
reads that field, so the label advertised a parameter the run did not honour.
Only the label changed — the exercised burst is identical, which is why the
recorded counters still stand. An older capture under
`contended_fifo_pool_4_acquisitions_256_contenders_16` is the same workload as
`contended_fifo_pool_4_contenders_16_rounds_8`.

The spawn counts are the task-churn evidence the issue asks for at this scope,
and they are what makes the uncontended bound meaningful: the counter
demonstrably moves under demand. They are no longer reported as a per-burst
column, because the counter is now per-pool and folded across a run; the
*uncontended* bounds below are where they do their work.

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
counted window after Criterion finishes: 3003 allocations over four iterations
of `contended_fifo_pool_4_contenders_16_rounds_8` in the corrected capture
(2176 in the original, 3097 in an earlier corrected run). Read it against the
uncontended figure of 2052 for the same shape — the two are within the
run-to-run spread noted above, so contention did not measurably change the
allocation count at this scope even though it raised the spawn count roughly
eightfold to sixteenfold. Both figures move between captures by more than the
correction accounts for, which is the same caution the allocation table states:
at this scope, prefer the spawn count.

## Known defects in contended pool acquisition

Capturing the pool baseline surfaced two pre-existing defects in the acquire
path. Neither was introduced by this work, and neither is fixed by it. Both are
recorded here because they are exactly the kind of hidden ownership cost Epic
[#635](https://github.com/leynos/wireframe/issues/635) exists to uncover.

One of the two, the stall, **stopped reproducing** after this branch was cut,
when `main` landed #535 ("Simplify client-pool waiter state transitions"). The
sections below record the original observation and then state what re-testing
found on the post-#535 tree, because a claim about live behaviour cannot be
left standing once the code beneath it changes — and because a defect that
stopped reproducing is itself evidence a later reader needs.

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

Measured on the named machine, before #535: the first failure lands at burst
913 of 20,000 at 16 contenders, and at roughly 1 round in 400 for the lighter
8-contender workload used by the regression tolerance in
`tests/runtime_ownership_benchmark_helpers.rs`. The post-#535 figures are in
[What re-testing found](#what-re-testing-found); they are higher for the FIFO
policy than the rate above. Reproducing it on a *long-lived* pool, with no
per-iteration rebuild and no task leak, is what proves it is an acquire-path
defect rather than an artefact of how the benchmark constructs its pool.

It is also what aborted the allocation binary's first capture. That binary ran
the contended scope through Criterion like any other, and panicked at
`benches/runtime_ownership_baselines_alloc.rs` with
`1 of 16 contenders failed; first: transport error: connection closed by peer` —
the defect reached from unmodified benchmark code driving unmodified library
code, with no probe scaffolding in between. That message names *this* defect
and not the stall, and the mapping is worth stating because the two are easy to
conflate. The `disconnected` constructor builds an `io::Error` with the text
`connection closed by peer` (`src/client/error.rs`), and the
`WireframeError::Io` arm renders it as `transport error: {error}`
(`src/error.rs`), so a spurious disconnect is what
`transport error: connection closed by peer` denotes. Criterion reports the
contender count separately, as `N of 16 contenders failed`, which is distinct
from the stall signature below — a stall produces no contender error at all,
only a burst that never completes. Criterion's timing pass runs several orders
of magnitude more iterations than an allocation window, so it hits the
interleaving nearly every run. The scope was moved out of the timed group and
reported once as an observation instead; the count itself was never in doubt,
because it comes from a single counted window that completed before the failure.

### Sustained contention stalls the acquire path

Under sustained contention the acquire path intermittently stops making
progress altogether. Every contender stalls with waiters still queued, and the
burst never completes. A `gdb` sample of the stalled process showed all 18
Tokio workers parked in `parking_lot::Condvar::wait` or `mio::Poll::poll`, with
CPU time frozen at `79s 8cs` across three samples 36 seconds apart — a true
deadlock, not slowness.

**This defect no longer reproduces on the current tree.** It was observed
against the pre-#535 scheduler, and re-testing on the post-#535 tree found zero
stalls across 80,000 rounds. See
[What re-testing found](#what-re-testing-found) for the figures and for why the
observation above is kept rather than deleted.

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

### What re-testing found

Both defects above were observed before this branch was cut. `main` then landed
issue [#535](https://github.com/leynos/wireframe/issues/535) ("Simplify
client-pool waiter state transitions"), which reworked the waiter state
transitions on exactly this path, so the observations were re-tested against
the post-#535 tree rather than left to stand unverified.

The stall did not reproduce. Two experiments drove `measure_pool_acquisition`
at the recorded shape (pool size 4, 16 acquisitions, 8 rounds per contender)
and classified every `Err` by its distinguishing text — the timeout message for
a stall, `connection closed by peer` for a spurious disconnect, anything else
as unclassified.

| Contenders | Rounds per policy | Stalls | FIFO disconnects | Round-robin disconnects |
| ---------- | ----------------- | ------ | ---------------- | ----------------------- |
| 8          | 24,000            | 0      | 327 (≈1 in 73)   | 38 (≈1 in 632)          |
| 16         | 16,000            | 0      | 144 (≈1 in 111)  | 11 (≈1 in 1,455)        |

Zero unclassified failures were observed in any run, so every error matched one
of the two known signatures.

The spurious `disconnected` therefore remains live, and it remains the dominant
observable failure at this scope. Its recorded rates have moved, and the
direction differs by policy: the FIFO rate is clearly above the 1-in-400
recorded for 8 contenders, while 16 contenders is in the region of the
originally recorded 1-in-913. Whether that is a genuine shift or machine and
load profile is not settled by these experiments, which ran on a loaded machine
— the correct regime for a contention defect, but not a controlled-quiet
measurement. The figures are recorded as observations, not as a revised rate.

**Read the stall result as "no longer reproducible", not as "fixed".** A
zero-count is a one-sided observation: 0 stalls in 16,000 rounds bounds the
per-round stall probability only at roughly 1 in 5,300 per policy, so it is
consistent both with the defect being gone and with it merely being rarer than
the sample can resolve. It does not prove absence at any contender count or
harness shape not exercised here.

The mechanism the rework removed is verifiable in the source, and it is worth
stating because it is what makes the result explicable rather than merely
observed. `stop_servicing` — the only writer that clears `is_servicing` to
false — has exactly one caller, `take_next_waiter_or_stop`
(`src/client/pool/scheduler.rs`). The immediate-lease arm instead resolves
through `next_waiter`, which selects a queued request and never clears the
flag, so a servicing worker can no longer be orphaned by a concurrent resolver.
The pre-#535 code cleared the guard on that path, leaving waiters queued with
no worker to serve them.

The disconnect path, by contrast, is unchanged: both the old and the new forms
still map a failed send to `disconnected`, which is why re-testing found it
alive while the stall was gone. That asymmetry is the reason the two defects
are recorded separately rather than as one.

The stall observation above is kept rather than deleted for two reasons. It is
the evidence that motivated the rework, and the recorded first-stall bursts
remain the only measurement of the defect's original frequency. A reader
comparing a future scheduler against this baseline needs both the historical
rate and the knowledge that the current tree no longer reaches it.

### Consequences for the harness

Criterion cannot time contended acquisition. A failure occurs during warm-up,
before any sample is recorded, which would abort the whole run. The benchmark
therefore reports the contended counters as `tracing` events after
`Criterion::final_summary`, guarded by `CONTENDED_BURST_TIMEOUT` (30 seconds).

The timeout was introduced for the stall, and the stall's re-test result does
not retire it. The spurious `disconnected` still fails bursts on the current
tree, and the guard is what keeps any future recurrence of a hang bounded: a
stalled burst surfaces as a reported error instead of a process that never
returns. Removing it because the one defect it was written for stopped
reproducing would trade a bounded failure for an unbounded one.

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

That rejection was measured against the pre-#535 scheduler, and #535 went on to
change the same path. The reversal is not therefore a closed question: a repair
built on the current state machine would be measured under different conditions
than the one that was rejected. What the rework does establish is narrower than
a repair — it removes the orphaned-worker mechanism, which is why the stall
stopped reproducing. Fixing the disconnect itself still belongs to #647.

The acquire path is the subject of
[ADR 013](adr-013-client-pool-scheduler-and-slot-ownership.md) and issue
[#647](https://github.com/leynos/wireframe/issues/647). Fixing it is out of
scope for #639, whose purpose is to record baselines before the ownership
topology changes. The spurious `disconnected` defect re-tested above is still
live, and it has no separate tracker: it belongs to the same path that #647
proposes to rework, so a reader looking for its disposition should start there.

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

Recorded, over 256 items. The benchmark IDs are `arc_shared_clone_items_256` and
`arc_fresh_per_item_items_256`; each probe's count is emitted as a `tracing`
event beside its ID, for the same reason the scope counts are.

| Probe                          | Allocations | Lower     | Estimate  | Upper     |
| ------------------------------ | ----------- | --------- | --------- | --------- |
| `arc_shared_clone_items_256`   | 1           | 2.3031 µs | 2.3177 µs | 2.3330 µs |
| `arc_fresh_per_item_items_256` | 256         | 3.4404 µs | 3.4553 µs | 3.4893 µs |

The delta is exactly 255, which is `ARC_PROBE_ITEMS - 1`. The counter therefore
resolves a per-item `Arc` allocation and sees nothing in a clone, which is the
resolution the criterion requires: a refactor that removes a *clone* from a
shared `Arc` will not move these numbers, and one that removes a per-item
construction will move them by at least the item count. The two timings differ
by only ~49 %, so a reader who ignored the counts could mistake these shapes
for equivalent — the count reported beside each ID is what makes the
distinction legible in a benchmark run.

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

### Allocation bracket

The pool allocation window has its own bracket, and it needed its own check
because its failure mode is silent: a count inflated by construction looks
exactly like a correct one. `verify_pool_bracket_excludes_construction` runs in
`benches/runtime_ownership_baselines_alloc.rs` before any measurement.

The check compares the bracketed and unbracketed windows at **zero
iterations**. That choice is the whole design. At a normal iteration count the
acquisition phase allocates on the order of 2 000 times while construction
allocates roughly 50, so an effect of a couple of percent sits against a
run-to-run spread several times larger. A probe of five readings each, taken
against the four-slot pool while this check was being designed, separated the
two terms as `build_only=[53, 50, 50, 50, 50]` against
`measure_only=[2372, 2303, 2231, 2052, 2052]`: the construction term is visible
in isolation, but a full-window comparison of `whole` against `bracketed`
produced overlapping minima (`[2102, 2334, 2563, 2582]` against
`[2276, 2208, 2462, 2612]`), so no threshold on that difference would be
stable. Erasing the measured phase removes the term rather than trying to see
past it: at zero iterations the bracketed window must read exactly **0**, and
the unbracketed one must read more than 0. The first is a clean equality with
no noise, and the second is the non-vacuity control that stops a counter wired
to nothing from satisfying it.

The check therefore asserts the *difference* and not its magnitude. It requires
the unbracketed reading to be positive, but does not pin the number: the
magnitude varies with the runtime's own allocation behaviour — the probe above
produced 53 once and 50 four times — so pinning it would make the check brittle
without making it stronger. What it establishes is the direction: construction
falls inside one window and outside the other. A third assertion requires the
bracketed window to be non-zero at a normal iteration count, so a window that
wrapped nothing at all cannot pass by being trivially clean.

The construction term is on the order of 50 allocations for a 4-slot pool,
which is why the correction did not move the published figure. Removing a
couple of percent from a quantity whose own run-to-run spread is ~33% is not
visible: under the corrected boundary the row has read 2052, 2381, 2580, and
2738 across four captures of the same binary, in no monotone order. What the
correction bought is a number that means what its label says, not a smaller
one, and a reader should not expect the two to differ.

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
- [#647](https://github.com/leynos/wireframe/issues/647) — the open proposal
  to replace spawn-on-demand pool servicing with a persistent scheduler actor.
  Both defects recorded here live on the path it proposes to rework; neither is
  its stated motivation.
- [#535](https://github.com/leynos/wireframe/issues/535) — the waiter
  state-transition rework whose arrival prompted the re-test in
  [What re-testing found](#what-re-testing-found).
- [ADR 011: runtime ownership and task-lifetime boundaries](adr-011-runtime-ownership-and-task-lifetime-boundaries.md)
- [ADR 012: prepared application and connection runtime](adr-012-prepared-application-and-connection-runtime.md)
- [ADR 013: client pool scheduler and slot ownership](adr-013-client-pool-scheduler-and-slot-ownership.md)
