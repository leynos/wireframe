# v0.1.0 to v0.2.0 migration guide

This guide highlights user-facing changes required when upgrading from v0.1.0
to v0.2.0.

## Configuration builder naming update

The configuration builder method name has been updated to use the
en-GB-oxendict "-ize" spelling.

- Old name: `BackoffConfig::normalised`
- New name: `BackoffConfig::normalized`

Update any references accordingly, including documentation and code examples.

```rust
let config = BackoffConfig::normalized(...);
```
