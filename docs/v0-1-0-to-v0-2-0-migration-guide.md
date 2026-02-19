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

## Cargo feature gate rename

The crate feature used to expose test helper APIs was renamed.

- Old feature: `test-helpers`
- New feature: `test-support`

When a crate enables this feature in `Cargo.toml`, update it as follows:

```toml
# Before
[dependencies]
wireframe = { version = "0.2.0", features = ["test-helpers"] }

# After
[dependencies]
wireframe = { version = "0.2.0", features = ["test-support"] }
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
