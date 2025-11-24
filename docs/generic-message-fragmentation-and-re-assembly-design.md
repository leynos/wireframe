# Comprehensive design: Generic message fragmentation & re‑assembly

## 1. Introduction and philosophy

Many robust network protocols, from modern Remote Procedure Call (RPC) systems
to legacy financial standards, require the ability to split a single logical
message into multiple physical frames. This can be necessary to respect Maximum
Transmission Unit (MTU) limits, handle large data payloads, or align with
encryption block sizes. `wireframe`'s current model, which processes one
inbound frame to one logical frame, cannot handle this.

This document details the design for a generic, protocol-agnostic fragmentation
and re-assembly layer. The core philosophy is to treat this as a **transparent
middleware**. Application-level code, such as handlers, should remain unaware
of the underlying fragmentation, dealing only with complete, logical messages.
This new layer will be responsible for automatically splitting oversized
outbound frames and meticulously re-assembling inbound fragments into a single,
coherent message before they reach the router.

> Status: design proposal. API names, trait bounds, and configuration shapes
> may change before stabilisation.

This feature is a critical component of the "Road to Wireframe 1.0," designed
to seamlessly integrate with and underpin the streaming and server-push
capabilities.

## 2. Design goals and requirements

The implementation must satisfy the following core requirements:

<!-- markdownlint-disable MD013 MD060 -->
| ID | Goal                                                                                                                                                                                                                   |
| --- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| G1 | Transparent inbound re-assembly → The router and handlers must always receive one complete, logical Frame.                                                                                                             |
| G2 | Transparent outbound fragmentation when a payload exceeds a configurable, protocol-specific size.                                                                                                                      |
| G3 | Pluggable Strategy: The logic for parsing and building fragment headers, detecting the final fragment, and managing sequence numbers must be supplied by the protocol implementation, not hard-coded in the framework. |
| G4 | Denial‑of‑service (DoS) protection: The re-assembly process must be hardened against resource exhaustion attacks via strict memory and time limits.                                                                    |
| G5 | Zero Friction: Protocols that do not use fragmentation must incur no performance or complexity overhead. This feature must be strictly opt-in.                                                                         |
<!-- markdownlint-enable MD013 MD060 -->

## 3. Core architecture: the `FragmentAdapter`

The feature will be implemented as a codec middleware called `FragmentAdapter`.
It is instantiated with a protocol-specific `FragmentStrategy` and wraps any
subsequent codecs in the chain.

```plaintext
Socket I/O ↔ [Compression] ↔ FragmentAdapter ↔ Router/Handlers
```

This layered approach ensures that fragmentation is handled on clear-text,
uncompressed data, as required by most protocol specifications.

### 3.1 State management for multiplexing

A critical requirement for modern protocols is the ability to handle
interleaved fragments from different logical messages on the same connection.
To support this, the `FragmentAdapter` will not maintain a single re-assembly
state, but a map of concurrent re-assembly processes.

```rust
use dashmap::DashMap;
use std::sync::atomic::AtomicU64;
use std::time::{Duration, Instant};

pub struct FragmentAdapter<S: FragmentStrategy> {
    strategy: S,
    /// Hard cap on the size of a re-assembled logical message.
    max_message_size: usize,
    /// Timeout for completing a partial message re-assembly.
    reassembly_timeout: Duration,
    /// Concurrently accessible map for in-flight message re-assembly.
    reassembly_buffers: DashMap<u64, PartialMessage>,
    /// Atomic counter for generating unique outbound message IDs.
    next_outbound_msg_id: AtomicU64,
}

/// State for a single, in-progress message re-assembly.
struct PartialMessage {
    /// The buffer holding the accumulating payload.
    buffer: BytesMut,
    /// The advertised total payload size, if known.
    expected_total: Option<usize>,
    /// The sequence number of the last fragment received.
    last_sequence: u64,
    /// The time the first fragment was received.
    started_at: Instant,
}
```

The use of `dashmap::DashMap` allows for lock-free reads and sharded writes,
providing efficient and concurrent access to the re-assembly buffers without
blocking the entire connection task.

