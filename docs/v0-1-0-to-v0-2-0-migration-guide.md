# v0.1.0 to v0.2.0 migration guide

This guide summarizes the breaking changes you need to address when migrating
from wireframe v0.1.0 to v0.2.0.

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
