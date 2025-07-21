# Testing Network Outages in `mxd`: A Comprehensive Tutorial

## Introduction to the Problem

Networked servers must be robust against unpredictable conditions – slow or
dropped connections, timeouts, partial data transmission, etc. In the `mxd`
server, which is an async Rust application, it’s crucial to **simulate network
outages** (timeouts, connection resets, partial sends) and verify that the
server handles them gracefully. However, inducing such failures reliably is
tricky without a controlled test environment.

This tutorial demonstrates how to refactor and test `mxd`’s server components
to **simulate unreliable network conditions**. The approach introduces a
transport abstraction to inject failures and uses `tokio-test::io::Builder` for
custom I/O streams. `rstest` is leveraged for parameterised tests and `mockall`
is used for mocking where appropriate. The outcome is a suite of tests that
ensures `mxd`’s server remains stable even when the network is not.

## Overview of `mxd`’s Server Networking

Before diving into the solution, let’s quickly review how `mxd` currently
handles network I/O in its server:

- **Connection Acceptance:** The `main.rs` file defines an asynchronous
  `accept_connections` loop. It listens on a `TcpListener` and spawns a task to
  handle each accepted `TcpStream`. If `listener.accept()` fails, it logs an
  error and continues.

- **Client Handling:** The core connection logic is in `handle_client` (called
  in each spawned task). This function performs an **initial handshake** with a
  client, then enters a loop to read and process transactions. Notably,
  `handle_client`:

  - Uses `tokio::io::split` to split the `TcpStream` into a reader and writer.

  - Reads a 12-byte handshake from the client with a **5-second timeout**. If
    the client doesn’t send the handshake in time, a timeout error code is sent
    to the client and the connection is closed.

  - Validates the handshake bytes using the `protocol` module (checking a “TRTP”
    protocol ID and version). If invalid, an error code is sent and the
    connection ends.

  - On successful handshake, sends back a handshake OK reply and proceeds.

- **Transaction Loop:** After handshake, `handle_client` creates a
  `TransactionReader` and `TransactionWriter` (from the `transaction` module)
  to handle the message framing. It then loops with `tokio::select!`, awaiting
  either:

  1. **Incoming Transaction:** `tx_reader.read_transaction()` which reads the
     next complete request frame (possibly composed of multiple fragments). If
     a transaction is received, it calls `handler::handle_request` to produce a
     response and writes the response back with `tx_writer.write_transaction`.

  1. **Shutdown Signal:** A shared shutdown `watch` channel to break the loop on
     server shutdown.

  The loop’s error handling is important for our tests:

  - If `read_transaction` returns an **I/O error** of kind *UnexpectedEof*
    (meaning the client closed the connection), the loop breaks **without
    error** (graceful termination).

  - If any other error occurs (e.g. a parsing error, timeout, or a non-EOF I/O
    error), `handle_client` returns an `Err` to indicate a connection error.

- **I/O Timeouts:** The `transaction` module imposes a default **5 second I/O
  timeout** on every read/write operation via `IO_TIMEOUT`. It wraps
  `AsyncReadExt::read_exact` and `AsyncWriteExt::write_all` calls in
  `tokio::time::timeout(...)`. For example, reading a frame header uses:

  ```rust
  timeout(timeout_dur, r.read_exact(&mut hdr_buf)).await
      .map_err(|_| TransactionError::Timeout)??;

  ```

  which yields a `TransactionError::Timeout` on elapsed time, or propagates any
  underlying I/O error (like EOF) as `TransactionError::Io`. Similarly, frame
  writes are done with `timeout(..., write_all(...))`. These timeouts will be a
  focus when simulating slow or stalled connections.

With this understanding, we can see the points where network issues manifest:

- **Handshake stage:** potential timeout or malformed data.

- **Reading transactions:** timeouts (no data), unexpected EOF (client drop), or
  other errors (connection reset).

- **Writing responses:** errors like broken pipe if the client disconnects
  mid-write, or partial writes.

