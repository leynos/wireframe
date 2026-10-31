# Zero-copy benchmark plan

This plan defines how the zero-copy migration in Epic
[#284](https://github.com/leynos/wireframe/issues/284) is measured. It closes
roadmap item `1.2.1` of the
[zero-copy migration roadmap](zero-copy-frame-and-payload-migration-roadmap.md)
and roadmap item `10.2.1` of the [project roadmap](roadmap.md), both of which
ask for a benchmark matrix covering inbound decode, middleware pass-through,
request-hook mutation, and outbound encode.

The plan is deliberately explicit about what the repository can measure today.
Three of the four stages in the roadmap have no harness at all. Recording that
gap, rather than substituting a codec-level number for a stage-level
measurement, is what makes the later comparison meaningful.

## Scope

In scope:

- the four stages named by roadmap items `1.2.1` and `10.2.1`;
- the default codec path (`LengthDelimitedFrameCodec`) and one protocol-native
  codec (`HotlineFrameCodec`);
- a small (32 byte) and a large (64 KiB) payload class;
- throughput, latency, allocation counts, and copied bytes as the four
  measurement kinds;
- the correctness coverage that must stay green alongside every measurement.

Out of scope:

- wire-format changes;
- multi-host or network-scale benchmarking;
- acceptance thresholds for stages that do not yet have a harness. The proposed
  thresholds live in
  [zero-copy benchmark baselines](zero-copy-benchmark-baselines.md).

## The four stages

The stages follow the path an outbound or inbound message takes, not the module
layout, because the migration changes ownership at three separate boundaries.

| Stage                   | Direction | Owning surface                                     | Harness today |
| ----------------------- | --------- | -------------------------------------------------- | ------------- |
| Inbound decode          | inbound   | `FrameCodec::decoder`, `FrameCodec::frame_payload` | Yes           |
| Middleware pass-through | inbound   | `ServiceRequest`, `ServiceResponse`                | No            |
| Request-hook mutation   | outbound  | `BeforeSendHook`, `send_pipeline`                  | No            |
| Outbound encode         | outbound  | `Serializer::serialize`, `wrap_payload`            | No            |

"Inbound decode" here means byte-buffer to frame: the codec decodes a wire
buffer and hands out a payload. It does not include application deserialization
into domain types, which the migration does not change. "Outbound encode" means
the two-step `serialize` then `wrap_payload` hand-off performed by
`src/app/outbound_encoding.rs`; the transport write that follows is outside the
migration's boundary.

## Existing harnesses

| Target                              | File                                         | Features               | Measures                              |
| ----------------------------------- | -------------------------------------------- | ---------------------- | ------------------------------------- |
| `codec_performance`                 | `benches/codec_performance.rs`               | `test-support`         | Encode, decode, fragmentation timing  |
| `codec_performance_alloc`           | `benches/codec_performance_alloc.rs`         | `test-support`         | Allocation counts for wrap and decode |
| `runtime_ownership_baselines`       | `benches/runtime_ownership_baselines/`       | `test-support,` `pool` | Actor, push, startup, pool timing     |
| `runtime_ownership_baselines_alloc` | `benches/runtime_ownership_baselines_alloc/` | `test-support,` `pool` | Allocation counts for the same scopes |

Only the first two are in scope. The runtime-ownership suites measure a
different programme ([#635](https://github.com/leynos/wireframe/issues/635)),
and are named here so a later reader does not mistake them for zero-copy
coverage.

Both codec targets drive shared helpers in
`wireframe_testing::codec_benchmarks`, which is the repository's established
pattern: the scenario is data, one `measure_*` entry point returns a `Result`,
and Criterion supplies the statistics.

## Measurement matrix

Every cell below is a workload. The matrix is the same for all four measurement
kinds, so a later harness only has to plug into an existing identity.

| Stage                   | Codec            | Payload | Workload label                         | Status   |
| ----------------------- | ---------------- | ------- | -------------------------------------- | -------- |
| Inbound decode          | length-delimited | 32 B    | `length_delimited_small`               | Measured |
| Inbound decode          | length-delimited | 64 KiB  | `length_delimited_large`               | Measured |
| Inbound decode          | hotline          | 32 B    | `hotline_small`                        | Measured |
| Inbound decode          | hotline          | 64 KiB  | `hotline_large`                        | Measured |
| Middleware pass-through | length-delimited | 32 B    | `middleware_passthrough_small`         | To build |
| Middleware pass-through | length-delimited | 64 KiB  | `middleware_passthrough_large`         | To build |
| Middleware pass-through | hotline          | 32 B    | `middleware_passthrough_hotline_small` | To build |
| Middleware pass-through | hotline          | 64 KiB  | `middleware_passthrough_hotline_large` | To build |
| Request-hook mutation   | length-delimited | 32 B    | `hook_mutation_small`                  | To build |
| Request-hook mutation   | length-delimited | 64 KiB  | `hook_mutation_large`                  | To build |
| Request-hook mutation   | hotline          | 32 B    | `hook_mutation_hotline_small`          | To build |
| Request-hook mutation   | hotline          | 64 KiB  | `hook_mutation_hotline_large`          | To build |
| Outbound encode         | length-delimited | 32 B    | `outbound_encode_small`                | To build |
| Outbound encode         | length-delimited | 64 KiB  | `outbound_encode_large`                | To build |
| Outbound encode         | hotline          | 32 B    | `outbound_encode_hotline_small`        | To build |
| Outbound encode         | hotline          | 64 KiB  | `outbound_encode_hotline_large`        | To build |

Two deliberate exclusions:

- The empty payload is already covered by the existing fragmentation suite and
  by unit tests; it is not a throughput case.
- Payload sizes are not swept. A sweep invites a curve fit that the migration
  does not need. The two classes bracket the allocation-sensitive regime: 32 B
  is below one allocator granule of interest, 64 KiB is comfortably inside the
  length-delimited maximum and large enough that a redundant copy is decisive.

The `hotline` rows exist to prove the plan is not length-delimited specific. If
a stage cannot be measured for `hotline` without contorting the helper, that is
itself a finding worth recording, because it means the stage's API is coupled
to a particular codec.

## Measurement boundaries

Boundaries are stated per stage because a benchmark number is meaningless
without them. The existing codec helpers set the standard the new stages must
match.

### Encode

`measure_encode` builds the payload, constructs the codec, and allocates the
`BytesMut` write buffer before the timer starts. The timed region repeats
`wrap_payload` and `Encoder::encode` into the reused buffer, calling
`wire.clear()` first. So the number is "steady-state encode into a reused
buffer", not "allocate and encode once".

### Decode

`measure_decode` seeds one encoded frame, freezes it, and then, inside the
timed region, copies those bytes into a fresh `BytesMut`, decodes, and drops
the buffer. The number therefore includes one wire-buffer copy per iteration.
That copy mirrors what the `Framed` reader does in production, so it belongs in
the measurement; a future reader must not subtract it.

### Fragmentation

`measure_fragmented_wrap` and `measure_unfragmented_wrap` use the fixed
1024-byte fragment cap `FRAGMENT_PAYLOAD_CAP_BYTES`, so the fragment count is a
function of the payload class alone. Fragmentation is measured as a paired
overhead between the two helpers rather than as an absolute.

### Allocation

`codec_performance_alloc` installs a counting global allocator and toggles a
gate around the measured window, so setup and reporting allocations are
excluded. It counts allocation operations, not bytes. The counts are also
embedded in benchmark labels as `wrap_allocs_<n>` and `decode_allocs_<n>`, so a
count change is visible in the benchmark name even when the timing is stable.

### Stages that do not exist yet

The three missing stages must adopt the same discipline:

- middleware pass-through: build the request once, run a no-op middleware
  chain, and time only the chain traversal and the `frame()` borrow;
- request-hook mutation: register one hook, time `invoke_before_send_hooks` on
  a pre-built buffer, and report the read-only hook case separately from the
  mutating hook case;
- outbound encode: time `encode_message_frame` only, with the serializer
  pre-constructed; report the serializer alone and the serializer plus
  `wrap_payload` as two numbers, because the migration removes the seam between
  them and a single combined number would hide the change.

Read-only and mutating paths are reported separately for every stage. The
migration's claim is that read-only paths stay zero-copy, so averaging a
copy-on-demand case into a pass-through case would erase the effect being
measured.

## Sampling and statistics

The codec targets use Criterion with default settings: 100 samples, a 3 second
warm-up, a 5 second measurement window, a 5 % significance level (95 %
confidence intervals on the change estimate), and a 1 % noise threshold. Each
workload uses `iter_custom` with the shared `measure_*` helper, so one
Criterion iteration is a batch of helper iterations and per-iteration
amortization happens inside the helper.

Each workload sets `Throughput::Bytes(payload_len)`, so Criterion reports both
time per iteration and bytes per second. New stages must set a throughput value
as well; a stage that cannot express one (middleware pass-through has no
meaningful byte count for a no-op chain) must document why and report time only.

Comparisons use Criterion baselines rather than a stored spreadsheet:

```sh
cargo bench --bench codec_performance --features test-support -- \
  --save-baseline pre-migration
cargo bench --bench codec_performance --features test-support -- \
  --baseline pre-migration
```

The second command prints the change estimate and confidence interval for each
workload, which is the evidence a threshold check consumes.

## Toolchain, features, and hardware

A baseline is only reproducible alongside the machine and build that produced
it. The environment stamp recorded with every measurement set is:

| Property  | Value                                                                                          |
| --------- | ---------------------------------------------------------------------------------------------- |
| CPU       | AMD Ryzen 9 3900 12-Core Processor, 24 hardware threads                                        |
| Memory    | 125 GiB                                                                                        |
| OS        | Rocky Linux 10.2, kernel 6.12.0-211.62.1.el10_2                                                |
| Toolchain | `nightly-2026-10-30` from `rust-toolchain.toml`                                                |
| Profile   | Cargo `release`, the default for `cargo bench`                                                 |
| Features  | `test-support` for the codec targets; add `pool` for runtime ownership                         |
| Linker    | `make bench-codec` sets `RUSTFLAGS`, which displaces the `mold` flag from `.cargo/config.toml` |

The toolchain is pinned by `rust-toolchain.toml`, so a later run resolves the
same `rustc` without a manual step. The linker row matters because a baseline
captured through `make` and one captured through a bare `cargo bench` may
differ in link step, not in generated code quality.

Compile admission is shared. A measured run records whether it waited, because
a heavily loaded host can distort a short workload even though the measurement
itself is unaffected by compilation.

## Reproducibility

Every measurement set is captured with the commit it describes and the command
that produced it:

```sh
set -o pipefail
COMMIT=$(git rev-parse HEAD)
printf 'commit=%s\n' "$COMMIT"
timeout 3600 make bench-codec 2>&1 | tee \
  "/tmp/bench-codec-$(git branch --show-current).out"
```

The commit is recorded in the report because the harness itself evolves. A
number is only comparable to another number taken from the same harness source;
when the harness changes, the previous baseline is re-taken rather than
reinterpreted.

## Copied bytes

No current harness measures copied bytes, and this is the largest real gap.
Allocation counts catch a new buffer; they do not catch a copy into an existing
one. Three copy sites are in scope:

1. `Serializer::serialize` returns `Vec<u8>`, so the bincode serializer
   allocates and fills a fresh vector for every outbound message
   (`src/serializer.rs`). This allocation is removable under roadmap item
   `11.1.2`.
2. `src/app/outbound_encoding.rs` calls `Bytes::from(bytes)`. When the vector's
   length differs from its capacity, which is the normal case, this allocates a
   24-byte shared control block and may relocate the payload.
3. `LengthDelimitedEncoder::encode` delegates to `tokio_util`, which copies the
   payload again into the `Framed` write buffer. That copy happens outside
   `encode_message_frame` and is not removable by roadmap item `11.1.2`.

Site 3 is the reason a byte-level metric is required: the migration's headline
claim is about copies, and only one of the three sites is visible to an
allocation counter. Two acceptable methods, in order of preference:

- instrument the copy sites behind a `benchmark-counters` feature and assert
  the per-message copied-byte count directly;
- use DHAT in a one-off run to obtain total allocated bytes per operation, and
  treat the delta between the two payload classes as the per-message copy.

Whichever method lands, the assertion belongs in a test, not only in a report,
so a regression fails the build.

## Correctness coverage

A performance win that breaks a contract is not a win. Every measurement set is
paired with the coverage below, and the migration must keep all of it green.

| Contract              | Coverage today                                                           |
| --------------------- | ------------------------------------------------------------------------ |
| Codec payload sharing | `src/codec/tests/mod.rs` pointer-identity assertions for wrap and decode |
| Mutation semantics    | `tests/middleware.rs`, `tests/middleware_order.rs`                       |
| Correlation           | `tests/correlation_id.rs`, middleware correlation-preservation tests     |
| Fragmentation         | `tests/connection_fragmentation.rs`, `tests/fragment_transport/`         |
| Hook behaviour        | `tests/features/client_request_hooks.feature` and its steps              |
| Wire output           | `tests/frame_codec.rs`, serializer round-trip tests                      |

The pointer-identity assertions are the model the new stages should copy: they
compare `as_ptr()` between the handed-in bytes and the frame payload, so a
regression fails immediately rather than showing up as a slow drift.

## Acceptance thresholds

Thresholds are published, with the measured baselines, in
[zero-copy benchmark baselines](zero-copy-benchmark-baselines.md). They are not
repeated here. The rules they follow are:

- a threshold applies per workload, not to an average across workloads;
- a throughput or latency threshold is expressed as a percentage change with a
  confidence interval, so a noisy run cannot pass by accident;
- the outbound default path must not allocate a new payload buffer between
  serialization and `wrap_payload`, evidenced by both an allocation count and a
  pointer-identity test;
- read-only stages must show zero additional allocations per operation;
- a stage without a harness has no threshold, and is reported as unmeasured
  rather than assumed unchanged.

## Gaps and follow-up work

| Gap                                        | Consequence                                    | Disposition               |
| ------------------------------------------ | ---------------------------------------------- | ------------------------- |
| No middleware pass-through harness         | Roadmap `1.2.2` cannot be completed            | Proposed issue, see below |
| No request-hook mutation harness           | Read-only hook claim is unmeasured             | Same proposal             |
| No outbound encode harness                 | The headline copy removal has no before number | Same proposal             |
| No copied-byte metric                      | Copy regressions are invisible                 | Same proposal             |
| No `hotline` rows for the three new stages | Codec-independence claim is untested           | Same proposal             |

These gaps are deliberately not closed by quietly narrowing the roadmap item.
The proposed follow-up issue carries the harness work, with acceptance criteria
that name the stages, the matrix rows, and the pointer-identity evidence
required. Until it lands, roadmap item `1.2.2` and its combined counterpart
`10.2.2` stay open with this document as the blocker record.

## References

- [ADR 008: zero-copy public byte container](adr-008-zero-copy-public-byte-container.md)
- [ADR 009: `Vec<u8>` migration rollout](adr-009-vec-u8-migration-rollout.md)
- [ADR 010: transport frame boundary](adr-010-transport-frame-boundary-for-zero-copy.md)
- [Frame and `Vec<u8>` inventory](frame-vec-u8-inventory.md)
- [Zero-copy benchmark baselines](zero-copy-benchmark-baselines.md)
- [Runtime-ownership baselines](runtime-ownership-baselines.md), the
  repository's existing baseline-document convention
