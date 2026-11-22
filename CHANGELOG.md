# Changelog

All notable changes to this project will be documented in this file.

## Unreleased

- Consult `RecoveryPolicyHook` on decoder failures from live server
  connections and, with the default `metrics` feature, emit
  `wireframe_codec_errors_total` once per failure, labelled with the error type
  and the applied policy. With no hook installed the applied policy remains
  disconnect, and a failure buffered when reading stops for a reason other than
  that failure — a response that ended the connection, a server shutdown, or
  memory pressure — is reported as that disconnect without consulting the hook.
  The built-in length-delimited codec now classifies an oversized frame as
  `error_type="framing"` instead of `"io"`.
  ([#878](https://github.com/leynos/wireframe/issues/878))
- Complete `WireframeProtocol` hook dispatch on prepared server connections.
  `on_eof` now runs when a peer disconnects mid-header or mid-frame, receiving
  the structured `EofError` and every byte the framed reader still held for the
  abandoned frame, length prefix included; a clean close at a frame boundary, a
  malformed frame, and a transport error reach no hook. A decoder opts in by
  returning `CodecError::Eof` from `decode_eof`; the bundled
  `LengthDelimitedFrameCodec` already does. A frame a push policy discards is
  counted by the new `wireframe_push_drops_total` counter, labelled by policy
  and by whether a dead-letter queue was absent, full, or closed, so a loss its
  producer was told had succeeded stays observable. An end-to-end suite now
  proves each hook's firing and order through a real `WireframeServer`.
  ([#879](https://github.com/leynos/wireframe/issues/879))

- **Breaking:** Remove the `wireframe::extractor` module, `AppDataStore`, and
  `WireframeApp::app_data`. No dispatch path invoked the extractor traits, so
  handlers could never receive `SharedState<T>`, `Message<T>`,
  `ConnectionInfo`, or `StreamingBody`. Capture an `Arc` in the handler closure
  for shared state and decode envelopes directly; see
  [ADR 017](docs/adrs/adr-0017-remove-route-extractors-and-app-data.md) and the
  [migration guide](docs/v0-3-0-to-v0-4-0-migration-guide.md#removed-extractors-and-app_data).
  ([#877](https://github.com/leynos/wireframe/issues/877))

- Give server connections a push path. `WireframeApp::with_push_queues` stores a
  `PushQueuesBuilder` template that each accepted connection clones, so every
  connection gets independent capacity, rate-limiter state, and dead-letter
  routing, and the template is validated during preparation.
  `WireframeProtocol::on_connection_setup` receives the connection's
  `PushHandle`, and `ConnectionContext` now carries the server-allocated
  `ConnectionId`. `WireframeApp::with_session_registry` shares a
  `SessionRegistry` that each connection joins before setup and leaves when its
  task ends; `SessionRegistry::allocate_and_publish` claims an identifier and
  publishes the handle in one atomic step, from `1`, never reusing one, and
  skipping an identifier a live entry occupies, and `SessionRegistry::insert`
  now returns whether it published the handle and refuses to replace a live
  entry. Deregistration removes an entry only while it still holds the ending
  connection's own handle. Pushed frames reach the client over the same
  transport as handler responses, subject to priority, fairness, rate limiting,
  and the dead-letter queue, and interleave with streamed responses. A clean
  peer close attempts to drain queued frames, but a write that fails or is
  abandoned drops the frames still behind it; on shutdown, a handler run that
  has not yet delivered a push batch keeps its lanes open until it does, after
  which queued frames are discarded and later pushes fail with
  `PushError::Closed`. A push whose encoded payload exceeds the connection
  codec's frame limit is refused with `PushError::FrameTooLarge` before it is
  queued, so a frame that is already unencodable cannot end the connection
  after its producer was told the push succeeded. That check runs when the
  frame is enqueued, before the protocol `before_send` hook; a hook that
  enlarges a frame past the codec's limit is instead rejected when the frame is
  written. A template's own `max_frame_len` and the codec's ceiling are both
  enforced, each with the payload measure it was configured with, so adding the
  codec limit never relaxes a template's bound. `PushError` gains `RateLimited`:
  `try_push` draws on the same rate limit as the awaited helpers and refuses
  rather than waiting when no permit is available. A preamble that encodes to
  no bytes, including the default `()`, decodes without reading, so a client
  that sends nothing still reaches connection setup and receives its first push.
  `PushHandle` and `PushQueuesBuilder` gain `Clone` implementations that do
  not require `F: Clone`. **Breaking:** `ConnectionContext` is
  `#[non_exhaustive]` and no longer a unit struct, and
  `WireframeApp::with_push_dlq` is deprecated since 0.4.0 in favour of a `dlq`
  sender on the queue template.
  ([#875](https://github.com/leynos/wireframe/issues/875))

- Add `WireframeApp::route_response` and `ResponseHandler<E>` so a route can
  return a whole `Response`: a single frame, a vector, an empty reply, a
  stream, or a multi-packet channel. Each prepared server connection now owns a
  private connection actor, so streamed and multi-packet responses are driven
  to completion through the connection's `FramePipeline` before the next
  inbound frame is dispatched; the read half may buffer one subsequent request
  while the response runs, but that frame is not dispatched until the response
  finishes. Data frames carry the request correlation id when the request
  supplied one, and keep their own identifiers when it did not; a protocol
  terminator is written as the protocol encoded it. Fragmentation, outbound
  metrics, serialization, codec wrapping, and `before_send` therefore run in
  one fixed order for every handler response. The protocol's `on_command_end`,
  `handle_error`, and `stream_end_frame` callbacks now run on the prepared path;
  `on_command_end` therefore fires once for an ordinary `route` reply, which
  existing protocols previously did not observe. A completed stream is
  terminated by the codec frame `stream_end_frame` returns, written without
  decoding or re-wrapping so codec metadata outside the payload survives; the
  prepared path no longer decodes that frame into an `Envelope` or restamps its
  correlation identifier. The frame is not byte-preserved: `before_send` still
  runs, and a hook that rewrites frames rewrites the terminator too.
  `on_connection_setup` is invoked there with the connection's `PushHandle`, as
  described above. A peer that disconnects while a streamed response is idle no
  longer pins the connection: the read half is watched during the response, and
  the response is bounded when a buffered request stops that observation.
  **Breaking:** `PreparedApp::handle_connection_result` and the deprecated
  `WireframeApp::handle_connection*` methods now require `S: 'static` so the
  connection actor can retain the serializer in its protocol hooks. The
  active-connection gauge now covers the preamble handshake, application
  processing, and teardown.
  ([#874](https://github.com/leynos/wireframe/issues/874))

- **Breaking:** Protocol hooks are now one owner per connection rather than six
  cloned forwarding closures. `ProtocolHooks` is an enum of `None`, `Protocol`,
  and `Custom`; replace a `ProtocolHooks { .. }` struct literal with
  `CustomProtocolHooks { .. }.into()`. A protocol-backed connection holds
  exactly one strong reference to the installed protocol once setup has
  completed, and `on_connection_setup` runs at most once per hooks value, while
  every packet-oriented outbound path — handler responses, push queues,
  streamed responses, multi-packet channels, and stream-end terminators — runs
  `before_send` exactly once per emitted packet. See the
  [v0.3-to-v0.4 migration guide](docs/v0-3-0-to-v0-4-0-migration-guide.md#protocol-hook-ownership-and-dispatch).
  ([#644](https://github.com/leynos/wireframe/issues/644))

- Make `wireframe_testing` compile against `tokio-util` 0.7.20, whose extra
  `Encoder` implementations made `.into()` conversions ambiguous. The lock file
  held 0.7.19, so a fresh install failed while CI passed. The lock file now
  selects 0.7.20, and the `metadata_routing` example and the tests build with
  it.

- Check the declared minimum Rust version on every pull request, every push to
  `main` and every week, with both the committed lock file and freshly resolved
  dependencies, as concordat RT-026 requires, and declare the same minimum on
  `wireframe_testing`.

- Correct rustdoc that overstated runtime integration: streaming request
  bodies, extractors and `app_data` are not delivered to route handlers, and
  `on_eof` is not consulted by the connection loop. `read_timeout_ms` does not
  close idle connections, and pool sockets open on first use. Mark
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
  helpers still bypass it. See the
  [v0.3-to-v0.4 migration guide](docs/v0-3-0-to-v0-4-0-migration-guide.md#protocol-hooks-on-application-responses).
  ([#547](https://github.com/leynos/wireframe/issues/547))

- **Breaking:** Add aggregate limits to `FragmentationConfig` and extend
  `Reassembler::new` with optional partial-count and buffered-byte caps. Update
  downstream struct literals and constructor calls as described in the
  [v0.3-to-v0.4 migration guide](docs/v0-3-0-to-v0-4-0-migration-guide.md).
  Default fragmentation now closes connections exceeding either aggregate cap.
  Retained fragment bytes also count towards connection memory-budget pacing
  and hard-cap aborts, even when message assembly is disabled. (#543)

- Breaking: `WireframeServer` now bounds the connection preamble read with
  `DEFAULT_PREAMBLE_TIMEOUT` (10 seconds) by default, so an unauthenticated
  connection cannot wait indefinitely for its first byte. Call
  `preamble_timeout(Duration)` to choose another deadline, or
  `disable_preamble_timeout()` to restore unbounded waiting when a peer
  legitimately needs it. `Duration::ZERO` still clamps to 1 ms. Addresses
  [#546](https://github.com/leynos/wireframe/issues/546) (finding
  `csf_b9252731043bde37f2f34d2f`). See
  `docs/v0-3-0-to-v0-4-0-migration-guide.md`.

- Breaking: `WireframeServer::new` now evaluates its `AppFactory` once per
  server run, prepares the result once, and shares that immutable application
  root across connections. Move per-connection state to `on_connection_setup`,
  or use `WireframeServer::from_app(app)` when the application is already
  built. See `docs/v0-3-0-to-v0-4-0-migration-guide.md` for migration examples.
  (#642)

- Breaking: `MessageAssemblyState` now enforces
  `FirstFrameHeader::total_body_len` as an authoritative declaration. A first
  frame whose body exceeds its declared total is rejected with
  `DeclaredTotalTooSmall`; a continuation that would overshoot the declaration
  is rejected with `DeclaredTotalExceeded`; and a completed body whose length
  differs from the declaration is rejected with `DeclaredTotalMismatch`.
  Protocol implementations must supply exact totals or leave `total_body_len` as
  `None`. Downstream exhaustive matches on `MessageAssemblyError` need arms
  for the three new variants. See
  [ADR 015](docs/adrs/adr-0015-declared-total-body-length-is-authoritative.md).
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
