# Wireframe: A Multi-Layered Testing Strategy

## 1. Introduction

This document outlines the comprehensive testing strategy for the `wireframe` library, synthesising the test plans from the designs for server-initiated messages, streaming responses, and message fragmentation. A robust, multi-layered approach to testing is non-negotiable for a low-level library like `wireframe`, where correctness, resilience, and performance are paramount.

The strategy is divided into four distinct layers, each building upon the last. This version has been enhanced with **concrete code examples** and **measurable objectives** to provide a clear, actionable guide for implementers and reviewers. This approach ensures that we establish a baseline of correctness with simple tests before moving on to the more complex and subtle failure modes that can emerge in an asynchronous, high-concurrency system.

## 2. Layer 1: Foundational Correctness (Unit & Integration)

**Objective:** To verify that each component behaves correctly in isolation and that its immediate interactions with other components are sound. These tests form the bedrock of our confidence in the library.

### 2.1 Push/Streaming API: Streaming Response Order

This test verifies that when a handler returns a `Response::Stream`, all frames from that stream are written to the outbound buffer in the correct sequence.

**Test Construction:** A mock connection actor is spawned, and a test handler is invoked that returns a stream of 10 distinct, identifiable frames. The actor's outbound write buffer is captured after the stream is fully processed.

```rust
#[tokio::test]
async fn test_streaming_response_order() {
    let (mut actor, _ctx) = MockConnection::new();

    let handler = |_req| async {
        let frames = (0..10).map(MyFrame::new);
        let stream = futures::stream::iter(frames.map(Ok));
        Ok(Response::Stream(Box::pin(stream)))
    };

    let response = handler(Request::new()).await.unwrap();
    actor.drive_response(response).await;

    let written_frames = actor.outbound_buffer();
    // Assert `written_frames` contains 10 frames in order from 0 to 9.
}

```

**Expected Outcome & Measurable Objective:** The write buffer must contain all 10 frames, correctly serialised, in the exact order they were sent. The `on_logical_response_end` hook must be called exactly once after the final frame is flushed. The objective is **100% frame delivery with strict ordering confirmed.**

### 2.2 Push Handle: High-Volume Throughput

This test ensures the `PushHandle` can sustain a high volume of messages without deadlocking or losing frames.

**Test Construction:** A fake connection actor is spawned with a large buffer. Its `PushHandle` is cloned and moved into a `tokio::spawn` block that rapidly pushes 10,000 frames in a loop. The test waits for both the producer and the actor to complete.

```rust
#[tokio::test]
async fn test_push_handle_volume() {
    let (mut actor, mut ctx) = MockConnection::new();
    let handle = ctx.push_handle();

    let producer = tokio::spawn(async move {
        for i in 0..10_000 {
            handle.push(MyFrame::new(i)).await.unwrap();
        }
    });

    let _ = tokio::join!(producer, actor.run());
    // Assert actor's internal counter confirms it received 10,000 frames
}

```

**Expected Outcome & Measurable Objective:** All 10,000 frames must be received by the connection actor's write task in the correct order. The objective is **zero frame loss under high volume, with test completion in &lt; 500ms on a standard CI runner.**

### 2.3 Fragment Re-assembly: Byte-for-Byte Accuracy

This test validates that the `FragmentAdapter` can correctly re-assemble a sequence of fragments into the original logical message.

**Test Construction:** For a given `FragmentStrategy`, a known payload is split into valid fragments. The `adapter.decode()` method is called repeatedly with each fragment, accumulating the partial state.

```rust
#[test]
fn test_fragment_reassembly() {
    let strategy = LenSeq16M;
    let adapter = FragmentAdapter::new(strategy);
    let payload = Bytes::from_static(b"hello world");

    let fragments = strategy.split_into_fragments(&payload);
    let mut partial_state = None;

    for frag_bytes in fragments {
        partial_state = adapter.decode(frag_bytes, partial_state).unwrap();
    }

    // Assert that the final state is a completed frame matching the payload
    assert!(matches!(partial_state, Some(Frame(..))));
}

```

