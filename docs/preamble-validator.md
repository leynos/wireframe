# Connection Preamble Validation

`wireframe` supports an optional connection preamble that is read as soon as a
client connects. The server decodes the preamble with
[`read_preamble`](../src/preamble.rs) and can invoke user-supplied callbacks on
success or failure. The helper uses `bincode` to decode any type implementing
`bincode::Decode` and reads exactly the number of bytes required.

The flow is summarised below:

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
    else Decode failure
        PreambleDecoder-->>Server: DecodeError
        Server->>FailureCallback: Invoke with error
    end
    Server-->>Client: (Continues or closes connection)
```

In the tests a `HotlinePreamble` struct illustrates the pattern, but any
preamble type may be used. Register callbacks via `on_preamble_decode_success`
and `on_preamble_decode_failure` on `WireframeServer`.