### 3.2 Canonical fragment header (November 2025 update)

Phase 7 introduces a reusable header carried by every fragment, regardless of
the concrete protocol strategy. The new `FragmentHeader` struct records three
fields that every strategy must surface:

- `message_id: MessageId` – a unique identifier for the logical message.
- `fragment_index: FragmentIndex` – a zero-based counter tracking fragment
  ordering.
- `is_last_fragment: bool` – signals that the logical message is complete.

These primitives live in `wireframe::fragment` alongside helper types such as
`FragmentSeries`, which validates ordering and completion for a single message.
The series keeps only lightweight state (message identifier, expected next
index, completion flag) so codecs can embed it without heap allocations.

```rust
use wireframe::fragment::{FragmentHeader, FragmentIndex, FragmentSeries, MessageId};

let mut series = FragmentSeries::new(MessageId::new(7));
let header = FragmentHeader::new(MessageId::new(7), FragmentIndex::zero(), false);
assert!(series.accept(header).is_ok());
```

By standardising this metadata, higher layers—behavioural tests, connection
actors, and observability hooks—can reason about fragments without depending on
protocol-specific codecs. The forthcoming `FragmentStrategy` continues to
control how headers are encoded on the wire, but it now produces a
`FragmentHeader` so downstream components share a single representation.

### 3.3 Fragmenter helper (17 November 2025 update)

To simplify outbound slicing before the full adapter arrives, the crate now
ships a small `Fragmenter` helper. It accepts a `NonZeroUsize` payload cap and
creates sequential fragments tagged with `MessageId`, `FragmentIndex`, and the
`is_last_fragment` flag described above. The helper exposes three entry points:

- `fragment_message` serialises any type implementing the `Message` trait and
  splits the resulting bytes into fragments.
- `fragment_bytes` chunks an owned `Vec<u8>` and allocates a fresh `MessageId`.
- `fragment_with_id` lets callers supply the identifier explicitly when a
  higher transport layer already tracks the logical message identifier.

Each call yields a `FragmentBatch`, a lightweight container that exposes the
shared `MessageId`, reports whether the message was fragmented, and can be
iterated to drain individual `FragmentFrame` structs. Those frames surface the
`FragmentHeader` alongside their payload bytes, making it trivial to wrap them
in a protocol-specific envelope or log detailed fragment metadata during tests.
The helper detects `FragmentIndex` overflow and returns an explicit error so
protocols can downgrade to streaming or reject the payload before building an
invalid sequence.

## 4. Public API: the `FragmentStrategy` trait

The power and flexibility of this feature come from the `FragmentStrategy`
trait. Protocol implementers will provide a type that implements this trait to
inject their specific fragmentation rules into the generic `FragmentAdapter`.

### 4.1 Trait Definition

The trait is designed to be context-aware and expressive, allowing it to model
a wide range of protocols.

```rust
use bytes::BytesMut;
use std::io;

/// Metadata decoded from a single fragment's header.
#
pub struct FragmentMeta {
    /// The size of the payload in this specific fragment.
    pub payload_len: usize,
    /// The total size of the fully re-assembled message, if known.
    /// This allows for efficient pre-allocation of the re-assembly buffer.
    pub total_message_len: Option<usize>,
    /// True if this is the final fragment of a logical message.
    pub is_final: bool,
    /// An identifier for the logical message this fragment belongs to.
    /// This is essential for multiplexing.
    pub msg_id: Option<u64>,
    /// The sequence number of this fragment within its logical message.
    pub seq: Option<u64>,
}

/// A trait that defines the fragmentation rules for a specific protocol.
pub trait FragmentStrategy: 'static + Send + Sync {
    /// The logical frame type this strategy operates on.
    type Frame: FrameLike;

    /// Determines the maximum payload size for fragments of a given logical frame.
    /// This allows for dynamic fragment sizes based on message type or content.
    fn max_fragment_payload(&self, frame: &Self::Frame) -> usize;

    /// Decodes a fragment header from the provided buffer.
    ///
    /// On success, returns `Ok(Some((meta, header_len)))` where `header_len` is the
    /// number of bytes consumed from the buffer for the header.
    ///
    /// Returns `Ok(None)` if the buffer does not yet contain a full header.
    /// Returns `Err` for a malformed header.
    fn decode_header(&self, src: &mut BytesMut) -> io::Result<Option<(FragmentMeta, usize)>>;

    /// Emits a header for an outbound fragment into the destination buffer.
    fn encode_header(
        &self,
        dst: &mut BytesMut,
        // The full logical frame being fragmented, for context.
        frame: &Self::Frame,
        // Metadata for the current fragment being encoded.
        payload_len: usize,
        is_final: bool,
        msg_id: u64,
        seq: u64,
    ) -> io::Result<()>;
}
```