**Expected Outcome & Measurable Objective:** The adapter must return `Ok(None)` for partial fragments and `Ok(Some(Frame))` for the final fragment. The re-assembled `Frame`'s payload must be byte-for-byte identical to the original payload. The objective is **100% byte-for-byte reconstruction accuracy.**

### 2.4 Fragment Splitting: Correctness of Generated Fragments

This test ensures that when a large frame is written, the `FragmentAdapter` splits it into the correct number of fragments with valid headers.

**Test Construction:** An adapter is instantiated with a small `max_fragment_payload` (e.g., 1 KiB). A 5 KiB frame is passed to its `write()` method, and the resulting output bytes are captured for inspection.

```rust
#[test]
fn test_fragment_splitting() {
    let strategy = LenSeq16M { max_payload: 1024 };
    let adapter = FragmentAdapter::new(strategy);
    let large_frame = MyFrame::new_of_size(5 * 1024);
    let mut buffer = BytesMut::new();

    adapter.write(large_frame, &mut buffer);

    // Assert buffer contains 5 distinct fragments with correct
    // lengths, sequence IDs, and final-frame flags.
}

```

**Expected Outcome & Measurable Objective:** The output buffer must contain exactly five fragments. Each fragment's header must be correct according to the strategy's rules. The objective is **correct fragment count and valid headers for all generated fragments.**

## 3. Layer 2: Resilience (Back-pressure & Error Paths)

**Objective:** To verify that the system behaves gracefully under resource contention (slow clients, full buffers) and when encountering expected errors (I/O failures, protocol violations).

### 3.1 Push/Streaming Back-pressure: Concurrent Push with Slow Consumer

This test confirms that back-pressure is correctly applied when the outbound queue is full.

**Test Construction:** A mock connection is created with an `outbound_queue_capacity` of 1 and a socket that stalls for 100ms on write. Two tasks concurrently attempt to `push()` frames.

```rust
#[tokio::test(flavor = "multi_thread")]
async fn test_back_pressure() {
    let (mut actor, mut ctx) = MockConnection::new_with_slow_socket(1, 100);
    let h1 = ctx.push_handle();
    let h2 = h1.clone();

    // This first push should succeed immediately.
    h1.push(MyFrame::new(1)).await.unwrap();

    // This second push should block until the slow socket unstalls.
    let start = Instant::now();
    let res = tokio::time::timeout(
        Duration::from_millis(150),
        h2.push(MyFrame::new(2))
    ).await;

    assert!(res.is_ok());
    assert!(start.elapsed().as_millis() >= 100);
}

```

**Expected Outcome & Measurable Objective:** The second `push()` call must be suspended until the first frame is drained from the queue. The objective is that **the second push call must take at least as long as the mock socket's stall time.**

### 3.2 Socket Write Failure: Error Propagation to Handles

This test validates that when a socket write fails, the error is correctly propagated to all active `PushHandle`s.

**Test Construction:** A mock `AsyncWrite` that returns `Err(io::ErrorKind::BrokenPipe)` is used. The first `push()` call triggers the error, terminating the actor. A subsequent `push()` call must fail immediately.

```rust
#[tokio::test]
async fn test_socket_write_failure() {
    let (mut actor, mut ctx) = MockConnection::new_with_failing_socket();
    let handle = ctx.push_handle();

    // This first push will trigger the write error and terminate the actor.
    let _ = handle.push(MyFrame::new(1)).await;

    // Give the actor time to terminate.
    tokio::task::yield_now().await;

    // This second push must fail immediately with the correct error.
    let err = handle.push(MyFrame::new(2)).await.unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::BrokenPipe);
}

```

**Expected Outcome & Measurable Objective:** The connection actor must terminate cleanly. Any subsequent calls on associated `PushHandle`s must immediately return a `BrokenPipe` error. The objective is that **failure propagation must occur within one scheduler tick.**