Our goal is to *simulate these conditions in tests*. Next, we’ll refactor the
code to allow injecting a fake transport, and then write tests for each failure
scenario.

## Introducing a Testable Transport Abstraction

Currently, `handle_client` is tied to a real `TcpStream`. To test network
failures, the function must run with a *simulated stream*. The design abstracts
the transport layer behind a trait or generics, enabling tests to substitute a
mock stream object.

**Refactoring** `handle_client`**:** A straightforward approach is to make
`handle_client` generic over the stream’s reader and writer. The Tokio docs
suggest writing connection handlers as functions parameterized by
`AsyncRead`/`AsyncWrite` implementors, rather than hard-coding `TcpStream`. We
can apply this by splitting the logic:

1. **Split at the call site:** In `accept_connections`, instead of calling
   `handle_client(socket, ...)` directly, we first split the socket and then
   call a new generic handler. For example:

   ```rust
   let (reader, writer) = tokio::io::split(socket);
   client_handler(reader, writer, peer, pool, shutdown_rx).await;

   ```

   where `client_handler` is our new generic function.

2. **Define** `client_handler` **with trait bounds:** It will take any `Reader`
   and `Writer` that implement the async read/write traits:

   ```rust
   async fn client_handler<R, W>(
       mut reader: R,
       mut writer: W,
       peer: SocketAddr,
       pool: DbPool,
       mut shutdown: watch::Receiver<bool>,
   ) -> Result<()> 
   where 
       R: AsyncRead + Unpin,
       W: AsyncWrite + Unpin, 
   {
       // ... perform handshake, then transaction loop ...
   }

   ```

   Inside, we use `TransactionReader::new(reader)` and
   `TransactionWriter::new(writer)` just as before – since those types are
   generic over any `AsyncRead`/`AsyncWrite`, this works seamlessly. The
   handshake logic will use the provided `reader` and `writer` as well.

With this change, `client_handler` no longer assumes a real network
`TcpStream`; any in-memory or mock stream can be supplied for testing.
**Importantly**, production code retains full functionality – actual TCP
listeners/streams still hand off to the generic handler. The refactor maintains
the same behaviour while enabling injection of test streams.

*Example – generic handler signature:*

```rust
use tokio::io::{AsyncRead, AsyncWrite};

async fn client_handler<R, W>(
    mut reader: R,
    mut writer: W,
    peer: SocketAddr,
    pool: DbPool,
    mut shutdown: watch::Receiver<bool>,
) -> anyhow::Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    // [Handshake phase]
    let mut buf = [0u8; protocol::HANDSHAKE_LEN];
    match tokio::time::timeout(protocol::HANDSHAKE_TIMEOUT, reader.read_exact(&mut buf)).await {
        Ok(Ok(_)) => { /* received handshake bytes */ }
        Ok(Err(e)) => {
            if e.kind() == io::ErrorKind::UnexpectedEof {
                return Ok(()); // client disconnected during handshake
            }
            return Err(e.into()); // other handshake read error
        }
        Err(_) => {
            // timeout occurred
            protocol::write_handshake_reply(&mut writer, protocol::HANDSHAKE_ERR_TIMEOUT).await?;
            return Ok(());
        }
    }
    // ... (validate handshake bytes and send reply or error) ...
    protocol::write_handshake_reply(&mut writer, protocol::HANDSHAKE_OK).await?;

    // [Transaction loop phase]
    let mut tx_reader = TransactionReader::new(reader);
    let mut tx_writer = TransactionWriter::new(writer);
    let ctx = HandlerContext::new(peer, pool);
    let mut session = Session::default();
    loop {
        tokio::select! {
            result = tx_reader.read_transaction() => match result {
                Ok(request_tx) => {
                    let frame_bytes = request_tx.to_bytes();
                    let response_tx = handle_request(&ctx, &mut session, &frame_bytes).await
                        .map_err(|e| anyhow::anyhow!(e))?; 
                    tx_writer.write_transaction(&response_tx).await?;
                }
                Err(TransactionError::Io(ref e)) if e.kind() == io::ErrorKind::UnexpectedEof => {
                    break; // client closed connection gracefully
                }
                Err(e) => {
                    // Log or propagate other errors (timeout, parse error, etc.)
                    return Err(e.into());
                }
            },
            _ = shutdown.changed() => {
                break; // server is shutting down
            }
        }
    }
    Ok(())
}
```