### 4.2 Configuration

Developers will enable fragmentation by adding the `FragmentAdapter` to their
codec chain via the `WireframeApp` builder.

```rust
// Pseudo‑API: enable fragmentation with a strategy on the codec stack.
WireframeServer::new(|| {
    let mut app = WireframeApp::new();
    app
        .codec(app.length_codec())
        .codec(FragmentAdapter::new(MySqlStrategy::new()))
        .route(/* ... */)
})
```

Configure the same max frame length for all codec instances on inbound and
outbound paths.

```rust,no_run
use async_stream::try_stream;
use wireframe::{app::WireframeApp, Response};

async fn fragmented() -> Response<Vec<u8>> {
    // `FragmentAdapter` splits oversized payloads automatically.
    Response::Vec(vec![vec![0_u8; 128 * 1024]])
}

async fn streamed() -> Response<Vec<u8>> {
    Response::Stream(Box::pin(try_stream! {
        yield vec![1, 2, 3];
    }))
}
```

`async-stream` is the canonical crate for constructing dynamic
`Response::Stream` values.

## 5. Implementation Logic

### 5.1 Inbound path (re‑assembly)

The re-assembly logic is the most complex part of the feature and must be
robust against errors and attacks.

1. **Header Decoding:** The adapter reads from the socket buffer and calls
   `strategy.decode_header()`. If it returns `Ok(None)`, it waits for more data.

2. **Payload Extraction:** Once a header is decoded, the adapter ensures the
   full payload for that fragment is available in the buffer before proceeding.

3. **Multiplexed State Management:**

   - If `meta.msg_id` is `None`, the fragment is treated as a standalone message
     (if `is_final`) or an error (if not `is_final` and a non-multiplexed
     re-assembly is already in progress).

   - If `meta.msg_id` is `Some(id)`, the adapter accesses the
     `reassembly_buffers` map.

   - **New Message (**`.entry().or_insert_with(...)`**):**

     - If `meta.is_final` is true, this is a single-fragment message. It is
       passed down the chain immediately without being buffered.

     - If `meta.is_final` is false, a new `PartialMessage` is created. The
       buffer is pre-allocated if `meta.total_message_len` is `Some`, and
       `expected_total` stores this hint. The `started_at` timestamp is
       recorded.

   - **Continuing Message (**`.get_mut()`**):**

     - The `last_sequence` is checked to ensure fragments are monotonic. An
       out-of-order fragment results in an error and the `PartialMessage` being
       dropped.

     - The buffer's potential new size is checked against `max_message_size`.
       Exceeding this limit results in an error.

     - The new payload is appended to the buffer.

   - **Final Fragment:** If `meta.is_final` is true, the full payload is
     extracted from the `PartialMessage`, the entry is removed from the map,
     then pass the complete logical frame down the codec chain.

4. **Timeout handling:** Run a background task within the
   `FragmentAdapter` that periodically iterates over the re‑assembly buffers,
   checks each `PartialMessage`’s `started_at` timestamp, and removes any entry
   that has exceeded the re‑assembly timeout, emitting a `WARN`‑level `tracing`
   event.

### 5.2 Outbound path (fragmentation)

The outbound path is simpler and purely procedural.

1. **Size Check:** When `write(frame)` is called, the adapter checks
   `frame.len()` against `strategy.max_fragment_payload(&frame)`.

2. **No Fragmentation:** If the frame is small enough, it is passed directly to
   the next codec in the chain.

