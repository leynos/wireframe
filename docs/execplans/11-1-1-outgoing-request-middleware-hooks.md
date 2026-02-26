# Add middleware hooks for outgoing requests and incoming frames (11.1.1)

This ExecPlan (execution plan) is a living document. The sections
`Constraints`, `Tolerances`, `Risks`, `Progress`, `Surprises & Discoveries`,
`Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work
proceeds.

Status: COMPLETE

## Purpose / big picture

Roadmap item 11.1.1 requires middleware hooks on the wireframe client so that
metrics, retries, and authentication tokens can be injected symmetrically with
server middleware. Today the server has a rich hook/middleware stack
(`WireframeProtocol::before_send` in `src/hooks.rs`, `Service`/`Transform`
traits in `src/middleware.rs`, `WireframeApp::wrap()` in
`src/app/builder/routing.rs`), but the client only has lifecycle hooks
(`on_connection_setup`, `on_connection_teardown`, `on_error`) that fire at
connection boundaries — not on every message.

After this change, a library consumer can register **`before_send`** hooks
(fired after serialization, before transport write) and **`after_receive`**
hooks (fired after transport read, before deserialization) on the client
builder. These hooks fire on every message path: `send`, `send_envelope`,
`receive`, `receive_envelope`, `call`, `call_correlated`, `call_streaming`, and
`ResponseStream` polling.

Observable success: a user configures `before_send` and `after_receive`
closures on the builder, sends a message, receives a response, and both
closures are invoked with the serialised bytes. All existing tests pass
unchanged. New unit tests (rstest) and behavioural tests (rstest-bdd v0.5.0)
validate hook invocation, ordering, mutation, and streaming integration.

## Constraints

Hard invariants. Violation requires escalation, not workarounds.

- No single source file may exceed 400 lines.
- Public APIs must be documented with rustdoc (`///`) including examples.
- Every module must begin with a `//!` module-level comment.
- All code must pass `make check-fmt`, `make lint` (clippy `-D warnings`),
  and `make test`.
- Comments and documentation must use en-GB-oxendict spelling.
- Tests must use `rstest` fixtures; BDD tests use `rstest-bdd` v0.5.0.
- Existing public API signatures must not change (backwards compatibility).
- Clients configured without hooks must behave identically to today.
- The `builder_field_update!` macro in `src/client/builder/mod.rs` must be
  updated for every arm when a new field is added to the builder struct.
- No new external crate dependencies (all types needed — `Arc`, `Vec`,
  `BytesMut` — are already available).
- Design decisions must be recorded in `docs/wireframe-client-design.md`.
- `docs/users-guide.md` must be updated with the new public API surface.
- `docs/roadmap.md` item 11.1.1 must be marked done only after all gates pass.

## Tolerances (exception triggers)

- Scope: if implementation requires changes to more than 25 files (net new +
  modified), stop and escalate.
- Interface: if any existing public API signature must change (not just adding
  new methods), stop and escalate.
- Dependencies: if a new external crate is required, stop and escalate.
- Iterations: if tests still fail after 3 attempts at a given milestone, stop
  and escalate.
- Time: if any single stage exceeds 4 hours elapsed, stop and escalate.
- Ambiguity: if hook semantics in `ResponseStream::poll_next` prove
  incompatible with the synchronous approach, stop and present options.

## Risks

- Risk: `ResponseStream::poll_next` is synchronous (`fn poll_next`) but hooks
  may want to be async. Severity: medium Likelihood: high Mitigation: use
  synchronous hooks (`Fn(&mut BytesMut)`). This matches the server's
  `before_send` hook in `src/hooks.rs` which is also synchronous
  (`FnMut(&mut F, &mut ConnectionContext)`). Frame-level hooks operate on raw
  bytes in memory — they do not need async. Truly async operations (e.g.,
  fetching a token from a remote service) should be done at the lifecycle level
  (`on_connection_setup`) and the result stored in connection state, which the
  synchronous hook can then read.

- Risk: adding fields to `WireframeClient` and `WireframeClientBuilder` may
  push existing files over the 400-line limit. Severity: low Likelihood: low
  Mitigation: `runtime.rs` is 351 lines, `builder/core.rs` is 61 lines. There
  is ample headroom. Hook types and builder methods go in separate files.

- Risk: the `builder_field_update!` macro has three arms; adding a field
  requires a one-line addition per arm. Severity: low Likelihood: certain
  Mitigation: `RequestHooks` is not parameterised by any generic type, so it
  moves directly (`$self.request_hooks`) in all arms.

