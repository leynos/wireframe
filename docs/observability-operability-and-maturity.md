# `wireframe` 1.0: A Guide to Observability, Operability, and Maturity

## 1. Introduction: Beyond Functional Correctness

A library is functionally correct when it performs its specified tasks according
to its API. A library is *mature* when it anticipates the realities of its
operational environment. For a low-level networking framework like `wireframe`,
this means acknowledging that production systems are complex, failures are
inevitable, and visibility into runtime behaviour is non-negotiable.

This guide details the cross-cutting strategies that will elevate `wireframe`
from a simple toolkit to a production-grade, operationally mature framework. It
is a practical blueprint for implementers, covering three essential domains:

1. **Pervasive Observability:** Making the library's internal state transparent
   and debuggable by default.

2. **Operability by Design:** Building features that make the library easy to
   run, manage, and recover gracefully.

3. **API and Design Maturity:** Adopting patterns that signal a commitment to
   developer ergonomics, extensibility, and long-term maintainability.

By embedding these principles into the core of the library, we provide users
with the tools they need to build, deploy, and maintain robust network services
with confidence.

## 2. Pervasive Observability with `tracing`

In a complex asynchronous application, `println!` is not debugging; it is
whispering into a hurricane. To understand runtime behaviour, we must have
structured, context-aware diagnostics. The `tracing` crate is the de facto
standard for this in the async Rust ecosystem, and its integration is a
first-class feature of `wireframe` 1.0.

### 2.1 The `tracing` Span Hierarchy

`tracing`'s key innovation is the `span`, which represents a unit of work with a
distinct beginning and end. By nesting spans, we can create a causal chain of
events, allowing us to understand how work flows through the system, even across
asynchronous boundaries and threads. `wireframe` will adopt a standard span
hierarchy.

- `connection`**:** A root span for each TCP connection, created on `accept()`.
  It provides the top-level context for all activity on that connection.

- `request`**:** A child of `connection`, created when an inbound frame is
  decoded and routed to a handler.

- `response_stream`**:** A child of `request`, created when a handler returns a
  multi-frame response.

**Implementation Example:** Instrumenting the connection actor.

```rust
use tracing::{info, info_span, Instrument};
use std::net::SocketAddr;

async fn run_connection_actor(
    connection_id: u64,
    peer_addr: SocketAddr,
    // ... other arguments
) {
    let conn_span = info_span!(
        "connection",
        id = connection_id,
        peer = %peer_addr,
    );

    // The .instrument() call attaches the span to the future.
    // All events inside this block will be associated with the connection.
    async move {
        info!("New connection established.");
        // ... actor logic ...
        info!("Connection closed.");
    }.instrument(conn_span).await;
}

```

**Measurable Objective:** Every spawned task and connection actor in the library
must be instrumented with a `tracing` span containing relevant, queryable
identifiers (e.g., `connection_id`, `peer_addr`).

### 2.2 Structured Lifecycle Events

Within each span, we will emit structured events at critical points in the
connection lifecycle. Using structured key-value pairs (`field = value`) instead
of simple strings allows logs to be automatically parsed, queried, and
aggregated by modern logging platforms.

**Implementation Example:** Logging key events.

```rust
// Inside the connection actor's read loop
let frame = read_frame().await?;
tracing::debug!(
    frame_type = %frame.type_name(),
    frame_len = frame.len(),
    "Frame received from client"
);

// Inside the PushHandle's drop-if-full logic
tracing::warn!(
    frame_type = %frame.type_name(),
    "Push queue full, dropping frame."
);

// When a response stream is fully processed
tracing::debug!(
    reason = "clean_completion",
    "Response stream closed."
);

```

### 2.3 From Logs to Metrics

The true power of structured `tracing` data is that it can be consumed by a
`Subscriber` that aggregates events into quantitative metrics. This enables the
creation of vital operational dashboards for monitoring the health and
performance of `wireframe`-based services.

The library will document and support the generation of the following key
metrics, suitable for export to systems like Prometheus or OpenTelemetry.

| Metric Name                       | Type      | Description                                                                                             |
| --------------------------------- | --------- | ------------------------------------------------------------------------------------------------------- |
| wireframe_connections_active      | Gauge     | The current number of active connections.                                                               |
| wireframe_frames_processed_total  | Counter   | Total frames processed, with labels for direction (inbound/outbound) and protocol_type.                 |
| wireframe_push_queue_depth        | Histogram | A distribution of the push queue depth, sampled periodically. Essential for diagnosing back-pressure.   |
| wireframe_pushes_dropped_total    | Counter   | The number of frames dropped due to a full push queue. Should be zero in a healthy system.              |
| wireframe_request_latency_seconds | Histogram | A distribution of handler processing times, from frame receipt to response completion.                  |
| wireframe_reassembly_errors_total | Counter   | The number of times message re-assembly failed due to timeouts, oversized messages, or sequence errors. |

**Measurable Objective:** A `wireframe` application, when configured with a
suitable `tracing-subscriber`, must emit all the metrics listed above, allowing
for the construction of a comprehensive operational health dashboard.

## 3. Operability by Design

Operability is about building systems that are predictable and controllable.
`wireframe` will incorporate several key features designed to make it a
well-behaved citizen in a production environment.

### 3.1 Coordinated, Graceful Shutdown

A network service that cannot be stopped cleanly is a liability. `wireframe`
will implement a canonical, proactive shutdown pattern to ensure orderly
termination.

**Implementation:** The core mechanism relies on
`tokio_util::sync::CancellationToken` for signalling and
`tokio_util::task::TaskTracker` for synchronisation.

