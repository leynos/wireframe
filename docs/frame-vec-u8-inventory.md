# Inventory of `Frame = Vec<u8>` trait bounds and APIs

This document records the current code paths, trait bounds, implementations,
and documentation that still assume `Frame = Vec<u8>` or otherwise expose owned
`Vec<u8>` values as the de facto frame or payload representation. Issue 287
uses this inventory to support epic 284 by bounding the migration scope without
prescribing a single implementation sequence. Issue 286 extends that work with
a middleware-specific analysis after PR #283 reduced frame-processing
allocations.[^adr-004][^issue-286]

## Scope and taxonomy

This inventory distinguishes four kinds of coupling:

| Category          | Meaning                                                                                                                                  | Migration sensitivity                                 |
| ----------------- | ---------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------- |
| `frame-bound`     | A trait, impl, or test uses `Vec<u8>` as a concrete frame type.                                                                          | High when the transport frame type changes.           |
| `payload-bound`   | A public application programming interface (API) exposes owned payload bytes as `Vec<u8>` without literally fixing `F::Frame = Vec<u8>`. | High for API compatibility, lower for codec plumbing. |
| `internal-only`   | Runtime code carries `Vec<u8>` internally without exposing it as a stable API.                                                           | Medium; usually easier to change behind shims.        |
| `docs/tests-only` | Examples, tests, and design text that still teach `Vec<u8>`-shaped framing.                                                              | Low runtime risk, but high drift risk.                |

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
- Epic 284 should cover both transport-frame substitution and public
  payload-owned APIs, but those should be tracked as separate workstreams under
  one umbrella so consuming developers do not have to write avoidable adapter
  boilerplate.

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

#### Middleware boundary analysis (issue 286)

Issue 286 asks for a focused read on the middleware layer, including its data
flow, impacted integration points, and conceptual adaptation strategies after
PR #283 moved more of the codec path toward `Bytes`.[^issue-286]

##### Middleware data flow

The middleware layer currently sits entirely inside an owned-`Vec<u8>` segment
of the request/response pipeline:

1. `src/app/inbound_handler.rs` builds or reuses the middleware chain through
   `WireframeApp::build_chains()`, wrapping each route handler inside
   `HandlerService<E>`.
2. `src/app/frame_handling/decode.rs` and `src/app/inbound_handler.rs` decode
   an inbound codec frame into an `Envelope`.
3. `src/app/frame_handling/response.rs` crosses the middleware boundary by
   calling `ServiceRequest::new(env.payload, env.correlation_id)`.
4. `src/middleware.rs` exposes that payload to middleware as
   `ServiceRequest`, with `frame()`, `frame_mut()`, `set_correlation_id()`, and
   `into_inner()`.
5. The terminal bridge in `src/middleware.rs` (`RouteService::call`) converts
   the request back into a packet with
   `E::from_parts(PacketParts::new(self.id, req.correlation_id(), req.into_inner()))`,
    invokes the registered handler, then returns
   `ServiceResponse::new(payload, correlation_id)`.
6. `src/app/frame_handling/response.rs` crosses back out of middleware by
   rebuilding `PacketParts` and `Envelope` from `resp.into_inner()`.
7. `src/app/codec_driver.rs` serializes that envelope to bytes via
   `Serializer::serialize`, converts the `Vec<u8>` into `Bytes`, and finally
   hands it to `FrameCodec::wrap_payload`.

The practical consequence is that middleware currently forms a hard
`Vec<u8>`-owned island between generic codec frames on the wire and the
`Bytes`-friendly codec boundary on the way back out.

##### Middleware integration points affected by a frame-type change

- `src/middleware.rs` is the primary public API surface:
  `ServiceRequest`, `ServiceResponse`, `Service`, `Transform`, `Next`,
  `from_fn`, `HandlerService::from_service`, and the `frame_mut()` /
  `into_inner()` editing model all currently depend on `Vec<u8>`.
- `src/app/frame_handling/response.rs` is the request/response bridge into and
  out of middleware. Any type change has to preserve correlation handling and
  packet reconstruction here.
- `src/app/inbound_handler.rs` owns middleware-chain construction and therefore
  any staging or compatibility story for mixed old/new middleware types.
- `tests/middleware.rs` and `tests/middleware_order.rs` demonstrate the current
  mutation semantics directly through `push`, `clear`, and `extend_from_slice`
  on `frame_mut()`.
- `docs/users-guide.md` teaches middleware authors to decode requests from
  `req.frame()` and rewrite replies through `response.frame_mut()`.
- `wireframe_testing/src/helpers/drive.rs` is an adjacent test surface: it
  still feeds raw `Vec<u8>` wire frames into apps, which matters for end-to-end
  middleware tests even though it is not itself the middleware API.

##### Middleware-specific risks

- Blast radius is high because every request passes through the middleware
  wrappers, and external middleware implementations are written against
  `Vec<u8>` constructors and mutation helpers today.
- Switching directly to immutable `Bytes` would break the current editing model
  around `frame_mut()`.
