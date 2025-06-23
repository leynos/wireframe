
# Comprehensive Design: Multi-Packet & Streaming Responses

## 1. Introduction & Philosophy

The initial version of `wireframe` established a robust, strictly request-response communication model. This design was intentionally simple but is insufficient for the rich, conversational protocols that define modern networking, such as database wire formats (MySQL, PostgreSQL), message brokers (MQTT), and RPC systems (gRPC). These protocols frequently require a single request to elicit a multi-part or open-ended sequence of response frames.

This document details the design for a first-class, protocol-agnostic streaming response feature. The core philosophy is to enable this complex functionality through a simple, declarative, and ergonomic API. By embracing modern asynchronous Rust patterns, we will avoid the complexities of imperative, sink-based APIs and provide a unified handler model that is both powerful for streaming and simple for single-frame replies. 1

This feature is a key component of the "Road to Wireframe 1.0," working in concert with asynchronous push messaging and fragmentation to create a fully duplex and capable framework.

## 2. Design Goals & Requirements

The implementation must satisfy the following core requirements:

<table class="not-prose border-collapse table-auto w-full" style="min-width: 50px">
<colgroup><col style="min-width: 25px"><col style="min-width: 25px"></colgroup><tbody><tr><td class="border border-neutral-300 dark:border-neutral-600 p-1.5" colspan="1" rowspan="1"><p>ID</p></td><td class="border border-neutral-300 dark:border-neutral-600 p-1.5" colspan="1" rowspan="1"><p>Requirement</p></td></tr><tr><td class="border border-neutral-300 dark:border-neutral-600 p-1.5" colspan="1" rowspan="1"><p><strong>G1</strong></p></td><td class="border border-neutral-300 dark:border-neutral-600 p-1.5" colspan="1" rowspan="1"><p>Allow a handler to send <strong>zero, one, or many</strong> frames for a single logical response.</p></td></tr><tr><td class="border border-neutral-300 dark:border-neutral-600 p-1.5" colspan="1" rowspan="1"><p><strong>G2</strong></p></td><td class="border border-neutral-300 dark:border-neutral-600 p-1.5" colspan="1" rowspan="1"><p>Provide transparent <strong>back-pressure</strong>: writers must suspend when outbound capacity is exhausted.</p></td></tr><tr><td class="border border-neutral-300 dark:border-neutral-600 p-1.5" colspan="1" rowspan="1"><p><strong>G3</strong></p></td><td class="border border-neutral-300 dark:border-neutral-600 p-1.5" colspan="1" rowspan="1"><p>Integrate with protocol-specific sequencing rules (e.g., per-command counters) without hard-coding any one protocol.</p></td></tr><tr><td class="border border-neutral-300 dark:border-neutral-600 p-1.5" colspan="1" rowspan="1"><p><strong>G4</strong></p></td><td class="border border-neutral-300 dark:border-neutral-600 p-1.5" colspan="1" rowspan="1"><p>Keep the <em>simple</em> “single-frame reply” path untouched; upgrading should be optional and ergonomic.</p></td></tr><tr><td class="border border-neutral-300 dark:border-neutral-600 p-1.5" colspan="1" rowspan="1"><p><strong>G5</strong></p></td><td class="border border-neutral-300 dark:border-neutral-600 p-1.5" colspan="1" rowspan="1"><p>Work symmetrically for <strong>servers and clients</strong> built with <code class="code-inline">wireframe</code>.</p></td></tr></tbody>
</table>

## 3. Core Architecture: Declarative Streaming

The cornerstone of this design is a move to a purely **declarative streaming model**. Instead of providing handlers with an imperative `FrameSink` to push frames into, handlers will declaratively return a description of the entire response. 5 This approach significantly simplifies the API surface, improves testability, and eliminates a class of resource management issues associated with sink-based designs.

### 3.1 The Connection Actor's Role

The existing connection actor model is well-suited to consume these declarative responses. When a handler returns a `Response::Stream`, the actor will take ownership of the stream and poll it for new frames within its main `select!` loop.

Back-pressure emerges naturally from this design. The `select!` loop awaits writing each frame to the socket. If the socket is slow and its buffer fills, the `write` operation will suspend the actor task. This suspension prevents the actor from polling the response stream for the next frame, which in turn propagates back-pressure all the way to the stream's producer without any explicit channel management.

### 3.2 The `async-stream` Crate

To provide an ergonomic way for developers to generate streams using imperative-style logic (e.g., inside a `for` loop), `wireframe` will adopt and recommend the `async-stream` crate. 9 This crate provides macros (

`stream!` and `try_stream!`) that transform imperative `yield` statements into a fully compliant `Stream` object. This gives developers the best of both worlds: the intuitive feel of imperative code generation without the API complexity of a separate `FrameSink` type.

## 4. Public API Surface

The public API is designed for clarity, performance, and ergonomic flexibility.