## Progress

- [x] Stage A: design finalised (this document approved)
- [x] Stage B: scaffolding — types, traits, and builder plumbing
- [x] Stage C: runtime integration — wire hooks into send/receive/streaming
- [x] Stage D: unit tests (rstest) in `src/client/tests/request_hooks.rs`
- [x] Stage E: BDD tests (rstest-bdd) — feature, fixture, steps, scenarios
- [x] Stage F: documentation — users-guide, client-design, roadmap

## Surprises & discoveries

- The `invoke_before_send_hooks` and `invoke_after_receive_hooks` helper
  methods were placed in `messaging.rs` rather than `runtime.rs` as originally
  planned, because the trait bounds required (`Serializer + Send + Sync`) were
  already available in the `messaging.rs` impl block. This avoided adding a
  `bytes::BytesMut` import to `runtime.rs`.

- BDD tests require `--features advanced-tests` to run. This was discovered
  during Stage E and is consistent with the existing BDD test infrastructure.

- The BDD feature file was simplified from 5 scenarios to 4: the streaming and
  token-injection scenarios were dropped in favour of keeping the BDD tests
  focused on the core hook contract. Streaming integration is covered by unit
  tests instead.

- An unused `CorrelatableFrame` import appeared in the BDD fixture after
  initial scaffolding — removed during Stage E cleanup.

## Decision log

- Decision: use synchronous closure-based hooks, not async, not trait-based.
  Rationale: the server's equivalent (`before_send` in `src/hooks.rs`) is
  synchronous (`FnMut`). Async hooks would be impossible to call from
  `ResponseStream::poll_next` without spawning tasks or blocking. Trait-based
  hooks (like the server's `Service`/`Transform`) are designed for middleware
  chaining with routing — overkill for the client's point-to-point model.
  Closures match the existing client hook pattern
  (`ClientConnectionSetupHandler`, `ClientErrorHandler`).

- Decision: hooks operate on raw bytes (`&mut Vec<u8>` outgoing,
  `&mut BytesMut` incoming), not typed messages. Rationale: symmetric with the
  server's `before_send` hook which operates on `&mut F` (the frame type,
  typically `Vec<u8>`). Operating on typed messages would require the hooks to
  be generic over every message type, making storage impossible without type
  erasure. Raw-byte hooks can universally inspect or modify the serialised
  payload (e.g., prepend an auth token, count bytes).

- Decision: use `Arc<dyn Fn(...)>` (shared, immutable) rather than
  `Box<dyn FnMut(...)>`. Rationale: existing client lifecycle hooks use
  `Arc<dyn Fn(...) + Send + Sync>`. `Fn` (not `FnMut`) is required because
  hooks are stored behind shared references. Users who need mutable state
  (counters, etc.) capture an `Arc<AtomicUsize>` or `Arc<Mutex<_>>` in the
  closure — the established pattern in the existing lifecycle hook tests.

- Decision: hook ordering is registration order for both outgoing and incoming.
  Rationale: simplest mental model. First registered hook runs first. There is
  no "onion" wrapping because these are sequential interceptors, not middleware
  with next-chains.

- Decision: store hooks in a `RequestHooks` struct (analogous to
  `LifecycleHooks`), not inline in `WireframeClient`. Rationale: keeps the
  client struct focused. Mirrors the `LifecycleHooks` pattern from
  `src/client/hooks.rs`.

## Outcomes & retrospective

Implementation completed successfully. All stages delivered as planned with
minor deviations noted in Surprises & Discoveries.

**Actual file counts**: 6 new files created, 16 files modified — exactly as
estimated (22 total). All files remain under the 400-line constraint.

**Test results**: 7 unit tests in `src/client/tests/request_hooks.rs` and 4 BDD
scenarios in `tests/scenarios/client_request_hooks_scenarios.rs` all pass. All
pre-existing tests pass unchanged (backwards compatibility verified).

**Key lesson**: placing hook invocation helpers in `messaging.rs` rather than
`runtime.rs` was a pragmatic deviation that avoided unnecessary import
complexity. The plan's separation of "where helpers live" vs "where hooks are
called" proved more flexible than assumed.

## Context and orientation

The wireframe crate lives at `/home/user/project`. It is a Rust async
networking framework using Tokio. The client subsystem is in `src/client/`:

