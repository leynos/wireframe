# Zero-copy benchmark baselines

This document publishes the measured baselines and proposed acceptance
thresholds for the zero-copy migration in Epic
[#284](https://github.com/leynos/wireframe/issues/284). It is the companion to
the [zero-copy benchmark plan](zero-copy-benchmark-plan.md), which defines the
measurement matrix; this document records what that matrix currently produces.

It is deliberately partial. Four of the sixteen matrix rows have a harness and
are measured below. The other twelve, and the copied-byte metric the migration
is really about, have no harness at all. Roadmap item `1.2.2` of the
[zero-copy migration roadmap](zero-copy-frame-and-payload-migration-roadmap.md)
and item `10.2.2` of the [project roadmap](roadmap.md) therefore stay open.
Publishing the measured part is what lets those items be closed later against
evidence rather than recollection.

## Status

| Measurement kind | Measured today                                           | Threshold published |
| ---------------- | -------------------------------------------------------- | ------------------- |
| Latency          | Codec-layer encode and decode, both codecs, both classes | Yes                 |
| Throughput       | Same eight workloads                                     | Yes                 |
| Allocations      | Wrap and decode counts, both codecs, both classes        | Yes                 |
| Copied bytes     | Nothing                                                  | No                  |
| Fragmentation    | Paired fragmented and unfragmented wrap                  | Informational only  |

The encode figures measure `wrap_payload` followed by `Encoder::encode`. They
do not cover the serializer-to-`wrap_payload` hand-off, which is the matrix's
`outbound_encode` row and remains unmeasured.

The rows are stated this way because a reader must not infer coverage from the
presence of a table. "Not measured" and "measured as unchanged" are different
claims, and only the second one can support a release decision.

## Environment

A baseline is only reproducible alongside the machine and build that produced
it. The capture machine and build are:

| Property  | Value                                                    |
| --------- | -------------------------------------------------------- |
| CPU       | AMD Ryzen 9 3900 12-Core Processor, 24 hardware threads  |
| Memory    | 125 GiB                                                  |
| OS        | Rocky Linux 10.2, kernel 6.12.0-211.62.1.el10_2          |
| Toolchain | `nightly-2026-10-30` from `rust-toolchain.toml`          |
| Compiler  | `rustc 1.101.0-nightly (92b5b8c87 2026-10-29)`           |
| Cargo     | `cargo 1.101.0-nightly (31d3246e0 2026-10-27)`           |
| Profile   | Cargo `release`, the default for `cargo bench`           |
| Features  | `test-support`                                           |
| Linker    | `make bench-codec` sets `RUSTFLAGS="-D warnings"`, which |
|           | displaces the `mold` flag from `.cargo/config.toml`      |

The toolchain is pinned, so a later run resolves the same `rustc` without a
manual step. The linker row is not incidental: a capture taken through `make`
and one taken through a bare `cargo bench` may differ in link step, and the
comparison rules below require both sides to use the same entry point.

## Measured revision and command

| Fact          | Value                                                                     |
| ------------- | ------------------------------------------------------------------------- |
| Revision      | `9faa56daaa1ef2d5c2bc9bd5f216fa18b99fe298`                                |
| Date (UTC)    | 2026-10-31                                                                |
| Command       | `timeout 6900 make bench-codec 2>&1 \| tee /tmp/bench-codec-<branch>.out` |
| Exit status   | `0`                                                                       |
| Sample budget | 100 samples per workload, 3 s warm-up, 5 s measurement                    |
| Host load     | load average roughly 11–27 on 24 hardware threads                         |

The host was running other agents' compile work throughout the capture. That is
the ordinary state of this machine, not an incident, but it means these are
*relative* baselines: they describe the codec as it stands under contention,
and they must be compared against a later capture taken under comparable
contention. Criterion reported between one and eleven outliers per workload,
which is what the outlier column below records.

Each figure is a single run. Criterion's confidence interval describes the
uncertainty of the mean within that run; it does not describe the spread
between runs on a loaded host.

## Baselines

### Encode

The timed region repeats `wrap_payload` followed by `Encoder::encode` into a
reused write buffer, after `wire.clear()`. Setup happens before the timer
starts. The number is therefore steady-state encode, not allocate-and-encode.

| Workload                 | Lower     | Estimate  | Upper     | Throughput   | Outliers |
| ------------------------ | --------- | --------- | --------- | ------------ | -------- |
| `length_delimited_small` | 28.740 ns | 29.799 ns | 31.111 ns | 1.0001 GiB/s | 11 %     |
| `length_delimited_large` | 1.2808 µs | 1.3228 µs | 1.3767 µs | 46.142 GiB/s | 8 %      |
| `hotline_small`          | 39.952 ns | 40.415 ns | 40.919 ns | 755.10 MiB/s | 5 %      |
| `hotline_large`          | 1.1615 µs | 1.1874 µs | 1.2168 µs | 51.402 GiB/s | 2 %      |

### Decode

The timed region copies the frozen wire bytes into a fresh `BytesMut`, decodes,
and drops the buffer. That copy mirrors what the `Framed` reader does in
production, so it belongs in the measurement and must not be subtracted by a
later reader.

| Workload                 | Lower     | Estimate  | Upper     | Throughput   | Outliers |
| ------------------------ | --------- | --------- | --------- | ------------ | -------- |
| `length_delimited_small` | 76.326 ns | 77.240 ns | 78.281 ns | 395.10 MiB/s | 5 %      |
| `length_delimited_large` | 1.2373 µs | 1.2551 µs | 1.2749 µs | 48.631 GiB/s | 6 %      |
| `hotline_small`          | 43.056 ns | 43.817 ns | 44.693 ns | 696.48 MiB/s | 9 %      |
| `hotline_large`          | 1.3592 µs | 1.4202 µs | 1.4937 µs | 42.978 GiB/s | 5 %      |

### Fragmentation

The fragment cap is fixed at 1024 bytes, so a 32-byte payload produces one
fragment and a 64 KiB payload produces 64. Fragmentation is reported as a pair
rather than an absolute; the ratio in the label comes from a 16-iteration
validation probe at setup, so it is indicative and not the confidence-interval
estimate.

| Workload                         | Lower     | Estimate  | Upper     | Outliers |
| -------------------------------- | --------- | --------- | --------- | -------- |
| `unfragmented/small`             | 11.987 ns | 12.394 ns | 12.888 ns | 8 %      |
| `fragmented/small_ratio_4.367`   | 105.41 ns | 107.45 ns | 109.69 ns | 6 %      |
| `unfragmented/large`             | 11.013 ns | 11.163 ns | 11.388 ns | 3 %      |
| `fragmented/large_ratio_959.494` | 12.482 µs | 12.994 µs | 13.573 µs | 9 %      |

The unfragmented large-payload figure is near the floor of what this harness
can time: `wrap_payload` on an already-built `Bytes` is close to free, so the
row measures per-iteration harness overhead more than wrapping. Do not read the
fragmented/unfragmented ratio as a protocol cost without that caveat: for the
large class the ratio of the *estimates* is about 1,164×, while the label
records 959× from a noisier 16-iteration probe.

### Allocations

The allocation counter is a process-global allocator that counts operations
while a gate is open. It counts allocation calls, not bytes. Labels embed the
totals as `wrap_allocs_<n>` and `decode_allocs_<n>`.

| Label                                                   | Wrap allocs | Decode allocs | Per decode iteration | Time (encode + decode) | Outliers |
| ------------------------------------------------------- | ----------- | ------------- | -------------------- | ---------------------- | -------- |
| `length_delimited_small_wrap_allocs_3_decode_allocs_48` | 3           | 48            | 3.0                  | 112.62 ns              | 3 %      |
| `length_delimited_large_wrap_allocs_3_decode_allocs_48` | 3           | 48            | 3.0                  | 2.4481 µs              | 3 %      |
| `hotline_small_wrap_allocs_3_decode_allocs_32`          | 3           | 32            | 2.0                  | 88.016 ns              | 2 %      |
| `hotline_large_wrap_allocs_3_decode_allocs_32`          | 3           | 32            | 2.0                  | 2.3255 µs              | 1 %      |

Three readings matter and none should be over-stated:

- The decode counts are per `VALIDATION_ITERATIONS` batch of 16, and the batch
  excludes setup, so 48 is exactly 3 allocations per decode iteration for the
  length-delimited codec and 32 is exactly 2 for Hotline. These include the
  wire-buffer copy the helper makes each iteration.
- The wrap counts are *not* per iteration. Three allocations across 16 encode
  iterations is below one per iteration, and the counter's own documentation
  says the window includes harness activity. The honest conclusion is that the
  encode loop adds no allocation the counter can attribute — not that encode is
  proven allocation-free.
- The time column is the sum of one encode and one decode batch
  (`iter_custom` returns `encode.elapsed + decode.elapsed`), so it is not
  comparable with the encode and decode tables above.

## Copied bytes

No copied-byte baseline is published because none was measured. This is the
largest gap, and it is the metric the migration's headline claim depends on.

Three copy sites are in scope, described in the
[benchmark plan](zero-copy-benchmark-plan.md#copied-bytes):

1. `Serializer::serialize` returns `Vec<u8>`, so the serializer allocates and
   fills a fresh vector per outbound message.
2. `src/app/outbound_encoding.rs` calls `Bytes::from(bytes)`, which allocates a
   shared control block and may relocate the payload when length and capacity
   differ.
3. `LengthDelimitedEncoder::encode` delegates to `tokio_util`, which copies the
   payload again into the `Framed` write buffer.

Only the second and third sites' allocations are visible to the counting
allocator at all, and none of the three is counted in bytes. The plan names two
acceptable methods — a `benchmark-counters` feature asserting per-message
copied bytes, or a one-off DHAT run using the delta between payload classes.
Until one lands, this document says "unmeasured" rather than quoting a number
derived from timings.

## Read-only and mutation paths

The migration's claim is that read-only paths stop copying and mutation paths
copy only on demand. A threshold that averages the two together cannot test
that claim, so the paths are kept separate.

| Path kind | Stage                   | Measured?        | Baseline           |
| --------- | ----------------------- | ---------------- | ------------------ |
| Read-only | Inbound decode          | Yes              | Tables above       |
| Read-only | Middleware pass-through | No               | None               |
| Mutation  | Request-hook mutation   | No               | None               |
| Mixed     | Outbound encode         | Codec layer only | Encode table above |

Inbound decode is read-only in the relevant sense: it hands out a payload
without copying it for a caller. The outbound encode rows are mixed: the codec
layer is measured, but the serializer-to-`wrap_payload` hand-off that the
migration removes is not.

## Acceptance thresholds

These thresholds are proposals for roadmap item `1.2.2`. They apply to the
eight measured workloads. A threshold is not a promise that the number will be
met; it is the line at which a later comparison must stop and explain itself.

### Comparison protocol

- Both sides run through the same entry point on the same machine with the same
  features. The baseline side is this revision with this document's capture.
- The primary comparison is Criterion's change estimate against this baseline.
  Save a named baseline when a stable handle is wanted:

  ```sh
  cargo bench --bench codec_performance --features test-support -- \
    --save-baseline zero-copy-pre-migration
  cargo bench --bench codec_performance --features test-support -- \
    --baseline zero-copy-pre-migration
  ```

  A bare `cargo bench` and `make bench-codec` do not link identically. If the
  baseline side is saved through `cargo`, the comparison side must be run the
  same way so the difference cancels.
- Record the host load average with each capture. A capture taken on an idle
  host is not comparable with one taken under the load described above.

### Throughput and latency regression limits

T1. A workload fails when the change estimate is worse than +10 % latency (or
equivalently more than 10 % lower throughput) **and** the lower confidence
bound of that change also exceeds +1 %, the noise threshold. The estimate shows
the regression; the lower bound rules out a noise-only artefact. This rule
applies per workload, never to an average: a 10 % loss on the default path is
not offset by a gain elsewhere.

T2. On the named baseline machine only, a large-payload workload must not fall
below these absolute floors, set at 90 % of the measured estimate and rounded
down. They are a tripwire for a structural change that a relative comparison
against a corrupted baseline could hide:

| Workload                        | Floor    |
| ------------------------------- | -------- |
| `length_delimited_large` encode | 41 GiB/s |
| `length_delimited_large` decode | 43 GiB/s |
| `hotline_large` encode          | 46 GiB/s |
| `hotline_large` decode          | 38 GiB/s |

Small-payload floors are deliberately not set. At tens of nanoseconds with 5–11
% outlier rates they would encode noise as policy; T1 covers them.

### Allocation and sharing limits

T3. No measured workload may increase its allocation count. Concretely, the
decode count stays at 3.0 allocations per iteration for the length-delimited
codec and 2.0 for Hotline, and the encode window stays at 3 allocations per
16-iteration batch.

T4. The post-migration default outbound path must show zero new payload-buffer
allocations between serialization and `wrap_payload`. This is evidenced by both
an allocation count and a pointer-identity test comparing `as_ptr()` across the
hand-off, modelled on the existing codec pointer-identity assertions. A timing
improvement alone does not satisfy T4.

T5. A read-only stage must add zero allocations per operation. A mutation stage
may allocate one payload buffer per mutating operation, and must not allocate
on the read-only branch.

T6. A stage with no harness has no threshold and is reported as unmeasured. It
is not assumed unchanged.

### Correctness coupling

No performance result is accepted on its own. The same candidate must keep the
coverage below green, because a benchmark that measures a broken contract is
worse than no benchmark.

| Contract              | Coverage                                                         | Status at capture |
| --------------------- | ---------------------------------------------------------------- | ----------------- |
| Codec payload sharing | `src/codec/tests/mod.rs` pointer-identity assertions             | Present           |
| Mutation semantics    | `tests/middleware.rs`, `tests/middleware_order.rs`               | Present           |
| Correlation           | `tests/correlation_id.rs` and middleware correlation tests       | Present           |
| Fragmentation         | `tests/connection_fragmentation.rs`, `tests/fragment_transport/` | Present           |
| Hook behaviour        | `tests/features/client_request_hooks.feature` and its steps      | Present           |
| Wire output           | `tests/frame_codec.rs`, serializer round-trip tests              | Present           |

"Present" means the suite exists at the measured revision; it does not mean it
was re-run for this document, which changes only Markdown. The last full suite
run reported green is `e865f7c3`, the head of PR
[#521](https://github.com/leynos/wireframe/pull/521). A release candidate must
re-run every suite under roadmap item `13.2.1`; a threshold check on an
unvalidated candidate is not evidence.

## Unmeasured stages

| Stage                             | Why unmeasured                                                  | What would measure it                                              |
| --------------------------------- | --------------------------------------------------------------- | ------------------------------------------------------------------ |
| Middleware pass-through           | No harness over `ServiceRequest`/`ServiceResponse`              | No-op middleware chain, `frame()` borrow timed                     |
| Request-hook mutation             | No harness over `BeforeSendHook`                                | Read-only and mutating hook cases reported separately              |
| Outbound encode                   | Encode bench covers the codec, not `serialize` → `wrap_payload` | Serializer alone and serializer plus `wrap_payload` as two numbers |
| Copied bytes                      | Allocation counter counts operations, not bytes                 | Counter feature or DHAT, as above                                  |
| Hotline rows for the three stages | The stages have no harness for any codec                        | Extend the same harness once it exists                             |

These gaps are carried by a focused follow-up issue with acceptance criteria
that name the stages, the matrix rows, and the pointer-identity evidence
required. They are not closed here by narrowing the roadmap item.

## Reproducing

```sh
set -o pipefail
COMMIT=$(git rev-parse HEAD)
printf 'commit=%s\n' "$COMMIT"
cat /proc/loadavg
timeout 3600 make bench-codec 2>&1 | tee \
  "/tmp/bench-codec-$(git branch --show-current).out"
```

The commit and load average are recorded because the harness and the host both
evolve. A number is comparable only to one taken from the same harness source
under comparable contention; when the harness changes, the baseline is re-taken
rather than reinterpreted.

## Captures

| Capture          | Revision   | Date       | Covers                                     |
| ---------------- | ---------- | ---------- | ------------------------------------------ |
| Initial baseline | `9faa56da` | 2026-10-31 | Encode, decode, fragmentation, allocations |

## References

- [Zero-copy benchmark plan](zero-copy-benchmark-plan.md)
- [Zero-copy migration guide outline](zero-copy-migration-guide-outline.md)
- [Zero-copy release plan](zero-copy-release-plan.md)
- [Zero-copy frame and payload migration roadmap](zero-copy-frame-and-payload-migration-roadmap.md)
- [Frame and `Vec<u8>` inventory](frame-vec-u8-inventory.md)
- [Runtime-ownership baselines](runtime-ownership-baselines.md), the
  repository's existing baseline-document convention
- [ADR 008: zero-copy public byte container](adr-008-zero-copy-public-byte-container.md)
- [ADR 009: `Vec<u8>` migration rollout](adr-009-vec-u8-migration-rollout.md)
- [ADR 010: transport frame boundary](adr-010-transport-frame-boundary-for-zero-copy.md)
