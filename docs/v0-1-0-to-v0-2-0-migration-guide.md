# v0.1.0 to v0.2.0 migration guide

This guide summarizes the breaking changes that must be addressed when
migrating from wireframe v0.1.0 to v0.2.0.

## Configuration builder naming update

The configuration builder method name has been updated to use the
en-GB-oxendict "-ize" spelling.

- Old name: `BackoffConfig::normalised`
- New name: `BackoffConfig::normalized`

Update any references accordingly, including documentation and code examples.

```rust
let config = BackoffConfig::normalized(...);
```

## Payload accessors

The consuming payload accessors were renamed to follow Rust idioms.

- `PacketParts::payload(self)` has been removed. Use
  `PacketParts::into_payload(self)` instead.
- `FragmentParts::payload(self)` has been removed. Use
  `FragmentParts::into_payload(self)` instead.

```rust
// Before
let packet_payload = packet_parts.payload();
let fragment_payload = fragment_parts.payload();

// After
let packet_payload = packet_parts.into_payload();
let fragment_payload = fragment_parts.into_payload();
```

## Unified error surface

Wireframe now exposes one canonical crate-level error type:
`wireframe::WireframeError`. The crate-level result alias
`wireframe::Result<T>` also resolves to this type.

- Builder setup errors such as duplicate routes are now reported through
  `wireframe::WireframeError::DuplicateRoute`.
- Streaming and transport errors still use `Io`, `Protocol`, and `Codec`
  variants on the same `wireframe::WireframeError` type.

```rust
// Before
use wireframe::app::WireframeError as AppError;
use wireframe::response::WireframeError as StreamError;

// After
use wireframe::WireframeError;
use wireframe::Result;
```
