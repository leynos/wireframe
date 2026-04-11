# Inventory of `Frame = Vec<u8>` trait bounds and APIs

This document records the current code paths, trait bounds, implementations,
and documentation that still assume `Frame = Vec<u8>` or otherwise expose owned
`Vec<u8>` values as the de facto frame or payload representation. Issue 287
uses this inventory to support epic 284 by bounding the migration scope without
prescribing a single implementation sequence.[^adr-004]

## Scope and taxonomy

This inventory distinguishes four kinds of coupling:

| Category          | Meaning                                                                                              | Migration sensitivity                                 |
| ----------------- | ---------------------------------------------------------------------------------------------------- | ----------------------------------------------------- |
| `frame-bound`     | A trait, impl, or test uses `Vec<u8>` as a concrete frame type.                                      | High when the transport frame type changes.           |
| `payload-bound`   | A public API exposes owned payload bytes as `Vec<u8>` without literally fixing `F::Frame = Vec<u8>`. | High for API compatibility, lower for codec plumbing. |
| `internal-only`   | Runtime code carries `Vec<u8>` internally without exposing it as a stable API.                       | Medium; usually easier to change behind shims.        |
| `docs/tests-only` | Examples, tests, and design text that still teach `Vec<u8>`-shaped framing.                          | Low runtime risk, but high drift risk.                |

Two important boundaries are out of scope for this inventory:

- General `Vec<u8>` usage that is clearly about message bodies, fragment
  payloads, or unrelated buffering rather than transport frames or payload
  ownership hand-offs.
- Selecting `Bytes`, `BytesMut`, or another abstraction as the replacement.
  This document records coupling and trade-offs only.

## Executive summary

The codebase no longer hard-codes `F::Frame = Vec<u8>` at the top-level codec
or protocol abstraction. `FrameCodec` is generic over `Self::Frame`, and
`WireframeProtocol` is generic over `type Frame: FrameLike`.[^codec][^hooks]
The main remaining migration scope is instead concentrated in public
payload-owned APIs:

- `PacketParts`, `Envelope`, `ServiceRequest`, and `ServiceResponse` all own
  `Vec<u8>` payloads or frames.
- Client request hooks and preamble replay APIs still expose mutable or owned
  `Vec<u8>` buffers.
- `Serializer::serialize` still returns `Vec<u8>`, which keeps outbound
  serialization byte-owned even when codecs use `Bytes`.

The main adjacent constraint is not a literal `Vec<u8>` bound. The
`ConnectionActor` requires `F: FrameLike + CorrelatableFrame + Packet`, which
the default codec frame type (`Bytes`) does not satisfy.[^unified-path] This is
why the current server path routes actor output through `Envelope` and the
codec driver rather than sending transport frames through the actor directly.

## Inventory summary

| Category          | Representative surfaces                                         | Primary files                                                                                          |
| ----------------- | --------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------ |
| `frame-bound`     | `CorrelatableFrame for Vec<u8>`, test-only `Packet for Vec<u8>` | `src/correlation.rs`, `src/connection/test_support.rs`                                                 |
| `payload-bound`   | Packets, middleware, client request hooks, serializer output    | `src/app/envelope.rs`, `src/middleware.rs`, `src/client/hooks.rs`, `src/serializer.rs`                 |
| `internal-only`   | DLQ sender, response forwarding, preamble replay                | `src/app/builder/core.rs`, `src/app/frame_handling/response.rs`, `src/rewind_stream.rs`                |
| `docs/tests-only` | Protocol tests, guide examples, design diagrams                 | `tests/wireframe_protocol.rs`, `docs/users-guide.md`, `docs/asynchronous-outbound-messaging-design.md` |

## Confirmed runtime surfaces

### True `frame-bound` surfaces

- `src/correlation.rs` implements `CorrelatableFrame` for `Vec<u8>`.
  This makes raw byte vectors participate directly in generic frame plumbing,
  even though the implementation is a no-op correlation carrier.
- `src/connection/test_support.rs` implements `Packet` for `Vec<u8>` in test
  support. This is not production API, but it proves the connection actor stack
  still treats `Vec<u8>` as a first-class frame type in its generic harnesses.

These are the only confirmed runtime code paths where `Vec<u8>` is promoted to
an actual frame type today. The public codec and protocol traits themselves do
not require it.

### Public `payload-bound` surfaces

#### Packets and envelopes

- `src/app/envelope.rs` defines `PacketParts` with
  `payload: Vec<u8>`, `PacketParts::new(..., payload: Vec<u8>)`, and
  `PacketParts::into_payload(self) -> Vec<u8>`.