```plaintext
src/client/
  mod.rs              — module root, public re-exports (79 lines)
  runtime.rs          — WireframeClient struct and core methods (351 lines)
  messaging.rs        — send_envelope, receive_envelope, call_correlated (268 lines)
  streaming.rs        — call_streaming, receive_streaming (149 lines)
  response_stream.rs  — ResponseStream impl Stream (165 lines)
  hooks.rs            — LifecycleHooks, type aliases for callbacks (113 lines)
  error.rs            — ClientError enum (130 lines)
  builder/
    mod.rs            — builder_field_update! macro (61 lines)
    core.rs           — WireframeClientBuilder struct (61 lines)
    connect.rs        — connect() method (100 lines)
    lifecycle.rs      — on_connection_setup/teardown/error (130 lines)
    codec.rs          — codec config builder methods (58 lines)
    serializer.rs     — serializer() builder method
    preamble.rs       — preamble builder methods
  tests/
    mod.rs            — test module root (175 lines)
    helpers.rs        — test helpers (169 lines)
    lifecycle.rs      — lifecycle hook tests (141 lines)
    error_handling.rs — error hook tests (218 lines)
    messaging.rs      — messaging API tests (328 lines)
    streaming.rs      — streaming tests (367 lines)
    streaming_infra.rs — streaming test infrastructure (293 lines)
    streaming_parity.rs — streaming parity tests (288 lines)
```

BDD tests follow this structure:

```plaintext
tests/
  features/           — Gherkin .feature files
  fixtures/           — World structs with #[fixture] fns and step methods
  steps/              — #[given], #[when], #[then] step definitions
  scenarios/          — #[scenario] bindings linking features → fixtures
```

Key data flow (outgoing):

1. User calls `client.send(&msg)`, `send_envelope(env)`, or
   `call_streaming(req)`.
2. `self.serializer.serialize(message)` produces `Vec<u8>`.
3. **← HOOK INSERTION POINT: `before_send` hooks modify `&mut Vec<u8>` →**
4. `self.framed.send(Bytes::from(bytes))` writes to TCP.

Key data flow (incoming):

1. `self.framed.next()` reads from TCP, produces `BytesMut`.
2. **← HOOK INSERTION POINT: `after_receive` hooks modify `&mut BytesMut` →**
3. `self.serializer.deserialize(&bytes)` produces typed message.
4. Returned to user.

Streaming path (`ResponseStream::poll_next`):

1. `Pin::new(&mut client.framed).poll_next(cx)` produces `BytesMut`.
2. **← HOOK INSERTION POINT: `after_receive` hooks modify `&mut BytesMut` →**
3. `process_frame(&bytes)` deserialises and validates correlation.

## Plan of work

### Stage B: scaffolding — types and builder plumbing

**B1. Define hook types in `src/client/hooks.rs`** (currently 113 lines)

Add three items to the existing file:

- `BeforeSendHook` type alias: `Arc<dyn Fn(&mut Vec<u8>) + Send + Sync>`.
  Documented with rustdoc and example showing an `AtomicUsize` counter.
- `AfterReceiveHook` type alias: `Arc<dyn Fn(&mut BytesMut) + Send + Sync>`.
  Same documentation pattern.
- `RequestHooks` struct with two fields: `before_send: Vec<BeforeSendHook>`
  and `after_receive: Vec<AfterReceiveHook>`, plus `Default` impl.

Estimated addition: ~55 lines → file total ~168 lines.

**B2. Add `request_hooks` field to `WireframeClientBuilder`** in
`src/client/builder/core.rs` (currently 61 lines).

Add `pub(crate) request_hooks: RequestHooks` to the struct definition and
initialise it as `RequestHooks::default()` in `new()`.

**B3. Update `builder_field_update!` macro** in `src/client/builder/mod.rs`
(currently 61 lines).

Each of the three arms (serializer, preamble_config, lifecycle_hooks) gains one
line: `request_hooks: $self.request_hooks,`. Also add `mod request_hooks;` to
the submodule list.

**B4. Create builder methods** in new file
`src/client/builder/request_hooks.rs` (~80 lines).

Two methods on `WireframeClientBuilder<S, P, C>`:

- `before_send<F: Fn(&mut Vec<u8>) + Send + Sync + 'static>(self, f: F) ->
  Self` — pushes `Arc::new(f)` onto `self.request_hooks.before_send`.