### 4.1 The `Response` Enum

The `Response` enum is the primary return type for all handlers. It is enhanced to provide optimised paths for common response patterns.

Rust

```
use futures_core::stream::Stream;
use std::pin::Pin;

/// Represents the full response to a request.
pub enum Response<F, E> {
    /// A single frame reply. The most common case.
    Single(F),

    /// An optimised response for a small, known list of frames.
    /// Avoids the overhead of boxing and dynamic dispatch for simple multi-part replies.
    Vec(Vec<F>),

    /// A potentially unbounded stream of frames for complex or dynamically generated responses.
    Stream(FrameStream<F, E>),

    /// A response that sends no frames, indicating the request was handled
    /// but produced no output (e.g., after a server push).
    Empty,
}

/// A type alias for a type-erased, dynamically dispatched stream of frames.
pub type FrameStream<F, E> =
    Pin<Box<dyn Stream<Item = Result<F, WireframeError<E>>> + Send + 'static>>;
```

This design allows simple, single-frame handlers to remain unchanged (`Ok(my_frame.into())`) while providing powerful and efficient options for more complex cases.

### 4.2 The `WireframeError` Enum

To enable more robust error handling, a generic error enum will be introduced. This allows the framework and protocol implementations to distinguish between unrecoverable transport failures and logical, protocol-level errors.

Rust

```
/// A generic error type for wireframe operations.
#
pub enum WireframeError<E> {
    /// An error occurred in the underlying transport (e.g., socket closed).
    /// These are typically unrecoverable for the connection.
    Io(std::io::Error),

    /// A protocol-defined error occurred (e.g., invalid request).
    /// The framework can pass this to the protocol layer to be formatted
    /// into a proper error frame before closing the stream.
    Protocol(E),
}

// Implement `From<std::io::Error>` for ergonomics.
impl<E> From<std::io::Error> for WireframeError<E> {
    fn from(e: std::io::Error) -> Self {
        WireframeError::Io(e)
    }
}
```

## 5. Handler Implementation Patterns

The following examples illustrate how developers will use the new API.

### 5.1 Single-Frame Reply (Unchanged)

Existing code continues to work without modification, fulfilling goal **G4**.

Rust

```
async fn handle_ping(_req: Request) -> Result<Response<MyFrame, MyError>, MyError> {
    // `MyFrame` implements `Into<Response<...>>`
    Ok(build_pong_frame().into())
}
```

### 5.2 Small, Multi-Part Result (`Response::Vec`)

For simple, fixed-size multi-part responses, like a MySQL result set header, `Response::Vec` is both ergonomic and performant.

Rust

```
async fn handle_select_headers(_req: Request) -> Result<Response<MySqlFrame, MyError>, MyError> {
    // Pre-build frames for: column-count, column-def, EOF
    let frames = vec!;
    Ok(Response::Vec(frames))
}
```

### 5.3 Large or Dynamic Stream (`Response::Stream`)

For large or dynamically generated result sets, like a PostgreSQL `COPY OUT` command, `async-stream` provides an intuitive way to generate the stream.

Rust

```
use async_stream::try_stream;

async fn handle_copy_out(req: PgCopyRequest) -> Result<Response<PgFrame, PgError>, PgError> {
    let response_stream = try_stream! {
        // First, yield the row description frame.
        yield PgFrame::row_description(&req.columns);

        // Now, iterate over the data source and yield a frame for each row.
        // The `?` operator will correctly propagate errors into the stream.
        for row in database::fetch_rows_for_copy(&req.table)? {
            yield PgFrame::data_row(row)?;
        }

        // Finally, yield the completion message.
        yield PgFrame::command_complete("COPY");
    };

    Ok(Response::Stream(Box::pin(response_stream)))
}
```

## 6. Stream Lifecycle and Error Handling

### 6.1 Stream Termination

A response stream is considered complete when the underlying `Stream` implementation returns `Poll::Ready(None)`. The connection actor will detect this and call the `on_logical_response_end` hook on the `WireframeProtocol` trait, allowing the protocol implementation to reset any per-command state.

### 6.2 Error Propagation

If the stream yields an `Err(WireframeError<E>)`, the connection actor will:

1. **If** `WireframeError::Io`**:** Immediately terminate the connection, as this indicates a transport-level failure.

2. **If** `WireframeError::Protocol(e)`**:** Pass the typed error `e` to a new `handle_error` callback on the `WireframeProtocol` trait. This gives the protocol layer a chance to serialize a proper error frame to send to the client before the stream is terminated.

### 6.3 Cancellation Safety & `Drop` Semantics

The design is inherently cancellation-safe. The `select!` macro in the connection actor will drop the `FrameStream` future if another branch (e.g., a shutdown signal) completes first. Because `StreamExt::next()` is cancellation-safe, no frames will be lost; the stream will simply be dropped. 12