- The same file defines `Envelope` with `payload: Vec<u8>` and
  `Envelope::new(id, correlation_id, payload: Vec<u8>)`.
- `Packet` itself is payload-oriented through
  `fn into_parts(self) -> PacketParts` and
  `fn from_parts(parts: PacketParts) -> Self`.

These are not transport-frame constraints, but they are high-sensitivity public
APIs because routing, middleware reconstruction, and packet rebuilding all
depend on owned payload bytes.

#### Middleware request and response wrappers

- `src/middleware.rs` fixes `ServiceRequest` to
  `inner: FrameContainer<Vec<u8>>`.
- `ServiceRequest::new(frame: Vec<u8>, correlation_id)` and
  `ServiceRequest::frame_mut(&mut self) -> &mut Vec<u8>` expose owned, mutable
  bytes directly to middleware.
- `ServiceResponse` mirrors the same contract with
  `inner: FrameContainer<Vec<u8>>`,
  `ServiceResponse::new(frame: Vec<u8>, ...)`,
  `frame_mut(&mut self) -> &mut Vec<u8>`, and `into_inner(self) -> Vec<u8>`.

This is one of the clearest byte-ownership APIs in the crate. Any migration
that changes the type here becomes a user-visible middleware API change.

#### Client request hooks

- `src/client/hooks.rs` exports
  `BeforeSendHook = Arc<dyn Fn(&mut Vec<u8>) + Send + Sync>`.
- `src/client/builder/request_hooks.rs` exposes the same shape through
  `before_send<F>(...) where F: Fn(&mut Vec<u8>) + Send + Sync + 'static`.
- `src/client/messaging.rs` applies these hooks via
  `invoke_before_send_hooks(&self, bytes: &mut Vec<u8>)`.

This is a payload-bound API rather than a literal `Frame = Vec<u8>` bound, but
it is still a compatibility surface because the client promises mutable access
to serialized outbound bytes before transport framing.

#### Client preamble leftovers

- `src/client/mod.rs` exports `ClientPreambleSuccessHandler<T>` as a callback
  returning `io::Result<Vec<u8>>`.
- `src/client/preamble_exchange.rs` stores and executes that callback through
  `perform_preamble_exchange(...) -> Result<Vec<u8>, ClientError>` and
  `run_preamble_exchange(...) -> Result<Vec<u8>, ClientError>`.
- `src/rewind_stream.rs` then replays those leftover bytes through
  `RewindStream::new(leftover: Vec<u8>, inner)`.

This is another public owned-bytes contract. It is adjacent to transport
framing because the leftover bytes are replayed before framed communication
starts, but the surface is about preamble buffering rather than `F::Frame`.

#### Serializer output

- `src/serializer.rs` defines
  `Serializer::serialize<M>(&self, value: &M) -> Result<Vec<u8>, ...>`.
- `BincodeSerializer` preserves that same return type.

This does not force transport frames to be `Vec<u8>`, because codecs can still
wrap those bytes into `Bytes` or protocol-native frame structs. It does,
however, keep outbound message serialization owned and mutable at the boundary
where several other APIs also expect `Vec<u8>`.

### `internal-only` runtime surfaces

- `src/app/builder/core.rs` stores `push_dlq` as
  `Option<mpsc::Sender<Vec<u8>>>`.
- `src/app/builder/config.rs` exposes that internal channel through
  `with_push_dlq(self, dlq: mpsc::Sender<Vec<u8>>) -> Self`.
- `src/app/frame_handling/response.rs` shows the current payload flow
  explicitly: `ServiceRequest::new(env.payload, env.correlation_id)` moves the
  inbound envelope payload into middleware, and
  `PacketParts::new(env.id, resp.correlation_id(), resp.into_inner())`
  reconstructs an outbound envelope from `Vec<u8>`.

These are lower-risk than the public packet and middleware APIs, but they are
still part of the migration blast radius because they connect the public byte
contracts to the internal send path.

## Adjacent constraints that matter but do not name `Vec<u8>`

These surfaces are essential migration context even though they are not
themselves `Frame = Vec<u8>` bindings:

- `src/connection/mod.rs` requires
  `F: FrameLike + CorrelatableFrame + Packet` for the `ConnectionActor`.
- `src/app/builder/protocol.rs` stores protocols as
  `WireframeProtocol<Frame = F::Frame, ProtocolError = ()>`.
- `src/codec.rs` defines the default `LengthDelimitedFrameCodec` with
  `Bytes` frames, not `Vec<u8>`.

