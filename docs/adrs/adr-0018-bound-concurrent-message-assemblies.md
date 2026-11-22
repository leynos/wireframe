# Architectural decision record (ADR) 018: bound concurrent message assemblies

## Status

Accepted.

`MessageAssemblyState` bounds the number of concurrently in-flight assemblies,
charges a fixed per-entry cost against the aggregate budgets, keeps running
byte totals instead of rescanning, and evicts by a start-time index instead of
a full scan. The change is implemented by issue
[#830](https://github.com/leynos/wireframe/issues/830).

## Date

2026-11-22.

## Context and problem statement

`MessageAssemblyState` accepted a first frame for any number of distinct
`MessageKey` values and retained every partial assembly until it completed or
timed out. Two properties combined into a denial-of-service vector:

- the aggregate budgets compared only buffered payload and metadata bytes, so a
  first frame with empty metadata and an empty body added an assembly that
  counted as zero bytes against both budgets; and
- every first frame called `purge_expired_at`, which walked every assembly, and
  `total_buffered_bytes` rescanned the map on every admission, every
  continuation, and every memory-pressure check.

A peer could therefore keep adding cheap, empty assemblies within the timeout
window while each later frame paid O(n) work, giving quadratic CPU cost without
ever crossing a byte-pressure threshold. This is algorithmic complexity
exhaustion (CWE-407). It is the message-assembler counterpart of the fragment
reassembler cap in [#543](https://github.com/leynos/wireframe/issues/543) and
its expiry scan in [#829](https://github.com/leynos/wireframe/issues/829).

## Decision drivers

- Aggregate budgets must bound the state the connection actually retains, not
  only the bytes a peer happens to carry.
- Per-frame work must not grow with the number of buffered assemblies.
- The server path must be protected by default, while a directly constructed
  `MessageAssemblyState` keeps its existing uncapped behaviour.
- Existing error precedence must survive: duplicate keys, malformed
  declarations, size limits, and single-frame messages keep their current
  outcomes.

## Options considered

### Option A: cap, per-entry charge, running totals, and a deadline index

Reject a first frame that would exceed a configurable concurrent-assembly cap.
Charge a fixed per-entry cost against the aggregate budgets. Maintain payload
totals and a start-time index incrementally as assemblies are inserted,
appended, completed, or evicted.

### Option B: concurrent-assembly cap only

Bound the number of assemblies without changing the byte accounting. Empty
assemblies remain free until the cap is reached, and the per-frame scans remain.

### Option C: per-entry charge only

Make empty assemblies cost budget without bounding their count. A sufficiently
large budget still permits many entries, and the O(n) scans remain.

### Option D: keep scanning and tighten the byte budgets

Retain the existing implementation and lower the aggregate limits. The
quadratic work remains reachable within any non-zero budget, and legitimate
payloads lose headroom.

| Topic                      | Option A | Option B | Option C | Option D |
| -------------------------- | -------- | -------- | -------- | -------- |
| Bounds empty assemblies    | Yes      | Yes      | No       | Partly   |
| Removes per-frame scans    | Yes      | No       | No       | No       |
| Bounds CPU per frame       | Yes      | No       | No       | No       |
| Preserves payload headroom | Yes      | Yes      | Yes      | No       |
| Changes default behaviour  | Server   | Server   | Server   | All      |

_Table 1: Trade-offs between the considered options._

## Decision outcome

Option A is adopted.

### Per-entry charge

`MESSAGE_ASSEMBLY_ENTRY_OVERHEAD_BYTES` is 64 bytes. The charged total is the
retained payload and metadata plus one fixed charge per live assembly:

```text
total_charged_bytes = total_buffered_bytes + buffered_count × 64
```

Admission and memory-pressure checks use the charged total. A new first frame
charges its metadata, its body, and one overhead; a continuation charges only
the incoming body because the entry already exists. `total_buffered_bytes`
remains the exact retained payload, and its public meaning does not change.
Arithmetic saturates rather than wrapping.

### Concurrent-assembly cap

The server path caps concurrent assemblies at 64 by default, matching the
fragment reassembler's concurrency cap.
`WireframeApp::max_concurrent_assemblies` overrides it, and `NonZeroUsize::MAX`
is effectively unbounded. The cap is checked on a new first frame only, after
the duplicate-key, declared-total, and size-limit checks and after the
single-frame return. A rejection reports
`MessageAssemblyError::TooManyConcurrentAssemblies` with the key, the attempted
count, and the limit. Directly constructed `MessageAssemblyState` values are
uncapped unless `with_max_concurrent_assemblies` is called.

### Incremental accounting and expiry

A private `AssemblyTracker` owns the assembly map, the running payload total,
and a `BTreeSet<(Instant, MessageKey)>` holding one deadline record per live
partial. Every insert, append, take, and purge updates the map, the total, and
the index as one step. Expiry reads the earliest record and removes entries
while their age has reached the timeout, so an idle purge costs O(1) and a
sweep costs time proportional to the number evicted. The expiry predicate is
unchanged: an assembly expires when
`now.saturating_duration_since(started_at) >= timeout`.

### Consequences

- Empty assemblies are no longer free, and the number of concurrent assemblies
  is bounded on the default server path.
- Admission, continuation, completion, and pressure checks no longer scan the
  map.
- A caller that relied on unbounded concurrency from `MessageAssemblyState`
  must opt out explicitly with a larger cap; the direct constructors keep their
  previous behaviour.
- `MessageAssemblyError` gains a variant, which is a compile-time break for
  exhaustive downstream matches, consistent with ADR 015.
- A configuration whose per-connection or in-flight budget is smaller than
  64 bytes can no longer admit any assembly; the derived defaults in
  `default_memory_budgets` are far above that floor.

## References

- [#543](https://github.com/leynos/wireframe/issues/543): fragment reassembler
  concurrent-partial cap.
- [#829](https://github.com/leynos/wireframe/issues/829): fragment reassembler
  expiry scan.
- [#830](https://github.com/leynos/wireframe/issues/830): this change.
- [ADR 002](adr-0002-streaming-requests-and-shared-message-assembly.md):
  shared message assembly.
- [ADR 015](adr-0015-declared-total-body-length-is-authoritative.md): the
  declared total is authoritative.