3. **Fragmentation:** If the frame is too large:

   - A new `msg_id` is generated via
     `next_outbound_msg_id.fetch_add(1, Ordering::Relaxed)`.

   - The adapter iterates through the frame's payload in chunks of
     `max_fragment_payload`.

   - For each chunk, it calls `strategy.encode_header()` to write the fragment
     header (with the correct `msg_id`, `seq`, and `is_final` flag) into a
     buffer, propagating any error, followed by the payload chunk.

   - Each fully formed fragment is then passed individually to the next
     codec.

## 6. Synergy with other 1.0 features

This feature is designed as a foundational layer that other features build upon.

- **Streaming Responses:** Handlers will use `Response::Stream` to yield
  *logical* frames. If a yielded frame is larger than the configured limit, the
  `FragmentAdapter` will automatically and transparently split it into multiple
  physical fragments before it reaches the socket.

- **Asynchronous Pushes:** Similarly, a call to `push_handle.push(large_frame)`
  sends a single logical frame. The `FragmentAdapter` ensures it is correctly
  fragmented before transmission. The application code remains blissfully
  unaware of the underlying network constraints.

## 7. Measurable objectives and success criteria

<!-- markdownlint-disable-next-line MD013 -->
| Category        | Objective                                                                                                                                   | Success Metric                                                                                                                                                                    |
| --------------- | ------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| API Correctness | The FragmentStrategy trait and FragmentAdapter are implemented exactly as specified in this document.                                       | 100% of the public API surface is present and correctly typed.                                                                                                                    |
| Functionality   | A large logical frame is correctly split into N fragments, and a sequence of N fragments is correctly re-assembled into the original frame. | An end-to-end test confirms byte-for-byte identity of a payload at the configured max_message_size after being fragmented and re-assembled.                                       |
| Multiplexing    | The adapter can correctly re-assemble two messages whose fragments are interleaved.                                                         | A test sending fragments A1, B1, A2, B2, A3, B3 must result in two correctly re-assembled messages, A and B.                                                                      |
| Resilience      | The adapter protects against memory exhaustion from oversized messages.                                                                     | A test sending fragments that exceed max_message_size must terminate the connection and not allocate beyond the configured cap (including allocator overhead).                    |
| Resilience      | The adapter protects against resource leaks from abandoned partial messages.                                                                | A test that sends an initial fragment but never the final one must result in the partial buffer being purged after the reassembly_timeout duration has passed.                    |
| Performance     | The overhead for messages that do not require fragmentation is minimal.                                                                     | A criterion benchmark passing a stream of small, non-fragmented frames through the FragmentAdapter must show < 5% throughput degradation compared to a build without the adapter. |

## 8. Design decisions (14 November 2025, updated 17 November 2025)

- Adopted `FragmentHeader`, `MessageId`, and `FragmentIndex` as the canonical,
  serialiser-agnostic metadata describing every fragment emitted or consumed by
  the transport layer. These types now live in `wireframe::fragment` so any
  protocol component can construct, inspect, or log fragment details without
  talking to codec internals.
- Added `FragmentSeries` to enforce ordering invariants per logical message,
  surfacing precise diagnostics (`MessageMismatch`, `IndexMismatch`,
  `SeriesComplete`, `IndexOverflow`). This helper keeps the re-assembly logic
  deterministic and enables behavioural tests to assert transport-level
  guarantees without standing up a full codec pipeline.
- Introduced `Fragmenter`, `FragmentBatch`, and `FragmentFrame` as reusable
  outbound helpers. They generate monotonic `MessageId` values, split large
  payloads into capped fragments, and return typed collections that are simple
  to wrap in protocol-specific envelopes or feed into behavioural tests before
  the full `FragmentAdapter` lands.
- Added a transport-agnostic `Reassembler` that mirrors the outbound helper on
  the inbound path. It buffers fragments per `MessageId` via `FragmentSeries`,
  drops the partial buffer when ordering breaks, enforces a configurable
  `max_message_size`, and exposes caller-driven timeout purging. This prevents
  abandoned assemblies from exhausting memory.
