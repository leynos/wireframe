# Architectural decision record (ADR) 017: remove the unwired route extractors and application data API

## Status

Accepted.

The `wireframe::extractor` module, `AppDataStore`, and `WireframeApp::app_data`
are removed before 1.0. Route handlers keep their current `Handler<E>` shape
and receive only a reference to the routed packet. A handler that needs shared
state captures an `Arc` in its closure. The decision is implemented by issue
[#877](https://github.com/leynos/wireframe/issues/877).

## Date

2026-11-09.

## Context and problem statement

`wireframe::extractor` published an Actix-style extraction layer:
`MessageRequest`, `Payload`, `FromMessageRequest`, `Message<T>`,
`SharedState<T>`, `ConnectionInfo`, `StreamingBody`, and `ExtractError`.
`WireframeApp::app_data<T>` stored `Send + Sync` values in an `AppDataStore`,
and `MessageRequest::state::<T>` could read them back.

None of that ran while serving a connection. `Handler<E>` is
`Arc<dyn Fn(&E) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>`, so
a route handler receives one borrowed packet and no extractor parameters. Code
under `src/app` and `src/server` never builds a `MessageRequest`, never calls
`FromMessageRequest::from_message_request`, and never reads the application
data store. `PreparedApp` carried the store between builder states, but only
for a future connection-runtime slice tracked by
[#643](https://github.com/leynos/wireframe/issues/643); the value was never
consulted during dispatch.

The gap was first documented rather than hidden. Issue
[#870](https://github.com/leynos/wireframe/pull/870) corrected rustdoc that
promised runtime behaviour the code did not have, and issue
[#873](https://github.com/leynos/wireframe/pull/873) recorded the same gap in
the users' guide. Documenting the gap is not a substitute for deciding whether
the API should exist. The project is pre-1.0, so the choice is between wiring
the layer into dispatch now and removing it while removal is still cheap.

## Decision drivers

- Public API must describe implemented behaviour. An extractor trait that no
  runtime invokes is a promise the library cannot keep.
- Wiring extractors into dispatch changes the handler signature. That signature
  interacts with the response work and the reply-context boundary in
  [RFC 0003](rfcs/0003-native-message-adapter-boundary.md), which are not
  settled.
- Extraction failure has no defined response. A handler returns `()`, and reply
  construction belongs to middleware, so there is no universal place to turn an
  `ExtractError` into a protocol response.
- Typed extraction through `Message<T>` assumes a serializer path. The
  serializer abstraction in [ADR 005](adr-005-serializer-abstraction.md) does
  not define a single decode shape that every protocol accepts.
- The documented closure-captured `Arc` pattern already provides shared state
  without a new dispatch stage.
- Removing the API before 1.0 is a documented migration. Removing it after 1.0
  would be a breaking change for a feature that never worked.

## Requirements

### Functional requirements

- Route handlers must continue to receive a reference to the routed packet, and
  dispatch must behave exactly as it does today.
- Shared state must remain available to handlers through a supported pattern:
  capture an `Arc` in the closure registered with `WireframeApp::route`.
- The independent request and message utilities that do work must remain
  available: `message::Message`, the `request` module, and `PacketParts`.
- Current documentation must describe only implemented behaviour.

### Technical requirements

- Delete `wireframe::extractor`, `wireframe::app_data_store`, and
  `WireframeApp::app_data` together with their builder and `PreparedApp`
  plumbing.
- Keep the `request` module and its `#[cfg(not(loom))]` gate, and keep the
  `message::Message` prelude export.
- Pin the absence of the removed symbols with a compile-fail fixture.
- Keep `cargo` dependencies that other modules still use: `dashmap` for
  `SessionRegistry`, and `bincode`, `thiserror`, and `tokio-stream` for their
  existing consumers.
- Pass the repository gates, including `make test-ui` and `make test-loom`.

## Options considered

### Option A: wire extraction into dispatch

Build a `MessageRequest` for every inbound packet, run each declared extractor,
and pass the results to the handler. `app_data` values would populate the
request before dispatch.

This requires a new handler signature, extractor ordering rules, a failure
response contract, and a typed decode path that works with every serializer. It
also has to settle how extracted parameters coexist with the reply context that
RFC 0003 proposes. Those are the same boundaries the response work is already
blocked on.

### Option B: remove the extractor and app-data API before 1.0

Delete the unused public surface and migrate users to closure-captured state.
Keep request and message building blocks that work on their own. Record the
decision so a future typed-handler layer is designed against a settled response
boundary.

### Option C: keep the API and document the gap

Leave the types in place, relying on rustdoc and the users' guide to say that
dispatch does not invoke them. This keeps a misleading public surface and makes
the eventual removal a post-1.0 breaking change.

| Topic                      | Option A                      | Option B                      | Option C                   |
| -------------------------- | ----------------------------- | ----------------------------- | -------------------------- |
| Public API is accurate     | Yes, after dispatch is wired  | Yes, by removal               | No, documented gap remains |
| Depends on response design | Yes, blocks on RFC 0003       | No                            | No                         |
| Shared state available     | Yes, through `SharedState<T>` | Yes, through a captured `Arc` | No, unless captured        |
| Migration cost before 1.0  | New dispatch work             | Documented removal            | Deferred                   |
| Cost after 1.0             | None                          | Not applicable                | Breaking removal later     |

_Table 1: Comparison of the considered options._

## Decision outcome

Option B is adopted.

`Handler<E>` keeps its current shape. The following public symbols are removed:

| Removed symbol                             | Replacement                                                   |
| ------------------------------------------ | ------------------------------------------------------------- |
| `wireframe::extractor` module              | No module; import the individual primitives below             |
| `wireframe::extractor::MessageRequest`     | Build only the metadata a handler needs                       |
| `wireframe::extractor::Payload`            | Borrow `Packet::payload_bytes` or the packet's `PacketParts`  |
| `wireframe::extractor::FromMessageRequest` | Use a decoder appropriate to the payload format               |
| `wireframe::extractor::Message<T>`         | `message::Message::from_bytes` on a bincode payload           |
| `wireframe::extractor::SharedState<T>`     | Capture an `Arc<T>` in the handler closure                    |
| `wireframe::extractor::ConnectionInfo`     | No peer address reaches route handlers                        |
| `wireframe::extractor::StreamingBody`      | `request::RequestBodyReader::new(stream)` on a stream you own |
| `wireframe::extractor::ExtractError`       | Define a protocol- or application-specific error              |
| `wireframe::app_data_store::AppDataStore`  | Capture an `Arc` in the handler closure                       |
| `WireframeApp::app_data`                   | Capture an `Arc` in the handler closure                       |

_Table 2: Removed symbols and their replacements._

`message::Message::from_bytes` decodes a bincode payload only. A protocol with
another payload format should decode with its matching serializer, for example
`Serializer::deserialize` bound to a type implementing `DecodeWith`.

The following types are retained because they are useful without extractor
dispatch:

- `message::Message` remains the typed payload encode/decode trait, and its
  prelude export is unchanged.
- `wireframe::request` retains `RequestParts`, `RequestBodyStream`,
  `RequestBodyReader`, `body_channel`, and `DEFAULT_BODY_CHANNEL_CAPACITY`. The
  server still does not deliver request streams to route handlers; those types
  remain building blocks for code that owns both ends.
- `PacketParts` remains the envelope component value type.

ADR 012 already separates the builder from the prepared template. Removing
`app_data` simplifies that boundary: `PreparedApp` no longer carries state that
no connection runtime reads. Roadmap item 20.1.3 therefore loses its app-data
consumption note and continues to cover connection-local runtime ownership and
lifecycle finalization under
[#643](https://github.com/leynos/wireframe/issues/643).

A future typed-handler or extractor layer must be additive. It must build on
the response and reply-context boundary in RFC 0003, and it must specify how an
extraction failure becomes a protocol response before it exposes a public
trait. It should not restore the removed names unchanged.

## Goals and non-goals

- Goals:
  - Remove public API that no runtime invokes.
  - Keep shared state available through the closure-captured `Arc` pattern.
  - Preserve the independent request and message building blocks.
  - Provide a migration guide and a compile-fail guard against reintroduction.
- Non-goals:
  - Designing a replacement typed-handler layer.
  - Wiring streaming request bodies into handler dispatch. That remains ADR
    002 work.
  - Changing the `Handler<E>` signature or dispatch order.

## Migration plan

1. Record this decision and index it in the documentation contents.
2. Delete `src/extractor/`, `src/app_data_store.rs`, and
   `src/app/builder/state.rs`, and remove the store fields and transitions from
   `WireframeApp` and `PreparedApp`.
3. Delete extractor-specific tests. Keep streaming and request primitive
   coverage, and update the behavioural streaming suite to use
   `RequestBodyReader::new`. Add one compile-fail fixture covering both
   `wireframe::extractor::SharedState` and `WireframeApp::app_data`.
4. Add a migration-guide section and a changelog entry that map every removed
   symbol to its replacement.
5. Correct the README, users' guide, roadmap, and design records so that they
   describe only implemented behaviour, marking historical design text as
   superseded by this ADR.

## Known risks and limitations

- Alpha users of `wireframe::extractor` or `WireframeApp::app_data` must
  migrate. The migration guide supplies before-and-after examples.
- Closure capture is less ergonomic than extractor parameters. It is the only
  pattern the runtime supports today.
- Route handlers do not receive the peer address. Applications that need it
  must capture connection information where the runtime exposes it, and the
  current runtime exposes it to connection setup and protocol hooks rather than
  to route handlers.
- Deleting the types removes the placeholder that documented intended
  ergonomics. The design record is preserved, and future work is directed to
  RFC 0003 rather than to the removed names.

## Outstanding decisions

None for this ADR. A future typed-handler or extractor layer needs its own ADR
once the RFC 0003 response and reply-context boundary is settled.

## Verification

- The compile-fail fixture `tests/ui/removed_extractor_and_app_data.rs` fails to
  compile because neither `wireframe::extractor::SharedState` nor
  `WireframeApp::app_data` exists.
- The request and message suites continue to pass without the extractor module.
- `make check-fmt`, `make lint`, `make test`, `make test-doc`, `make test-ui`,
  and `make test-loom` pass.

## References

- [Issue #877](https://github.com/leynos/wireframe/issues/877)
- [Issue #870](https://github.com/leynos/wireframe/pull/870), corrected rustdoc
  for unwired runtime behaviour
- [Issue #873](https://github.com/leynos/wireframe/pull/873), users' guide
  record of building blocks not yet wired into the server
- [ADR 002: streaming requests and shared message assembly](adr-002-streaming-requests-and-shared-message-assembly.md)
- [ADR 005: serializer abstraction](adr-005-serializer-abstraction.md)
- [ADR 012: prepared application templates and connection-local runtimes](adr-012-prepared-application-and-connection-runtime.md)
- [RFC 0003: native message adapter boundary](rfcs/0003-native-message-adapter-boundary.md)
- [v0.3.0 to v0.4.0 migration guide](v0-3-0-to-v0-4-0-migration-guide.md)