### 3.3 Graceful Shutdown: Co-ordinated Task Termination

This test ensures that a server-wide shutdown signal leads to the clean termination of all active connection tasks.

**Test Construction:** `tokio_util::sync::CancellationToken` is used to signal shutdown to multiple spawned connection tasks. A `tokio_util::task::TaskTracker` waits for all tasks to complete.

```rust
#[tokio::test]
async fn test_graceful_shutdown() {
    let tracker = TaskTracker::new();
    let token = CancellationToken::new();

    for _ in 0..10 {
        let conn_token = token.clone();
        tracker.spawn(async move {
            MyConnection::run(conn_token).await;
        });
    }

    token.cancel();
    tracker.close();

    // This will only complete if all tasks terminate correctly.
    tracker.wait().await;
}

```

**Expected Outcome & Measurable Objective:** All active connection tasks must terminate. The main server task must exit cleanly. The objective is that `tracker.wait()` **must complete within a reasonable timeout (e.g., 1 second).**

### 3.4 Fragmentation Limit: DoS Protection

This test confirms that the `FragmentAdapter` protects against memory exhaustion by enforcing the `max_message_size`.

**Test Construction:** An adapter is configured with `max_message_size(1024)`. Fragments are decoded that would total more than this limit.

```rust
#[test]
fn test_fragmentation_limit() {
    let adapter = FragmentAdapter::new(strategy)
        .with_max_message_size(1024);

    // Send two 600-byte fragments, which will exceed the 1024-byte limit.
    let res1 = adapter.decode(frag1_600_bytes, None);
    let res2 = adapter.decode(frag2_600_bytes, res1.unwrap());

    assert!(matches!(res2, Err(e) if e.kind() == io::ErrorKind::InvalidData));
}

```

**Expected Outcome & Measurable Objective:** The `decode()` method must return an `InvalidData` error. The objective is that **the error must be returned on the exact fragment that crosses the threshold, not later.**

### 3.5 Fragmentation Sequence Error: Protocol Correctness

This test ensures the adapter correctly handles out-of-order fragments for protocols that require sequencing.

**Test Construction:** A strategy that enforces sequencing (e.g., `LenSeq16M`) is used. Fragments with a gap in the sequence ID are fed to the decoder.

```rust
#[test]
fn test_fragmentation_sequence_error() {
    // Create fragment for seq 0, then seq 2, skipping 1.
    let res1 = adapter.decode(frag_seq_0, None);
    let res2 = adapter.decode(frag_seq_2, res1.unwrap());

    // Assert that an error is returned due to the sequence gap.
    assert!(matches!(res2, Err(e) if e.kind() == io::ErrorKind::InvalidData));
}

```

**Expected Outcome & Measurable Objective:** The `decode()` method must return an `InvalidData` error upon detecting the sequence violation. The objective is that **the specific protocol error ("sequence gap", "duplicate sequence") should be identifiable from the error message.**

## 4. Layer 3: Advanced Correctness (Logic & Concurrency)

**Objective:** To uncover subtle, hard-to-find bugs related to state, concurrency, and complex interactions that simple unit tests might miss.

### 4.1 Stateful Logic Verification with `proptest`

This test uses property-based testing to validate the complex state machine of the `FragmentAdapter`.

- **Tooling:** `proptest`

- **Target Area:** `FragmentAdapter`

**Test Construction:** A `proptest` strategy generates a sequence of "actions" (e.g., `SendFragment(bytes)`, `SendCompleteFrame(bytes)`). The test runner applies these actions to both the real `FragmentAdapter` and a simple, validated "model" of its state. The test asserts that the model and the real adapter's state always agree.

