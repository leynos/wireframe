# Comprehensive Design: Generic Message Fragmentation & Re-assembly

## 1. Introduction & Philosophy

Many robust network protocols, from modern RPC systems to legacy financial
standards, require the ability to split a single logical message into multiple
physical frames. This can be necessary to respect MTU limits, handle large data
payloads, or align with encryption block sizes. `wireframe`'s current model,
which processes one inbound frame to one logical frame, cannot handle this.

This document details the design for a generic, protocol-agnostic fragmentation
and re-assembly layer. The core philosophy is to treat this as a **transparent
middleware**. Application-level code, such as handlers, should remain unaware of
the underlying fragmentation, dealing only with complete, logical messages. This
new layer will be responsible for automatically splitting oversized outbound
frames and meticulously re-assembling inbound fragments into a single, coherent
message before they reach the router.

This feature is a critical component of the "Road to Wireframe 1.0," designed to
seamlessly integrate with and underpin the streaming and server-push
capabilities.

## 2. Design Goals & Requirements

The implementation must satisfy the following core requirements:

<!-- markdownlint-disable MD013 -->

| ID | Goal                                                                                                                                                                                                                   |
| -- | ------------------ |
| G1 | Transparent inbound re-assembly → The router and handlers must always receive one complete, logical Frame.                                                                                                             |
| G2 | Transparent outbound fragmentation when a payload exceeds a configurable, protocol-specific size.                                                                                                                      |
| G3 | Pluggable Strategy: The logic for parsing and building fragment headers, detecting the final fragment, and managing sequence numbers must be supplied by the protocol implementation, not hard-coded in the framework. |
| G4 | DoS Protection: The re-assembly process must be hardened against resource exhaustion attacks via strict memory and time limits.                                                                                        |
| G5 | Zero Friction: Protocols that do not use fragmentation must incur no performance or complexity overhead. This feature must be strictly opt-in.                                                                         |

<!-- markdownlint-enable MD013 -->

## 3. Core Architecture: The `FragmentAdapter`

The feature will be implemented as a `FrameProcessor` middleware called
`FragmentAdapter`. It is instantiated with a protocol-specific
`FragmentStrategy` and wraps any subsequent processors in the chain.

```
Socket I/O ↔ ↔ [Compression] ↔ FragmentAdapter ↔ Router/Handlers
```

This layered approach ensures that fragmentation is handled on clear-text,
uncompressed data, as required by most protocol specifications.

### 3.1 State Management for Multiplexing

A critical requirement for modern protocols is the ability to handle interleaved
fragments from different logical messages on the same connection. To support
this, the `FragmentAdapter` will not maintain a single re-assembly state, but a
map of concurrent re-assembly processes.

Rust

```
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
    /// The sequence number of the last fragment received.
    last_sequence: u64,
    /// The time the first fragment was received.
    started_at: Instant,
}
```

The use of `dashmap::DashMap` allows for lock-free reads and sharded writes,
providing efficient and concurrent access to the re-assembly buffers without
blocking the entire connection task.

## 4. Public API: The `FragmentStrategy` Trait

The power and flexibility of this feature come from the `FragmentStrategy`
trait. Protocol implementers will provide a type that implements this trait to
inject their specific fragmentation rules into the generic `FragmentAdapter`.

### 4.1 Trait Definition

The trait is designed to be context-aware and expressive, allowing it to model a
wide range of protocols.

Rust

```
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
    );
}
```

### 4.2 Configuration

Developers will enable fragmentation by adding the `FragmentAdapter` to their
`FrameProcessor` chain via the `WireframeApp` builder.

Rust

```
// Example: Configuring a server for MySQL-style fragmentation.
WireframeServer::new(|| {
    WireframeApp::new()
       .frame_processor(
            FragmentAdapter::new(MySqlStrategy)
               .with_max_message_size(64 * 1024 * 1024) // 64 MiB
               .with_reassembly_timeout(Duration::from_secs(30))
        )
       .route(...)
})
```

## 5. Implementation Logic

### 5.1 Inbound Path (Re-assembly)

