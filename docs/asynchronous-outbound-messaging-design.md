# Comprehensive design: asynchronous outbound messaging

## 1. Introduction & philosophy

The initial versions of `wireframe` established a robust, strictly
request-response communication model. While effective for many RPC-style
protocols, this model is insufficient for the rich, bidirectional conversations
common in modern network services like database clients, message brokers, and
real-time applications.

This document details the design for a first-class, asynchronous outbound
messaging feature. The core philosophy is to evolve `wireframe` from a simple
request-response router into a fully **asynchronous, duplex message bus**. This
will be achieved by providing a generic, protocol-agnostic facility that allows
any part of an application—a request handler, a background timer, a separate
worker task—to push frames to a live connection at any time.

Earlier releases spawned a short-lived worker per request. This approach made
persistent state awkward and required extra synchronization when multiple tasks
needed to write to the same socket. The new design promotes each connection to
a **stateful actor** that owns its context for the lifetime of the session.
Actor state keeps sequencing rules and push queues local to one task,
drastically simplifying concurrency while enabling unsolicited frames.

This feature is a cornerstone of the "Road to Wireframe 1.0" and is designed to
be synergistic with the planned streaming and fragmentation capabilities,
creating a cohesive and powerful framework for a wide class of network
protocols.

### Implementation status