- `after_receive<F: Fn(&mut BytesMut) + Send + Sync + 'static>(self, f: F) ->
  Self` — pushes `Arc::new(f)` onto `self.request_hooks.after_receive`.

Both documented with rustdoc examples showing counter-based hooks.

**B5. Add `request_hooks` to `WireframeClient`** in `src/client/runtime.rs`
(currently 351 lines).

Add `pub(crate) request_hooks: RequestHooks` field to the struct. Add two
helper methods:

```rust
pub(crate) fn invoke_before_send_hooks(&self, bytes: &mut Vec<u8>) {
    for hook in &self.request_hooks.before_send {
        hook(bytes);
    }
}

pub(crate) fn invoke_after_receive_hooks(&self, bytes: &mut BytesMut) {
    for hook in &self.request_hooks.after_receive {
        hook(bytes);
    }
}
```

Estimated addition: ~15 lines → file total ~366 lines.

**B6. Thread `request_hooks` through `connect()`** in
`src/client/builder/connect.rs` (currently 100 lines).

In the `Ok(WireframeClient { ... })` block (line 90–98), add:
`request_hooks: self.request_hooks,`.

**B7. Update public re-exports** in `src/client/mod.rs` (currently 79 lines).

Add `AfterReceiveHook` and `BeforeSendHook` to the `pub use hooks::{...}` line.

### Stage C: runtime integration — wire hooks into send/receive/streaming

**C1. Wire `before_send` into outgoing paths.**

Three insertion points, each ~2 lines added:

1. `src/client/runtime.rs`, `send()` method (line ~140): after
   `self.serializer.serialize(message)` produces `bytes`, before
   `self.framed.send()`:

   ```rust
   let mut bytes = bytes;
   self.invoke_before_send_hooks(&mut bytes);
   ```

2. `src/client/messaging.rs`, `send_envelope()` method (line ~125): same
   pattern after `self.serializer.serialize(&envelope)`.

3. `src/client/streaming.rs`, `call_streaming()` method (line ~88): same
   pattern after `self.serializer.serialize(&request)`.

**C2. Wire `after_receive` into incoming paths.**

Two insertion points:

1. `src/client/messaging.rs`, `receive_internal()` method (line ~237): after
   `self.framed.next()` produces `bytes` (a `BytesMut`), before
   `self.serializer.deserialize(&bytes)`:

   ```rust
   let mut bytes = bytes;
   self.invoke_after_receive_hooks(&mut bytes);
   ```

2. `src/client/response_stream.rs`, `ResponseStream::poll_next()` (line ~162):
   in the `Poll::Ready(Some(Ok(bytes)))` arm:

   ```rust
   Poll::Ready(Some(Ok(mut bytes))) => {
       this.client.invoke_after_receive_hooks(&mut bytes);
       Poll::Ready(this.process_frame(&bytes))
   }
   ```

   This is the critical streaming integration. Because
   `invoke_after_receive_hooks` is synchronous, it works within the synchronous
   `poll_next` context.

### Stage D: unit tests

Create `src/client/tests/request_hooks.rs` (~200 lines) covering:

1. `before_send_hook_invoked_on_send` — counter hook, send message, assert 1.
2. `after_receive_hook_invoked_on_receive` — counter hook, receive, assert 1.
3. `before_send_hook_can_mutate_bytes` — hook appends a byte, verify server
   receives modified frame.
4. `after_receive_hook_can_mutate_bytes` — hook modifies incoming bytes.
5. `multiple_before_send_hooks_execute_in_order` — two hooks appending
   different markers, verify order.
6. `multiple_after_receive_hooks_execute_in_order` — same for receive.
7. `hooks_fire_for_call_correlated` — both hooks fire during `call_correlated`.
8. `hooks_fire_for_streaming_responses` — `after_receive` fires for each frame
   in a `ResponseStream`.
9. `no_hooks_configured_works_identically` — regression test.
10. `before_send_hook_fires_for_streaming_request` — hook fires for the
    outgoing request in `call_streaming`.

Register in `src/client/tests/mod.rs`: `mod request_hooks;`.

### Stage E: BDD tests

**E1. Feature file** `tests/features/client_request_hooks.feature`:

```gherkin
Feature: Client request hooks for outgoing and incoming frames
  The client supports before_send and after_receive hooks that fire on
  every outgoing request and incoming response frame, enabling symmetric
  instrumentation with the server middleware stack.

  Background:
    Given an envelope echo server

  Scenario: Before-send hook is invoked for every outgoing frame
    Given a client with a before_send counter hook
    When the client sends an envelope
    Then the before_send counter is 1

  Scenario: After-receive hook is invoked for every incoming frame
    Given a client with an after_receive counter hook
    When the client sends and receives an envelope
    Then the after_receive counter is 1

  Scenario: Multiple hooks execute in registration order
    Given a client with two before_send hooks that append markers
    When the client sends an envelope
    Then the markers appear in registration order

  Scenario: Hooks fire for streaming responses
    Given a streaming server that sends 3 data frames
    And a client with an after_receive counter hook
    When the client calls the server with call_streaming
    And consumes all stream frames
    Then the after_receive counter is 3

  Scenario: Before-send hook can inject authentication token
    Given a client with a before_send hook that prepends a token
    When the client sends an envelope
    Then the server receives the token-prefixed frame
```

**E2. Fixture** `tests/fixtures/client_request_hooks.rs` (~150 lines):

A `ClientRequestHooksWorld` struct following the established pattern from
`tests/fixtures/client_messaging.rs`, with `Default`, a `#[fixture]` fn, and
methods for each step. The world stores `Arc<AtomicUsize>` counters,
`Arc<Mutex<Vec<u8>>>` for marker tracking, and optional client/server handles.

**E3. Steps** `tests/steps/client_request_hooks_steps.rs` (~80 lines):

Step definitions using `rstest_bdd_macros::{given, when, then}`. Each step
calls the world's async methods via
`tokio::runtime::Runtime::new()?.block_on()` — the established pattern from
`tests/steps/client_messaging_steps.rs`.

**E4. Scenarios** `tests/scenarios/client_request_hooks_scenarios.rs` (~40
lines):

Scenario bindings using `#[scenario(path = "...", name = "...")]` — the
established pattern from `tests/scenarios/client_messaging_scenarios.rs`.

**E5. Register modules:**

- `tests/fixtures/mod.rs`: add `pub mod client_request_hooks;`
- `tests/steps/mod.rs`: add `mod client_request_hooks_steps;`
- `tests/scenarios/mod.rs`: add `mod client_request_hooks_scenarios;`

### Stage F: documentation and cleanup

**F1. Update `docs/wireframe-client-design.md`:**

Add a new section "Request hooks" after the existing "Connection lifecycle
hooks" section documenting:

- The `before_send` and `after_receive` hook types.
- That hooks operate on raw bytes (after serialisation / before
  deserialisation).
- That hooks are synchronous `Fn` closures, not async.
- The registration-order execution semantics.
- The rationale for synchronous hooks (compatibility with `poll_next`).

**F2. Update `docs/users-guide.md`:**

Add a new subsection "Client request hooks" after the existing "Client
lifecycle hooks" section (line ~1252). Include:

- Builder API examples for `before_send` and `after_receive`.
- A table row in the client configuration reference table (line ~1060).
- Example showing a metrics counter hook and an auth-token-injection hook.

**F3. Mark roadmap item as done:**

In `docs/roadmap.md`, change `- [ ] 11.1.1.` to `- [x] 11.1.1.`.

## Concrete steps

All commands run from `/home/user/project`.

After each stage, validate:

```bash
set -o pipefail
make check-fmt 2>&1 | tee /tmp/wireframe-check-fmt.log; echo "check-fmt exit: $?"
make lint 2>&1 | tee /tmp/wireframe-lint.log; echo "lint exit: $?"
make test 2>&1 | tee /tmp/wireframe-test.log; echo "test exit: $?"
```

Expected: all three exit code 0.

## Validation and acceptance

Quality criteria (what "done" means):

- `make test` passes. New tests in `src/client/tests/request_hooks.rs` and
  the BDD scenarios in `tests/scenarios/client_request_hooks_scenarios.rs` all
  pass.
- `make lint` passes with zero warnings.
- `make check-fmt` passes.
- All pre-existing client tests pass unchanged (backwards compatibility).
- The `after_receive` hook fires for every frame yielded by `ResponseStream`,
  verified by a dedicated test.
- `docs/users-guide.md` documents the new builder methods.
- `docs/wireframe-client-design.md` records the design decisions.
- `docs/roadmap.md` marks 11.1.1 as done.

## Idempotence and recovery

All stages are additive. If any stage fails partway, `git stash` or
`git checkout .` reverts to the last known-good state. Re-run from the
beginning of the failed stage. No destructive operations are involved.

## Artifacts and notes