In the above pseudocode, we essentially mirrored the logic from
`handle_client`, but on generic `reader`/`writer`. This refactoring sets the
stage for injecting **simulated failures** in tests by providing custom
`reader`/`writer` types.

## Simulating Network Failures with `tokio-test::io::Builder`

With the transport abstracted, we can create **dummy streams** to simulate
various network outage scenarios. Tokio’s testing utilities include
`tokio_test::io::Builder`, which allows building an object that implements
`AsyncRead` and `AsyncWrite` with predetermined behaviour. We can script a
sequence of reads/writes and even inject errors.

For example, the Tokio documentation demonstrates using `Builder` to simulate a
simple echo conversation by preloading expected inputs and outputs. We will use
a similar approach for failure scenarios.

**1. Simulating a Handshake Timeout:** In this scenario, the client connects
but **never sends the handshake bytes**, causing the server’s 5-second timeout
to elapse. Testing without a real 5-second delay is possible by pausing the
Tokio clock in tests via `#[tokio::test(start_paused = true)]`, then advancing
the clock programmatically to trigger the timeout.

Using `Builder`, we create a `reader` that yields **no data at all** (so the
server will be stuck waiting), and after advancing time past 5 seconds, the
handshake read future will time out. We also set up a `writer` to capture the
handshake timeout error reply the server should send.

```rust
use tokio_test::io::Builder;
use std::io;
#[rstest]
#[tokio::test(start_paused = true)]
async fn handshake_times_out() {
    // The server expects 12-byte handshake, but client sends nothing.
    let test_reader = Builder::new()
        .build(); // no .read() means EOF or hang
    let expected_reply = protocol::PROTOCOL_ID.iter().chain(&protocol::HANDSHAKE_ERR_TIMEOUT.to_be_bytes()).copied().collect::<Vec<u8>>();
    let test_writer = Builder::new()
        .write(&expected_reply)  // expect the server to write a timeout handshake reply
        .build();
    let peer = "127.0.0.1:0".parse().unwrap();
    let pool = dummy_db_pool(); // (Use an in-memory or mock DB pool if needed)
    let (_shutdown_tx, shutdown_rx) = watch::channel(false);
    
    // Run the handler with the test reader/writer
    let result = client_handler(test_reader, test_writer, peer, pool, shutdown_rx).await;
    assert!(result.is_ok());
    // At this point, no handshake received. Advance time to trigger timeout.
    tokio::time::advance(protocol::HANDSHAKE_TIMEOUT + Duration::from_secs(1)).await;
    // The server should have sent a handshake timeout error and closed the connection.
}
```

In the above test, `Builder::new().build()` for the reader yields an I/O object
that returns EOF immediately on reads (since no `.read` is queued). The
server’s `read_exact` will wait, but after we advance the virtual clock 5+
seconds, the `timeout` will return `Err`, causing the server to write a timeout
error reply. We expect the reply to be 8 bytes (`"TRTP"` + error code 3), which
we queued as an expected write. The `test_writer` is configured with
`.write(&expected_reply)` to assert that those exact bytes are written. If the
server fails to write this or writes different bytes, the test will fail.
Finally, we assert that `client_handler` returned `Ok(())` – it should return
normally after handling the timeout (not as an error).

**2. Simulating an Invalid Handshake:** Here, the client does send data, but
it’s an incorrect handshake (e.g., wrong protocol ID). We expect the server to
detect this and send an error reply with code `HANDSHAKE_ERR_INVALID`, then end
the connection. Using `Builder`:

```rust
#[rstest]
#[tokio::test]
async fn handshake_invalid_protocol() {
    // Prepare a 12-byte handshake with an incorrect protocol ID "WRNG" instead of "TRTP".
    let mut bad_handshake = *b"WRNG\0\0\0\0\0\0\0\0";
    // Ensure version bytes are correct to isolate protocol ID error
    bad_handshake[8..10].copy_from_slice(&protocol::VERSION.to_be_bytes());
    let test_reader = Builder::new()
        .read(&bad_handshake)  // client sends the bad handshake and then EOF
        .read_eof()            // simulate client closing right after
        .build();
    // Server should write a handshake invalid error reply ("TRTP" + code=1)
    let expected_code = protocol::HANDSHAKE_ERR_INVALID.to_be_bytes();
    let expected_reply = [b'T', b'R', b'T', b'P', expected_code[0], expected_code[1], expected_code[2], expected_code[3]];
    let test_writer = Builder::new()
        .write(&expected_reply)
        .build();
    let result = client_handler(test_reader, test_writer, peer, pool, shutdown_rx).await;
    assert!(result.is_ok());
}
```

In this test, we queue the handshake bytes `"WRNG..."` as the reader input. The
server’s `parse_handshake` will return `HandshakeError::InvalidProtocol`.
According to `handle_client`, this triggers sending an error reply with code=1
and returning `Ok(())`. Our `test_writer` expects exactly those 8 bytes. We
also appended `.read_eof()` after the handshake bytes to indicate the client
closed the connection (this ensures the server’s next read sees EOF instead of
hanging). The test verifies that `client_handler` completes without propagating
an error (it handled the invalid handshake gracefully).

**3. Simulating Client Disconnect During Handshake:** If a client drops the
connection midway through the handshake (e.g., sends nothing or partial
handshake then disconnects), the server’s `reader.read_exact` will return an
`UnexpectedEof` error immediately. The code treats an EOF during handshake as a
normal early disconnect and returns `Ok` (no reply sent). We can simulate this
by having the test reader immediately return EOF (without sending any bytes):

```rust
#[tokio::test]
async fn handshake_client_disconnect() {
    let test_reader = Builder::new()
        .read_eof()  // simulate immediate EOF on first read attempt
        .build();
    let test_writer = Builder::new().build();
    let result = client_handler(test_reader, test_writer, peer, pool, shutdown_rx).await;
    assert!(result.is_ok());
    // Nothing should be written, handshake was never completed
}
```

Here, `test_writer` expects no writes (we didn’t call `.write()` on it). If the
server mistakenly attempted to send something, the test would catch an
unexpected write. We assert the handler returns `Ok`, meaning it handled the
disconnect silently.

**4. Simulating a Read Timeout During Transactions:** After a successful
handshake, if the client stops sending data in the middle of a transaction, the
server’s `TransactionReader` will eventually hit the 5-second `IO_TIMEOUT` on a
`read_exact`. This produces `TransactionError::Timeout` which propagates out of
`read_transaction`. In the `handle_client` loop, any error that isn’t an EOF
leads to an `Err` return. We want to test that a stalled connection causes a
timeout error and that our code handles it as expected (likely logging and
closing the connection).

Simulating this involves a two-part interaction:

- **Handshake phase:** Send a valid handshake to proceed.

- **Transaction phase:** Send a partial transaction (e.g., send only a frame
  header indicating more data to come, then stall).

Using `Builder`, we can script the reader to first provide a correct handshake,
then provide one frame header and no payload. For instance, suppose we craft a
frame header with `total_size = 100` and `data_size = 50` for the first
fragment, but we never send the remaining fragment bytes. The server will read
the header and 50 bytes of payload, then expect another frame (because
`remaining = total_size - data_size` is not zero). If we don’t send the next
fragment, `read_frame` will timeout on the next `read_exact` for the header of
fragment 2.

Rather than actually waiting 5 seconds, we can again use `start_paused` and
advance time. For brevity, we may pseudo-code this test:

```rust
#[tokio::test(start_paused = true)]
async fn transaction_read_timeout() {
    // 1. Handshake: send a correct 12-byte handshake
    let handshake = build_valid_handshake_bytes();
    // 2. First transaction frame: send a header claiming two fragments
    let mut hdr = [0u8; transaction::HEADER_LEN];
    let total_size: u32 = 100;
    let first_frag_size: u32 = 50;
    // construct FrameHeader fields (flags=0, is_reply=0, type=1, id=42, error=0)
    let frame_header = FrameHeader {
        flags: 0, is_reply: 0, ty: 1, id: 42, error: 0,
        total_size, data_size: first_frag_size
    };
    frame_header.write_bytes(&mut hdr);
    // Provide the frame header and 50 bytes of payload
    let payload_fragment = vec![0u8; first_frag_size as usize];
    let test_reader = Builder::new()
        .read(&handshake)
        .read(&hdr)
        .read(&payload_fragment)
        // do not send the second fragment, just stall...
        .build();
    let test_writer = Builder::new()
        .write(build_handshake_ok_reply())  // expect handshake OK reply from server
        // (no further writes expected, since we anticipate a timeout before any response)
        .build();

    let result_fut = client_handler(test_reader, test_writer, peer, pool, shutdown_rx);
    // Advance time by 5+ seconds to trigger the transaction read timeout
    tokio::time::pause();
    tokio::pin!(result_fut);
    tokio::time::advance(transaction::IO_TIMEOUT + Duration::from_secs(1)).await;
    // Now poll the future to completion
    let result = result_fut.await;
    assert!(result.is_err());
    if let Err(e) = result {
        assert!(e.to_string().contains("I/O timeout"));  // the error should indicate a timeout
    }
}
```

In this test, after sending one fragment, the server will be awaiting the next
fragment. By advancing the clock past `IO_TIMEOUT` (5s) with no more data, the
next `read_frame` call should time out, causing `read_transaction` to return
`TransactionError::Timeout`. Our handler then returns an error (which we
assert). We also ensure the handshake was completed successfully by expecting
the handshake OK reply on the writer.

**5. Simulating Connection Reset During Read:** A connection reset (e.g., TCP
RST) would typically surface as an I/O error other than EOF. We can simulate
this by configuring the test reader to return an error on read. For example,
using `Builder`’s ability to inject errors:

```rust
use std::io::ErrorKind;
let test_reader = Builder::new()
    .read(&handshake_bytes)       // send valid handshake
    .read_error(io::Error::new(ErrorKind::ConnectionReset, "conn reset"))
    .build();
```

Here, after the handshake, any attempt by the server to read further will
immediately get a `ConnectionReset` error. In `handle_client`, this is caught
by the generic `Err(e)` arm (not EOF), and the function will return an error.
Assert that the result is an `Err` matching the expected kind.

**6. Simulating Partial Write Failures:** So far, we’ve focused on read-side
issues. But what if the server fails while **writing** to the client (for
instance, the client disconnects just as the server sends a response)? In such
a case, `TransactionWriter::write_transaction` might return an error (likely a
broken pipe). Our handler would propagate that error out.

To test this, we need a writer that simulates an error on write. The `Builder`
can expect writes and also inject errors. One strategy is to have the writer
expect part of a write and then error out on the next write. However, since
`TransactionWriter` uses `write_all` internally, it will loop until all bytes
are written or an error occurs. We can force an error on the *first* write call
to simulate an immediate failure. For example:

```rust
let test_writer = Builder::new()
    .write_error(io::Error::new(ErrorKind::BrokenPipe, "broken pipe"))
    .build();
```

If we run a scenario where the server will definitely attempt a write (e.g.,
after a successful handshake and processing a transaction), this writer will
cause that write to fail.

*Example test for write failure:* Suppose we send a valid handshake and then a
valid request transaction. We set the writer to error on the response write. We
expect `client_handler` to return an error. For conciseness, here’s a sketch:

```rust
#[tokio::test]
async fn response_write_failure() {
    let test_reader = Builder::new()
        .read(&handshake_bytes)
        .read(&one_valid_request_frame_bytes)
        .read_eof()  // no more data from client
        .build();
    let test_writer = Builder::new()
        .write(&handshake_ok_reply_bytes)      // expect handshake OK
        .write_error(io::Error::new(ErrorKind::BrokenPipe, "pipe")) // fail on writing response
        .build();
    let result = client_handler(test_reader, test_writer, peer, pool, shutdown_rx).await;
    assert!(result.is_err());
}
```

In this test, after reading the request, the server will call
`tx_writer.write_transaction(&resp)`. The first `.write_all` on the handshake
reply succeeded, but the next `.write_all` for the response triggers our
injected BrokenPipe error. This error propagates and we assert that
`client_handler` returns `Err`. (We don’t necessarily need to assert on the
exact error kind in the result, but we could.)

By combining these techniques – **custom readers/writers via**
`tokio-test::io::Builder` and **Tokio’s paused time** – we create a
deterministic test suite for network failures.

## Parameterizing Tests with `rstest`

The preceding examples reveal a common pattern of setup and assertion across
scenarios. Leveraging the `rstest` crate avoids repetitive code by
parameterising the scenarios. The `#[rstest]` attribute defines multiple cases
for a single test function.

For instance, we might create a single test function
`test_network_outage_scenarios` with parameters indicating the scenario type.
Each case would configure the test reader/writer and expected outcome
accordingly:

```rust
use rstest::rstest;
enum Scenario { HandshakeTimeout, HandshakeInvalid, ReadTimeout, ConnReset, WriteFail }

#[rstest(scenario, 
    case(Scenario::HandshakeTimeout),
    case(Scenario::HandshakeInvalid),
    case(Scenario::ReadTimeout),
    case(Scenario::ConnReset),
    case(Scenario::WriteFail),
)]
#[tokio::test(start_paused = true)]
async fn test_network_outage_scenarios(scenario: Scenario) {
    let peer = "127.0.0.1:0".parse().unwrap();
    let pool = dummy_db_pool();
    let (_tx, rx) = watch::channel(false);
    let (test_reader, test_writer, should_error) = match scenario {
        Scenario::HandshakeTimeout => {
            let r = Builder::new().build();
            let w = Builder::new()
                .write(&timeout_error_reply_bytes)
                .build();
            (r, w, /*expect_err=*/ false)
        }
        Scenario::HandshakeInvalid => {
            // ... configure r and w as in previous example ...
            (r, w, false)
        }
        Scenario::ReadTimeout => {
            // ... configure r and w for transaction timeout scenario ...
            (r, w, /*expect_err=*/ true)
        }
        Scenario::ConnReset => {
            // ... configure r to return ErrorKind::ConnectionReset after handshake ...
            (r, w, true)
        }
        Scenario::WriteFail => {
            // ... configure writer to fail on response write ...
            (r, w, true)
        }
    };
    let result = client_handler(test_reader, test_writer, peer, pool, rx).await;
    if should_error {
        assert!(result.is_err(), "Expected error, but got Ok");
    } else {
        assert!(result.is_ok(), "Expected Ok, but got Err: {:?}", result.err());
    }
}
```

In the snippet above, each `case(...)` macro provides a different `Scenario`
variant. The test builds the appropriate `test_reader`/`test_writer` and then
invokes `client_handler`. A `should_error` flag asserts the expected outcome.
This single parametrised test replaces multiple individual tests, reducing
duplication. All scenarios still run in isolation with distinct setups, thanks
to `rstest`.

## Using `mockall` for Additional Flexibility

While `tokio-test::io::Builder` covers most needs, there are situations where
explicit mocking might be useful. The `mockall` crate can generate mocks for
our abstractions. For example, if we had defined a trait
`trait Transport: AsyncRead + AsyncWrite + Unpin {}` (or a trait with specific
async methods for read/write), we could use `mockall` to create a
`MockTransport` and program its behaviour (return errors on certain calls,
etc.).

However, mocking `AsyncRead/Write` directly can be complex. An easier target
for mocking might be higher-level components:

- **Accept Loop Simulation:** We could define a trait for the listener:

  ```rust
  trait Listener {
      async fn accept(&self) -> io::Result<(Box<dyn Stream>, SocketAddr)>;
  }
  ```

  Using `mockall`, we could simulate a listener that returns a predefined
  sequence of connections or errors. This way, one could test how
  `accept_connections` reacts to, say, a series of successful accepts followed
  by an error or immediate shutdown. For instance, a mock listener could be set
  to return one `MockTransport` (representing a client) and then an `Err` to
  simulate a network interface error. The test would then verify that
  `accept_connections` logs the error and continues or exits properly.

- **Isolating Business Logic:** In our `client_handler` tests above, we mostly
  ignored the actual `handle_request` logic by using dummy minimal
  transactions. If we wanted to focus purely on the network layer and not
  depend on real database calls or command processing, we could abstract the
  request handling. For example, introduce an interface:

  ```rust
  trait RequestHandler {
      async fn handle(&mut self, req: Transaction) -> Result<Transaction>;
  }
  ```

  implement it with the real logic for production, and use a mock in tests that
  just returns a canned response. This way, in a test for a write failure, we
  don’t invoke the real DB or commands at all – the mock could simply return a
  simple “OK” response transaction when called. Then we only simulate the
  network failing on sending that response. Such a mock ensures our test is
  laser-focused on networking behaviour.

In summary, **use** `mockall` **when stubbing out parts of the system that are
not the primary target of the test**. For testing network outages in `mxd`, the
tests largely rely on `tokio-test` to simulate the transport. However, if, for
example, database access or external services were intertwined with the network
handling, mocking them out would be essential to create repeatable unit tests.

## Async Testing Best Practices and Final Thoughts

Our refactoring and tests align with async Rust best practices in several ways:

- **Deterministic Timing:** By using `#[tokio::test(start_paused)]` and
  controlling the clock, we avoid making tests that actually sleep for seconds.
  This speeds up the test suite and avoids flakiness due to timing. Always
  ensure that any time-based logic (like `timeout` calls) in the code can be
  fast-forwarded in tests.

- **Trait-Based Abstraction:** Introducing a trait or generic interface for the
  transport layer follows the dependency-inversion principle. It not only makes
  testing easier but also means the server code could be extended to other
  transport types (imagine swapping `TcpStream` with a TLS stream or an
  in-memory channel) without changing the core logic. This decoupling is a win
  for maintainability.

- **Using In-Memory Channels:** We saw that `tokio::io::duplex` was already used
  in `mxd`’s own tests for normal conditions. We built on that idea with
  `tokio-test::io::Builder` to handle error cases. Both provide lightweight
  in-process channels that behave like network streams, which is far preferable
  to spawning real sockets in unit tests.

- **Granular Testing:** Rather than trying to simulate a full multi-connection
  environment at once, we tested one connection at a time in various failure
  modes. This isolates issues and makes tests simpler. The use of `JoinSet` in
  `accept_connections` and multi-task concurrency is tested indirectly by these
  unit tests, though integration tests or an end-to-end test (spinning up the
  server and connecting with a real socket) may also be considered for
  additional confidence. Those, however, are typically slower and less
  deterministic, so unit tests like we’ve written are invaluable for covering
  edge cases.

Following this tutorial enables confident extension of the `mxd` test suite. We
demonstrated how to simulate timeouts, abrupt disconnects, and I/O errors for
both reads and writes. With parameterized tests and careful use of mocks, the
server’s resilience under adverse network conditions can be validated
thoroughly. This not only prevents regressions but also documents the intended
behaviour (for example, that a timeout should result in a specific error code
to the client, or that an EOF is treated as a graceful shutdown).

**In conclusion**, testing for network outages in async Rust requires a mix of
clever abstractions and tools:

- **Refactor for testability:** ensure the code accepts fake implementations of
  I/O.

- **Tokio’s test tools** simulate I/O and control time.

- **Rstest and mockall** keep tests clean, avoid repetition, and isolate
  concerns.

With these techniques, `mxd`’s server is well-equipped to handle the messy
reality of networks, and we have high-confidence tests to prove it.