Taken together, these show that the main frame-level tension is between generic
actor requirements and codec frame capabilities, not between the actor and
`Vec<u8>` specifically. The current unified server path therefore routes actor
output through `Envelope` and lets the codec driver perform the final
`wrap_payload` step.[^unified-path]

For epic 284, this means transport-frame generalization and payload-API
generalization should be tracked separately. A code path can be generic over
`F::Frame` while still exposing `Vec<u8>` in middleware, packets, or hooks.

## Tests, examples, and documentation that still teach `Vec<u8>`

### Tests and harnesses

- `tests/wireframe_protocol.rs` defines a test codec with `type Frame = Vec<u8>`
  and a protocol implementation with `type Frame = Vec<u8>`.
- `src/connection/test_support.rs` uses `Packet for Vec<u8>` to drive actor
  tests.

These are not runtime blockers, but they confirm that `Vec<u8>` remains the
default teaching and testing shape for protocol hooks.

### User and design documentation

- `docs/users-guide.md` still includes protocol examples with
  `type Frame = Vec<u8>` and client hook examples using `&mut Vec<u8>`.
- `docs/asynchronous-outbound-messaging-design.md` still contains diagrams and
  narrative that show `WireframeProtocol<Frame = Vec<u8>, ProtocolError = ()>`.
- `docs/wireframe-client-design.md` describes `before_send` as operating on
  `&mut Vec<u8>`.

These documents need follow-up whenever the public byte-oriented APIs change,
otherwise the written guidance will drift from the implementation.

## What is already generic today

Several prominent abstractions already distinguish transport frames from owned
payload bytes:

- `FrameCodec` is generic over `Self::Frame` and already supports `Bytes` and
  protocol-native frame structs.[^codec]
- `WireframeProtocol` is generic over `type Frame: FrameLike`, so the protocol
  trait itself is not coupled to `Vec<u8>`.[^hooks]
- `ProtocolHooks<F, E>` is also generic over `F`.

This means epic 284 does not start from a codebase that hard-codes `Vec<u8>`
everywhere. The remaining work is concentrated in the payload-owned surfaces
listed above and in the actor/codec boundary described by the unified
codec-path work.[^unified-path]

## Generalization paths and conceptual risks

The inventory suggests several non-prescriptive workstreams:

- Separate transport-frame work from payload-ownership work. Changing
  `FrameCodec::Frame` does not automatically solve middleware, packet, or
  serializer APIs that still promise owned `Vec<u8>` values.
- Decide whether client hooks should continue to promise mutable owned bytes,
  or whether a future API should distinguish read-only shared bytes from
  editable outbound buffers.
- Treat preamble leftovers as their own compatibility surface. They are not
  regular framed payloads, but the API still exposes owned byte vectors.
- Keep the actor-boundary problem separate from the `Vec<u8>` inventory. The
  `ConnectionActor` currently wants `Packet + CorrelatableFrame`; changing only
  byte container types will not remove that constraint.

The main conceptual risk is overstating the migration scope by treating every
`Vec<u8>` as a frame problem. In practice, most runtime-sensitive surfaces are
about payload ownership and mutability, while true frame-level `Vec<u8>`
coupling is now limited.

## Open questions for epic 284

- Should epic 284 cover only transport-frame substitution, or also public
  payload-owned APIs such as `PacketParts`, middleware, and client hooks?
- Is `Serializer::serialize -> Vec<u8>` still the desired boundary if codecs
  move further toward zero-copy `Bytes` usage?
- Does `CorrelatableFrame for Vec<u8>` still need to exist outside tests once
  protocol and actor examples stop using raw byte vectors?
- Should client preamble leftovers remain owned `Vec<u8>` values, or be
  treated as another buffer abstraction entirely?

## Coordination notes

No matching roadmap item for this inventory work was found in the local docs
set on 2026-04-11, so nothing was marked done. If epic 284 is updated outside
the repository, the prepared link text is:

```plaintext
Add inventory reference: docs/frame-vec-u8-inventory.md
Link label: Frame = Vec<u8> inventory
```

[^adr-004]: See [ADR 004](adr-004-pluggable-protocol-codecs.md) for the
    generic codec decision and the current `Bytes` default frame type.
[^codec]: See
          [Pluggable protocol codecs](execplans/pluggable-protocol-codecs.md)
    and `src/codec.rs`.
[^hooks]: See `src/hooks.rs` and the hook overview in
    [users-guide.md](users-guide.md).
[^unified-path]: See
    [docs/execplans/9-3-1-fragment-adapter-trait.md](execplans/9-3-1-fragment-adapter-trait.md)
    for the actor/codec-driver boundary and why `Bytes` does not flow through
    `ConnectionActor` directly today.
