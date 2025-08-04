# Message Versioning and Compatibility

> **Status:** Targeted for a post-1.0 release — see the
> [schema evolution roadmap item](roadmap.md#4-extended-features) for progress.

## Motivation

Binary protocols often evolve. New features require changes to message
structures, while older clients may still be deployed in the field. `wireframe`
needs a strategy to support multiple message versions without breaking
compatibility.

## Goals

- **Version negotiation**: Determine a mutually supported protocol version when
  a client connects.
- **Per-message versions**: Allow message types to define version numbers.
- **Compatibility checks**: Detect incompatible versions at runtime and surface
  meaningful errors.
- **Ergonomic API**: Provide Actix-like helpers for routing and version guards.

## Actix Web Inspiration

Actix Web uses "scopes" and guards to group endpoints by path or header values.
A similar approach can work for binary protocols: routes can be grouped under a
`VersionScope`, and a custom guard can inspect a version field before dispatch.

## Proposed Design

### 1. VersionedMessage Trait

```rust
pub trait VersionedMessage: Message {
    const VERSION: u16;
}
```

Derive macros will implement this trait for message structs. Handlers can then
declare their expected version by accepting `VersionedMessage` types.

### 2. Envelope Extensions

Extend `Envelope` with an optional `version` field:

```rust
pub struct Envelope {
    id: u32,
    correlation_id: u32,
    version: Option<u16>,
    msg: Vec<u8>,
}
```

The serializer will encode and decode this field transparently. Older clients
that lack a version field will represent it as `None`.

### 3. Version Guards

Introduce a `version_guard` method on `WireframeApp` to register handlers for a
specific version. Internally this works like Actix Web's `scope` with a guard:

```rust
app.version_guard(2).route(MessageId::Login, handle_login_v2);
```

If a message's version does not match any registered guard, a configurable
fallback handler can respond with an error.

### 4. Compatibility Checks

Handlers for newer versions can implement `From<OldVersion>` to convert legacy
messages. A helper `ensure_compatible` function will attempt the conversion
when a mismatched version is received.

```rust
async fn handle_login_v2(req: Message<LoginV2>) { /* ... */ }

app.version_guard(1).route(MessageId::Login, |m: Message<LoginV1>| {
    let upgraded: LoginV2 = m.into();
    handle_login_v2(Message(upgraded))
});
```

### 5. Handshake Negotiation

During the optional connection preamble, both sides can exchange the highest
protocol version they support. The server selects a version and stores it in
the connection state so extractors and middleware can access it. If no
compatible version exists, the connection is rejected early.

## Future Work

- The features described in this document are considered post-1.0 scope.
- Tooling to generate migration code between versions.
- Optional compile-time features for strict version matching or automatic
  upgrades.
- Documentation examples showing how to deprecate old versions gracefully.