An initial connection actor with its biased write loop is implemented in
`src/connection.rs`. The remaining sections explain the rationale behind this
design and possible refinements. See
[Section 3](#3-core-architecture-the-connection-actor) for details.

## 2. Design goals & requirements

The implementation must satisfy the following core requirements:

| ID | Requirement                                                                                                                                            |
| --- | ------------------------------------------------------------------------------------------------------------------------------------------------------ |
| G1 | Any async task must be able to push frames to a live connection.                                                                                       |
| G2 | Ordering-safety: Pushed frames must interleave correctly with normal request/response traffic and respect any per-message sequencing rules.            |
| G3 | Back-pressure: Writers must block (or fail fast) when the peer cannot drain the socket, preventing unbounded memory consumption.                       |
| G4 | Generic—independent of any particular protocol; usable by both servers and clients built on wireframe.                                                 |
| G5 | Preserve the simple “return a reply” path for code that does not need pushes, ensuring backward compatibility and low friction for existing users.     |

## 3. Core architecture: the connection actor

The foundation of this design is the **actor-per-connection** model, where each
network connection is managed by a dedicated, isolated asynchronous task. This
approach serializes all I/O for a given connection, eliminating the need for
complex locking and simplifying reasoning about concurrency.

In previous iterations, a connection's logic lived in short-lived worker tasks
spawned per request. Converting those workers into long-running actors allows
`wireframe` to maintain per-connection state—such as sequence counters, command
metadata, and pending pushes—without cross-task sharing. Handlers now send
commands back to the actor instead of writing directly to the socket,
centralizing all output in one place.

### 3.1 Prioritized message queues

To handle different classes of outbound messages, each connection actor will
manage two distinct, bounded `tokio::mpsc` channels for pushed frames:

1. `high_priority_push_rx: mpsc::Receiver<F>`: For urgent, time-sensitive
   messages like heartbeats, session control notifications, or protocol-level
   pings.

2. `low_priority_push_rx: mpsc::Receiver<F>`: For standard, non-urgent
   background messages like log forwarding or secondary status updates.

The bounded nature of these channels provides an inherent and robust
back-pressure mechanism. When a channel's buffer is full, any task attempting
to push a new message will be asynchronously suspended until space becomes
available.

### 3.2 The prioritized write loop

The connection actor's write logic will be implemented within a
`tokio::select!` loop. Crucially, this loop will use the `biased` keyword to
ensure a strict, deterministic polling order. This prevents high-volume yet
critical control messages from being starved by large data streams.

The polling order will be:

1. **Graceful Shutdown Signal:** The `CancellationToken` will be checked first
   to ensure immediate reaction to a server-wide shutdown request.

2. **High-Priority Push Channel:** Messages from `high_priority_push_rx` will be
   drained next.

3. **Low-Priority Push Channel:** Messages from `low_priority_push_rx` will be
   processed after all high-priority messages.

4. **Handler Response Stream:** Frames from the active request's
   `Response::Stream` will be processed last.

```rust
// Simplified pseudo-code for the actor's write loop
loop {
    tokio::select! {
        biased;

        // 1. Highest priority: Graceful shutdown
        _ = shutdown_token.cancelled() => {
            tracing::info!("Shutdown signal received, terminating connection.");
            break;
        },

        // 2. High-priority server pushes
        Some(frame) = high_priority_push_rx.recv() => {
            send_frame(frame).await?;
        },

        // 3. Low-priority server pushes
        Some(frame) = low_priority_push_rx.recv() => {
            send_frame(frame).await?;
        },

        // 4. Standard response stream for the current request
        Some(result) = resp_stream.next() => {
            let frame = result?;
            send_frame(frame).await?;
        },

        // All sources are idle or complete
        else => { break; }
    }
}
```

#### 3.2.1 Fairness for low-priority frames

Continuous bursts of urgent messages can prevent the low-priority queue from
ever being drained. To mitigate this without removing the deterministic bias,
each `ConnectionActor` tracks how many high-priority frames have been processed
in a row. After a configurable threshold (`max_high_before_low`), the actor
checks `low_priority_push_rx.try_recv()` and, if a frame is present, processes
it and resets the counter.

An optional time slice (for example 100 µs) can also be configured. When the
elapsed time spent handling high-priority frames exceeds this slice, and the
low queue is not empty, the actor yields to a low-priority frame. Application
builders expose `with_fairness(FairnessConfig)` where `FairnessConfig` groups
the counter threshold and an optional `time_slice`. The counter defaults to 8
while `time_slice` is disabled. Setting the counter to zero disables the
threshold logic and relies solely on `time_slice` for fairness, preserving
strict high-priority ordering otherwise.

This fairness mechanism ensures low-priority traffic continues to progress even
under sustained high-priority load.

<!-- markdownlint-disable MD033 -->

The flow diagram below summarizes the fairness logic.

<description>The diagram shows how the actor yields to the low-priority queue
after N high-priority frames.</description>

<!-- markdownlint-enable MD033 -->

```mermaid
flowchart TD
    A[Start select! loop] --> B{High-priority frame available?}
    B -- Yes --> C[Process high-priority frame]
    C --> D[Increment high_priority_counter]
    D --> E{high_priority_counter >= max_high_before_low?}
    E -- Yes --> F{Low-priority frame available?}
    F -- Yes --> G[Process low-priority frame]
    G --> H[Reset high_priority_counter]
    F -- No --> I[Continue]
    E -- No --> I
    B -- No --> J{Low-priority frame available?}
    J -- Yes --> K[Process low-priority frame]
    J -- No --> I
    I --> A
    H --> A
    K --> A
```

The following sequence diagram illustrates the runtime behaviour:

```mermaid
sequenceDiagram
    participant Client
    participant ConnectionActor
    participant HighQueue
    participant LowQueue

    loop While processing frames
        ConnectionActor->>HighQueue: Poll high-priority frame
        alt High-priority frame received
            ConnectionActor->>ConnectionActor: after_high()
            alt Max high before low or time slice reached
                ConnectionActor->>LowQueue: Try receive low-priority frame
                alt Low-priority frame available
                    ConnectionActor->>ConnectionActor: after_low()
                end
            end
        else No high-priority frame
            ConnectionActor->>ConnectionActor: reset_high_counter()
        end
    end
```

### 3.3 Connection actor overview

Diagram: Class diagram of the connection actor and related types.

```mermaid
classDiagram
    class ConnectionActor {
        - context: ConnectionContext
        - high_priority_queue: mpsc::Receiver&lt;Frame&gt;
        - low_priority_queue: mpsc::Receiver&lt;Frame&gt;
        - response_stream: Stream&lt;Response&gt;
        - shutdown_signal: CancellationToken
        + run()
        + handle_push()
        + handle_response()
    }
    class PushHandle {
        + push(frame: Frame)
        + try_push(frame: Frame)
        + push_with_policy(frame: Frame, policy: PushPolicy)
    }
    class SessionRegistry {
        - sessions: DashMap&lt;ConnectionId, Weak&lt;ConnectionActor&gt;&gt;
        + register(connection_id, actor)
        + get_handle(connection_id): Option&lt;PushHandle&gt;
    }
    ConnectionActor o-- PushHandle : exposes / queues frames
    SessionRegistry o-- PushHandle : provides
```

Sequence diagram: Outbound frame flow from actor to socket.

```mermaid
sequenceDiagram
    participant Client
    participant ConnectionActor
    participant Outbox
    participant Socket

    Client->>ConnectionActor: Initiate connection/request
    Note over ConnectionActor: Manages high/low priority queues
    ConnectionActor->>Outbox: enqueue outbound frame
    ConnectionActor->>Outbox: dequeue request
    Outbox-->>ConnectionActor: frame
    ConnectionActor->>Socket: Write outbound frame
    Socket-->>Client: Delivers outbound message
    Note over Outbox: Holds frames while the socket is busy.
```

### 3.4 Actor state management

The connection actor polls four sources: a shutdown token, high- and
low-priority push channels, and an optional response stream. Earlier drafts
tracked a boolean for each source, leading to verbose state updates. The actor
now stores each receiver as an `Option` and counts how many sources have closed.

```rust
enum RunState {
    Active,
    ShuttingDown,
    Finished,
}

struct ActorState {
    run_state: RunState,
    closed_sources: usize,
    total_sources: usize,
}
```

`total_sources` is calculated when the actor starts. Whenever a receiver
returns `None`, it is set to `None` and `closed_sources` increments. When
`closed_sources == total_sources` the loop exits. This consolidation clarifies
progress through the actor lifecycle and reduces manual flag management.

## 4. Public API surface

The public API is designed for ergonomics, safety, and extensibility.

### 4.1 The `PushHandle`

The primary user-facing primitive is the `PushHandle`, a cloneable handle that
provides the capability to send frames to a specific connection.

```rust
// The internal state, managed by an Arc for shared ownership.
struct PushHandleInner<F> {
    high_prio_tx: mpsc::Sender<F>,
    low_prio_tx: mpsc::Sender<F>,
    // Other shared state like rate limiters can be added here.
}

// The public, cloneable handle.
#[derive(Clone)]
  pub struct PushHandle<F>(Arc<PushHandleInner<F>>);

pub enum PushPolicy {
    ReturnErrorIfFull,
    DropIfFull,
    WarnAndDropIfFull,
}

impl<F: FrameLike> PushHandle<F> {
    /// Push a high-priority frame. Awaits if the queue is full.
    pub async fn push_high_priority(&self, frame: F) -> Result<(), PushError>;

    /// Push a low-priority frame. Awaits if the queue is full.
    pub async fn push_low_priority(&self, frame: F) -> Result<(), PushError>;

    /// Push a frame according to a specific policy for when the queue is full.
    pub fn try_push(
        &self,
        frame: F,
        priority: PushPriority,
        policy: PushPolicy,
        ) -> Result<(), PushError>;
    }
```

The following sequence diagram illustrates the flow when a producer pushes a
high-priority frame. It shows how the `PushHandle` coordinates rate-limiting,
queue availability, and an optional dead-letter queue.

<!-- markdownlint-disable MD033 -->
<description>The diagram shows rate-limiting and dead-letter queue fallback
when enqueueing a frame.</description>
<!-- markdownlint-enable MD033 -->

```mermaid
sequenceDiagram
    participant Producer
    participant PushHandle
    participant RateLimiter
    participant PushQueues
    participant DLQ
    Producer->>PushHandle: push_high_priority(frame)
    alt Rate-limiting enabled
        PushHandle->>RateLimiter: acquire token
        RateLimiter-->>PushHandle: token granted
    end
    PushHandle->>PushQueues: try_send(frame)
    alt Queue full
        alt DLQ configured
            PushHandle->>DLQ: try_send(frame)
            alt DLQ full
                PushHandle->>PushHandle: log warning (throttled)
            else DLQ accepted
                DLQ-->>PushHandle: frame queued
            end
        else No DLQ
            PushHandle->>PushHandle: log/drop per policy
        end
    else Enqueued
        PushQueues-->>PushHandle: frame queued
    end
```

The example below demonstrates pushing frames and returning a streamed
response. The [`async-stream`](https://docs.rs/async-stream) crate is the
canonical way to build dynamic `Response::Stream` values.

```rust,no_run
use async_stream::try_stream;
use std::sync::Arc;
use wireframe::{app::{Envelope, WireframeApp}, Response};

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let app = WireframeApp::new()?
        .route(1, Arc::new(|_: &Envelope| Box::pin(async {
            Response::Stream(Box::pin(try_stream! {
                yield b"ack".to_vec();
            }))
        })))?;

      let (push, mut conn) = wireframe_testing::connect(app).await?;
      tokio::spawn(async move {
          push.push_high_priority(b"urgent".to_vec()).await.unwrap();
          push.push_low_priority(b"stats".to_vec()).await.unwrap();
      });

  while let Some(frame) = conn.next().await {
      println!("{:?}", frame);
  }
  Ok(())
}
```

Class diagram: Push API types, policies, and errors.

```mermaid
classDiagram
    class FrameLike {
    }
    class PushPriority {
        <<enum>>
        High
        Low
    }
    class PushPolicy {
        <<enum>>
        ReturnErrorIfFull
        DropIfFull
        WarnAndDropIfFull
    }
    class PushError {
        <<enum>>
        QueueFull
        Closed
    }
    class PushHandleInner {
        high_prio_tx: mpsc::Sender<F>
        low_prio_tx: mpsc::Sender<F>
    }
    class PushHandle~F~ {
        +push_high_priority(frame: F): Result<(), PushError>
        +push_low_priority(frame: F): Result<(), PushError>
        +try_push(frame: F, priority: PushPriority, policy: PushPolicy): Result<(), PushError>
    }
    class PushQueues~F~ {
        +high_priority_rx: mpsc::Receiver<F>
        +low_priority_rx: mpsc::Receiver<F>
        +builder(): PushQueuesBuilder~F~
        +recv(): Option<(PushPriority, F)>
    }
    class PushQueuesBuilder~F~ {
        +high_capacity(cap: usize): PushQueuesBuilder~F~
        +low_capacity(cap: usize): PushQueuesBuilder~F~
        +rate(rate: Option<usize>): PushQueuesBuilder~F~
        +dlq(sender: Option<mpsc::Sender<F>>): PushQueuesBuilder~F~
        +build(): (PushQueues~F~, PushHandle~F~)
    }

    PushHandleInner <.. PushHandle~F~ : contains
    PushQueues~F~ o-- PushQueuesBuilder~F~ : builder()
    PushQueuesBuilder~F~ o-- PushHandle~F~ : build()
    PushHandle --> PushPriority
    PushHandle --> PushPolicy
    PushHandle --> PushError
```

The diagram uses `~F~` to represent the `<F>` generic parameter because Mermaid
treats angle brackets as HTML.

Flowchart: Routing of pushes through priority queues and policy outcomes.

```mermaid
flowchart TD
    Producer[Producer]
    Handle[PushHandle~F~]
    HighQueue[High Priority Queue]
    LowQueue[Low Priority Queue]
    Policy[PushPolicy]
    Error[PushError or Drop]

    Producer -->|push_high_priority| Handle
    Handle -->|priority: High| HighQueue
    Producer -->|push_low_priority| Handle
    Handle -->|priority: Low| LowQueue

    Producer -->|try_push| Policy
    Policy -->|Queue available| Handle
    Handle -->|priority: High| HighQueue
    Handle -->|priority: Low| LowQueue
    Policy -->|ReturnErrorIfFull| Error
    Policy -->|DropIfFull| Error
    Policy -->|WarnAndDropIfFull| Error
```

This API gives developers fine-grained control over both the priority and the
back-pressure behaviour of their pushed messages.

### 4.2 The `SessionRegistry`

To allow background tasks to discover and message active connections, a
`SessionRegistry` will be provided. To prevent memory leaks, this registry
**must** be implemented using non-owning `Weak` references.

```rust
use dashmap::DashMap;
use std::sync::{Arc, Weak};

// The registry stores Weak pointers, preventing it from keeping connections alive.
pub struct SessionRegistry<F>(DashMap<ConnectionId, Weak<PushHandleInner<F>>>);

impl<F> SessionRegistry<F> {
    /// Attempts to retrieve a live PushHandle for a given connection.
    /// Entries whose handles have been dropped are removed lazily.
    pub fn get(&self, id: &ConnectionId) -> Option<PushHandle<F>> {
        let guard = self.0.get(id);
        let inner = guard.as_ref().and_then(|w| w.upgrade());
        drop(guard);
        if inner.is_none() {
            self.0.remove_if(id, |_, weak| weak.strong_count() == 0);
        }
        inner.map(|inner| PushHandle::from_arc(inner))
    }

    /// Inserts a new handle into the registry.
    pub fn insert(&self, id: ConnectionId, handle: &PushHandle<F>) {
        // Downgrade the Arc to a Weak pointer for storage.
        let weak_ref = Arc::downgrade(&handle.0);
        self.0.insert(id, weak_ref);
    }

    /// Removes a handle, typically called on connection teardown.
    pub fn remove(&self, id: &ConnectionId) {
        self.0.remove(id);
    }

    /// Returns all live session handles for broadcast or diagnostics.
    pub fn active_handles(&self) -> Vec<(ConnectionId, PushHandle<F>)> {
        let mut handles = Vec::with_capacity(self.0.len());
        for entry in self.0.iter() {
            let id = *entry.key();
            if let Some(h) = entry.value().upgrade() {
                handles.push((id, PushHandle(h)));
            }
        }
        handles
    }
}
```

`active_handles()` prunes stale entries as it collects the remaining live
handles. When a side-effect free snapshot is needed, `prune()` can be called
separately before iterating. `DashMap::retain` acquires per-bucket write locks,
so pruning while collecting may contend more than the previous post-collection
`remove_if` sweep. Maintenance tasks may instead invoke `prune()` to avoid this
contention.

The diagram below summarizes the data structures and how they interact when
storing session handles. `SessionRegistry` maps `ConnectionId`s to weak
references of `PushHandleInner<F>` so closed connections do not stay alive.

```mermaid
classDiagram
    class ConnectionId {
        +u64 value
        +from(u64) ConnectionId
    }
    class PushHandleInner~F~ {
        +high_prio_tx: mpsc::Sender<F>
        +low_prio_tx: mpsc::Sender<F>
    }
    class PushHandle~F~ {
        +from_arc(arc: Arc<PushHandleInner<F>>) PushHandle<F>
        +downgrade() Weak<PushHandleInner<F>>
        +push(frame: F)
        +try_push(frame: F)
    }
    class SessionRegistry~F~ {
        +get(id: &ConnectionId) Option<PushHandle<F>>
        +insert(id: ConnectionId, handle: &PushHandle<F>)
        +remove(id: &ConnectionId)
        +prune()
    }
    SessionRegistry~F~ --> "*" ConnectionId : uses as key
    SessionRegistry~F~ --> "*" PushHandleInner~F~ : stores Weak refs
    PushHandle~F~ --> PushHandleInner~F~ : wraps Arc
    PushHandle~F~ ..> SessionRegistry~F~ : used in insert/get
```

Sequence diagram: Background task acquires a handle and enqueues a frame.

```mermaid
sequenceDiagram
    participant Task as Background Task
    participant Registry as SessionRegistry
    participant Conn as Connection (PushHandle)
    participant Queue as Frame Queue

    Task->>Registry: get(ConnectionId)
    alt If handle exists and alive
        Registry-->>Task: PushHandle
        Task->>Conn: push(frame)
        Conn->>Queue: enqueue(frame)
    else If handle missing or dropped
        Registry-->>Task: None
    end
```

### 4.3 Configuration via the `WireframeProtocol` trait

To provide a clean, organized, and extensible configuration API, all
protocol-specific logic and callbacks will be encapsulated within a single
`WireframeProtocol` trait. This is a significant ergonomic improvement over
using a collection of individual closures.

```rust
pub trait WireframeProtocol: Send + Sync + 'static {
    type Frame: FrameLike;
    type ProtocolError;

    /// Called once when a new connection is established.
    /// This is the ideal place to store the PushHandle in a SessionRegistry.
    fn on_connection_setup(
        &self,
        handle: PushHandle<Self::Frame>,
        ctx: &mut ConnectionContext
    );

    /// Called just before any frame (pushed or response) is written to the socket.
    /// Allows for last-minute mutations, like setting sequence IDs.
    fn before_send(&self, frame: &mut Self::Frame, ctx: &mut ConnectionContext);

    /// Called after a request/response command cycle is complete.
    fn on_command_end(&self, ctx: &mut ConnectionContext);

    // Other protocol-specific callbacks can be added here in the future.
}

// The application builder becomes clean and declarative.
WireframeApp::new().with_protocol(MySqlProtocolImpl);
```

Class diagram: WireframeProtocol, ProtocolHooks, and builder wiring.

```mermaid
classDiagram
    class WireframeProtocol {
        <<trait>>
        +Frame: FrameLike
        +ProtocolError
        +on_connection_setup(PushHandle<Frame>, &mut ConnectionContext)
        +before_send(&mut Frame, &mut ConnectionContext)
        +on_command_end(&mut ConnectionContext)
    }
    class ProtocolHooks {
        -before_send: Option<BeforeSendHook<F>>
        -on_command_end: Option<OnCommandEndHook>
        +before_send(&mut self, &mut F, &mut ConnectionContext)
        +on_command_end(&mut self, &mut ConnectionContext)
        +from_protocol(protocol: Arc<P>)
    }
    class ConnectionContext {
        <<struct>>
    }
    class WireframeApp {
        -protocol: Option<Arc<dyn WireframeProtocol<Frame=Vec<u8>, ProtocolError=()>>>
        +with_protocol(protocol)
        +protocol()
        +protocol_hooks()
    }
    class ConnectionActor {
        -hooks: ProtocolHooks<F>
        -ctx: ConnectionContext
    }
    WireframeApp --> "1" WireframeProtocol : uses
    WireframeApp --> "1" ProtocolHooks : creates
    ProtocolHooks --> "1" WireframeProtocol : from_protocol
    ConnectionActor --> "1" ProtocolHooks : uses
    ConnectionActor --> "1" ConnectionContext : owns
    ProtocolHooks --> "1" ConnectionContext : passes to hooks
    WireframeProtocol --> "1" ConnectionContext : uses
    WireframeProtocol --> "1" PushHandle : uses
    WireframeProtocol <|.. ProtocolHooks : implemented by
```

`ConnectionContext` is intentionally empty today. It offers a stable extension
point for per-connection data without breaking existing protocol
implementations.

## 5. Error handling & resilience

### 5.1 `BrokenPipe` on connection loss

The primary error condition for a `PushHandle` is the termination of its
associated connection. When the connection actor terminates (due to a socket
error, clean shutdown, or graceful cancellation), the receiving end of the
internal `mpsc` channels will be dropped. Any subsequent attempt to use a
`PushHandle` will fail with an error analogous to `io::ErrorKind::BrokenPipe`,
clearly signalling to the producer task that the connection is gone.

### 5.2 Optional dead letter queue (DLQ) for critical messages

For applications where dropping a message is unacceptable (e.g., critical
notifications, audit events), the framework will support an optional Dead
Letter Queue.

**Implementation:** The `WireframeApp` builder will provide a method,
`with_push_dlq(mpsc::Sender<F>)`, to configure a DLQ. If provided, any frame
that would normally be dropped by the `PushPolicy::DropIfFull` or
`WarnAndDropIfFull` policies will instead be sent to this channel. A separate
part of the application is then responsible for consuming from the DLQ to
inspect, log, and potentially retry these failed messages.

The following sequence diagram illustrates how frames are routed when a DLQ is
configured:

```mermaid
sequenceDiagram
    participant Producer
    participant PushHandle
    participant HighPrioQueue
    participant DLQ as DeadLetterQueue
    Producer->>PushHandle: try_push(frame, priority, DropIfFull)
    PushHandle->>HighPrioQueue: try_send(frame)
    alt Queue full
        PushHandle->>DLQ: try_send(frame)
        alt DLQ full
            PushHandle->>PushHandle: log error (frame lost)
        else DLQ has space
            DLQ-->>PushHandle: frame accepted
        end
    else Queue has space
        HighPrioQueue-->>PushHandle: frame accepted
    end
    PushHandle-->>Producer: Ok or logs
```

### 5.3 Typed protocol errors

`WireframeError` distinguishes transport failures from protocol logic errors. A
`WireframeError::Protocol(e)` returned from a handler will be forwarded to the
`handle_error` callback on the installed `WireframeProtocol`. This allows the
protocol implementation to serialize a domain-specific error frame before the
current command is terminated.

Class diagram: Typed protocol errors and propagation via hooks.

```mermaid
classDiagram
    class WireframeProtocol {
        +on_connection_setup(PushHandle<Frame>, &mut ConnectionContext)
        +before_send(&mut Frame, &mut ConnectionContext)
        +on_command_end(&mut ConnectionContext)
        +handle_error(ProtocolError, &mut ConnectionContext)
        <<trait>>
        type Frame
        type ProtocolError
    }

    class ProtocolHooks {
        +on_connection_setup : Option<OnConnectionSetupHook<F>>
        +before_send        : Option<BeforeSendHook<F>>
        +on_command_end     : Option<OnCommandEndHook>
        +handle_error       : Option<HandleErrorHook<E>>
        +on_connection_setup(PushHandle<F>, &mut ConnectionContext)
        +before_send       (&mut F, &mut ConnectionContext)
        +on_command_end    (&mut ConnectionContext)
        +handle_error      (E, &mut ConnectionContext)
        +from_protocol     (protocol: Arc<P>)
        <<generic<F, E>>
    }

    class ConnectionActor {
        -hooks: ProtocolHooks<F, E>
        +run(&mut self, out: &mut Vec<F>) -> Result<(), WireframeError<E>>
        +handle_response(res: Option<Result<F, WireframeError<E>>>, out: &mut Vec<F>, state: &mut State) -> Result<(), WireframeError<E>>
        <<generic<F, E>>
    }

    WireframeProtocol <|.. ProtocolHooks : uses
    ProtocolHooks <|-- ConnectionActor : member

    class WireframeError {
        <<enum<E=()>>
        +Protocol(E)
        +Io(std::io::Error)
    }
```

Sequence diagram: Handling of protocol vs I/O errors in the actor.

```mermaid
sequenceDiagram
    participant Client
    participant ConnectionActor
    participant ProtocolHooks
    participant Protocol

    Client->>ConnectionActor: Send/receive frames
    ConnectionActor->>ProtocolHooks: before_send / on_command_end
    ConnectionActor->>Protocol: Process frame
    Protocol-->>ConnectionActor: Result or ProtocolError
    alt ProtocolError
        ConnectionActor->>ProtocolHooks: handle_error(error, ctx)
        ProtocolHooks-->>ConnectionActor: (handled, continue)
    else IO Error
        ConnectionActor-->>Client: Return WireframeError (terminates)
    end
```

## 6. Synergy with other 1.0 features

This design is explicitly intended to work in concert with the other major
features of the 1.0 release.

- **Streaming Responses:** The prioritized write loop (Section 3.2) naturally
  handles the interleaving of pushed messages and streaming responses, ensuring
  that urgent pushes can interrupt a long-running data stream.

- **Message Fragmentation:** Pushes occur at the *logical frame* level. The
  `FragmentAdapter` will operate at a lower layer in the codec stack,
  transparently splitting any large pushed frames before they are written to
  the socket. The `PushHandle` and the application code that uses it remain
  completely unaware of fragmentation.

```rust,ignore
// Codec stack with explicit frame-size limits and fragmentation.
use wireframe::app::WireframeApp;
const MAX_FRAME: usize = 64 * 1024;
let app = WireframeApp::new()
    .expect("failed to create app")
    .buffer_capacity(MAX_FRAME);
let codec = app.length_codec();

// Wrap the length-delimited frames with the fragmentation adapter.
// Pseudocode pending actual adapter API naming:
// let codec = FragmentAdapter::new(FragmentConfig::default()).wrap(codec);
```

## 7. Use cases

### 7.1 Server-initiated MySQL packets

MariaDB may spontaneously push packets while a client is idle. Two notable
examples are an `OK` packet with session tracker data and a `LOCAL INFILE`
request. The design presented here enables these packets without changing the
existing request/response model.

Each connection task owns an mpsc outbox channel and exposes a `PushHandle`
through a registry or the `on_connection_setup` hook. Any async task can call
`push_high_priority()` or `push_low_priority()` on this handle to queue a frame
for delivery. Sequence IDs reset to zero on command completion to maintain
protocol integrity.

```mermaid
sequenceDiagram
    participant AppTask as Application Task
    participant SessionRegistry
    participant ConnectionActor
    participant Outbox
    participant Socket
    AppTask->>SessionRegistry: get PushHandle for session
    AppTask->>ConnectionActor: push(OK packet or LOCAL INFILE)
    ConnectionActor->>Outbox: enqueue frame
    ConnectionActor->>Outbox: dequeue request
    Outbox-->>ConnectionActor: frame
    ConnectionActor->>Socket: write frame (when idle or after command completes)
```

### 7.2 Heartbeat pings (WebSocket)

A background timer can periodically send Ping frames to keep a WebSocket
connection alive. `push_high_priority()` ensures these heart-beats are written
even while a large response stream is in progress.

```mermaid
sequenceDiagram
    participant Timer as Heartbeat Timer
    participant ConnectionActor
    participant Outbox
    participant Socket
    Timer->>ConnectionActor: push_high_priority(Ping frame)
    ConnectionActor->>Outbox: enqueue Ping in high-priority queue
    ConnectionActor->>Outbox: dequeue request
    Outbox-->>ConnectionActor: Ping
    ConnectionActor->>Socket: write Ping frame (even during response stream)
```

### 7.3 Broker-side MQTT `PUBLISH`

An MQTT broker can deliver retained messages or fan-out new `PUBLISH` frames to
all subscribed clients via their `PushHandle`s. The `try_push` method allows
the broker to drop non-critical messages when a subscriber falls behind.

```mermaid
sequenceDiagram
    participant Broker as MQTT Broker
    participant SessionRegistry
    participant ConnectionActor as Client ConnectionActor
    participant Socket
    Broker->>SessionRegistry: get PushHandle for subscriber
    Broker->>ConnectionActor: try_push(PUBLISH frame)
    alt Queue not full
        ConnectionActor->>Socket: write PUBLISH frame
    else Queue full
        Note over ConnectionActor: Drop frame due to full queue.
    end
```

## 8. Measurable objectives & success criteria

| Category        | Objective                                                                                                           | Success Metric                                                                                                                                                                              |
| --------------- | ------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| API Correctness | The PushHandle, SessionRegistry, and WireframeProtocol trait are implemented exactly as specified in this document. | 100% of the public API surface is present and correctly typed.                                                                                                                              |
| Functionality   | Pushed frames are delivered reliably and in the correct order of priority.                                          | A test with concurrent high-priority, low-priority, and streaming producers must show that all frames are delivered and that the final written sequence respects the strict priority order. |
| Back-pressure   | A slow consumer must cause producer tasks to suspend without consuming unbounded memory.                            | A test with a slow consumer and a fast producer must show the producer's push().await call blocks, and the process memory usage remains stable.                                             |
| Resilience      | The SessionRegistry must not leak memory when connections are terminated.                                           | A long-running test that creates and destroys thousands of connections must show no corresponding growth in the SessionRegistry's size or the process's overall memory footprint.           |
| Performance     | The overhead of the push mechanism should be minimal for connections that do not use it.                            | A benchmark of a simple request-response workload with the push feature enabled (but unused) should show < 2% performance degradation compared to a build without the feature.              |
| Performance     | The latency for a high-priority push under no contention should be negligible.                                      | The time from push_high_priority().await returning to the frame being written to the socket buffer should be < 10µs.                                                                        |

## 9. Deprecating message versions gracefully

Describe a forward-compatible path for retiring message versions while
maintaining service continuity, observability, and explicit timelines.

- Advertise supported versions in handshake headers, and negotiate the highest
  mutually supported version.
- Map deprecated frames to the current internal model; avoid branching deep in
  the codebase.
- Emit structured telemetry for deprecated usage, with connection identifiers,
  version, and feature flags.
- Publish a removal schedule, and gate final removal with a kill-switch.

```rust,ignore
// Pseudocode: negotiate, adapt, and surface deprecation telemetry
pub fn on_connection_setup(&self, handle: PushHandle<Frame>, ctx: &mut ConnectionContext) {
    let their = ctx.peer_versions();            // e.g., [1, 2]
    let ours  = [2, 3];                         // server supported
    let agreed = negotiate(&their, &ours);      // pick 2
    ctx.set_active_version(agreed);
}

pub fn before_send(&self, frame: &mut Frame, ctx: &mut ConnectionContext) {
    match ctx.active_version() {
        1 => adapt_to_v1(frame),                // down-map
        2 | 3 => {}
        _ => {}                                 // unreachable
    }
}

pub fn on_command_end(&self, ctx: &mut ConnectionContext) {
    if ctx.active_version() == 1 {
        tracing::warn!(target: "deprecation",
            version = 1, conn = %ctx.id(),
            "deprecated message version in use");
    }
}
```

```plaintext
Removal timeline
1. Announce deprecation of v1 with telemetry and docs.
2. After N releases, disable v1 by default; allow override flag `--allow-v1`.
3. After M releases, remove v1; map any residual v1 frames to DLQ with reason.
```
