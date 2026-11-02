# Zero-copy benchmark baselines

This document publishes the measured baselines and proposed acceptance
thresholds for the zero-copy migration in Epic
[#284](https://github.com/leynos/wireframe/issues/284). It is the companion to
the [zero-copy benchmark plan](zero-copy-benchmark-plan.md), which defines the
measurement matrix; this document records what that matrix currently produces.

All sixteen matrix rows now have a harness, and all four measurement kinds —
latency, throughput, allocations, and copied bytes — are reported. The
remaining work on roadmap item `1.2.2` of the
[zero-copy migration roadmap](zero-copy-frame-and-payload-migration-roadmap.md)
and item `10.2.2` of the [project roadmap](roadmap.md) is agreement on the
proposed thresholds, not further measurement. Until a performance reviewer
accepts them, the thresholds below are proposals and those items stay open.

## Status

| Measurement kind | Measured today                                                          | Threshold published |
| ---------------- | ----------------------------------------------------------------------- | ------------------- |
| Latency          | All four stages, both codecs, both payload classes                      | Yes, proposed       |
| Throughput       | Same sixteen rows; bytes/s for codec and outbound, messages/s otherwise | Yes, proposed       |
| Allocations      | Codec wrap/decode plus the three pipeline stages                        | Yes, proposed       |
| Copied bytes     | Per-seam byte counts asserted by the test suite                         | Yes, deterministic  |
| Fragmentation    | Paired fragmented and unfragmented wrap                                 | Informational only  |

The rows are stated this way because a reader must not infer coverage from the
presence of a table. "Not measured" and "measured as unchanged" are different
claims, and only the second one can support a release decision.

Two units appear under throughput and the difference is deliberate. The codec
and outbound stages traverse the payload, so payload bytes per second is a real
rate. Middleware pass-through and request-hook dispatch read a buffer without
traversing it, so their cost does not scale with payload size; reporting bytes
per second there would divide one message by a few nanoseconds and claim
terabytes per second for a 64 KiB frame. Those two groups report messages per
second instead. The [justified exceptions](#justified-exceptions) section
records the other deliberate measurement choices.

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
| Features  | `test-support,benchmark-counters`                        |
| Linker    | `make bench-codec` sets `RUSTFLAGS="-D warnings"`, which |
|           | displaces the `mold` flag from `.cargo/config.toml`      |

The toolchain is pinned, so a later run resolves the same `rustc` without a
manual step. The features row is part of the contract, not an incidental flag:
with `benchmark-counters` off the copy recorders compile to empty inline
functions, so a capture taken without it cannot report copied bytes at all. The
linker row is also not incidental — a capture taken through `make` and one
taken through a bare `cargo bench` may differ in link step, and the comparison
rules below require both sides to use the same entry point.

## Measured revision and command

| Fact          | Value                                                                           |
| ------------- | ------------------------------------------------------------------------------- |
| Revision      | `0483d86f` plus working-tree content `7b9dd4829a672ec9a887bad05dd1b5378a87c43b` |
| Date (UTC)    | 2026-11-02                                                                      |
| Command       | `make bench-codec`, captured with `tee /tmp/bench-codec-<branch>.out`           |
| Exit status   | `0`                                                                             |
| Sample budget | 100 samples per workload, 3 s warm-up, 5 s measurement                          |
| Host load     | load average between roughly 9 and 22 on 24 hardware threads                    |

The working tree was uncommitted when the capture ran, so the commit alone does
not identify the code. `7b9dd482` is the tree object the harness and library
sources hashed to at capture time; reproducing the numbers needs that content,
not merely the parent commit.

The host was running other agents' compile work throughout the capture. That is
the ordinary state of this machine, not an incident, but it means these are
*relative* baselines: they describe the code as it stands under contention, and
they must be compared against a later capture taken under comparable
contention. Criterion reported between zero and eleven outliers per workload,
which the outlier columns record.

Each figure is a single run. Criterion's confidence interval describes the
uncertainty of the mean within that run; it does not describe the spread
between runs on a loaded host.

A first capture is preserved as superseded. It reported the serializer-only
stage at about 4.6 ns for every payload class, including 64 KiB, which is not a
plausible copy cost. The harness had handed only the *length* of the serialized
buffer to `black_box`, so the optimizer folded the length to a constant and
deleted both the allocation and the fill. The harness now hands the buffer
slice to `black_box`, and the same stage measures 14.6 ns for 32 bytes and
1.1777 µs for 64 KiB. The throughput units for the two read-only stages changed
in the same revision. Because the harness changed, the baseline was re-taken
rather than reinterpreted, and only the re-taken capture is published below.

## Baselines

### Encode

The timed region repeats `wrap_payload` followed by `Encoder::encode` into a
reused write buffer, after `wire.clear()`. Setup happens before the timer
starts. The number is therefore steady-state encode, not allocate-and-encode,
and it covers the transport copy only: the serializer-to-`wrap_payload` seam is
a separate stage measured under [outbound encode](#outbound-encode). It does
not measure the serializer.

| Workload                 | Lower     | Estimate  | Upper     | Throughput   | Outliers |
| ------------------------ | --------- | --------- | --------- | ------------ | -------- |
| `length_delimited_small` | 26.560 ns | 26.763 ns | 26.974 ns | 1.1136 GiB/s | 6.00%    |
| `length_delimited_large` | 1.1398 µs | 1.1550 µs | 1.1719 µs | 52.842 GiB/s | 9.00%    |
| `hotline_small`          | 37.614 ns | 38.172 ns | 38.781 ns | 799.47 MiB/s | 5.00%    |
| `hotline_large`          | 1.1776 µs | 1.2031 µs | 1.2345 µs | 50.733 GiB/s | 8.00%    |

### Decode

The timed region copies the frozen wire bytes into a fresh `BytesMut`, decodes,
and drops the buffer. That copy mirrors what the `Framed` reader does in
production, so it belongs in the measurement and must not be subtracted by a
later reader.

| Workload                 | Lower     | Estimate  | Upper     | Throughput   | Outliers |
| ------------------------ | --------- | --------- | --------- | ------------ | -------- |
| `length_delimited_small` | 75.871 ns | 77.292 ns | 78.787 ns | 394.83 MiB/s | 2.00%    |
| `length_delimited_large` | 1.3126 µs | 1.3329 µs | 1.3528 µs | 45.790 GiB/s | 1.00%    |
| `hotline_small`          | 48.666 ns | 49.914 ns | 51.193 ns | 611.40 MiB/s | 1.00%    |
| `hotline_large`          | 1.1662 µs | 1.1986 µs | 1.2472 µs | 50.921 GiB/s | 3.00%    |

### Fragmentation

The fragment cap is fixed at 1024 bytes, so a 32-byte payload produces one
fragment and a 64 KiB payload produces 64. Fragmentation is reported as a pair
rather than an absolute; the ratio in the label comes from a 16-iteration
validation probe at setup, so it is indicative and not the confidence-interval
estimate.

| Workload                         | Lower     | Estimate  | Upper     | Outliers |
| -------------------------------- | --------- | --------- | --------- | -------- |
| `unfragmented/small`             | 10.672 ns | 10.721 ns | 10.774 ns | 6.00%    |
| `fragmented/small_ratio_4.852`   | 116.79 ns | 122.99 ns | 129.48 ns | 6.00%    |
| `unfragmented/large`             | 10.802 ns | 10.882 ns | 10.969 ns | 8.00%    |
| `fragmented/large_ratio_284.000` | 11.099 µs | 11.297 µs | 11.522 µs | 4.00%    |

The unfragmented large-payload figure is near the floor of what this harness
can time: `wrap_payload` on an already-built `Bytes` is close to free, so the
row measures per-iteration harness overhead more than wrapping. Do not read the
fragmented/unfragmented ratio as a protocol cost without that caveat: for the
large class the ratio of the *estimates* is about 1,038×, while the label
records 284× from a noisier 16-iteration probe. The ratio moved between the two
captures because the probe is small and the host was loaded; the paired
absolute figures are the stable reading.

### Codec allocations

The allocation counter is a process-global allocator that counts operations
while a gate is open. It counts allocation calls, not bytes. Labels embed the
totals as `wrap_allocs_<n>` and `decode_allocs_<n>`.

| Label                                                   | Wrap allocs | Decode allocs | Per decode iteration | Time (encode + decode) | Outliers |
| ------------------------------------------------------- | ----------- | ------------- | -------------------- | ---------------------- | -------- |
| `length_delimited_small_wrap_allocs_3_decode_allocs_48` | 3           | 48            | 3.0                  | 118.22 ns              | 9.00%    |
| `length_delimited_large_wrap_allocs_3_decode_allocs_48` | 3           | 48            | 3.0                  | 2.5316 µs              | 6.00%    |
| `hotline_small_wrap_allocs_3_decode_allocs_32`          | 3           | 32            | 2.0                  | 83.407 ns              | 2.00%    |
| `hotline_large_wrap_allocs_3_decode_allocs_32`          | 3           | 32            | 2.0                  | 2.5169 µs              | 9.00%    |

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

The window also never executes `Serializer::serialize`, because this benchmark
drives the codec directly. Sites 1 and 2 of the [copied bytes](#copied-bytes)
section are therefore outside this harness rather than invisible to the
allocator.

### Middleware pass-through

The chain is built from the library's `Transform` and `Service` traits and
terminated by a service that returns the request frame unchanged. The frame is
recycled between iterations, so the timed region is chain traversal rather than
allocation. Both access modes run the same chain; the mutating policy rewrites
one frame byte before continuing.

Throughput is reported as messages per second because the stage borrows the
frame without traversing it.

| Workload                                                 | Lower     | Estimate  | Upper     | Throughput     | Outliers |
| -------------------------------------------------------- | --------- | --------- | --------- | -------------- | -------- |
| `middleware_passthrough_length_delimited_small_readonly` | 47.272 ns | 48.797 ns | 50.924 ns | 20.493 Melem/s | 11.00%   |
| `middleware_passthrough_length_delimited_small_mutating` | 46.322 ns | 46.968 ns | 47.626 ns | 21.291 Melem/s | 0%       |
| `middleware_passthrough_length_delimited_large_readonly` | 46.405 ns | 47.081 ns | 47.775 ns | 21.240 Melem/s | 5.00%    |
| `middleware_passthrough_length_delimited_large_mutating` | 48.610 ns | 50.261 ns | 52.044 ns | 19.896 Melem/s | 9.00%    |
| `middleware_passthrough_hotline_small_readonly`          | 47.347 ns | 48.391 ns | 49.562 ns | 20.665 Melem/s | 8.00%    |
| `middleware_passthrough_hotline_small_mutating`          | 45.359 ns | 45.990 ns | 46.674 ns | 21.744 Melem/s | 3.00%    |
| `middleware_passthrough_hotline_large_readonly`          | 46.306 ns | 47.142 ns | 48.032 ns | 21.213 Melem/s | 7.00%    |
| `middleware_passthrough_hotline_large_mutating`          | 47.440 ns | 48.881 ns | 50.758 ns | 20.458 Melem/s | 2.00%    |

The constant cost across payload classes is the expected result: a pass-through
that hands out a shared payload does not touch its bytes. The mutating rows are
the same order of magnitude because a one-byte rewrite is still one write.

### Request-hook dispatch

Dispatch runs through the client's own before-send loop rather than a harness
replica, so the harness and the production path cannot drift apart without
changing the client. The hook is registered once and the buffer built once,
outside the timer.

| Workload                                        | Lower     | Estimate  | Upper     | Throughput     | Outliers |
| ----------------------------------------------- | --------- | --------- | --------- | -------------- | -------- |
| `hook_mutation_length_delimited_small_readonly` | 2.8832 ns | 2.9207 ns | 2.9612 ns | 342.39 Melem/s | 0%       |
| `hook_mutation_length_delimited_small_mutating` | 3.0201 ns | 3.0676 ns | 3.1184 ns | 325.99 Melem/s | 5.00%    |
| `hook_mutation_length_delimited_large_readonly` | 3.0901 ns | 3.1573 ns | 3.2258 ns | 316.73 Melem/s | 5.00%    |
| `hook_mutation_length_delimited_large_mutating` | 3.0061 ns | 3.0776 ns | 3.1578 ns | 324.93 Melem/s | 4.00%    |
| `hook_mutation_hotline_small_readonly`          | 2.9220 ns | 2.9649 ns | 3.0118 ns | 337.27 Melem/s | 5.00%    |
| `hook_mutation_hotline_small_mutating`          | 3.0177 ns | 3.0832 ns | 3.1545 ns | 324.34 Melem/s | 9.00%    |
| `hook_mutation_hotline_large_readonly`          | 3.1051 ns | 3.1750 ns | 3.2514 ns | 314.96 Melem/s | 2.00%    |
| `hook_mutation_hotline_large_mutating`          | 3.0365 ns | 3.1307 ns | 3.2236 ns | 319.42 Melem/s | 5.00%    |

Every row is within half a nanosecond of the others, which is what an
O(1)-in-payload dispatch should produce. A regression that made a hook walk the
buffer would appear as a payload-class split here.

### Outbound encode

The stage is the two-step hand-off performed by `encode_message_frame`. The
migration removes the seam between the steps, so they are measured separately:
`outbound_serialize` is `Serializer::serialize` alone and `outbound_encode` is
`serialize` followed by `wrap_payload`.

The harness serializer sizes its output exactly, like `bincode::encode_to_vec`,
so the vector's length equals its capacity. Setup happens before the timer, and
both passes hand their resulting buffer slice to `black_box` so the optimizer
cannot delete the fill.

| Workload                                    | Lower     | Estimate  | Upper     | Throughput   | Outliers |
| ------------------------------------------- | --------- | --------- | --------- | ------------ | -------- |
| `outbound_serialize_length_delimited_small` | 14.192 ns | 14.630 ns | 15.079 ns | 2.0370 GiB/s | 2.00%    |
| `outbound_encode_length_delimited_small`    | 20.685 ns | 21.115 ns | 21.567 ns | 1.4115 GiB/s | 7.00%    |
| `outbound_serialize_length_delimited_large` | 1.1638 µs | 1.1777 µs | 1.1922 µs | 51.824 GiB/s | 3.00%    |
| `outbound_encode_length_delimited_large`    | 1.2029 µs | 1.2224 µs | 1.2426 µs | 49.932 GiB/s | 3.00%    |
| `outbound_serialize_hotline_small`          | 13.417 ns | 13.824 ns | 14.290 ns | 2.1559 GiB/s | 5.00%    |
| `outbound_encode_hotline_small`             | 22.567 ns | 23.562 ns | 24.583 ns | 1.2649 GiB/s | 0%       |
| `outbound_serialize_hotline_large`          | 1.1588 µs | 1.1714 µs | 1.1853 µs | 52.103 GiB/s | 7.00%    |
| `outbound_encode_hotline_large`             | 1.1897 µs | 1.2023 µs | 1.2153 µs | 50.763 GiB/s | 4.00%    |

The two passes differ by about 45 ns for a 64 KiB payload. `wrap_payload` does
not copy the payload — it wraps the existing `Bytes` in the codec's frame type
— so the difference is frame construction and the extra call, not a second 64
KiB traversal. The serializer pass accounts for essentially all of the
per-message payload work.

### Pipeline allocations

The pipeline allocation groups use the same counting allocator as the codec
baselines and embed the batch total in the label. The window is
`VALIDATION_ITERATIONS` (16) operations and includes harness plumbing, so the
label total is a budget to hold steady, not a per-operation payload figure.
Timing is the same measurement as the corresponding non-allocation group.

#### Middleware pass-through allocations

| Label                                                              | Time      | Outliers |
| ------------------------------------------------------------------ | --------- | -------- |
| `middleware_passthrough_length_delimited_small_readonly_allocs_35` | 54.640 ns | 8.00%    |
| `middleware_passthrough_length_delimited_small_mutating_allocs_35` | 48.436 ns | 5.00%    |
| `middleware_passthrough_length_delimited_large_readonly_allocs_35` | 47.690 ns | 1.00%    |
| `middleware_passthrough_length_delimited_large_mutating_allocs_35` | 46.230 ns | 5.00%    |
| `middleware_passthrough_hotline_small_readonly_allocs_35`          | 50.472 ns | 6.00%    |
| `middleware_passthrough_hotline_small_mutating_allocs_35`          | 50.841 ns | 0%       |
| `middleware_passthrough_hotline_large_readonly_allocs_35`          | 49.807 ns | 3.00%    |
| `middleware_passthrough_hotline_large_mutating_allocs_35`          | 49.953 ns | 4.00%    |

All eight rows record 35 allocations per 16 requests, about two per request.
Those are the boxed futures the `Service` trait's object-safe dispatch
requires, not payload buffers: the frame is recycled, and the copied-byte
counters show the payload is not written. The count is the same for both access
modes and both payload classes, which is the signature of fixed async plumbing
rather than size-dependent work.

#### Request-hook dispatch allocations

| Label                                                    | Time      | Outliers |
| -------------------------------------------------------- | --------- | -------- |
| `hook_mutation_length_delimited_small_readonly_allocs_3` | 2.8924 ns | 9.00%    |
| `hook_mutation_length_delimited_small_mutating_allocs_3` | 3.0869 ns | 4.00%    |
| `hook_mutation_length_delimited_large_readonly_allocs_3` | 3.0397 ns | 4.00%    |
| `hook_mutation_length_delimited_large_mutating_allocs_3` | 3.0260 ns | 7.00%    |
| `hook_mutation_hotline_small_readonly_allocs_3`          | 2.9989 ns | 2.00%    |
| `hook_mutation_hotline_small_mutating_allocs_3`          | 2.9935 ns | 4.00%    |
| `hook_mutation_hotline_large_readonly_allocs_3`          | 2.8153 ns | 2.00%    |
| `hook_mutation_hotline_large_mutating_allocs_3`          | 2.9207 ns | 2.00%    |

Three allocations across 16 dispatches is below one per operation and is
harness noise around a zero-allocation loop.

#### Outbound encode allocations

| Label                                                 | Time      | Outliers |
| ----------------------------------------------------- | --------- | -------- |
| `outbound_serialize_length_delimited_small_allocs_18` | 14.347 ns | 4.00%    |
| `outbound_encode_length_delimited_small_allocs_18`    | 21.962 ns | 0%       |
| `outbound_serialize_length_delimited_large_allocs_18` | 1.1661 µs | 2.00%    |
| `outbound_encode_length_delimited_large_allocs_18`    | 1.2999 µs | 5.00%    |
| `outbound_serialize_hotline_small_allocs_18`          | 15.676 ns | 1.00%    |
| `outbound_encode_hotline_small_allocs_18`             | 19.747 ns | 7.00%    |
| `outbound_serialize_hotline_large_allocs_18`          | 1.1380 µs | 7.00%    |
| `outbound_encode_hotline_large_allocs_18`             | 1.1604 µs | 7.00%    |

Eighteen allocations per 16 operations is one serializer vector per message
plus two harness allocations. The wrapping pass adds none, which agrees with
the copied-byte result below: `wrap_payload` moves the existing `Bytes` rather
than allocating a new payload buffer.

## Copied bytes

Copied bytes are counted at each seam by `src/benchmark_counters.rs`, behind the
`benchmark-counters` feature. The result is deterministic — it does not depend
on host load — and the numbers below come from the assertion in
`wireframe_testing/tests/copied_bytes.rs`, not from a timing run.

| Seam                          | Stage that incurs it       | Bytes per message |
| ----------------------------- | -------------------------- | ----------------- |
| Serializer materialization    | `Serializer::serialize`    | `P`               |
| `Vec<u8>` to `Bytes` hand-off | `bytes_from_vec`           | `0`               |
| Frame construction            | `FrameCodec::wrap_payload` | `0`               |
| Transport framing copy        | `Encoder::encode`          | `P`               |
| Outbound total                | serialize + wrap + encode  | `2P`              |

`P` is the payload length, 32 bytes or 64 KiB depending on the class, and the
table holds for both the length-delimited codec and Hotline.

The hand-off is zero because `bytes` 1.12.1's `From<Vec<u8>>` for `Bytes` never
copies the payload: an exact-capacity vector takes the boxed slice, and a
vector with spare capacity allocates a shared control block that still points
at the vector's buffer. The counter exists as a guard against a later `bytes`
change or a hand-off written some other way. Frame construction is zero by
construction: both codecs store the handed-in `Bytes` in their frame type.

The assertion lives in `wireframe_testing/tests/copied_bytes.rs` rather than in
a report, so a regression fails the build. It covers all four codec and
payload-class workloads and states the pre-migration outbound total as two
payload copies per message: one serializer materialization and one transport
framing copy. A post-migration capture must halve that total.

Allocation counts are not a substitute for this measurement. An allocation
counter cannot see a copy into an already-allocated buffer, which is exactly
what the transport framing copy does, so a DHAT total or an allocation delta
between payload classes is an allocation proxy only and cannot satisfy the
copied-byte gate. The copy-site counters are the sole copied-byte gate.

The codec allocation benchmark does not execute the serializer-to-`Bytes` path,
so its windows do not observe the first two sites at all. That is a harness
boundary, not a limit of the allocator: a bench that ran the serializer would
count its allocation.

## Read-only and mutation paths

The migration's claim is that read-only paths stop copying and mutation paths
copy only on demand. A threshold that averages the two together cannot test
that claim, so the paths are kept separate.

| Path kind | Stage                   | Measured?                                                        | Baseline                |
| --------- | ----------------------- | ---------------------------------------------------------------- | ----------------------- |
| Read-only | Inbound decode          | Yes                                                              | Decode table above      |
| Read-only | Middleware pass-through | Yes, `readonly` rows                                             | Middleware table above  |
| Mutation  | Middleware mutation     | Yes, `mutating` rows                                             | Middleware table above  |
| Read-only | Request-hook read       | Yes, `readonly` rows                                             | Hook table above        |
| Mutation  | Request-hook mutation   | Yes, `mutating` rows                                             | Hook table above        |
| Mixed     | Outbound encode         | Yes, serializer-only and serialize-plus-wrap reported separately | Outbound table above    |
| All       | Copied bytes            | Yes, per seam                                                    | Copied-byte table above |

Inbound decode is read-only in the relevant sense: it hands out a payload
without copying it for a caller, though the harness pays the reader's
wire-buffer copy. The outbound stage is mixed by nature — it always writes —
which is why its two passes are separate rows rather than an access-mode split.

## Acceptance thresholds

These thresholds are proposals for roadmap items `1.2.2` and `10.2.2`. They
apply to the sixteen measured workloads. A threshold is not a promise that the
number will be met; it is the line at which a later comparison must stop and
explain itself. None is accepted until a performance reviewer agrees to it.

### Comparison protocol

- Both sides run through the same entry point on the same machine with the same
  features. The baseline side is this revision with this document's capture.
- The primary comparison is Criterion's change estimate against this baseline.
  Save a named baseline when a stable handle is wanted:

  ```sh
  cargo bench --bench codec_performance --bench pipeline_performance \
    --features test-support,benchmark-counters -- --save-baseline zero-copy-pre-migration
  cargo bench --bench codec_performance --bench pipeline_performance \
    --features test-support,benchmark-counters -- --baseline zero-copy-pre-migration
  ```

  A bare `cargo bench` and `make bench-codec` do not link identically. If the
  baseline side is saved through `cargo`, the comparison side must be run the
  same way so the difference cancels.
- Record the host load average with each capture. A capture taken on an idle
  host is not comparable with one taken under the load described above.
- Compare only like units. A stage measured in messages per second is compared
  against the same stage, never against a bytes-per-second row.

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
| `length_delimited_large` encode | 47 GiB/s |
| `length_delimited_large` decode | 41 GiB/s |
| `hotline_large` encode          | 45 GiB/s |
| `hotline_large` decode          | 45 GiB/s |

Small-payload floors are deliberately not set. At tens of nanoseconds with 0–11
% outlier rates they would encode noise as policy; T1 covers them.

### Allocation, copy, and sharing limits

T3. No measured workload may increase its allocation count. Concretely, the
decode count stays at 3.0 allocations per iteration for the length-delimited
codec and 2.0 for Hotline; the codec encode window stays at 3 allocations per
16-iteration batch; middleware pass-through stays at 35 per batch; hook
dispatch at 3; and each outbound pass at 18.

T4. The post-migration default outbound path must show zero new payload-buffer
allocations and zero new copied bytes between serialization and `wrap_payload`.
This is evidenced by both an allocation count and a pointer-identity test
comparing `as_ptr()` across the hand-off, modelled on the existing codec
pointer-identity assertions. A timing improvement alone does not satisfy T4.

T5. A read-only stage must add no payload-buffer allocation and no payload byte
copy per operation. The middleware chain's async plumbing allocates about two
control blocks per request, and that fixed count must not grow with payload
class; payload allocation is separately evidenced by the copied-byte counters.
A mutation stage may copy one payload buffer per mutating operation and must
not copy on its read-only branch.

T6. The copied-byte assertion in `wireframe_testing/tests/copied_bytes.rs` must
pass unchanged. A post-migration outbound total above one payload copy per
message fails the gate.

T7. A stage with no harness has no threshold and is reported as unmeasured. It
is not assumed unchanged. No stage is in that position now; the
[justified exceptions](#justified-exceptions) section records the measurement
choices that remain.

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
| Copied bytes          | `wireframe_testing/tests/copied_bytes.rs` per-seam assertions    | Present           |

"Present" means the suite exists at the measured revision. A release candidate
must re-run every suite under roadmap item `13.2.1`; a threshold check on an
unvalidated candidate is not evidence.

## Justified exceptions

Every matrix row is measured. The remaining deviations are recorded here rather
than left implicit:

- **Middleware and hook throughput is in messages per second.** Both stages
  borrow a buffer without traversing it, so bytes per second would report an
  impossible rate for large payloads. The time column remains comparable across
  payload classes; only the throughput unit differs.
- **Hook dispatch is O(1) in payload size.** The four rows within half a
  nanosecond of each other is the evidence, not a missing row. A hook that
  walked the buffer would show a payload-class split.
- **The codec encode window's wrap count is below one allocation per
  iteration.** Three allocations across 16 iterations is not proof that
  encoding is allocation-free, only that this counter cannot attribute an
  allocation to the loop.
- **The codec allocation time column is an encode-plus-decode sum.** It is not
  comparable with the separate encode and decode tables.
- **Fragmentation ratios come from a 16-iteration probe** and vary between
  captures. The paired absolute figures are the stable reading.
- **Serializer-only timing needs the buffer to escape.** Handing only the
  length to `black_box` let the optimizer delete the allocation and fill; the
  superseded capture in
  [measured revision and command](#measured-revision-and-command) records the
  effect.

## Reproducing

```sh
set -o pipefail
COMMIT=$(git rev-parse HEAD)
printf 'commit=%s\n' "$COMMIT"
cat /proc/loadavg
timeout 3600 make bench-codec 2>&1 | tee \
  "/tmp/bench-codec-$(git branch --show-current).out"
```

The commit, tree content, and load average are recorded because the harness and
the host both evolve. A number is comparable only to one taken from the same
harness source under comparable contention; when the harness changes, the
baseline is re-taken rather than reinterpreted.

## Captures

| Capture          | Revision                     | Date       | Covers                                                       |
| ---------------- | ---------------------------- | ---------- | ------------------------------------------------------------ |
| Initial baseline | `9faa56da`                   | 2026-10-31 | Encode, decode, fragmentation, allocations                   |
| Superseded       | `0483d86f` + tree `96050eb7` | 2026-11-02 | All rows; serializer timing invalidated by `black_box` scope |
| Published        | `0483d86f` + tree `7b9dd482` | 2026-11-02 | All sixteen rows and copied bytes                            |

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