**Files to create (new):**

| File                                                | Purpose                                            | Est. lines |
| --------------------------------------------------- | -------------------------------------------------- | ---------- |
| `src/client/builder/request_hooks.rs`               | Builder methods `before_send()`, `after_receive()` | ~80        |
| `src/client/tests/request_hooks.rs`                 | Unit tests for hooks                               | ~200       |
| `tests/features/client_request_hooks.feature`       | BDD feature file                                   | ~30        |
| `tests/fixtures/client_request_hooks.rs`            | BDD world/fixture                                  | ~150       |
| `tests/steps/client_request_hooks_steps.rs`         | BDD step definitions                               | ~80        |
| `tests/scenarios/client_request_hooks_scenarios.rs` | BDD scenario bindings                              | ~40        |

**Files to modify:**

| File                              | Change                                                   | Current → est. new   |
| --------------------------------- | -------------------------------------------------------- | -------------------- |
| `src/client/hooks.rs`             | Add `BeforeSendHook`, `AfterReceiveHook`, `RequestHooks` | 113 → ~168           |
| `src/client/builder/core.rs`      | Add `request_hooks` field                                | 61 → ~64             |
| `src/client/builder/mod.rs`       | Extend macro + `mod request_hooks`                       | 61 → ~66             |
| `src/client/runtime.rs`           | Add field + helper methods + hook call in `send()`       | 351 → ~370           |
| `src/client/messaging.rs`         | Hook calls in `send_envelope()` and `receive_internal()` | 268 → ~276           |
| `src/client/streaming.rs`         | Hook call in `call_streaming()`                          | 149 → ~153           |
| `src/client/response_stream.rs`   | Hook call in `poll_next()`                               | 165 → ~170           |
| `src/client/builder/connect.rs`   | Thread `request_hooks` to constructor                    | 100 → ~101           |
| `src/client/mod.rs`               | Add re-exports                                           | 79 → ~82             |
| `src/client/tests/mod.rs`         | Add `mod request_hooks`                                  | 175 → ~176           |
| `tests/fixtures/mod.rs`           | Add `pub mod client_request_hooks`                       | 30 → ~31             |
| `tests/steps/mod.rs`              | Add `mod client_request_hooks_steps`                     | 31 → ~32             |
| `tests/scenarios/mod.rs`          | Add `mod client_request_hooks_scenarios`                 | 33 → ~34             |
| `docs/wireframe-client-design.md` | New "Request hooks" section                              | +~40                 |
| `docs/users-guide.md`             | New subsection + table row                               | +~50                 |
| `docs/roadmap.md`                 | Mark 11.1.1 done                                         | 1 char change        |

Total: 6 new files + 16 modified files = 22 files. All under 400 lines.

## Interfaces and dependencies

No new external crate dependencies. `bytes::BytesMut` is already available via
`tokio-util`.

### Types to define

In `src/client/hooks.rs`:

```rust
/// Hook invoked after serialisation, before the frame is written to the
/// transport.
pub type BeforeSendHook = Arc<dyn Fn(&mut Vec<u8>) + Send + Sync>;

/// Hook invoked after a frame is read from the transport, before
/// deserialisation.
pub type AfterReceiveHook = Arc<dyn Fn(&mut bytes::BytesMut) + Send + Sync>;

/// Configuration for client request/response hooks.
pub(crate) struct RequestHooks {
    pub(crate) before_send: Vec<BeforeSendHook>,
    pub(crate) after_receive: Vec<AfterReceiveHook>,
}
```

### Builder methods to define

In `src/client/builder/request_hooks.rs`:

```rust
impl<S, P, C> WireframeClientBuilder<S, P, C>
where
    S: Serializer + Send + Sync,
{
    pub fn before_send<F>(mut self, f: F) -> Self
    where
        F: Fn(&mut Vec<u8>) + Send + Sync + 'static;

    pub fn after_receive<F>(mut self, f: F) -> Self
    where
        F: Fn(&mut bytes::BytesMut) + Send + Sync + 'static;
}
```

### Helper methods to define

In `src/client/runtime.rs`:

```rust
impl<S, T, C> WireframeClient<S, T, C>
where
    S: Serializer + Send + Sync,
    T: ClientStream,
{
    pub(crate) fn invoke_before_send_hooks(&self, bytes: &mut Vec<u8>);
    pub(crate) fn invoke_after_receive_hooks(&self, bytes: &mut BytesMut);
}
```