```rust
// Inside the main server accept loop
let tracker = TaskTracker::new();
let shutdown_token = CancellationToken::new();

// In the signal handler for SIGINT/SIGTERM
shutdown_token.cancel();
tracker.close();
tracker.wait().await; // Wait for all tasks to finish

// When spawning a connection
let token_clone = shutdown_token.clone();
tracker.spawn(run_connection_actor(..., token_clone));

// Inside the connection actor's select! loop
tokio::select! {
    biased;
    _ = shutdown_token.cancelled() => {
        tracing::info!("Shutdown signal received, closing connection gracefully.");
        // Perform cleanup (e.g., send final protocol message)
        break;
    },
    // ... other branches ...
}

```

**Measurable Objective:** When a server shutdown is initiated via the
cancellation token, 100% of active connection tasks must terminate cleanly
within a configurable timeout (e.g., 5 seconds), and the main server process
must exit with a success code.

### 3.2 Intelligent Error Handling with Typed Errors

Conflating unrecoverable transport errors with recoverable protocol errors makes
robust error handling impossible. `wireframe` will provide a generic error enum
to give developers the necessary information to react intelligently.

**Implementation:**

```rust
pub enum WireframeError<E> {
    /// A fatal I/O error occurred (e.g., socket closed).
    /// The connection should be terminated.
    Io(std::io::Error),

    /// A protocol-specific logical error (e.g., invalid query).
    /// The framework may be able to format a reply before
    /// closing the logical stream.
    Protocol(E),
}

```

When a handler's response stream yields a `WireframeError::Protocol(e)`, the
connection actor can pass this typed error to a protocol-specific callback. This
allows the implementation to serialize a proper error frame (e.g., an SQL error
code) and send it to the client before terminating the current operation, rather
than just abruptly closing the connection.

### 3.3 Resilient Messaging with Dead Letter Queues (DLQ)

For applications where dropping a pushed message is not an option (e.g.,
critical audit logs), `wireframe` will support an optional Dead Letter Queue.

**Implementation:** The `WireframeApp` builder will accept an `mpsc::Sender` for
a DLQ. If a push fails because the primary queue is full, the frame is routed to
this sender instead of being dropped.

```rust
// Inside PushHandle, when try_send fails with a full queue
match self.tx.try_send(frame) {
    Ok(_) => Ok(()),
    Err(mpsc::error::TrySendError::Full(failed_frame)) => {
        if let Some(dlq_tx) = &self.dlq_tx {
            if dlq_tx.try_send(failed_frame).is_err() {
                tracing::error!("Primary push queue and DLQ are both full. Frame lost.");
            }
        } else {
            tracing::warn!("Push queue full, dropping frame.");
        }
        Ok(())
    },
    // ... handle closed channel error ...
}

```

A separate consumer task, managed by the application developer, is then
responsible for processing messages from the DLQ's receiver.

**Measurable Objective:** When the DLQ is enabled, the rate of permanently lost
pushed frames must be zero, provided the DLQ consumer can keep up. The
`wireframe_pushes_dropped_total` metric must remain at zero.

## 4. Achieving Technical Maturity

Technical maturity is reflected in an API that is not just powerful, but also
ergonomic, extensible, and idiomatic.

### 4.1 An Ergonomic, Trait-Based API

To avoid a sprawling collection of configuration closures, protocol-specific
logic will be encapsulated within a single, cohesive `WireframeProtocol` trait.
This promotes better organisation, reusability, and makes the framework easier
to extend.

**Implementation:**

```rust
pub trait WireframeProtocol: Send + Sync + 'static {
    // Associated types for frames and errors
    type Frame: FrameLike;
    type ProtocolError;

    /// Called to handle a request frame.
    async fn handle_request(
        &self,
        req: Request<Self::Frame>,
        ctx: &ConnectionContext,
    ) -> Result<Response<Self::Frame, Self::ProtocolError>, WireframeError<Self::ProtocolError>>;

    /// Called just before a frame is written to the socket.
    fn before_send(&self, frame: &mut Self::Frame, ctx: &mut ConnectionContext);

    // ... other callbacks like on_connection_setup, on_command_end ...
}

// Configuration becomes clean and declarative.
WireframeApp::new().with_protocol(MySqlProtocolImpl);

```

This design is more maintainable and aligns with Rust's powerful trait-based
abstraction model.

### 4.2 A Commitment to Quality Assurance

A mature library demonstrates its commitment to quality through a rigorous and
multi-faceted testing strategy that goes beyond simple unit tests. `wireframe`'s
QA process will be a core part of its development.

- **Stateful Property Testing (**`proptest`**):** For verifying complex,
  stateful protocol conversations, ensuring that thousands of random-but-valid
  sequences of operations do not lead to an invalid state.

- **Concurrency Verification (**`loom`**):** For proving the absence of data
  races and deadlocks in concurrency hotspots, `loom` will be used to perform
  permutation testing, providing a level of assurance that traditional tests
  cannot.

- **Performance Benchmarking (**`criterion`**):** To quantify the performance
  impact of new features and prevent regressions, a comprehensive suite of micro
  and macro benchmarks will be maintained.

For more detailed information and comprehensive, worked examples of these
testing methodologies, please refer to the companion document, *Wireframe: A
Multi-Layered Testing Strategy*.

By integrating these strategies, `wireframe` will not only be rich in features
but will stand as a benchmark for robust, observable, and operationally mature
systems engineering in the Rust ecosystem.
