# Zero-copy benchmark plan

This plan defines how the zero-copy migration in Epic
[#284](https://github.com/leynos/wireframe/issues/284) is measured. It closes
roadmap item `1.2.1` of the
[zero-copy migration roadmap](zero-copy-frame-and-payload-migration-roadmap.md)
and roadmap item `10.2.1` of the [project roadmap](roadmap.md), both of which
ask for a benchmark matrix covering inbound decode, middleware pass-through,
request-hook mutation, and outbound encode.

All four stages now have a harness. The plan records, per stage, what the timed
region contains and what it deliberately excludes, so a later comparison can
tell a real change from a measurement artefact.

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
- acceptance thresholds that have not been agreed by a performance reviewer.
  The proposed thresholds live in
  [zero-copy benchmark baselines](zero-copy-benchmark-baselines.md).

## The four stages

The stages follow the path an outbound or inbound message takes, not the module
layout, because the migration changes ownership at three separate boundaries.

| Stage                   | Direction | Owning surface                                     | Harness                |
| ----------------------- | --------- | -------------------------------------------------- | ---------------------- |
| Inbound decode          | inbound   | `FrameCodec::decoder`, `FrameCodec::frame_payload` | `codec_performance`    |
| Middleware pass-through | inbound   | `ServiceRequest`, `ServiceResponse`                | `pipeline_performance` |
| Request-hook mutation   | outbound  | `BeforeSendHook`, `send_pipeline`                  | `pipeline_performance` |
| Outbound encode         | outbound  | `Serializer::serialize`, `wrap_payload`            | `pipeline_performance` |

"Inbound decode" here means byte-buffer to frame: the codec decodes a wire
buffer and hands out a payload. It does not include application deserialization
into domain types, which the migration does not change. "Outbound encode" means
the two-step `serialize` then `wrap_payload` hand-off performed by
`src/app/outbound_encoding.rs`; the transport write that follows is outside the
migration's boundary.

## Existing harnesses

| Target                              | File                                         | Features                             | Measures                                    |
| ----------------------------------- | -------------------------------------------- | ------------------------------------ | ------------------------------------------- |
| `codec_performance`                 | `benches/codec_performance.rs`               | `test-support`, `benchmark-counters` | Encode, decode, fragmentation timing        |
| `codec_performance_alloc`           | `benches/codec_performance_alloc.rs`         | `test-support`, `benchmark-counters` | Allocation counts for wrap and decode       |
| `pipeline_performance`              | `benches/pipeline_performance.rs`            | `test-support`, `benchmark-counters` | Middleware, hook, and outbound stage timing |
| `pipeline_performance_alloc`        | `benches/pipeline_performance_alloc.rs`      | `test-support`, `benchmark-counters` | Allocation counts for the same three stages |
| `runtime_ownership_baselines`       | `benches/runtime_ownership_baselines/`       | `test-support,` `pool`               | Actor, push, startup, pool timing           |
| `runtime_ownership_baselines_alloc` | `benches/runtime_ownership_baselines_alloc/` | `test-support,` `pool`               | Allocation counts for the same scopes       |

The first four are in scope; `make bench-codec` runs exactly those. The
runtime-ownership suites measure a different programme
([#635](https://github.com/leynos/wireframe/issues/635)), and are named here
so a later reader does not mistake them for zero-copy coverage.

Every target drives shared helpers in `wireframe_testing::codec_benchmarks`,
which is the repository's established pattern: the scenario is data, one
`measure_*` entry point returns a `Result`, and Criterion supplies the
statistics. A single counting mechanism in that crate — `stats_alloc`'s
`INSTRUMENTED_SYSTEM`, installed by reference — serves both allocation targets,
so the two cannot drift in accounting.

## Measurement matrix

Every cell below is a workload. The matrix is the same for all four measurement
kinds, so a later harness only has to plug into an existing identity.

| Stage                   | Codec            | Payload | Workload label prefix               | Status   |
| ----------------------- | ---------------- | ------- | ----------------------------------- | -------- |
| Inbound decode          | length-delimited | 32 B    | `length_delimited_small`            | Measured |
| Inbound decode          | length-delimited | 64 KiB  | `length_delimited_large`            | Measured |
| Inbound decode          | hotline          | 32 B    | `hotline_small`                     | Measured |
| Inbound decode          | hotline          | 64 KiB  | `hotline_large`                     | Measured |
| Middleware pass-through | length-delimited | 32 B    | `middleware_passthrough_length_` …  | Measured |
| Middleware pass-through | length-delimited | 64 KiB  | `middleware_passthrough_length_` …  | Measured |
| Middleware pass-through | hotline          | 32 B    | `middleware_passthrough_hotline_` … | Measured |
| Middleware pass-through | hotline          | 64 KiB  | `middleware_passthrough_hotline_` … | Measured |
| Request-hook mutation   | length-delimited | 32 B    | `hook_mutation_length_delimited_` … | Measured |
| Request-hook mutation   | length-delimited | 64 KiB  | `hook_mutation_length_delimited_` … | Measured |
| Request-hook mutation   | hotline          | 32 B    | `hook_mutation_hotline_` …          | Measured |
| Request-hook mutation   | hotline          | 64 KiB  | `hook_mutation_hotline_` …          | Measured |
| Outbound encode         | length-delimited | 32 B    | `outbound_serialize_length_` …      | Measured |
| Outbound encode         | length-delimited | 64 KiB  | `outbound_serialize_length_` …      | Measured |
| Outbound encode         | hotline          | 32 B    | `outbound_serialize_hotline_` …     | Measured |
| Outbound encode         | hotline          | 64 KiB  | `outbound_serialize_hotline_` …     | Measured |

The label is a prefix rather than a whole identifier because each row is
reported more than once. Two suffixes complete it:

- `_readonly` or `_mutating` for the middleware and hook stages, because the
  migration's claim is that read-only paths stay zero-copy and a mutating path
  copies on demand. Averaging the two would erase the effect.
- No access-mode segment for the outbound rows, because that stage always
  writes. Its two passes are distinguished by the stage segment instead:
  `outbound_serialize_*` is `Serializer::serialize` alone, `outbound_encode_*`
  is serialize plus `FrameCodec::wrap_payload`. The pair is what isolates the
  seam the migration changes.

Full identifiers are therefore, for example,
`middleware_passthrough_hotline_large_mutating` and
`outbound_encode_length_delimited_small`. The allocation targets append
`_allocs_<n>` to the same identifier.

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
to a particular codec. All three new stages measure both codecs.

## Measurement boundaries

Boundaries are stated per stage because a benchmark number is meaningless
without them.

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

### Middleware pass-through

`measure_middleware_passthrough` builds the chain from the library's own
`Transform` and `Service` traits and terminates it in a service that returns
the request frame unchanged. The frame vector is taken out of a reused buffer
with `std::mem::take` and returned by the response, so no iteration allocates a
payload. The timed region is chain traversal plus the middleware's frame
borrow, and nothing else.

The chain does not use the `from_fn` adapter. That adapter requires one future
type across every continuation lifetime
(`F: for<'a> Fn(ServiceRequest, Next<'a, S>) -> Fut`), and a middleware that
forwards to `next.call` returns a future borrowing `Next`, so its type varies
with the lifetime and cannot satisfy the bound. The harness mirrors `FnService`
instead: the same traits, the same `Send + Sync` bounds, and the same
`Next::new(&self.inner)` dispatch, with only the per-request callback erased
behind a policy trait. This is a documented exception, not a dropped row.

### Request-hook mutation

`measure_hook_dispatch` registers one `BeforeSendHook` and dispatches through
the client's own before-send loop, not a replica. The read-only hook calls
`bytes.len()` through `black_box`; the mutating hook rewrites the first byte.
The buffer is built once, outside the timer, and the loop runs in place.

### Outbound encode

`measure_outbound_serialize` times `Serializer::serialize` alone.
`measure_outbound_wrap` times serialize followed by `FrameCodec::wrap_payload`,
via the real `encode_message_frame` seam. Both construct the serializer, the
message, and the codec before the timer starts, and both reset the copied-byte
counters immediately before it.

The harness serializer wraps a fixed payload and allocates its output exactly,
mirroring `bincode::encode_to_vec`, which sizes its output with a `SizeWriter`
before allocating. Mirroring that matters: a naive growing `Vec` would leave
spare capacity and manufacture a hand-off copy the default serializer does not
make, which would misreport the very seam under measurement.

### Allocation

The allocation targets install a counting global allocator and toggle a gate
around the measured window, so setup and reporting allocations are excluded. It
counts allocation operations, not bytes. The counts are embedded in benchmark
labels as `_allocs_<n>`, so a count change is visible in the benchmark name
even when the timing is stable.

### Copied bytes

`src/benchmark_counters.rs` counts copied **bytes** at three seams, behind the
`benchmark-counters` feature, and the recorders compile to empty inline
functions when the feature is off:

1. `record_serialized` — the serializer materializes an owned `Vec<u8>`. It is
   recorded at the production outbound boundaries, `encode_message_frame` and
   `serialize_and_send`, not in `EncodeWith::encode_with`: a Serde-backed
   serializer never reaches that method, so recording there would miss those
   messages and double-count a harness that calls `serialize` itself.
2. `record_handoff` — `Bytes::from(Vec<u8>)` had to relocate the payload. It
   is recorded by `bytes_from_vec`, which compares `as_ptr()` before and after.
   With `bytes` 1.12.1 neither an exact-capacity vector nor one with spare
   capacity copies the payload: the second case allocates a shared control
   block that still points at the vector's buffer. The seam therefore measures
   zero today and exists as a guard against a later `bytes` change or a
   hand-off written some other way.
3. `record_framed` — the codec copies the payload into the transport write
   buffer. `wrap_payload` builds a frame but writes no transport buffer, so
   this seam belongs to `Encoder::encode` and is probed by
   `measure_framed_copies`, not by the outbound wrap pass.

Copied bytes are a separate measurement kind from allocation counts, and the
two are not interchangeable: an allocation counter cannot see a copy into an
already-allocated buffer, which is exactly what site 3 does. The assertion
lives in `wireframe_testing/tests/copied_bytes.rs` rather than in a report, so
a regression fails the build. It covers all four codec and payload-class
workloads and states the pre-migration outbound total as two payload copies per
message: one serializer materialization and one transport framing copy.

## Sampling and statistics

The targets use Criterion with default settings: 100 samples, a 3 second
warm-up, a 5 second measurement window, a 5 % significance level (95 %
confidence intervals on the change estimate), and a 1 % noise threshold. Each
workload uses `iter_custom` with the shared `measure_*` helper, so one
Criterion iteration is a batch of helper iterations and per-iteration
amortization happens inside the helper.

Codec encode and outbound workloads set `Throughput::Bytes(payload_len)`: both
sides move a known payload, so bytes per second is the meaningful rate.
Middleware pass-through and hook dispatch set `Throughput::Elements(1)`,
because their cost is per message and does not scale with payload size.
Reporting 64 KiB per message there produced terabyte-per-second figures that
described neither the stage nor any user-visible throughput; the
[justified exceptions](zero-copy-benchmark-baselines.md#justified-exceptions)
section of the baselines records the reasoning. A stage that could not express
any throughput value would have to document why and report time only; none of
the four now needs that exception.

Comparisons use Criterion baselines rather than a stored spreadsheet:

```sh
cargo bench --bench codec_performance --bench pipeline_performance \
  --features test-support,benchmark-counters -- --save-baseline pre-migration
cargo bench --bench codec_performance --bench pipeline_performance \
  --features test-support,benchmark-counters -- --baseline pre-migration
```

The second command prints the change estimate and confidence interval for each
workload, which is the evidence a threshold check consumes.

The benchmark entry point runs with `RUSTFLAGS="-D warnings"`. A warning in a
benchmark is a defect in the harness, so it fails the run rather than printing
and continuing.

## Toolchain, features, and hardware

A baseline is only reproducible alongside the machine and build that produced
it. The environment stamp recorded with every measurement set is:

| Property  | Value                                                                                             |
| --------- | ------------------------------------------------------------------------------------------------- |
| CPU       | AMD Ryzen 9 3900 12-Core Processor, 24 hardware threads                                           |
| Memory    | 125 GiB                                                                                           |
| OS        | Rocky Linux 10.2, kernel 6.12.0-211.62.1.el10_2                                                   |
| Toolchain | `nightly-2026-10-30` from `rust-toolchain.toml`                                                   |
| Profile   | Cargo `release`, the default for `cargo bench`                                                    |
| Features  | `test-support,benchmark-counters` for the four in-scope targets; add `pool` for runtime ownership |
| Linker    | `make bench-codec` sets `RUSTFLAGS`, which displaces the `mold` flag from `.cargo/config.toml`    |

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
reinterpreted. The copied-byte instrumentation is such a change: it adds a
counter increment to the encode path, so the codec baselines were re-taken when
it landed.

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
| Copied bytes          | `wireframe_testing/tests/copied_bytes.rs` per-seam byte assertions       |

The pointer-identity assertions are the model the new stages copy: they compare
`as_ptr()` between the handed-in bytes and the frame payload, so a regression
fails immediately rather than showing up as a slow drift. The copied-byte
assertions extend the same idea from identity to volume.

## Acceptance thresholds

Thresholds are published, with the measured baselines, in
[zero-copy benchmark baselines](zero-copy-benchmark-baselines.md). They are not
repeated here. The rules they follow are:

- a threshold applies per workload, not to an average across workloads;
- a throughput or latency threshold is expressed as a percentage change with a
  confidence interval, so a noisy run cannot pass by accident;
- the outbound default path must not allocate or copy a new payload buffer
  between serialization and `wrap_payload`, evidenced by an allocation count, a
  pointer-identity test, and a copied-byte assertion;
- read-only stages must show zero additional allocations per operation;
- a stage without a harness has no threshold, and is reported as unmeasured
  rather than assumed unchanged. No stage currently needs that exception.

## Gaps and follow-up work

| Gap                                               | Consequence                                   | Disposition                                             |
| ------------------------------------------------- | --------------------------------------------- | ------------------------------------------------------- |
| Harnesses for the three non-codec stages          | Roadmap `1.2.2` could not be completed        | Closed by `pipeline_performance` and `_alloc` targets   |
| No copied-byte metric                             | Copy regressions were invisible               | Closed by `benchmark-counters` and `copied_bytes.rs`    |
| No `hotline` rows for the three new stages        | Codec-independence claim was untested         | Closed; both codecs are measured for every stage        |
| Thresholds need a performance reviewer's sign-off | A threshold cannot be "accepted" unilaterally | Open; recorded as a blocker in the baselines document   |
| Baseline comparison against a migrated candidate  | The migration's delta is not yet measurable   | Deferred to roadmap `13.2.1`, after the migration lands |

The remaining open items are agreement on the proposed thresholds and the
post-migration comparison. Neither is a harness gap, and neither is closed by
this document.

## References

- [ADR 008: zero-copy public byte container](adrs/adr-0008-zero-copy-public-byte-container.md)
- [ADR 009: `Vec<u8>` migration rollout](adrs/adr-0009-rollout-strategy-for-the-vec-u8-to-zero-copy-migration.md)
- [ADR 010: transport frame boundary](adrs/adr-0010-transport-frame-boundary-for-the-zero-copy-migration.md)
- [Frame and `Vec<u8>` inventory](frame-vec-u8-inventory.md)
- [Zero-copy benchmark baselines](zero-copy-benchmark-baselines.md)
- [Runtime-ownership baselines](runtime-ownership-baselines.md), the
  repository's existing baseline-document convention