The re-assembly logic is the most complex part of the feature and must be robust
against errors and attacks.

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
       buffer is pre-allocated if `meta.total_message_len` is `Some`. The
       `started_at` timestamp is recorded.

   - **Continuing Message (**`.get_mut()`**):**

     - The `last_sequence` is checked to ensure fragments are monotonic. An
       out-of-order fragment results in an error and the `PartialMessage` being
       dropped.

     - The buffer's potential new size is checked against `max_message_size`.
       Exceeding this limit results in an error.

     - The new payload is appended to the buffer.

   - **Final Fragment:** If `meta.is_final` is true, the full payload is
     extracted from the `PartialMessage`, the entry is removed from the map, and
     the complete logical frame is passed down the processor chain.

4. **Timeout Handling:** A separate, low-priority background task within the
   `FragmentAdapter` will periodically iterate over the `reassembly_buffers`,
   checking the `started_at` timestamp of each `PartialMessage`. Any entry that
   has exceeded `reassembly_timeout` is removed, and a `WARN`-level `tracing`
   event is emitted.

### 5.2 Outbound Path (Fragmentation)

The outbound path is simpler and purely procedural.

1. **Size Check:** When `write(frame)` is called, the adapter checks
   `frame.len()` against `strategy.max_fragment_payload(&frame)`.

2. **No Fragmentation:** If the frame is small enough, it is passed directly to
   the next `FrameProcessor` in the chain.

3. **Fragmentation:** If the frame is too large:

   - A new `msg_id` is generated via
     `next_outbound_msg_id.fetch_add(1, Ordering::Relaxed)`.

   - The adapter iterates through the frame's payload in chunks of
     `max_fragment_payload`.

   - For each chunk, it calls `strategy.encode_header()` to write the fragment
     header (with the correct `msg_id`, `seq`, and `is_final` flag) into a
     temporary buffer, followed by the payload chunk.

   - Each fully formed fragment is then passed individually to the next
     `FrameProcessor`.

## 6. Synergy with Other 1.0 Features

This feature is designed as a foundational layer that other features build upon.

- **Streaming Responses:** Handlers will use `Response::Stream` to yield
  *logical* frames. If a yielded frame is larger than the configured limit, the
  `FragmentAdapter` will automatically and transparently split it into multiple
  physical fragments before it reaches the socket.

- **Asynchronous Pushes:** Similarly, a call to `push_handle.push(large_frame)`
  sends a single logical frame. The `FragmentAdapter` ensures it is correctly
  fragmented before transmission. The application code remains blissfully
  unaware of the underlying network constraints.

## 7. Measurable Objectives & Success Criteria

<!-- markdownlint-disable MD013 -->

| Category        | Objective                                                                                                                                   | Success Metric                                                                                                                                                                    |
| --------------- | --------- | -------------------- |
| API Correctness | The FragmentStrategy trait and FragmentAdapter are implemented exactly as specified in this document.                                       | 100% of the public API surface is present and correctly typed.                                                                                                                    |
| Functionality   | A large logical frame is correctly split into N fragments, and a sequence of N fragments is correctly re-assembled into the original frame. | An end-to-end test confirms byte-for-byte identity of a 64 MiB payload after being fragmented and re-assembled.                                                                   |
| Multiplexing    | The adapter can correctly re-assemble two messages whose fragments are interleaved.                                                         | A test sending fragments A1, B1, A2, B2, A3, B3 must result in two correctly re-assembled messages, A and B.                                                                      |
| Resilience      | The adapter protects against memory exhaustion from oversized messages.                                                                     | A test sending fragments for a 2 GiB message against a 1 GiB max_message_size limit must terminate the connection and not allocate more than ~1 GiB of buffer memory.             |
| Resilience      | The adapter protects against resource leaks from abandoned partial messages.                                                                | A test that sends an initial fragment but never the final one must result in the partial buffer being purged after the reassembly_timeout duration has passed.                    |
| Performance     | The overhead for messages that do not require fragmentation is minimal.                                                                     | A criterion benchmark passing a stream of small, non-fragmented frames through the FragmentAdapter must show < 5% throughput degradation compared to a build without the adapter. |

<!-- markdownlint-enable MD013 -->