Similarly, if a handler panics or returns early, the `Stream` object it created is simply dropped. The connection actor will see the stream end as if it had completed normally, ensuring no resources are leaked and the connection does not hang. 17

## 7. Synergy with Other 1.0 Features

- **Asynchronous Pushes:** The connection actor's prioritised write loop (as defined in the outbound messaging design) will always poll for pushed messages *before* polling the response stream. This ensures that urgent, out-of-band messages are not starved by a long-running data stream.

- **Message Fragmentation:** Streaming occurs at the logical frame level. The `FragmentAdapter` will operate at a lower layer, transparently splitting any large frames yielded by the stream before they are written to the socket. The handler and streaming logic remain completely unaware of fragmentation.

## 8. Measurable Objectives & Success Criteria

<table class="not-prose border-collapse table-auto w-full" style="min-width: 75px">
<colgroup><col style="min-width: 25px"><col style="min-width: 25px"><col style="min-width: 25px"></colgroup><tbody><tr><td class="border border-neutral-300 dark:border-neutral-600 p-1.5" colspan="1" rowspan="1"><p>Category</p></td><td class="border border-neutral-300 dark:border-neutral-600 p-1.5" colspan="1" rowspan="1"><p>Objective</p></td><td class="border border-neutral-300 dark:border-neutral-600 p-1.5" colspan="1" rowspan="1"><p>Success Metric</p></td></tr><tr><td class="border border-neutral-300 dark:border-neutral-600 p-1.5" colspan="1" rowspan="1"><p><strong>API Correctness</strong></p></td><td class="border border-neutral-300 dark:border-neutral-600 p-1.5" colspan="1" rowspan="1"><p>The <code class="code-inline">Response</code> enum and <code class="code-inline">FrameStream</code> type alias are implemented exactly as specified in this document.</p></td><td class="border border-neutral-300 dark:border-neutral-600 p-1.5" colspan="1" rowspan="1"><p>100% of the public API surface is present and correctly typed.</p></td></tr><tr><td class="border border-neutral-300 dark:border-neutral-600 p-1.5" colspan="1" rowspan="1"><p><strong>Functionality</strong></p></td><td class="border border-neutral-300 dark:border-neutral-600 p-1.5" colspan="1" rowspan="1"><p>A handler returning a stream of N frames results in N frames being written to the socket in the correct order.</p></td><td class="border border-neutral-300 dark:border-neutral-600 p-1.5" colspan="1" rowspan="1"><p>A test suite confirms 100% frame delivery and strict ordering for <code class="code-inline">Response::Vec</code> and <code class="code-inline">Response::Stream</code>.</p></td></tr><tr><td class="border border-neutral-300 dark:border-neutral-600 p-1.5" colspan="1" rowspan="1"><p><strong>Ergonomics</strong></p></td><td class="border border-neutral-300 dark:border-neutral-600 p-1.5" colspan="1" rowspan="1"><p>The <code class="code-inline">async-stream</code> pattern is documented as the canonical approach for dynamic stream generation and is demonstrably simpler than a <code class="code-inline">FrameSink</code> API.</p></td><td class="border border-neutral-300 dark:border-neutral-600 p-1.5" colspan="1" rowspan="1"><p>The official examples and documentation exclusively use the declarative <code class="code-inline">Response</code> model.</p></td></tr><tr><td class="border border-neutral-300 dark:border-neutral-600 p-1.5" colspan="1" rowspan="1"><p><strong>Performance</strong></p></td><td class="border border-neutral-300 dark:border-neutral-600 p-1.5" colspan="1" rowspan="1"><p>The <code class="code-inline">Response::Vec</code> variant has measurably lower allocation and dispatch overhead than <code class="code-inline">Response::Stream</code> for small, fixed-size responses.</p></td><td class="border border-neutral-300 dark:border-neutral-600 p-1.5" colspan="1" rowspan="1"><p>A <code class="code-inline">criterion</code> benchmark confirms that <code class="code-inline">Response::Vec</code> is at least 50% faster and performs fewer allocations than <code class="code-inline">Response::Stream</code> for a response of 10 frames.</p></td></tr><tr><td class="border border-neutral-300 dark:border-neutral-600 p-1.5" colspan="1" rowspan="1"><p><strong>Error Handling</strong></p></td><td class="border border-neutral-300 dark:border-neutral-600 p-1.5" colspan="1" rowspan="1"><p>A <code class="code-inline">WireframeError::Protocol</code> error yielded from a stream correctly triggers the <code class="code-inline">handle_error</code> protocol callback without terminating the connection.</p></td><td class="border border-neutral-300 dark:border-neutral-600 p-1.5" colspan="1" rowspan="1"><p>An integration test confirms that a protocol-level error is correctly formatted and sent to the client, while the connection remains open.</p></td></tr></tbody>
</table>