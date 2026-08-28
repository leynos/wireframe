# Changelog

All notable changes to this project will be documented in this file.

## Unreleased

- Breaking: `WireframeServer::new` now evaluates its `AppFactory` once per
  server run, prepares the result once, and shares that immutable application
  root across connections. Move per-connection state to `on_connection_setup`,
  or use `WireframeServer::from_app(app)` when the application is already
  built. See `docs/v0-2-0-to-v0-3-0-migration-guide.md` for migration examples.
  (#642)

- **Client (breaking):** Remove `SocketOptions::linger` and
  `WireframeClientBuilder::linger`. Tokio-managed client sockets must not use
  duration-based `SO_LINGER`; call `WireframeClient::close().await` for
  graceful shutdown. (#624)
- Renamed internal module `src/app/connection.rs` to
  `src/app/inbound_handler.rs` and `src/server/connection.rs` to
  `src/server/connection_spawner.rs` to clarify directionality and eliminate
  naming ambiguity with the public `src/connection/` module. No public API
  changes.
- Deprecated `SharedState::new` (since 0.2.0); construct via `inner.into()`
  instead.
- Breaking: Marked `ServerError` as `#[non_exhaustive]`. Downstream consumers
  must add a wildcard arm when matching it.
- Breaking: Renamed `BackoffConfig::normalised` to `BackoffConfig::normalized`
  to align the public API spelling with American English.
- Breaking: Renamed the cargo feature flag `test-helpers` to
  `test-support`. Enable `test-support` to access exported test helper APIs. See
  `docs/v0-1-0-to-v0-2-0-migration-guide.md` for the required `Cargo.toml`
  dependency update.
- Exposed `MAX_PUSH_RATE` for configuring push queue rate limits.
- Refactored the application module into the `src/app/` directory
  (13 focused files) to keep module sizes under 400 lines; public API exports
  remain unchanged. (PR #282)
- Added a `Fragmenter` helper that slices oversized messages into sequential
  fragments, stamping each piece with a `FragmentHeader` for transparent
  transport-level reassembly.
- Breaking: Changed `FragmentError::IndexOverflow` and
  `FragmentationError::IndexOverflow` from unit variants to struct variants
  carrying a `last: FragmentIndex` field. This field records the final valid
  index observed before the counter would overflow `u32::MAX`.

  **Migration guide:**

  Pattern matches against the old unit variant must be updated to destructure
  or wildcard the new field:

  ```rust
  // Before (0.1.x): unit variant
  match err {
      FragmentError::IndexOverflow => { /* ... */ }
      // ...
  }

  // After (0.2+): struct variant with `last` field
  match err {
      FragmentError::IndexOverflow { last } => {
          eprintln!("overflow after fragment index {last}");
      }
      // ...
  }
  ```

  The same change applies to `FragmentationError::IndexOverflow`:

  ```rust
  // Before (0.1.x)
  Err(FragmentationError::IndexOverflow) => { /* ... */ }

  // After (0.2+)
  Err(FragmentationError::IndexOverflow { last }) => {
      log::warn!("cannot fragment: index overflow after {last}");
  }
  ```

  If the `last` value is not needed, use `{ .. }` to ignore it:

  ```rust
  match err {
      FragmentError::IndexOverflow { .. } => { /* handle overflow */ }
      // ...
  }
  ```