- Switching directly to another mutable buffer type could still impose
  widespread boilerplate if consumers have to manually convert between
  `Vec<u8>`, `Bytes`, and `BytesMut`.

##### Conceptual adaptation strategies

The following strategies are intentionally non-prescriptive, but they bound the
design space:

- Track middleware payload APIs as a separate epic-284 workstream from
  transport-frame substitution. The two interact, but they do not need to land
  in a single breaking change.
- Prefer developer ergonomics over exposing raw buffer taxonomy. Middleware
  authors should not have to hand-write repetitive conversions between byte
  container types to achieve routine inspection and mutation.
- If middleware internals move toward a `Bytes`-compatible representation, keep
  mutation opt-in and edit-oriented. A copy-on-write or edit-on-demand surface
  is conceptually safer than forcing every middleware author to manage shared
  byte ownership directly.
- Preserve the bounded special case for client preamble leftovers. That path is
  one-shot and replay-only, so it does not need to be pulled into the
  middleware or transport-frame redesign unless the broader public byte APIs
  standardize on a new shared abstraction.

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

- `src/app/builder/core.rs` stores `push_dlq` as a dead-letter queue (DLQ)
  sender: `Option<mpsc::Sender<Vec<u8>>>`.
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
  `FrameCodec::Frame` does not automatically solve middleware, packet,
  serializer, or client-hook APIs that still promise owned `Vec<u8>` values.
- Keep the actor-boundary problem separate from the `Vec<u8>` inventory. The
  `ConnectionActor` currently wants `Packet + CorrelatableFrame`; changing only
  byte container types will not remove that constraint.
- Plan for a long-term outbound boundary that is `Bytes`-compatible rather than
  `Vec<u8>`-centric, while retaining `Vec<u8>` compatibility adapters for as
  long as public mutation hooks still require owned mutable bytes.
- Remove `CorrelatableFrame for Vec<u8>` from the core surface once raw
  `Vec<u8>` stops being a documented or production frame shape. If a bridge is
  still useful, keep it in docs, `test-support`, or a feature-gated
  compatibility shim.
- Keep client preamble leftovers as owned `Vec<u8>` values for now. Revisit
  that path only if the broader public byte APIs converge on a shared buffer
  abstraction.

The main conceptual risk is overstating the migration scope by treating every
`Vec<u8>` as a frame problem. In practice, most runtime-sensitive surfaces are
about payload ownership and mutability, while true frame-level `Vec<u8>`
coupling is now limited.

## Resolved direction for epic 284

The current direction is now explicit:

- Epic 284 covers both transport-frame substitution and public payload-owned
  APIs such as `PacketParts`, middleware, and client hooks.
- Those changes should be tracked as separate workstreams under the same
  umbrella so the migration can preserve developer ergonomics and minimize
  boilerplate.
- The long-term outbound boundary should be `Bytes`-compatible rather than
  `Vec<u8>`-centric, with `Vec<u8>` retained only as a compatibility adapter
  while mutable byte-edit hooks still require it.
- `CorrelatableFrame for Vec<u8>` should leave the core surface once raw
  `Vec<u8>` stops serving as a documented or production frame shape.
- Client preamble leftovers remain a separate compatibility surface and should
  stay as owned `Vec<u8>` for now.

## Coordination notes

No matching roadmap item for this inventory work was found in the local docs
set on 2026-04-11, so nothing was marked done. If epic 284 is updated outside
the repository, the prepared link text and next-step summary are:

```plaintext
Add inventory reference: docs/frame-vec-u8-inventory.md
Link label: Frame = Vec<u8> inventory

Epic 284 workstreams:
1. Transport-frame substitution and actor/codec boundary work.
2. Public payload-owned API work: PacketParts, Envelope, middleware, and
   client byte hooks, with minimal consumer boilerplate.
3. Compatibility cleanup: move Vec<u8>-only bridges such as
   CorrelatableFrame for Vec<u8> out of the core surface once no longer needed.
4. Deferred: keep client preamble leftovers on Vec<u8> until broader public
   byte APIs justify standardization.
```

[^adr-004]: See
            [Architecture Decision Record (ADR) 004](adr-004-pluggable-protocol-codecs.md)
    for the generic codec decision and the current `Bytes` default frame type.
[^issue-286]: Middleware follow-up requested by
    [@leynos](https://github.com/leynos), with context from
    [issue #286](https://github.com/leynos/wireframe/issues/286),
    [PR #283](https://github.com/leynos/wireframe/pull/283), and the linked
    [comment](https://github.com/leynos/wireframe/pull/283#issuecomment-3167997856).
[^codec]: See
          [Pluggable protocol codecs](execplans/pluggable-protocol-codecs.md)
    and `src/codec.rs`.
[^hooks]: See `src/hooks.rs` and the hook overview in
    [users-guide.md](users-guide.md).
[^unified-path]: See
    [docs/execplans/9-3-1-fragment-adapter-trait.md](./execplans/9-3-1-fragment-adapter-trait.md)
    for the actor/codec-driver boundary and why `Bytes` does not flow through
    `ConnectionActor` directly today.
