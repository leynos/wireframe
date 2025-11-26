# Connection Preamble Validation

`wireframe` supports an optional connection preamble that is read as soon as a
client connects. The server decodes the preamble with
[`read_preamble`](../src/preamble.rs) and can invoke user-supplied callbacks on
success or failure. The helper uses `bincode` to decode any type implementing
`bincode::BorrowDecode` and reads exactly the number of bytes required.

The flow is summarized below:

```mermaid
sequenceDiagram
    participant Client
    participant Server
    participant PreambleDecoder
    participant SuccessCallback
    participant FailureCallback

    Client->>Server: Connects and sends preamble bytes
    Server->>PreambleDecoder: Reads and decodes preamble
    alt Decode success
        PreambleDecoder-->>Server: Decoded preamble (T)
        Server->>SuccessCallback: Invoke with preamble data
        SuccessCallback-->>Client: Optional response
    else Decode failure
        PreambleDecoder-->>Server: DecodeError
        Server->>FailureCallback: Invoke with error
    end
    Server-->>Client: (Continues or closes connection)
```

The success callback receives the decoded preamble and a mutable `TcpStream`.
It may write a handshake response before the connection is passed to
`WireframeApp`. The failure callback is also asynchronous and receives the
mutable stream so it can emit an error reply before the connection closes. Use
`preamble_timeout` to cap how long the read may take; the timeout follows the
failure callback path. In the tests, a `HotlinePreamble` struct illustrates the
pattern, but any preamble type may be used. Register callbacks via
`on_preamble_decode_success` and `on_preamble_decode_failure` on
`WireframeServer`.

## Call Order

`WireframeServer::with_preamble::<T>()` must be called **before** registering
callbacks with `on_preamble_decode_success` or `on_preamble_decode_failure`.
The method converts the server to use a custom preamble type, dropping any
callbacks configured on the default `()` preamble. Registering callbacks after
calling `with_preamble::<T>()` ensures they are retained.

## Preamble processing flow with timeout handling

Sequence diagram showing how the accept loop, connection task, preamble
decoding, timeout handling, and callbacks coordinate before handing the
connection to the application:

```mermaid
sequenceDiagram
    actor Client
    participant WireframeServer
    participant AcceptLoop
    participant ConnectionTask
    participant ProcessStream
    participant ReadPreamble
    participant PreambleFailureHandler
    participant WireframeApp

    Client->>WireframeServer: connect
    WireframeServer->>AcceptLoop: start accept_loop
    AcceptLoop->>AcceptLoop: listener.accept()
    AcceptLoop->>ConnectionTask: spawn_connection_task(stream, factory, hooks)

    ConnectionTask->>ProcessStream: process_stream(stream, peer_addr, factory, on_success, on_failure, preamble_timeout)

    alt preamble_timeout is Some
        ProcessStream->>ProcessStream: timeout(preamble_timeout, read_preamble)
        ProcessStream->>ReadPreamble: read_preamble(stream)
        alt preamble read completes in time
            ReadPreamble-->>ProcessStream: Ok(preamble, leftover)
        else preamble read times out
            ProcessStream-->>ProcessStream: Err(timeout_error)
        end
    else preamble_timeout is None
        ProcessStream->>ReadPreamble: read_preamble(stream)
        ReadPreamble-->>ProcessStream: Result(preamble or error)
    end

    alt preamble_result is Ok
        ProcessStream->>ProcessStream: invoke on_success if Some
        opt on_success is Some
            ProcessStream->>WireframeApp: on_preamble_success(preamble, stream)
            WireframeApp-->>ProcessStream: Result
        end
        ProcessStream->>WireframeApp: hand off stream and preamble
    else preamble_result is Err (decode failure or timeout)
        alt on_failure is Some
            ProcessStream->>PreambleFailureHandler: on_preamble_failure(error, stream)
            PreambleFailureHandler-->>ProcessStream: io::Result
            ProcessStream->>Client: optional protocol error reply via stream
            ProcessStream->>Client: close connection
        else on_failure is None
            ProcessStream->>ProcessStream: log error with peer_addr
            ProcessStream->>Client: close connection
        end
    end
```