```rust
proptest! {
    #[test]
    fn test_fragment_adapter_state(
        actions in prop::collection::vec(any::<Action>(), 1..50)
    ) {
        let mut model = Model::new();
        let mut real = FragmentAdapter::new(...);

        for action in actions {
            model.apply(action);
            real.apply(action);
            prop_assert_eq!(model.state, real.state);
        }
    }
}

```

**Measurable Objective:** The test suite must pass **1,000,000 generated test cases** without failure.

### 4.2 Concurrency Fuzzing with `loom`

This test uses permutation testing to exhaustively explore all possible concurrent interleavings of the core write loop, ensuring it is free of data races and deadlocks.

- **Tooling:** `loom`

- **Target Area:** Connection Actor Write Loop (`select!(biased; ...)` logic)

**Test Construction:** The core `select!` loop and `PushHandle` `send()` calls are wrapped in a `loom::model`. Multiple `loom::thread`s concurrently push high-priority, low-priority, and handler-response frames. `loom` explores all possible execution orders.

```rust
#[test]
fn test_write_loop_concurrency() {
    loom::model(|| {
        let conn = loom::sync::Arc::new(MockConnection::new());
        let c1 = conn.clone();
        let t1 = loom::thread::spawn(move || {
            c1.push_high_prio().unwrap();
        });

        let c2 = conn.clone();
        let t2 = loom::thread::spawn(move || {
            c2.push_low_prio().unwrap();
        });

        t1.join().unwrap();
        t2.join().unwrap();
        conn.assert_state_is_consistent();
    });
}

```

**Measurable Objective:** The test must explore **all permutations for 2-3 concurrent producers** without finding data races or deadlocks.

### 4.3 Interaction Fuzzing with `proptest`

This test validates the priority logic of the write loop under a random mix of inputs.

- **Tooling:** `proptest`

- **Target Area:** Combined features (Push, Streaming)

**Test Construction:** A property test generates a random mix of high-priority pushes, low-priority pushes, and multi-frame `Response::Stream`s for a single connection. The test asserts that the final output stream respects the strict priority order (`shutdown > high > low > stream`) and that no frames are ever lost or reordered within their own channel.

**Measurable Objective:** The test suite must pass **1,000,000 generated test cases**, verifying frame ordering and completeness on every run.

## 5. Layer 4: Performance & Benchmarking

**Objective:** To quantify the performance characteristics of the library, prevent regressions, and validate that the overhead of new features is acceptable.

### 5.1 Micro-benchmark: `PushHandle` Contention

This benchmark measures the overhead of the underlying `mpsc` channel's lock under contention.

- **Tooling:** `criterion`

- **Target Area:** `PushHandle`

**Test Construction:** Benchmark the time taken for N threads to push one message each through the same `PushHandle`.

**Measurable Objective:** Performance with **4 producer threads should be no more than 15% slower** than with 1 producer thread, indicating low contention.

### 5.2 Micro-benchmark: `FragmentAdapter` Throughput

This benchmark measures the raw byte-shuffling performance of the fragmentation and re-assembly logic.

- **Tooling:** `criterion`

- **Target Area:** `FragmentAdapter`

**Test Construction:** Benchmark the time taken to split a large (e.g., 64 MiB) frame into fragments and, separately, to re-assemble those fragments.

**Measurable Objective:** Throughput must **exceed 5 GiB/s** on a standard CI runner.

### 5.3 Macro-benchmark: End-to-End Throughput & Latency

This benchmark measures the performance of the entire system under a realistic workload.

- **Tooling:** `criterion`, custom client

- **Target Area:** Full server/client loop

**Test Construction:** A full `wireframe` server/client pair is set up over a `tokio::io::duplex` stream. The test measures:

1. **Throughput:** The number of messages per second in a simple request-response workload.

2. **Latency:** The round-trip time for a single message.

3. **Push Latency:** The time from `push_handle.push()` to the client receiving the frame.

**Measurable Objective:** For small frames on [localhost](http://localhost): **Throughput &gt; 1M req/sec, P99 Latency &lt; 50µs, Push latency &lt; 20µs.**