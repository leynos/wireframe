# ADR 0002: Streaming requests and shared message assembly

## Status

Proposed

## Context

Wireframe already provides protocol-agnostic support for streaming responses
(`Response::Stream`) and multi-packet responses, with back-pressure emerging
from the connection actor’s ownership of the outbound write loop.[^streaming]
Wireframe also has a protocol-agnostic fragmentation and reassembly layer that
can transparently reconstruct logical messages before routing.[^fragmentation]

Downstream protocol implementations still end up writing substantial bespoke
plumbing for large inbound messages and multi-frame requests:

- inbound request payloads must be fully buffered and reassembled before
  dispatch, even when a handler could process the body incrementally;
- many protocols share similar “fixed header + total size + per-frame chunk
  size + continuation frames” patterns, but each crate re-implements the
  buffering, limit enforcement, and continuity checks; and
- memory limits and early-abort policies are often pushed into each protocol
  crate, leading to inconsistent back-pressure and denial-of-service (DoS)
  behaviour across the ecosystem.

This is in tension with Wireframe’s 1.0 philosophy: provide a protocol-agnostic
“duplex frame highway” where cross-cutting hardening and ergonomics are solved
centrally, with clear extension points for protocol-specific
rules.[^philosophy] The production resilience guidance also emphasises explicit
resource caps, timeouts, and early termination at the framework layer, rather
than ad hoc per-protocol handling.[^hardening]

## Decision

Wireframe will add first-class, protocol-agnostic support for streaming
requests and shared multi-frame message assembly. The design is additive and
opt-in: existing handlers and protocols continue to work unchanged.

### 1) First-class streaming request bodies

Wireframe will expose a handler-facing request shape that can separate metadata
from the payload, so handlers can choose between buffered and streaming
consumption.

- Handlers MAY receive `RequestParts` (header / metadata) plus a `Body`-like
  stream, rather than a fully reassembled `Vec<u8>`.
- The payload stream will be representable as either `impl Stream<Item =
  bytes::Bytes>` or an `AsyncRead
  ` wrapper (or both), so protocol crates can integrate with existing parsers.
- The default remains “buffered request” to preserve Wireframe’s existing
  transparent assembly ergonomics for small messages and simple protocols.

This decision is symmetrical with `Response::Stream`, but applies the same
capability-maturity constraints as existing features: opt-in, explicit, and
safe by default.[^philosophy]

### 2) A generic multi-frame message assembler abstraction

Wireframe will provide a generic message assembly hook (conceptually a
`MessageAssembler`) so protocol crates can supply protocol-specific parsing and
continuity rules, while Wireframe supplies the shared buffering machinery and
limit enforcement.

At a minimum, the hook must allow a protocol to provide:

- a per-frame header parser (including “first frame” versus “continuation
  frame” handling);
- a message key for multiplexing interleaved assemblies;
- declared or inferred expected total size (when available);
- per-fragment length expectations; and
- continuity validation (ordering, missing fragments, duplicate fragments).

Wireframe will provide, centrally:

- buffering and assembly state management;
- enforcement of maximum total message size, maximum fragment size, timeouts,
  and “max in-flight bytes”; and
- a clear failure mode (back-pressure where safe, early abort where required).

This generalises the intent of the existing fragmentation adapter design
without introducing protocol-specific assumptions.[^fragmentation]

### 3) Configurable per-connection memory budgets and back-pressure

Wireframe will standardise memory budgeting for inbound assembly and buffering
at the connection layer, so protocols do not need to implement their own
resource accounting to remain safe.

- Budgets MUST be configurable per connection (and MAY be configurable
  globally).
- Budgets MUST cover: bytes buffered per message, bytes buffered per
  connection, and bytes buffered across in-flight assemblies.
- When budgets are approached, Wireframe SHOULD apply back-pressure by pausing
  further reads / assembly work rather than eagerly buffering more data.
- When a hard cap is exceeded, Wireframe MUST abort early and release partial
  state.

This aligns with the existing hardening guidance and extends it beyond
fragmentation to cover streaming request bodies and other assembly
paths.[^hardening]

### 4) A “write from reader/stream” transport helper

Wireframe will provide a transport-level helper (conceptually
`send_streaming(frame_header, body_reader)`) that:

- reads from an `AsyncRead` or byte stream;
- emits frames using a configured chunk size (with protocol-provided headers);
- handles flushing and timeouts consistently; and
- integrates with the existing connection actor, instrumentation, and hooks.

This reduces duplicate “read N bytes, stamp header, write frame” loops across
protocol implementations while keeping header semantics protocol-defined.

### 5) Testkit utilities for fragmentation and streaming

Wireframe will publish official test utilities that can:

- feed partial frames / fragments into an in-process app;
- simulate slow readers and writers to exercise back-pressure; and
- assert emitted frames and reassembly outcomes deterministically.

These utilities should build on the existing `wireframe_testing` companion
crate, and may be re-exported as `wireframe::testkit` behind a dedicated
feature to keep the core crate lightweight.[^testing]

## Consequences

- Wireframe gains an explicit “streaming request body” surface alongside the
  existing streaming response surface, with clear opt-in semantics.
- The fragmentation / reassembly implementation is factored so its buffering
  and limit enforcement can be reused for other multi-frame assembly patterns.
- Connection configuration must grow to represent per-connection memory
  budgets, and the connection actor must consistently apply those budgets for
  inbound buffering and assembly.
- Transport code gains a reusable “write from reader/stream” helper, reducing
  duplicated protocol implementations and making timeout and chunking behaviour
  consistent.
- The test helper story becomes first-class, enabling downstream protocol crates
  to write behavioural tests for fragmentation and streaming without
  re-building bespoke harnesses.

[^streaming]:
    See `docs/multi-packet-and-streaming-responses-design.md` and
    [ADR 0001](0001-multi-packet-streaming-response-api.md).

[^fragmentation]:
    See `docs/generic-message-fragmentation-and-re-assembly-design.md`.

[^philosophy]:
    See `docs/the-road-to-wireframe-1-0-feature-set-philosophy-and-capability-maturity.md`.

[^hardening]:
    See `docs/hardening-wireframe-a-guide-to-production-resilience.md`.

[^testing]:
    See `docs/wireframe-testing-crate.md`.
