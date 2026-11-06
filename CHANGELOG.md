# Changelog

All notable changes to this project will be documented in this file.

## Unreleased

- Make `wireframe_testing` compile against `tokio-util` 0.7.20, whose extra
  `Encoder` implementations made `.into()` conversions ambiguous. The lock file
  held 0.7.19, so a fresh install failed while CI passed. The lock file now
  selects 0.7.20, and the `metadata_routing` example and the tests build with
  it.

- Set the workspace version to `0.4.0-alpha1`, and rename the older migration
  guides to `docs/v0-2-0-migration-guide.md` and
  `docs/v0-3-0-migration-guide.md`, matching the v0.4.0 guide.

- Audit the README, users' guide, and migration guide against the code for
  v0.4.0-alpha1, and rewrite them. Rename the migration guide to
  `docs/v0-4-0-migration-guide.md`, matching other df12 projects. Every
  documented Rust example now runs as a doctest, and a new suite validates the
  other fences and relative links. The guide previously described a
  `PayloadBytes` serializer API that does not exist, and streaming bodies and
  extractors as usable from route handlers.

- Correct rustdoc that overstated runtime integration: streaming request
  bodies, extractors and `app_data` are not delivered to route handlers,
  `RecoveryPolicyHook` and `on_eof` are not consulted by the connection loop,
  the runtime does not emit `wireframe_codec_errors_total`, `read_timeout_ms`
  does not close idle connections, and pool sockets open on first use. Mark
  `WireframeApp::handle_connection*` deprecated since 0.4.0, the release that
  deprecates them, and reopen roadmap item 8.1.4.

- Run doctests in CI. `make test` selects `--all-targets`, which excludes them,
  so no documented example was being compiled. Repair the `wireframe_testing`
  doctests, stop excluding that crate from `make test-doc`, restore it as a
  mutation target, and re-export `drive_with_frame_with_capacity_mut` and
  `drive_with_frames_with_capacity_mut` from its crate root.
  ([#578](https://github.com/leynos/wireframe/issues/578))

- **Breaking:** Protocol `before_send` hooks now run for `WireframeApp`
  handler responses, not only actor-driven push and multi-packet frames. The
  hook sees the serialized, codec-wrapped frame before codec encoding and must
  keep it within `FrameCodec::max_frame_length`; the direct `send_response*`
  helpers still bypass it, and actor-owned traffic keeps its own hook pipeline.
  See the
  [v0.3-to-v0.4 migration guide](docs/v0-4-0-migration-guide.md#protocol-hooks-on-application-responses).
  ([#547](https://github.com/leynos/wireframe/issues/547))

- **Breaking:** Add aggregate limits to `FragmentationConfig` and extend
  `Reassembler::new` with optional partial-count and buffered-byte caps. Update
  downstream struct literals and constructor calls as described in the
  [v0.3-to-v0.4 migration guide](docs/v0-4-0-migration-guide.md). Default
  fragmentation now closes connections exceeding either aggregate cap. Retained
  fragment bytes also count towards connection memory-budget pacing and
  hard-cap aborts, even when message assembly is disabled. (#543)

- Breaking: `WireframeServer` now bounds the connection preamble read with
  `DEFAULT_PREAMBLE_TIMEOUT` (10 seconds) by default, so an unauthenticated
  connection cannot wait indefinitely for its first byte. Call
  `preamble_timeout(Duration)` to choose another deadline, or
  `disable_preamble_timeout()` to restore unbounded waiting when a peer
  legitimately needs it. `Duration::ZERO` still clamps to 1 ms. Addresses
  [#546](https://github.com/leynos/wireframe/issues/546) (finding
  `csf_b9252731043bde37f2f34d2f`). See `docs/v0-4-0-migration-guide.md`.

- Breaking: `WireframeServer::new` now evaluates its `AppFactory` once per
  server run, prepares the result once, and shares that immutable application
  root across connections. Move per-connection state to `on_connection_setup`,
  or use `WireframeServer::from_app(app)` when the application is already
  built. See `docs/v0-4-0-migration-guide.md` for migration examples. (#642)

- Breaking: `MessageAssemblyState` now enforces
  `FirstFrameHeader::total_body_len` as an authoritative declaration. A first
  frame whose body exceeds its declared total is rejected with
  `DeclaredTotalTooSmall`; a continuation that would overshoot the declaration
  is rejected with `DeclaredTotalExceeded`; and a completed body whose length
  differs from the declaration is rejected with `DeclaredTotalMismatch`.
  Protocol implementations must supply exact totals or leave `total_body_len` as
  `None`. Downstream exhaustive matches on `MessageAssemblyError` need arms
  for the three new variants. See
  [ADR 015](docs/adr-015-declared-total-body-length-contract.md).
  ([#544](https://github.com/leynos/wireframe/issues/544))
- Added scheduler and slot poison-recovery coverage, and extracted
  `scheduler_tests.rs` to keep `scheduler.rs` below 400 lines. (#539)
- Added client-pool lifecycle and slot-rotation coverage that kills the
  surviving shutdown and rotation mutants: a closed pool refuses later handle
  acquisitions, round-robin acquisition dispatches work on every configured
  slot, and waiters queued at shutdown are drained rather than left blocked.
  The drain test synchronizes on the per-pool `PoolTaskChurnObserver`
  `capacity_waits` counter, so it waits for acquisitions to park rather than
  for elapsed time. (#593)
- Client pool slot ordering rotates over indices rather than cloning slot
  owners, with regression coverage for cursor advancement, empty pools, and
  `usize` overflow in non-power-of-two pools, a behavioural scenario proving
  rotation across two live slots, and bounded
  `wireframe_pool_acquisitions_total` telemetry.
  ([#536](https://github.com/leynos/wireframe/issues/536))
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
- **Breaking:** Mark `ClientError` as `#[non_exhaustive]` and add the typed
  `PoolSaturated` error for rejected pool acquisitions; downstream exhaustive
  matches need a wildcard arm.
  ([#550](https://github.com/leynos/wireframe/issues/550))
- Breaking: Renamed `BackoffConfig::normalised` to `BackoffConfig::normalized`
  to align the public API spelling with American English.
- Breaking: Renamed the cargo feature flag `test-helpers` to
  `test-support`. Enable `test-support` to access exported test helper APIs. See
  `docs/v0-2-0-migration-guide.md` for the required `Cargo.toml` dependency
  update.
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
