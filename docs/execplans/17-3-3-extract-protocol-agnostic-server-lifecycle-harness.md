# Extract a protocol-agnostic server lifecycle harness (17.3.3)

This ExecPlan (execution plan) is a living document. The sections `Constraints`,
`Tolerances (exception triggers)`, `Risks`, `Progress`,
`Surprises & discoveries`, `Decision log`, `Outcomes & retrospective`,
`Conformance basis`, and `Verification plan` must be kept up to date as work
proceeds.

Status: APPROVED — every decision accepted, including D23 option 1 on
2026-11-24. Implementation authorized on 2026-11-25 and in progress. PRE-1
([#974](https://github.com/leynos/wireframe/issues/974)), the server change
D23 selects, is delivered as prerequisite PR
[#977](https://github.com/leynos/wireframe/pull/977) beneath this branch; see
`Upstream prerequisite`.

[#962](https://github.com/leynos/wireframe/pull/962), merged on 2026-11-25, pins
`ServerShutdown`'s auto traits for
[#956](https://github.com/leynos/wireframe/issues/956). Line citations are to
`10809b0d`, that pull request's head before it merged; the measurements ran on
its patch-identical predecessor `a4d2218b`, with the same `src/server/`.

This revision replaces the 2026-08-23 design wholesale. That design wrapped a
raw `JoinHandle` and a `oneshot` shutdown sender, ran a detached cleanup task
from `Drop`, and carried an `Abandoned` outcome because aborting a server task
did not stop the server. Every upstream change it was waiting for has since
landed, and `main` now provides `WireframeServer::spawn()` and its
`ServerShutdown` control, which make most of that machinery unnecessary. The
harness below is a thin, protocol-neutral wrapper over `ServerShutdown`.
`Revision history` at the end records what changed and why.

## Purpose / big picture

Today the only way to get a running Wireframe server inside a test is
`wireframe_testing::spawn_wireframe_pair`. That helper builds the server for
you and hands back a client whose type is fixed to
`WireframeClient<BincodeSerializer, RewindStream<TcpStream>, ()>`. A downstream
crate that speaks a different protocol — its own frame codec, its own typed
handshake preamble, or its own client type — cannot use it. It must hand-roll
listener reservation, address capture, readiness waiting, shutdown, and
cleanup, which is where port races and hanging suites come from.

After this change a test author configures a `WireframeServer` however they
like — any serializer, codec, preamble, or worker count — hands the still
unbound server to `wireframe_testing::spawn_wireframe_server`, and gets back a
`RunningWireframeServer`: a small, non-generic handle that knows its bound
address, has already observed readiness, and can be stopped safely and
repeatedly.

Observable success, stated as behaviour a person can check:

1. `make test` passes, and the new integration test
   `server_harness_lifecycle::spawn_then_connect_round_trip` fails before the
   change and passes after it.
2. After `handle.shutdown().await` returns `Ok(())`, a TCP connect to
   `handle.local_addr()` is refused immediately, with no polling.
3. Calling `shutdown()` twice returns `Ok(())` both times, and
   `handle.shutdown_outcome()` reports `Some(ShutdownOutcome::Completed)` after
   each call.
4. A test can drop a `RunningWireframeServer` without calling `shutdown`, on a
   thread that has no Tokio runtime, and the listener is still released.
5. A server whose application factory returns an error fails
   `spawn_wireframe_server` with
   `TestError::Server(ServerError::FactoryBuild(_))` and leaves no listener
   bound.
6. The existing suites `tests/client_pair_harness.rs` and
   `tests/features/client_pair_harness.feature` pass unmodified after
   `spawn_wireframe_pair` delegates to the new helper.
7. `make test-wireframe-testing` reports the exhaustive tests of the pure
   outcome functions, and `make test` reports the four property tests.

## Constraints

Hard invariants. Violating one requires escalation, not a workaround.

1. `spawn_wireframe_pair` and `spawn_wireframe_pair_default` keep their exact
   signatures and return type `TestResult<WireframePair>`, and every
   *reachable* failure path keeps its `TestError` variant and message shape
   (Table 1). `WireframePair` stays concrete. The commitment is RFC 0001 §5.5
   and roadmap 17.3.3, which require the pair helpers to stay as additive,
   source-compatible wrappers for the existing harness audience;
   `wireframe_testing` is at 0.3.0 and has no `publish = false`
   (`wireframe_testing/Cargo.toml:3`). This is retained existing API, not a
   compatibility shim.
2. Do not add default type parameters to the new helpers, and do not narrow any
   bound below the `where` clause quoted in `Context and orientation`.
3. `ready_signal` is reserved for the harness. It cannot be *enforced*:
   `ready_signal` is an unconditional builder setter
   (`src/server/config/mod.rs:172`) and the server exposes no accessor that
   would reveal a caller-set sender. The harness overwrites it, so a
   caller-installed receiver never fires. Document the footgun; do not pretend
   it is prevented.
4. No source file may exceed 400 lines (`AGENTS.md`).
   `wireframe_testing/src/client_pair.rs` is already 387 lines, so EP-M3 must
   leave it shorter than it found it.
5. `Drop` for `RunningWireframeServer` performs no I/O, spawns nothing, takes
   no lock, and cannot panic. With this design it only drops fields.
6. Do not modify anything under `src/`. The harness is a consumer of the
   server's lifecycle API, as ADR 0014's migration plan states
   (`docs/adrs/adr-0014-server-shutdown-ownership-and-terminal-observation.md:254`).
   If a server change appears necessary, stop and escalate.
7. Do not touch the Makefile or CI workflows. Roadmap 17.3.5 has already made
   the companion crate a gated target; this plan needs nothing more.
8. No new entry in any `[dependencies]` or `[dev-dependencies]` table. The
   harness emits no log or trace events, so it needs neither `log` nor
   `tracing`.
9. Every new module begins with a `//!` comment, and every item, public or
   private, carries a `///` comment: the workspace lints deny `missing_docs` and
   `missing_docs_in_private_items` (`Cargo.toml:206`, `Cargo.toml:193`).
   Public functions returning `Result` carry `# Errors`.
10. No `unwrap` or `expect` outside test code; the workspace denies both
    (`Cargo.toml:154-155`), and `wireframe_testing` inherits the workspace
    lints (`wireframe_testing/Cargo.toml:25-26`).
11. All prose, comments, and identifiers use en-GB-oxendict spelling.
12. Red-Green-Refactor, with each test failing on its own assertion rather than
    on a shared compile error. See Stage B0.

## Tolerances (exception triggers)

Stop and escalate when any of these is reached. Counts exclude this ExecPlan
file. The comparison base is the branch directly beneath this one in its stack,
so the prerequisite layers do not count; with that branch as `BASE`, count net
lines with `git diff --numstat BASE...HEAD` and file counts with
`git diff --name-only BASE...HEAD`.

- **Code** (Rust sources and tests, including BDD feature files): more than
  1 500 net added lines. The estimate in `Plan of work` is about 1 250.
- **Configuration** (`Cargo.toml` files, lockfile, workflows, Makefile): any
  change at all. Constraints 7 and 8 make the budget zero.
- **Documentation** (Markdown other than this plan): more than 450 net added
  lines. The estimate is about 330.
- **Files**: more than 26 changed files. The estimate is 22.
- **Interface**: the public surface must grow beyond
  `Interfaces and dependencies`, or any existing public signature must change.
- **Constraint 1**: any change to a reachable cell of Table 1.
- **Iterations**: a single test still failing after four corrective attempts.
- **Flakiness**: any new test failing more than once in ten consecutive full
  `make test` runs. Run the full suite, not a focused target, because isolation
  removes the contention that surfaces timing races.
- **Case duration**: any property case exceeding its ten-second internal
  timeout. That is a hang, not slowness; do not raise the bound.
- **Time**: any milestone exceeding four hours of wall-clock work, excluding
  time spent queued for the shared compile pool.
- **Ambiguity**: two readings of RFC 0001 that would produce materially
  different public API.

Budget history: the 2026-08-23 first draft set 14 files and 1 200 lines and
breached both on paper before its second milestone; the expert review raised
them to 28 files and 1 800 lines for the `JoinHandle` design. This
re-derivation lowers them because `ServerShutdown` removes the detached cleanup
task, the drop-grace option, the transition table, and the diagnostics module,
and because `client_pair.rs` now shrinks. No other plan in `docs/execplans/`
touches this surface, so there is no further history to draw on.

## Progress

- [x] (2026-11-22) Plan re-derived against `a7eecaff`.
- [x] (2026-11-22) Decisions D1 to D22 accepted. D18 and D20 were reopened on
      2026-11-24 and accepted again with D23.
- [x] (2026-11-24) Rebased onto #962 and stacked; auto-trait probe compiled;
      in-flight listener behaviour measured. D23 raised.
- [x] (2026-11-24) D23 accepted: option 1. D18 and D20 stand again, subject to
      PRE-1.
- [x] (2026-11-25) PRE-1 raised as
      [#974](https://github.com/leynos/wireframe/issues/974).
- [x] (2026-11-25) Implementation authorized.
- [x] (2026-11-25) PRE-1 implemented and delivered as
      [#977](https://github.com/leynos/wireframe/pull/977), a prerequisite PR
      beneath this branch; the feature PR is retargeted onto it. This branch is
      rebased on `dcce5481`, whose `src/server/` contains the change, so fact 13
      and AXIOM-7 are re-read against it. It is not yet merged to `main`: this
      assignment carries no merge authority, and chapter 3 of the pull-request
      lifecycle places prerequisite work beneath the original PR instead of
      hiding it in the feature diff.
- [x] (2026-11-25) EP-M0 baseline recorded on the rebased tree: `make
      check-fmt`, `make lint`, `make typecheck` and `make test` are green on
      `dcce5481` (see `Artefacts and notes`).
- [ ] PRE-1 merged to `main`.
- [x] (2026-11-25) EP-M1 Harness, options, spawn helpers, rstest and BDD
      coverage: `server_harness` exists with the handle, options, pure outcome
      functions, and three spawn helpers; twelve rstest case instances and four
      behavioural scenarios pass.
- [x] (2026-11-25) EP-M2 Exhaustive and property tests: the three pure
      functions are enumerated exhaustively, and three property tests plus two
      named witnesses discharge INV-5 to INV-7.
- [x] (2026-11-25) EP-M3 `WireframePair` delegates; duplicated machinery
      removed: `Drop`, `PendingServer`, the bounded abort, and the `oneshot`
      sender are deleted, `client_pair.rs` shrank from 387 to 369 lines, and
      the two mapping functions carry Table 1.
- [x] (2026-11-25) EP-M4 Documentation, RFC amendment, roadmap tick.

## Risks

- Risk: PRE-1 is delayed, or lands in a form that does not release the listener
  while a request is in flight (fact 13). Severity: high. Likelihood: low.
  Mitigation: EP-M0 gates on PRE-1's own in-flight tests; the harness's
  in-flight cases fail without it, so a partial fix cannot pass unnoticed. Do
  not substitute a harness-owned abort; that is D23's rejected option 2.

- Risk: PRE-1 has been measured by five probes only, and the full suite has not
  been run with it. The listener field is shared by `spawn()` and
  `run_with_shutdown`, so both are affected. Severity: medium. Likelihood: low.
  Mitigation: PRE-1's acceptance requires the full suite.

- Risk: `wait_for_listener_release` has a fixed one-second budget
  (`wireframe_testing/src/integration_helpers.rs:79-80`). On a heavily loaded
  host the drop-path tests could time out. Severity: medium. Likelihood: low.
  Mitigation: once PRE-1 lands, release needs only the accept loops to exit, so
  one second is generous; `tests/server_shutdown.rs:47` already relies on the
  same helper for the same mechanism. If the flakiness tolerance trips,
  escalate rather than adding a second helper with a longer budget.

- Risk: `drained()` waits for in-flight connection tasks as well as accept
  loops (`src/server/runtime/server/mod.rs:157-165`;
  `src/server/connection_spawner.rs:74`). A test that leaves a request in
  flight and shuts down with no bound will hang. Severity: medium. Likelihood:
  medium. Mitigation: the harness default bounds the drain at five seconds; the
  pair, which keeps an unbounded drain, closes its client first. The bound test
  holds its handler on a `Notify` and releases it explicitly.

- Risk: the patch-coverage target is 80 % (`codecov.yml:4`), and two paths
  are reachable only defensively (Residual gaps). Severity: low. Likelihood:
  medium. Mitigation: the pure outcome functions carry those cases under
  exhaustive unit tests; if the ratchet still fails, report it rather than
  writing hollow tests.

- Risk: `proptest!` cannot take an `async fn`. Severity: low. Likelihood:
  certain. Mitigation: follow the compiled precedent at
  `tests/server_shutdown.rs:144-185` — a synchronous property body that builds a
  `current_thread` runtime and calls `block_on`, returning `TestCaseResult`.

## Context and orientation

Read this section first if you have never worked in this repository.

### What Wireframe is

Wireframe is a Rust library for building servers and clients that speak custom
binary protocols over TCP. The root package is `wireframe` (sources under
`src/`). The companion package `wireframe_testing` (`wireframe_testing/`) ships
test helpers. The workspace has four members: `.`, `crates/wireframe-loom`,
`crates/wireframe-verification`, and `wireframe_testing` (`Cargo.toml:15`).

### Vocabulary

- **Frame**: one codec-defined unit of bytes on the wire.
- **Preamble**: an optional typed handshake value exchanged once, before any
  framed traffic.
- **Typestate**: a marker type parameter encoding a compile-time phase.
  `WireframeServer` is `Unbound` before it owns a listener and `Bound`
  afterwards (`src/server/mod.rs:336`, `:340`).
- **Readiness signal**: a `tokio::sync::oneshot::Sender<()>` the server fires
  once, after spawning its accept-loop workers.
- **Server control**: `wireframe::server::ServerShutdown`, the cloneable handle
  `WireframeServer::spawn()` returns. `stop()` requests shutdown; `drained()`
  waits for the terminal outcome.
- **Drain**: the server's graceful stop — accept loops exit, the listener is
  released, and in-flight connection tasks finish.
- **Plateau**: a milestone end state that is correct, coherent, and safe to
  stop at.
- **Non-vacuity**: evidence that a passing check *could* have failed had the
  implementation been wrong.

### The server API the harness drives

Verified on `a7eecaff`. `WireframeServer` is declared at
`src/server/mod.rs:274-294`:

```rust
pub struct WireframeServer<
    F,
    T = (),
    S = Unbound,
    Ser = BincodeSerializer,
    Ctx = (),
    E = Envelope,
    Codec = LengthDelimitedFrameCodec,
>
```

The methods the harness drives:

```rust
// src/server/config/binding.rs:141-146, on Unbound
pub fn bind_existing_listener(self, std_listener: StdTcpListener)
    -> Result<BoundServer<F, T, Ser, Ctx, E, Codec>, ServerError>;

// src/server/config/mod.rs:172, on any typestate
pub fn ready_signal(self, tx: tokio::sync::oneshot::Sender<()>) -> Self;

// src/server/runtime/server/mod.rs:338, on Bound
pub async fn spawn(self) -> Result<ServerShutdown, ServerError>;

// src/server/shutdown.rs:91 and :123
pub fn stop(&self);
pub async fn drained(&self) -> Result<(), ServerError>;
```

The impl block providing `spawn` (`src/server/runtime/server/mod.rs:170-179`)
has exactly this `where` clause. Copy it verbatim onto the new helpers:

```rust
where
    F: AppFactory<Ser, Ctx, E, Codec>,
    T: Preamble,
    Ser: Serializer + FrameMetadata<Frame = Envelope> + Send + Sync + 'static,
    Ctx: Send + 'static,
    E: Packet + 'static,
    Codec: FrameCodec,
    Envelope: DecodeWith<Ser> + EncodeWith<Ser>,
```

Note that it now reads `E: Packet + 'static`; RFC 0001 §5.2 still lists
`E: Packet`.

### Facts that shape the design

Each drives a decision. Facts 1 to 12 and 14 were read from source; fact 13 was
measured.

1. **`spawn()` returns after preparation, not after readiness.** It builds and
   prepares the application, spawns the supervisor and an observer task, and
   returns the control (`src/server/runtime/server/mod.rs:338-364`). Readiness
   is sent later, inside the supervisor, after the accept loops are spawned
   (`:125-155`). So the harness must still install and await a readiness signal;
   `ready_signal` is honoured on the `spawn()` path (`:68-69`, `:151-155`).
2. **`spawn()` reports startup failures before returning a control.** The
   reachable errors are `ServerError::FactoryBuild`, when the application
   factory returns `Err` (`src/server/runtime/startup.rs:101-107`), and
   `ServerError::Prepare` (`startup.rs:131-137`). A factory returning
   `Result<WireframeApp, E>` is accepted through the closure blanket impl
   (`src/server/mod.rs:91-128`, `:226-240`).
3. **Nothing fails between `spawn()` returning and readiness.** The prepared
   path clones only `Arc`s, spawns onto the task tracker, records a metric, and
   sends readiness (`src/server/runtime/server/mod.rs:125-155`). The readiness
   receiver therefore observes a dropped sender only if the supervisor dies,
   which no public input can cause. That arm is defensive.
4. **`stop()` is synchronous and idempotent** — a `CancellationToken::cancel`
   (`src/server/shutdown.rs:91`).
5. **`drained()` is repeatable.** Each call reads the same `watch`-published
   terminal descriptor and returns an equivalent result
   (`src/server/shutdown.rs:123-154`). `ServerError` is not `Clone`
   (`src/server/error.rs:12`), but `drained()` rebuilds an equivalent error on
   every call, sharing the original through `Arc` in `source`.
6. **`drained()` returning `Ok` guarantees the listener is released.** Its
   documentation says so (`src/server/shutdown.rs:95-98`), and
   `tests/server_shutdown.rs:34` asserts a refused connection immediately after
   it, without polling.
7. **`drained()` never completes on its own.** It needs `stop()` or the drop
   of the last control (`src/server/runtime/accept.rs:263-266`).
8. **Dropping the last control cancels the server synchronously.** The shared
   inner state holds a `DropGuard` on the stop token
   (`src/server/shutdown.rs:47`, `:58`); dropping it calls `cancel()` without
   spawning, so no runtime is needed on the dropping thread.
   `tests/server_shutdown.rs:47` asserts the listener is released afterwards,
   with no request in flight. Fact 13 covers the other case.
9. **`ServerError::AbnormalTermination` is unreachable through a real
   server.** The supervisor's prepared path always returns `Ok(())`, a panic
   before readiness is impossible (fact 3), and cancellation requires aborting a
   `JoinHandle` that only the server's observer task owns. It is still
   constructible, with public fields `message` and `source`
   (`src/server/error.rs:32-38`): `source` is `Some` for a returned error and
   `None` for a panic or cancellation
   (`src/server/runtime/server/observer.rs:30-47`).
10. **`ServerShutdown` exposes no address and is not `Debug`**
    (`src/server/shutdown.rs:33-37`). The harness must capture the address
    itself and write a manual `Debug`.
11. **Read the address before binding.** `bind_to_listener` calls
    `set_nonblocking` then `TcpListener::from_std` on the same socket
    (`src/server/config/binding.rs:50-53`), so the address cannot change.
    Reading `local_addr()` from the std listener first yields a real
    `io::Error` instead of the `Option` that `Bound::local_addr` returns
    (`binding.rs:174`).
12. **`unused_listener()` binds `"localhost:0"`**
    (`wireframe_testing/src/integration_helpers.rs:52`), which may resolve to
    `::1`. Assert `addr.ip().is_loopback()`, never an IPv4 range.
13. **A request in flight no longer holds the listener open** (re-read on
    2026-11-25 against PRE-1, `dcce5481`). Before PRE-1 the supervisor kept its
    own `Arc` to the listener for the whole of `run_with_shutdown`, including
    the final `tracker.wait()`, so the accept loops exiting was not enough to
    release it. PRE-1 drops that handle immediately after spawning the accept
    loops, before readiness is signalled, so the socket's lifetime is the
    loops' lifetime and an in-flight request keeps only its connection task
    alive. Table 5 records the measurement that
    motivated the change, and PRE-1's own
    `tests/server_listener_release.rs` is the permanent evidence for the new
    behaviour.
14. **The pair still uses the pre-#657 machinery.** `client_pair.rs` spawns
    `run_with_shutdown` in a task with a `oneshot` shutdown sender
    (`:302-312`), awaits readiness without a bound (`:318`), joins without a
    bound (`:152`), and runs a detached 100 ms abort from `Drop` and from a
    `PendingServer` guard (`:167-237`). Both `abort()` call sites (`:202`,
    `:208`) disappear under this plan.

### Where tests live

Since 17.3.5, `make test` runs `test --workspace --all-targets --all-features`
(`Makefile:136`), so both the root package's `tests/` and `wireframe_testing`'s
own tests are gated locally. CI runs `make test-wireframe-testing`
(`.github/workflows/ci.yml:99`) and `make test-doc` (`:94`), and runs the root
package's tests inside the shared coverage action on pull requests
(`:176-186`); it does not run `make test` itself.

- Integration, behavioural, and property tests go in the root `tests/` tree,
  beside `tests/client_pair_harness.rs`, `tests/server_shutdown.rs`, and the
  BDD infrastructure.
- Behaviour-driven tests: features in `tests/features/`, worlds in
  `tests/fixtures/`, steps in `tests/steps/`, bindings in `tests/scenarios/`,
  all compiled into the `bdd` target, which requires `advanced-tests`
  (`Cargo.toml:280-283`); `--all-features` enables it. Registration lines:
  `tests/fixtures/mod.rs:12`, `tests/steps/mod.rs:11`,
  `tests/scenarios/mod.rs:15`.
- Unit tests of the harness's pure functions go in `#[cfg(test)]` modules
  inside `wireframe_testing`, where they can stay `pub(crate)`.

### Skills and documents

Load `leta` (run `leta workspace add .`), `rust-router` and from it
`rust-async-and-concurrency`, `rust-unit-testing`, `proptest`, `execplans`, and
`en-gb-oxendict`. Read
`docs/rfcs/rfc-0001-protocol-agnostic-test-harness-lifecycle.md` (§5.2, §5.3,
§5.5, §5.6, §6, §7.1, §7.5, §11),
`docs/adrs/adr-0014-server-shutdown-ownership-and-terminal-observation.md`,
`docs/wireframe-testing-crate.md` (the
`## In-process server/client pair harness` section at line 555), and
`docs/documentation-style-guide.md`. Read `docs/rstest-bdd-users-guide.md` and
`docs/wireframe-rstest-bdd-users-guide.md` before writing the BDD suite; step
functions are synchronous and fixture names must match step parameter names
exactly. Read `docs/rust-doctest-dry-guide.md` before writing doctests.

## Conformance basis

Upstream artefacts at `a7eecaff`:

- `docs/rfcs/rfc-0001-protocol-agnostic-test-harness-lifecycle.md`, Status
  Proposed, Created 2026-07-18, no revisions. The governing technical design.
  There is no separate Terms of Reference document.
- `docs/roadmap.md` item 17.3.3 (lines 839-858). 17.3.4 is out of scope;
  17.3.5 is complete.
- `docs/adrs/adr-0014-server-shutdown-ownership-and-terminal-observation.md`,
  Status Proposed. Its `Outstanding decisions` say "The testing harness
  migration is tracked separately and may choose its own convenience wrapper
  over `ServerShutdown`" (`:272-273`). This plan is that migration.
- ADRs 0011, 0012, and 0013, Status Proposed. ADR 0012's Phase 1 testkit
  drive helper, which an earlier revision feared would duplicate this work,
  shipped separately as `drive_prepared_with_frames`
  (`wireframe_testing/src/helpers/drive.rs:275`); it drives a prepared
  application over a duplex stream and does not overlap this harness.
- `AGENTS.md` and `docs/documentation-style-guide.md`.

### Upstream prerequisite

PRE-1 is the server change accepted in D23. It is not part of this plan, whose
Constraint 6 still forbids touching `src/`. It is
[#974](https://github.com/leynos/wireframe/issues/974), raised as a follow-up
to #655 and delivered in its own pull request before EP-M1.

Delivered on 2026-11-25 as
[#977](https://github.com/leynos/wireframe/pull/977), with the first of #974's
two candidate forms: the supervisor drops its listener handle immediately after
spawning the accept loops, which is the form #974 measured, so the socket's
lifetime is exactly the accept loops' lifetime. A first revision used the
second form, dropping before the tracker wait; review found that leaves the
contract conditional, because the supervisor still holds its handle while it
waits on the tracker when every loop ends unexpectedly. Its regression suite
`tests/server_listener_release.rs` covers `stop()`, dropping the last control,
and a resolved `run_with_shutdown` future, each with a response parked
mid-stream; all three fail without the change. This branch is rebased onto it,
so the harness is developed against the released-listener behaviour rather than
against a promise of it. Merge to `main` is outstanding work outside this
plan's authority.

- Change: in `PreparedServerRuntime::run_with_shutdown`
  (`src/server/runtime/server/mod.rs`), drop the supervisor's own listener
  handle once every accept loop holds its clone. The measured form was
  `drop(self.listener)` before the startup duration is recorded at line 148.
  #974 also offers dropping it after the stop request and before the tracker
  wait, which keeps the supervisor's ownership while running but has not been
  measured. Either form satisfies this plan. Correct the field's comment at
  line 70, and the description of `drained()` and drop behaviour in
  `docs/users-guide.md` and `docs/developers-guide.md`.
- Acceptance: with a request held in flight, the listener refuses connections
  after `stop()`, after the last control is dropped, and after a
  `run_with_shutdown` future is asked to stop, each before the request finishes;
  `drained()` still waits for the request; these tests fail without the
  change; and the full suite passes.

```plaintext
#962 (auto traits) -> EP-M3 [pins the traits the pair inherits]
PRE-1              -> EP-M1 [hard block: INV-4 in-flight cases, D18, D20]
PRE-1              -> EP-M3 [hard block: INV-8 drop with a request in flight]
```

The RFC predates `ServerShutdown`. Several of its sentences describe a
`JoinHandle`-and-`abort` mechanism this plan does not use; each is a recorded
deviation (D17 to D21) with a Stage E amendment. Trace links:

```plaintext
ROADMAP-17.3.3 -> RFC-5.2 -> EP-M1 -> tests/server_harness_lifecycle.rs::spawn_then_connect_round_trip
ROADMAP-17.3.3 -> RFC-5.2 -> EP-M1 -> tests/server_harness_lifecycle.rs::caller_listener_keeps_its_address
ROADMAP-17.3.3 -> RFC-5.3 -> EP-M1 -> tests/server_harness_lifecycle.rs::factory_error_fails_spawn_and_releases_listener
ROADMAP-17.3.3 -> RFC-5.3 -> EP-M1 -> wireframe_testing/src/server_harness/lifecycle.rs (await_readiness tests)
ROADMAP-17.3.3 -> RFC-5.3 -> EP-M1 -> tests/server_harness_lifecycle.rs::shutdown_is_idempotent
ROADMAP-17.3.3 -> RFC-5.3 -> EP-M1 -> tests/server_harness_lifecycle.rs::cancelled_shutdown_then_shutdown_completes
ROADMAP-17.3.3 -> RFC-5.3 -> EP-M1 -> tests/server_harness_lifecycle.rs::drop_inside_runtime_releases_listener
ROADMAP-17.3.3 -> RFC-5.3 -> EP-M1 -> tests/server_harness_lifecycle.rs::drop_outside_runtime_releases_listener
ROADMAP-17.3.3 -> RFC-5.3 -> EP-M1 -> tests/server_harness_lifecycle.rs::shutdown_bound_is_reported
ROADMAP-17.3.3 -> RFC-6   -> EP-M1 -> tests/server_harness_lifecycle.rs::parallel_handles_use_distinct_listeners
ROADMAP-17.3.3 -> RFC-5.3 -> INV-1 -> EP-M2 -> wireframe_testing/src/server_harness/outcome.rs (merge_outcome tests)
ROADMAP-17.3.3 -> RFC-6   -> INV-2 -> EP-M2 -> wireframe_testing/src/server_harness/outcome.rs (classify_drain tests)
ROADMAP-17.3.3 -> RFC-5.3 -> INV-3 -> EP-M2 -> wireframe_testing/src/server_harness/outcome.rs (drain_bound tests)
ROADMAP-17.3.3 -> RFC-6   -> INV-4 -> EP-M1 -> drop tests plus the seeded leaked-control fault
ROADMAP-17.3.3 -> RFC-7.5 -> INV-5 -> EP-M2 -> tests/server_harness_lifecycle_props.rs::shutdown_traces_converge
ROADMAP-17.3.3 -> RFC-7.5 -> INV-6 -> EP-M2 -> tests/server_harness_lifecycle_props.rs::connect_schedules_leave_nothing_behind
ROADMAP-17.3.3 -> RFC-7.5 -> INV-7 -> EP-M2 -> tests/server_harness_lifecycle_props.rs::parallel_handles_are_independent
ROADMAP-17.3.3 -> RFC-5.5 -> INV-8 -> EP-M3 -> wireframe_testing/src/client_pair.rs (pair error-mapping tests)
ROADMAP-17.3.3 -> RFC-5.6 -> EP-M3 -> wireframe_testing/src/server_harness/ (one copy of the lifecycle code)
```

RFC sections deliberately not discharged here:

- §5.4 (`spawn_wireframe_server_and_connect`) and §7.1's two connector bullets
  go to 17.3.4, because both presuppose the connector.
- §7.2 (protocol-generic proof) and §7.3 (`trybuild`) go to 17.3.4.
- §7.4 (companion-crate gates, issue #578) was discharged by 17.3.5 in
  `531d05cf`.

## Verification plan

### Axioms (assumed, not verified here)

- **AXIOM-1**: Tokio's `oneshot` delivers at most one value, and its receiver
  resolves `Err(RecvError)` if the sender is dropped unsent.
- **AXIOM-2**: `tokio::time::timeout` resolves `Err(Elapsed)` no earlier than
  its duration when the inner future has not completed.
- **AXIOM-3** (repository-owned, outside this plan's code): `stop()` is
  idempotent; every `drained()` call returns an equivalent result once the
  terminal outcome is published; `Ok` from `drained()` implies the listener is
  released; and dropping the last `ServerShutdown` clone requests shutdown.
  Evidence: ADR 0014 and `tests/server_shutdown.rs:34`, `:47`, `:88`, `:144`.
  This plan does not re-verify the server; it verifies that the harness relies
  on these properties correctly. It does **not** include prompt listener
  release while a request is in flight (fact 13); that is AXIOM-7.
- **AXIOM-4**: binding `"localhost:0"` yields a free ephemeral loopback port,
  and `bind_existing_listener` keeps the same socket.
- **AXIOM-5**: `spawn()` requires an entered Tokio runtime because it calls
  `tokio::spawn`; the spawn helpers are `async` and therefore always run inside
  one.
- **AXIOM-7** (supplied by PRE-1, not true at `10809b0d`): once shutdown is
  requested, the listener is released when the accept loops exit, whether or
  not a request is still in flight. Evidence:
  `tests/server_listener_release.rs` in
  [#977](https://github.com/leynos/wireframe/pull/977), which covers `stop()`,
  the last control's drop, and a resolved `run_with_shutdown` future with a
  response parked mid-stream; the last column of Table 5 records the
  measurement.

### The design decision that makes verification tractable

The harness's own logic is small, and almost all of it is a decision about a
value, not an effect. The implementation therefore isolates three pure, total
functions in `wireframe_testing/src/server_harness/outcome.rs`, which the
handle calls and which are tested exhaustively:

```rust
/// How a handle's shutdown attempts have concluded so far.
pub enum ShutdownOutcome { Completed, AbnormalTermination, TimedOut }

/// Classify one bounded drain into an outcome and the caller-facing result.
pub(crate) fn classify_drain(
    drained: Result<Result<(), ServerError>, Elapsed>,
) -> (ShutdownOutcome, TestResult<()>);

/// Fold a newly observed outcome into the handle's record.
pub(crate) const fn merge_outcome(
    previous: Option<ShutdownOutcome>,
    observed: ShutdownOutcome,
) -> ShutdownOutcome;

/// Choose the bound for the next drain.
pub(crate) const fn drain_bound(
    previous: Option<ShutdownOutcome>,
    configured: Option<Duration>,
) -> Option<Duration>;
```

The effectful parts — `stop()`, awaiting `drained()`, dropping the control —
are each a single call into the server API whose behaviour is AXIOM-3.
`shutdown()` is then four lines: stop, await the drain under `drain_bound`,
`classify_drain`, `merge_outcome`. The thing enumerated is the thing that runs.

### Invariants

**INV-1 — the outcome record is monotone.** `merge_outcome` is total, and
`Completed` and `AbnormalTermination` are absorbing: once recorded, no later
observation replaces them. `TimedOut` is replaced by any later observation.

- Method: exhaustive enumeration of
  `Option<ShutdownOutcome> × ShutdownOutcome`, twelve pairs.
- Rationale: a closed finite domain, so enumeration is the truth rather than a
  sample of it.
- Artefact: `#[cfg(test)]` module in `server_harness/outcome.rs`, test
  `merge_outcome_is_monotone`.
- Evidence: `make test-wireframe-testing`.
- Non-vacuity: assert the enumeration visited twelve pairs. Negative control:
  let `TimedOut` overwrite `Completed`; the absorption assertion must fail.

**INV-2 — failures are classified distinctly.** `classify_drain` maps a clean
drain to `(Completed, Ok(()))`, a server error to
`(AbnormalTermination, Err(TestError::Server(_)))` preserving the original
error, and an elapsed bound to `(TimedOut, Err(TestError::Timeout(_)))`.

- Method: exhaustive over the three input classes, with constructed inputs:
  `Ok(Ok(()))`, `Ok(Err(ServerError::AbnormalTermination { message, source }))`
  for both `source: None` and `source: Some(_)`, and an `Elapsed` obtained from
  `tokio::time::timeout(Duration::ZERO, std::future::pending::<()>()).await`.
- Rationale: the abnormal path is unreachable through a real server (fact 9),
  so this is the only place it can be exercised.
- Artefact: `server_harness/outcome.rs`, test
  `classify_drain_distinguishes_every_class`.
- Evidence: `make test-wireframe-testing`.
- Non-vacuity: assert the variant and the preserved `message`. Negative
  control: map the server error to `TestError::Msg`; the test must fail.

**INV-3 — a repeated shutdown after a timeout does not block.** `drain_bound`
returns `Some(Duration::ZERO)` when the previous outcome is `TimedOut`, and the
configured bound otherwise.

- Method: exhaustive over four previous outcomes times two configured classes
  (`None`, `Some(d)`), eight cases.
- Rationale: this is the harness's implementation of RFC §5.3's "after a
  timeout, a later call reports the same timeout failure rather than blocking
  again" (see D18).
- Artefact: `server_harness/outcome.rs`, test
  `drain_bound_never_blocks_after_timeout`.
- Evidence: `make test-wireframe-testing`, plus the runtime half in
  `shutdown_bound_is_reported`, which asserts the repeated call returns within
  50 ms.
- Non-vacuity: negative control — return the configured bound after
  `TimedOut`; the runtime test's 50 ms assertion must fail.

**INV-4 — no handle leaves a server running.** Each `RunningWireframeServer`
owns exactly one `ServerShutdown`, never clones it, and never exposes it, so
dropping the handle drops the last control and requests shutdown (AXIOM-3).
With AXIOM-7, the listener is then released even while a request is in flight.
The connection task serving that request is not cancelled; if its handler never
returns, the task lives until the runtime ends.

- Method: structural — the `control` field is private and the module contains
  no `.clone()` of it, checked in review with
  `rg -n 'control\.clone|&self\.control' wireframe_testing/src/server_harness` —
  plus runtime tests on every way a handle can end: explicit shutdown, drop
  inside a runtime, drop on a thread without a runtime, drop after a cancelled
  shutdown future, and a failed spawn.
- Rationale: the failure mode is a leaked clone, which is not observable from
  a single call but is observable as a listener that never releases.
- Artefact: `tests/server_harness_lifecycle.rs` drop and failure cases.
- Evidence: `make test`.
- Non-vacuity: seeded fault — in a scratch commit, leak a clone with
  `std::mem::forget(control.clone())` in the spawn helper; every drop test must
  then fail with `wait_for_listener_release`'s "remained bound for one second"
  error. Record the transcript and discard the commit. Second control: run the
  in-flight cases against a base without PRE-1; each must fail the same way.

**INV-5 — shutdown traces converge.** Any generated trace of completed and
cancelled `shutdown()` calls, followed by either a final `shutdown()` or
`Drop`, leaves the listener released; every completed call after the first
returns the same result as the first; and `shutdown_outcome()` never changes
once it is `Completed`.

- Method: property test. Operations: `Shutdown`, and `CancelledShutdown` (poll
  the future once with `futures::poll!`, then drop it); trace length `0..=5`;
  terminal action `Shutdown` or `Drop`.
- Rationale: covers RFC §7.5's first two bullets across orderings, against a
  real server, socket, and runtime.
- Artefact: `tests/server_harness_lifecycle_props.rs::shutdown_traces_converge`.
- Evidence: `make test`.
- Non-vacuity: generation alone does not guarantee that the material classes
  occur, so three named witness traces run as `#[rstest]` cases through the same
  `exercise_shutdown_trace` function: `Drop` with an empty trace (drop without
  shutdown), `[Shutdown]` then `Drop` (drop after shutdown), and
  `[CancelledShutdown]` then `Shutdown`. Negative control: make `shutdown()`
  skip `stop()` when the previous outcome is `None`; the drain then never
  finishes and every witness fails on the drain bound.

**INV-6 — connect schedules leave nothing behind.** After readiness, a
generated schedule of zero to three connections, each either closed by the
client or left open idle, followed by `shutdown()`, returns `Ok(())` within the
default bound, and a connect to the address is then refused.

- Method: property test.
- Rationale: RFC §7.5's third bullet; exercises the graceful drain with idle
  peers, which `tests/server_shutdown_idle_peer.rs:69` shows the server
  releases.
- Artefact:
  `tests/server_harness_lifecycle_props.rs::connect_schedules_leave_nothing_behind`.
- Evidence: `make test`.
- Non-vacuity: the connection count range includes zero and three; assert at
  least one generated case held an idle connection open across `shutdown()`.

**INV-7 — handles are independent.** A generated pool of two to four handles
binds distinct addresses; shutting down one, chosen by generated index, leaves
every other handle accepting connections.

- Method: property test, every server pinned to `.workers(1)`.
- Artefact:
  `tests/server_harness_lifecycle_props.rs::parallel_handles_are_independent`.
- Evidence: `make test`.
- Non-vacuity: the index is generated, so first, middle, and last positions are
  reachable; assert at least one handle survives. Negative control: give every
  handle a clone of one listener via `try_clone`; the distinct-address
  assertion must fail.

**INV-8 — pair compatibility.** Every reachable cell of Table 1 holds after
EP-M3.

- Method: the existing pair cases unchanged, exhaustive unit tests of the two
  extracted mapping functions, and one new characterization case,
  `drop_with_request_in_flight_releases_listener`, in
  `tests/client_pair_harness.rs`. Add that case before delegating and see it
  pass against today's pair; it must still pass afterwards.
- Artefact: `#[cfg(test)]` module in `wireframe_testing/src/client_pair.rs`;
  `tests/client_pair_harness.rs`; `tests/features/client_pair_harness.feature`.
- Evidence: `make test-wireframe-testing` and `make test`.
- Non-vacuity: each case asserts the variant and the message prefix. Negative
  control: return the typed variant instead of re-wrapping; every mapping case
  must fail.

All property tests use `cases: 16`, `max_shrink_time: 30_000`, and wrap each
case body in a ten-second `tokio::time::timeout` inside `block_on`, so a hang
becomes a printable, shrinkable failure. That internal timeout must not use
`ServerHarnessOptions`, which is the thing under test.

### Methods deliberately not used

- **A transition table over `Phase × Op`.** The previous revision's central
  verification device. With `ServerShutdown`, the handle has no lifecycle
  phases of its own: `stop()` and `drained()` are idempotent, so the only state
  the harness keeps is the outcome record, which INV-1 covers.
- **Stateright, Kani, Verus.** No concurrent protocol is introduced, and the
  pure functions have domains of twelve or fewer cases. RFC §7.5 says a model
  checker is not warranted in the first release.
  `crates/wireframe-verification` also cannot see `wireframe_testing`
  (`crates/wireframe-verification/Cargo.toml:11`).
- **`insta`.** There is no multivariant output to snapshot.

### Residual gaps

- `ShutdownOutcome::AbnormalTermination` is unreachable through a real server
  (fact 9) and is exercised only by INV-2's constructed inputs.
- The readiness arms for an elapsed bound and for a dropped sender are
  unreachable through the public API (fact 3). In-crate tests of
  `await_readiness` exercise both with a constructed receiver.
- `ServerError::Bind` from `bind_existing_listener` is reachable only if
  `set_nonblocking` or `TcpListener::from_std` fails, which a test cannot force.
- An `AbnormalTermination` from `drained()` "does not by itself prove that the
  drain completed" (`src/server/shutdown.rs:97-98`). If it ever occurs, the
  harness reports it but cannot claim the listener is released; `Drop` still
  releases the control.

## Interfaces and dependencies

At the end of EP-M3 the following exists, exactly. Submodules are private; only
`server_harness` is `pub`.

```rust
// wireframe_testing/src/server_harness/mod.rs
//! Protocol-agnostic server lifecycle harness.

mod handle;
mod lifecycle;
mod options;
mod outcome;

pub use self::{
    handle::RunningWireframeServer,
    lifecycle::{spawn_wireframe_server, spawn_wireframe_server_on, spawn_wireframe_server_with_options},
    options::ServerHarnessOptions,
    outcome::ShutdownOutcome,
};
```

```rust
// options.rs
/// Timeout policy for a [`RunningWireframeServer`].
#[derive(Clone, Debug, PartialEq)]
pub struct ServerHarnessOptions { /* private fields */ }

impl Default for ServerHarnessOptions {
    /// Five seconds to observe readiness and five seconds to drain on shutdown.
    fn default() -> Self;
}

impl ServerHarnessOptions {
    /// `None` waits indefinitely.
    #[must_use] pub const fn with_readiness_timeout(self, timeout: Option<Duration>) -> Self;
    /// `None` waits indefinitely, matching `WireframePair::shutdown`.
    #[must_use] pub const fn with_shutdown_timeout(self, timeout: Option<Duration>) -> Self;
    #[must_use] pub const fn readiness_timeout(&self) -> Option<Duration>;
    #[must_use] pub const fn shutdown_timeout(&self) -> Option<Duration>;
}
```

```rust
// outcome.rs
/// How a [`RunningWireframeServer`]'s shutdown attempts have concluded.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ShutdownOutcome {
    /// The server drained: accept loops exited and the listener is released.
    Completed,
    /// The server reported `ServerError::AbnormalTermination`.
    AbnormalTermination,
    /// Shutdown was requested but the drain did not finish within the bound.
    /// Not terminal: a later `shutdown` call may still observe completion.
    TimedOut,
}
```

```rust
// handle.rs
/// A bound, ready, running Wireframe server owned by a test.
///
/// Dropping this handle requests shutdown, so binding it to `_` stops a server
/// you probably meant to keep.
#[must_use]
pub struct RunningWireframeServer { /* local_addr, control, options, outcome */ }

impl RunningWireframeServer {
    /// The address the server was bound to.
    #[must_use] pub const fn local_addr(&self) -> SocketAddr;

    /// The most recent shutdown outcome, or `None` before any shutdown call
    /// has concluded.
    #[must_use] pub const fn shutdown_outcome(&self) -> Option<ShutdownOutcome>;

    /// Request shutdown and wait, under the configured bound, for the drain.
    ///
    /// # Errors
    ///
    /// Returns `TestError::Server` carrying `ServerError::AbnormalTermination`
    /// if the server terminated abnormally, or `TestError::Timeout` if the
    /// drain did not finish within the bound. A call after a timeout re-checks
    /// without waiting.
    pub async fn shutdown(&mut self) -> TestResult<()>;
}

impl Debug for RunningWireframeServer { /* local_addr and outcome only */ }
```

There is deliberately no `impl Drop`: dropping the `control` field is the whole
of the cleanup (D20).

```rust
// lifecycle.rs — the where clause is the verbatim block quoted in
// `Context and orientation`.
pub async fn spawn_wireframe_server<F, T, Ser, Ctx, E, Codec>(
    server: WireframeServer<F, T, Unbound, Ser, Ctx, E, Codec>,
) -> TestResult<RunningWireframeServer>;

pub async fn spawn_wireframe_server_on<F, T, Ser, Ctx, E, Codec>(
    server: WireframeServer<F, T, Unbound, Ser, Ctx, E, Codec>,
    listener: std::net::TcpListener,
) -> TestResult<RunningWireframeServer>;

pub async fn spawn_wireframe_server_with_options<F, T, Ser, Ctx, E, Codec>(
    server: WireframeServer<F, T, Unbound, Ser, Ctx, E, Codec>,
    listener: Option<std::net::TcpListener>,
    options: ServerHarnessOptions,
) -> TestResult<RunningWireframeServer>;
```

The four re-exports are added to `wireframe_testing/src/lib.rs` beside the
`client_pair` re-exports at `:45-46`, under the same `#[cfg(not(loom))]`.

No new dependency in any manifest. `futures` (for `poll!`) is already a
`wireframe_testing` dependency (`wireframe_testing/Cargo.toml:39`) and a root
dependency (`Cargo.toml:44`).

### Startup sequence

`spawn_wireframe_server_with_options` runs these steps, each mapping its
failure onto one `TestError` variant:

1. Use the supplied listener, or reserve one with `unused_listener()`
   (`TestError::Io`).
2. Read `local_addr()` from the std listener (`TestError::Io`).
3. Install a harness-owned readiness sender with `ready_signal` and call
   `bind_existing_listener` (`TestError::Server` carrying `Bind`).
4. Call `spawn().await` (`TestError::Server` carrying `FactoryBuild` or
   `Prepare`). From here the harness owns the control.
5. Await readiness under the readiness bound. On success, return the handle.
   On an elapsed bound, call `stop()`, await `drained()` under the shutdown
   bound, and return `TestError::Timeout` with the readiness `Elapsed`; if that
   cleanup also fails, return one `TestError::Msg` naming both failures, as RFC
   §5.4 prescribes for combined failures. On a dropped sender (defensive), also
   call `stop()` and await `drained()` under the shutdown bound, because a live
   server's drain never finishes unasked (fact 7); then return its error as
   `TestError::Server`, or a `TestError::Msg` stating that the server exited
   before signalling readiness if the drain was clean.

### Table 3: how a caller identifies the failing stage

RFC §6 asks that errors identify the lifecycle stage. With typed variants, the
stage is identified by the operation and the variant together, without parsing
text (D19):

<!-- markdownlint-disable MD013 -->

| Operation  | Variant                                         | Stage                           |
| ---------- | ----------------------------------------------- | ------------------------------- |
| `spawn_*`  | `TestError::Io`                                 | listener reservation or address |
| `spawn_*`  | `TestError::Server(ServerError::Bind(_))`       | bind                            |
| `spawn_*`  | `TestError::Server(FactoryBuild or Prepare)`    | application startup             |
| `spawn_*`  | `TestError::Timeout(_)`                         | readiness                       |
| `spawn_*`  | `TestError::Msg(_)`                             | readiness, with cleanup failure |
| `shutdown` | `TestError::Timeout(_)`                         | drain bound                     |
| `shutdown` | `TestError::Server(AbnormalTermination { .. })` | server termination              |

<!-- markdownlint-enable MD013 -->

## Milestones and plateaus

### EP-M0 — re-derived plan, accepted decisions, green baseline

- Outcome: this plan re-derived against `a7eecaff` (done 2026-11-22); every
  decision accepted (done 2026-11-24); implementation authorized (done
  2026-11-25); PRE-1 delivered as prerequisite PR #977 and present in this
  branch's tree at `dcce5481`, with fact 13 and AXIOM-7 re-read against it;
  `make check-fmt`, `make lint`, `make typecheck` and `make test` recorded
  green on the rebased tree; the Stage A auto-trait probe recorded (done
  2026-11-24).
- Acceptance evidence: gate transcripts under `/tmp`; the probe's outcome in
  `Surprises & discoveries`; PRE-1's merge to `main` remains outstanding and is
  the only EP-M0 item not satisfied as originally worded.
- Conformance check: confirm RFC 0001, ADR 0014, and the cited server API are
  unchanged since `a7eecaff`. If `main` has moved, re-verify every
  `Facts that shape the design` entry before EP-M1; a path-only check is not
  enough (see `Surprises & discoveries`, 2026-11-22).
- Recovery: nothing to undo.

### EP-M1 — the harness exists and is proven by example

- Outcome: `wireframe_testing::server_harness` exists with the handle, the
  options type, the pure outcome functions, and the three spawn helpers.
  `client_pair.rs` is untouched, so two implementations coexist; that matches
  RFC §8 stage 1 and is not compatibility machinery, because EP-M3 deletes the
  old one.
- Requirements: RFC §5.2, §5.3, §6, and §7.1 except its two connector bullets.
- Acceptance evidence: the nine cases in `tests/server_harness_lifecycle.rs`
  and the two in-crate `await_readiness` cases pass; the four scenarios in
  `tests/features/server_harness_lifecycle.feature` pass; INV-4's seeded fault
  is recorded.
- Conformance check: the `where` clause matches verbatim; no default type
  parameter; no file over its budget; submodules private; no `impl Drop`.
- Recovery: additive; revert the milestone's commits.
- Remaining gaps: no exhaustive or property coverage; RFC §5.6 not yet
  discharged.

### EP-M2 — the invariants are discharged

- Outcome: the exhaustive unit tests (INV-1 to INV-3) and the three property
  tests (INV-5 to INV-7) exist and pass, and each has been seen to fail against
  its negative control.
- Requirements: RFC §7.5.
- Acceptance evidence: `make test` and `make test-wireframe-testing` green;
  six negative-control transcripts; one classification run for INV-5.
- Conformance check: ten consecutive full `make test` runs with no
  intermittent failure.
- Recovery: additive.

### EP-M3 — the pair delegates and the duplication goes

- Outcome: `spawn_wireframe_pair` builds its default server, calls the harness's
  `bind_with_readiness` and `start_bound` in turn with both bounds set to
  `None` (Stage D), and maps failures through two extracted functions so that
  every reachable cell of Table 1 holds. `WireframePair::shutdown` closes the
  client, then calls the inner handle's `shutdown()`. `spawn_bounded_shutdown`,
  `PendingServer`, the `ServerShutdownHandle` alias, the `oneshot` shutdown
  sender, both `abort()` call sites, and `WireframePair`'s `impl Drop` are
  deleted. RFC §5.6 is discharged.
- Requirements: RFC §5.5, §5.6; INV-8.
- Acceptance evidence: every pre-existing pair test passes unmodified; the
  mapping unit tests pass; the auto-trait assertions hold.
- Conformance check: `git diff` shows both constructor signatures
  byte-identical; `client_pair.rs` is shorter than 387 lines.
- Recovery: the only milestone that changes existing behaviour; reverting it
  restores the EP-M2 plateau, where both implementations coexist and pass.
- Compatibility decision: required by Constraint 1; the named consumers are
  existing callers of `spawn_wireframe_pair`, including the repository's own
  suites.

### EP-M4 — documentation, RFC amendment, roadmap

- Outcome: the design record matches the code and roadmap 17.3.3 is ticked.
- Acceptance evidence: `make fmt`, `make markdownlint`, and `make nixie`
  pass; the roadmap checkbox is `[x]`.
- Conformance check: every deviation D17 to D21 has a matching RFC amendment.
  The plan moves to `COMPLETE` only then.
- Recovery: documentation only.

## Plan of work

### Per-file budget

<!-- markdownlint-disable MD013 -->

| File                                                | Budget   | Contents                                                       |
| --------------------------------------------------- | -------- | -------------------------------------------------------------- |
| `wireframe_testing/src/server_harness/mod.rs`       | 70       | module docs, private `mod`s, re-exports, one runnable doctest  |
| `wireframe_testing/src/server_harness/options.rs`   | 110      | `ServerHarnessOptions`, `Default`, builders, getters, doctests |
| `wireframe_testing/src/server_harness/outcome.rs`   | 240      | `ShutdownOutcome`, three pure functions, their unit tests      |
| `wireframe_testing/src/server_harness/handle.rs`    | 150      | the handle, `Debug`, accessors, `shutdown`                     |
| `wireframe_testing/src/server_harness/lifecycle.rs` | 220      | entry points, readiness helpers, their unit tests              |
| `wireframe_testing/src/client_pair.rs`              | net −130 | delegation, two mapping functions and their tests              |
| `tests/server_harness_lifecycle.rs`                 | 380      | the nine rstest cases and shared helpers                       |
| `tests/server_harness_lifecycle_props.rs`           | 300      | three property tests                                           |
| BDD feature, world, steps, scenarios                | 230      | four scenarios                                                 |

<!-- markdownlint-enable MD013 -->

*Table 2: per-file line budgets. Code total is about 1 250 net lines across 13
code files; documentation adds about 330 lines across 6 files; with the three
`mod.rs` registrations and `lib.rs`, 22 files change.*

If `outcome.rs` approaches 400 lines, move its tests to a sibling
`outcome_tests.rs` included with `#[cfg(test)] #[path = …] mod tests;`.

### Stage A — baseline and probes (no production code)

Load the skills. Run `leta workspace add .`. Read `client_pair.rs`,
`src/server/shutdown.rs`, and `src/server/runtime/server/mod.rs:100-394` in
full. Record a green baseline of `make check-fmt`, `make lint`, and `make test`.

The auto-trait probe is done. On 2026-11-24 a scratch test asserted with
`static_assertions::assert_impl_all!` that `WireframePair` and `ServerShutdown`
are `Send`, `Sync`, and `Unpin`; it compiled. EP-M3 adds the permanent
assertions for the `wireframe_testing` types. #962 adds them for
`ServerShutdown` and `ServerError`.

### Stage B0 — compiling stubs

Land `server_harness/` with every public item present, `outcome.rs` real (it is
pure and small), and every other body `todo!()`. This exists so that Stage B's
tests fail on their own assertions rather than all sharing one
`unresolved import` error.

### Stage B — red tests

`tests/server_harness_lifecycle.rs`, one `#[rstest]` per case, all
`#[tokio::test]` on the default `current_thread` flavour except case 7. Pin
`.workers(1)` on every server. Reuse `wait_for_listener_release` from
`wireframe_testing`; copy the small `assert_connection_refused` helper from
`tests/server_shutdown.rs:318` rather than adding a polling loop.

1. `spawn_then_connect_round_trip` — build a server from
   `echo_app_factory(&counter)`, spawn it, connect a `TcpStream`, and assert
   `handle.local_addr().ip().is_loopback()` with a non-zero port.
2. `caller_listener_keeps_its_address` — reserve with `unused_listener()`,
   record its address, pass it to `spawn_wireframe_server_on`, and assert the
   handle reports the recorded address.
3. `factory_error_fails_spawn_and_releases_listener` — a factory returning
   `Err(io::Error::other("refused"))`. Assert the spawn fails with
   `TestError::Server(ServerError::FactoryBuild(_))`, then
   `wait_for_listener_release` on the reserved address.
4. `shutdown_is_idempotent` — call `shutdown()` twice; assert both return
   `Ok(())`, that a connect is refused immediately after the first, and that
   `shutdown_outcome()` is `Some(ShutdownOutcome::Completed)` after each.
5. `cancelled_shutdown_then_shutdown_completes` — pin a `shutdown()` future,
   poll it once with `futures::poll!`, drop it, then call `shutdown()` again
   and assert `Ok(())` and a refused connect.
6. `drop_inside_runtime_releases_listener` — two `#[case]`s, idle and with a
   request held in flight by a handler waiting on a `Notify`. Record the
   address, drop the handle, assert the drop returned without awaiting, then
   `wait_for_listener_release`. Release the handler only after that assertion.
7. `drop_outside_runtime_releases_listener` — a plain `#[test]`, with the same
   idle and in-flight cases as case 6, that builds a runtime, spawns inside it,
   moves the handle to a `std::thread::spawn` with no runtime, drops it there,
   joins the thread, then calls `wait_for_listener_release` inside the runtime.
8. `shutdown_bound_is_reported` — one `#[rstest]` with two `#[case]`s, using a
   test app whose handler waits on a `Notify`. Send one request and wait until
   the handler has started. With a 1 ms bound: expect `TestError::Timeout(_)`
   and `TimedOut`; call `shutdown()` again at once and assert it returns
   `TestError::Timeout(_)` within 50 ms (INV-3); assert
   `wait_for_listener_release` succeeds while the request is still held (D18);
   release the handler, then repeat `shutdown()` under a one-second deadline
   until it returns `Ok(())` with `Completed`. A single call there would race
   the supervisor's drain. With a 5 s bound: release the handler from a second
   task and expect `Ok(())` and `Completed`.
9. `parallel_handles_use_distinct_listeners` — spawn three handles, assert
    distinct addresses, shut one down, and assert the other two still accept.

If the in-flight cases push this file towards 400 lines, move them to
`tests/server_harness_in_flight.rs`; the file tolerance allows for it.

Two in-crate `#[tokio::test]` cases in `lifecycle.rs` cover the readiness arms.
A zero-duration readiness timeout cannot: Tokio rounds timer deadlines up to
the next millisecond, so the supervisor signals readiness first (200 of 200
measured runs). Instead, pass `await_readiness` a real control and a receiver
whose sender the test holds:

- `elapsed_readiness_stops_the_server` retains the sender unsent under a 5 ms
  bound; assert `TestError::Timeout(_)`, then a refused connect.
- `dropped_readiness_sender_stops_the_server` drops the sender; assert the
  "exited before signalling ready" message, then a refused connect.

`tests/features/server_harness_lifecycle.feature`:

```gherkin
@server-harness-lifecycle
Feature: Protocol-agnostic server lifecycle harness

  Scenario: A configured server becomes ready and accepts a connection
    Given a configured Wireframe echo server that has not been bound
    When the server lifecycle harness starts the server
    Then the harness reports a loopback address
    And a client can open a connection to that address

  Scenario: Stopping the harness twice is safe
    Given a configured Wireframe echo server that has not been bound
    When the server lifecycle harness starts the server
    And the harness is stopped twice
    Then both stop attempts succeed
    And the harness reports that it completed
    And the address refuses new connections

  Scenario: Dropping the harness without stopping it releases the port
    Given a configured Wireframe echo server that has not been bound
    When the server lifecycle harness starts the server
    And the harness is dropped without being stopped
    Then the address stops accepting connections within one second

  Scenario: A failing application factory is reported at startup
    Given a configured Wireframe server whose application factory fails
    When the server lifecycle harness starts the server
    Then starting the server fails with an application startup error
```

The third scenario's `Then` is a bounded wait because the listener is released
once the supervisor observes cancellation, not at the instant of the drop; its
step calls `wait_for_listener_release`.

Add `tests/fixtures/server_harness_lifecycle.rs` with a
`ServerHarnessLifecycleWorld` owning a `tokio::runtime::Runtime`, an
`Option<RunningWireframeServer>`, the recorded address, the start result, and
the stop results, following `tests/fixtures/client_pair_harness.rs:21-55`: the
fixture function is `server_harness_lifecycle_world`, returns
`TestResult<ServerHarnessLifecycleWorld>`, and world methods use
`runtime.block_on`. Give the world an explicit `Drop` that takes the handle and
blocks on its `shutdown()`, discarding the result, as
`tests/fixtures/client_pair_harness.rs:125-131` does; a world dropped
mid-scenario must not leave a server for the next scenario. Steps in
`tests/steps/server_harness_lifecycle_steps.rs` are synchronous, delegate to
world methods, and name their parameter `server_harness_lifecycle_world`
throughout, because `strict-compile-time-validation` turns a mismatch into a
compile error. Scenario bindings in
`tests/scenarios/server_harness_lifecycle_scenarios.rs` take
`server_harness_lifecycle_world: Result<ServerHarnessLifecycleWorld, TestError>`,
as `tests/scenarios/client_pair_harness_scenarios.rs:13` does. Register each
in its `mod.rs`.

`tests/server_harness_lifecycle_props.rs` follows `tests/server_shutdown.rs`
for structure:

```rust
proptest! {
    #![proptest_config(ProptestConfig { cases: 16, max_shrink_time: 30_000, ..ProptestConfig::default() })]

    #[test]
    fn shutdown_traces_converge(
        trace in prop::collection::vec(trace_operation_strategy(), 0..=5),
        terminal in terminal_action_strategy(),
    ) {
        run_bounded_case(exercise_shutdown_trace(trace, terminal))?;
    }
}

/// Run one property case on a fresh runtime, failing instead of hanging.
fn run_bounded_case(case: impl Future<Output = TestCaseResult>) -> TestCaseResult {
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| TestCaseError::fail(error.to_string()))?;
    runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(10), case)
            .await
            .map_err(|_| TestCaseError::fail("case exceeded 10 s"))?
    })
}
```

Use `prop_assert_eq!` on `shutdown_outcome()` and on results' variants rather
than `prop_assert!` on booleans, so a shrunk counter-example prints actual
against expected. Commit any `.proptest-regressions` file proptest writes.

Stage B ends when every new test fails for a reason you have written down and
no pre-existing test has changed.

### Stage C — implementation

Replace the `todo!()` bodies file by file, running the focused tests after each.

`handle.rs`:

```rust
pub async fn shutdown(&mut self) -> TestResult<()> {
    self.control.stop();
    let bound = outcome::drain_bound(self.outcome, self.options.shutdown_timeout());
    let drained = match bound {
        Some(bound) => timeout(bound, self.control.drained()).await,
        None => Ok(self.control.drained().await),
    };
    let (observed, result) = outcome::classify_drain(drained);
    self.outcome = Some(outcome::merge_outcome(self.outcome, observed));
    result
}
```

Nothing is mutated before the `await` except the idempotent `stop()`, so a
cancelled `shutdown()` future leaves the handle exactly as a fresh call would
find it; that is how RFC §5.3's cancellation guarantee is met without a state
machine. The handle derives nothing in `Drop`; dropping `control` is the
cleanup.

`lifecycle.rs` implements `Startup sequence` with three small private helpers
so the entry point stays short: `bind_with_readiness` (steps 1 to 3),
`await_readiness` (step 5's wait), and `clean_up_after_failed_readiness` (step
5's cleanup). `spawn_wireframe_server` passes `None` for the listener and
default options; `spawn_wireframe_server_on` passes the listener and default
options.

Add the four re-exports to `wireframe_testing/src/lib.rs`.

### Stage D — the pair delegates

Extract two `pub(crate)` functions in `client_pair.rs` and unit-test each
exhaustively over the variants in Table 1:

- `map_pair_startup_error(TestError) -> TestError` re-wraps
  `TestError::Server(e)` as
  `TestError::Msg(format!("server failed to start: {e}"))` and passes every
  other variant through unchanged. Binding errors are also `TestError::Server`,
  and Table 1 keeps them typed, so the pair must not map them. Make that
  structural rather than conditional: `lifecycle.rs` exposes its two halves as
  `pub(crate)` functions — `bind_with_readiness` (steps 1 to 3, returning the
  bound server, its address, and the readiness receiver) and `start_bound`
  (steps 4 and 5) — and the pair calls them in turn, applying the mapping only
  to `start_bound`'s error. The defensive "exited before signalling readiness"
  message in `start_bound` must be exactly today's text,
  `server exited before signalling ready` (`client_pair.rs:330-332`).
- `map_pair_shutdown_error(TestError) -> TestError` re-wraps
  `TestError::Server(ServerError::AbnormalTermination { source: None, .. })` as
  `Msg("server task join error: …")` and any other `TestError::Server` as
  `Msg("server error: …")`, matching today's two shapes at
  `client_pair.rs:153-156`.

Then rewrite the pair: `Running` holds the client and a
`RunningWireframeServer`, client first; `shutdown` closes the client, then
awaits the inner `shutdown()` and maps its error; `Drop` is deleted, because
dropping `Running` drops the client and then the server handle, which requests
shutdown. Add the permanent auto-trait assertions from Stage A to
`tests/client_pair_harness.rs`, asserting exactly the traits Stage A recorded.

Run the full pre-existing suite. Any change to a pre-existing test's expected
output is a Constraint 1 breach: stop and escalate.

### Table 1: the pair's failure paths

Re-derived from `wireframe_testing/src/client_pair.rs` at `a7eecaff`.

<!-- markdownlint-disable MD013 -->

| Failure path                   | Today (`client_pair.rs`)                           | Reachable                                | After EP-M3                                           |
| ------------------------------ | -------------------------------------------------- | ---------------------------------------- | ----------------------------------------------------- |
| listener reservation           | `:294` `TestError::Io`                             | yes                                      | `TestError::Io` (unchanged)                           |
| binding                        | `:295-297` `TestError::Server`                     | no                                       | `TestError::Server` (unchanged)                       |
| address readback               | `:298-300` `Msg` on `None`                         | no                                       | `TestError::Io` from the std listener                 |
| application startup error      | `:327-329` `Msg("server failed to start: …")`      | yes                                      | `Msg("server failed to start: …")` (unchanged)        |
| readiness: task join error     | `:323-326` `Msg("server task failed to start: …")` | yes (a panicking factory or preparation) | unchanged text; startup runs in an abort-on-drop task |
| readiness: exited before ready | `:330-332` `Msg`                                   | no                                       | `Msg` (defensive arm, same text)                      |
| readiness: guard already taken | `:320-322` `Msg`                                   | no                                       | removed with `PendingServer`                          |
| client connect                 | `:337-338` `TestError::Client`                     | yes                                      | `TestError::Client` (unchanged)                       |
| shutdown: server error         | `:156` `Msg("server error: …")`                    | no                                       | `Msg("server error: …")` (same text)                  |
| drop with a request in flight  | `:167-210` listener released in about 200 ms       | yes                                      | released promptly, given PRE-1                        |
| shutdown: join error           | `:153-155` `Msg("server task join error: …")`      | no                                       | `Msg("server task join error: …")` (same text)        |

<!-- markdownlint-enable MD013 -->

Every reachable cell is unchanged. The drop cell holds only with PRE-1, which
is why EP-M3 depends on it. The unreachable cells that change do so because the
machinery that produced them — a `JoinHandle`, a pending-server guard, the
`Option` from `Bound::local_addr` — no longer exists (D21).

### Stage E — documentation

1. Add `## Protocol-agnostic server lifecycle harness` to
   `docs/wireframe-testing-crate.md`, after
   `## In-process server/client pair harness` (line 555), with `### Public API`,
   `### Lifecycle`, `### Timeout policy`, `### Usage`, and `### Rationale`.
   Update `## Crate layout` (line 26). Include Table 3.
2. Expand the `spawn`/`stop`/`drained` paragraph in `docs/users-guide.md`
   under `## Running servers` (lines 1872, 1907-1915) with a pointer to the
   harness for tests, the `ready_signal` reservation, and the fact that the
   harness's `Drop` is a synchronous shutdown request.
3. Add a subsection to `docs/developers-guide.md` under
   `## Test infrastructure and framework` (line 2356) recording the async
   proptest idiom with a citation to `tests/server_shutdown.rs:144`, the
   pure-outcome-function pattern, and the rule that a test lifecycle helper
   wraps `ServerShutdown` rather than spawning `run_with_shutdown` itself.
4. Amend RFC 0001, appending a dated revision note:
   - §5.2: the handle holds the address, a `ServerShutdown`, the timeout
     policy, and the outcome record; step 4 becomes "start the server with
     `WireframeServer::spawn`"; the `Send + 'static` paragraph and the
     100 ms `Drop` grace period are removed; the bound list gains
     `E: Packet + 'static`.
   - §5.3: replace "bounds the join and aborts the task only after timeout"
     with a bounded drain after `stop()` and no abort (D18); describe a
     repeated call after a timeout as a non-blocking re-check; replace the
     detached-cleanup `Drop` paragraph with D20's synchronous release; restate
     startup failure handling per `Startup sequence`.
   - §5.5: the pair calls `spawn_wireframe_server_with_options` with no
     bounds until 17.3.4 supplies the connector; cite Table 1.
   - §5.6: list `handle.rs`, `lifecycle.rs`, `options.rs`, and `outcome.rs`;
     `connector.rs` arrives with 17.3.4.
   - §6: replace the "one shutdown sender and one task handle" invariant with
     "exactly one `ServerShutdown`, never cloned or exposed"; replace "join
     the server task" with "await the drain"; replace the stage-text
     requirement with Table 3 (D19); note that a panic now surfaces as
     `ServerError::AbnormalTermination` with `source: None` rather than as a
     `JoinError` (D21).
   - §7.1: replace the drop bullet's detached-task clauses with "`Drop`
     returns without waiting and the listener is released inside and outside
     a runtime".
   - §7.5: replace "exactly one joined task" with "one shutdown request and
     one released listener".
   - §11: record question 1 as resolved in favour of `ServerHarnessOptions`
     (D2); leave question 2 for 17.3.4.
5. Add no ADR. RFC 0001 is the governing design for this feature, and ADR
   0014 already anticipates a harness-owned wrapper. If a reviewer requires
   one, the next free number at `a7eecaff` is 0018 (`docs/adrs/adr-0018-…`),
   named and structured per `docs/documentation-style-guide.md` (from line
   520); recheck before writing.
6. Tick roadmap 17.3.3 with `mapsplice`, preserving its inline links verbatim.
7. Add an entry under `## Unreleased` in `CHANGELOG.md` (line 5), following
   the existing `wireframe_testing` entries at lines 131-151.

## Concrete steps

Run everything from the repository root. Gates:

```bash
export LOG_PREFIX="/tmp/17-3-3-$(git branch --show-current)"
set -o pipefail && make check-fmt 2>&1 | tee "${LOG_PREFIX}-check-fmt.log"
set -o pipefail && make lint      2>&1 | tee "${LOG_PREFIX}-lint.log"
set -o pipefail && make test      2>&1 | tee "${LOG_PREFIX}-test.log"
set -o pipefail && make test-wireframe-testing 2>&1 | tee "${LOG_PREFIX}-testing-crate.log"
set -o pipefail && make test-doc  2>&1 | tee "${LOG_PREFIX}-doc.log"
```

Expected at the tail of each green test log:

```plaintext
test result: ok. <N> passed; 0 failed; 0 ignored; 0 measured; <M> filtered out
```

For focused loops, reuse `make test`'s exact flags so the build fingerprint
matches and nothing recompiles: print the invocation with `make -n test` and
append `--test server_harness_lifecycle`,
`--test server_harness_lifecycle_props`, `--test client_pair_harness`, or
`--test bdd -- server_harness`. On this machine every compile waits for a slot
in the shared build-limits pool; a `waiting for a compile slot` line is
queueing, not a hang.

Flakiness check, once at the end of EP-M2, using the full suite:

```bash
for i in $(seq 1 10); do
  set -o pipefail && make test 2>&1 | tee "${LOG_PREFIX}-full-run-${i}.log" | tail -3
done
```

Documentation gates at EP-M4:

```bash
make fmt
set -o pipefail && make markdownlint 2>&1 | tee "${LOG_PREFIX}-markdownlint.log"
set -o pipefail && make nixie        2>&1 | tee "${LOG_PREFIX}-nixie.log"
```

`make doctest-benchmark` is not used: `scripts/doctest-benchmark.sh:13` searches
`src` only and cannot see `wireframe_testing`. `make test-doc` is the doctest
evidence.

Commit after each milestone and gate each commit. Delegate full gate runs to the
`scrutineer` sub-agent, and read the cited log on failure rather than
re-running.

## Validation and acceptance

### Red-Green-Refactor evidence

- **Red**: after Stage B0, each of the nine rstest cases fails on its own
  assertion or on a `todo!()` naming the function it reached. Paste the nine
  distinct failure lines.
- **Green**: the focused `server_harness_lifecycle` run reports nine passing
  cases after Stage C.
- **Refactor**: after the Stage C tidy-up and again after Stage D, the focused
  runs and `make test` pass.

### BDD evidence

The focused `bdd` run filtered to `server_harness` fails before Stage C and
reports four passing scenarios after it.

### Verification evidence

For each of INV-1 to INV-8 record the command, the passing result, the negative
control or seeded fault applied, and the failure it produced. An invariant
whose control still passes has not been verified.

### Quality criteria

- Tests: `make test`, `make test-wireframe-testing`, and `make test-doc` green
  with no `#[ignore]`.
- Lint and format: `make check-fmt` and `make lint` clean. Both now cover
  `wireframe_testing`, so the workspace lints apply to every new line.
- Documentation: `make markdownlint` and `make nixie` pass.
- Performance: the new tests add no more than ten seconds of test execution to
  `make test`. A full spawn-ready-connect-shutdown cycle on loopback runs in
  milliseconds; if the budget is breached, consolidate targets rather than
  reducing generated case counts.
- Security: none applicable; test-only scaffolding on loopback.

## Idempotence and recovery

Every stage is re-runnable and every gate is a pure read. EP-M1 and EP-M2 are
additive. EP-M3 is the only milestone that changes existing behaviour;
reverting it alone returns the repository to the EP-M2 plateau, where the new
harness and the old pair implementation coexist and both pass.

If a test hangs, it is almost always a drain waiting on an in-flight request
with no bound. Reproduce with the focused invocation plus
`-- --nocapture --test-threads=1`. Do not add a sleep.

If a run is interrupted, check `ss -ltn` for stray loopback listeners before
re-running.

## Surprises & discoveries

Entries below were read from source at `a7eecaff` on 2026-11-22 unless marked
measured or historical.

- Observation (measured, 2026-11-24): a request in flight keeps the listener
  accepting after `stop()` and after the last control is dropped. Each probe
  held one request in a handler and polled for a refused connect for two
  seconds. The last column repeats the probes with `drop(self.listener)` added
  after the accept loops are spawned (`src/server/runtime/server/mod.rs:148`),
  a change since reverted.

  <!-- markdownlint-disable MD013 -->

  | Probe                               | Connect refused while the request is stuck | With the listener dropped early |
  | ----------------------------------- | ------------------------------------------ | ------------------------------- |
  | `stop()`, client open               | no; only after the handler finished        | yes, about 100 ms               |
  | last control dropped, client open   | no; only after the handler finished        | yes, about 100 ms               |
  | last control dropped, client closed | no; only after the handler finished        | yes, about 100 ms               |
  | today's `WireframePair` dropped     | yes, about 200 ms                          | yes, about 100 ms               |

  <!-- markdownlint-enable MD013 -->

  *Table 5: when the listener stops accepting with a request in flight.*

  Impact: D18 and D20 claimed prompt release from reading the accept loop
  without reading the supervisor that also holds the listener. Both are
  reopened, and D23 raised. The five probes ran alone; the full suite has not
  been run with the listener dropped early.

- Observation (measured, 2026-11-24): `timeout(Duration::ZERO, ready_rx)` after
  `spawn()` returned readiness in 200 of 200 runs. Tokio rounds deadlines up to
  the next millisecond. Impact: the former AXIOM-6 was false; the readiness
  arms are tested in-crate with a constructed receiver.

- Observation: a rebase audit that checked only that the plan's cited paths
  still existed missed a false claim: the plan said `docs/repository-layout.md`
  did not exist, but it was added on 2026-09-27 in #705 (`1dadc1cc`). Evidence:
  the file is present and linked from `docs/contents.md:158`. Impact: EP-M0's
  conformance check now requires re-verifying every fact, not just every path.
  The file itself omits `crates/wireframe-loom`, raised as
  [#944](https://github.com/leynos/wireframe/issues/944).

- Observation (measured, 2026-11-24): `WireframePair` and `ServerShutdown` are
  `Send`, `Sync`, and `Unpin`; a scratch `assert_impl_all!` compiled. #962 pins
  `ServerShutdown: Send + Sync + Unpin` (`src/server/shutdown.rs:40`) and
  `ServerError: Send + Sync` (`src/server/error.rs:42`), closing the gap raised
  as [#956](https://github.com/leynos/wireframe/issues/956). Impact: delegation
  cannot remove an auto trait from `WireframePair`.

- Observation: `WireframeServer::spawn()` and `ServerShutdown` remove the
  need for a harness-owned task handle, detached cleanup, and abort. Evidence:
  `Facts that shape the design`, items 4 to 8. Impact: D17; the previous
  revision's transition table, drop-grace option, and diagnostics module are
  withdrawn.

- Observation: `spawn()` does not wait for readiness. Evidence:
  `src/server/runtime/server/mod.rs:338-364` against `:151-155`. Impact: the
  harness still installs and awaits `ready_signal`.

- Observation: `FactoryBuild` is now the reachable pre-readiness fault, and
  nothing between `spawn()` returning and readiness can fail. Evidence: facts 2
  and 3. Impact: the previous revision's panicking-`Clone` test is replaced by
  a factory returning `Err`; the dropped-sender readiness arm is defensive.

- Observation: `ServerError::AbnormalTermination` is unreachable through a
  real server, but its `source` field distinguishes a returned error (`Some`)
  from a panic or cancellation (`None`). Evidence: fact 9. Impact: D21; INV-2
  uses constructed inputs.

- Observation: RFC 0001 has no §11.1 or §11.2; §11 holds two numbered
  questions. Its §6 says "Explicit shutdown and connector-failure cleanup join
  the server task" (`:363`); earlier revisions of this plan misquoted it as
  "every cleanup path". Impact: citations corrected.

- Observation: `make lint` and `make test` are workspace-wide and
  `wireframe_testing` inherits the workspace lints. Evidence: `Makefile:136`,
  `Makefile:216-234`; `wireframe_testing/Cargo.toml:25-26`. Impact: the
  previous D10 is superseded; every new line is linted.

- Observation: CI does not run `make test`; it runs `make test-doc`,
  `make test-wireframe-testing`, and the root package's tests inside the shared
  coverage action on pull requests. Jobs and test steps carry `timeout-minutes`
  (`.github/workflows/ci.yml:61`, `:87`, `:93`, `:98`), and
  `.config/nextest.toml:14-15` terminates slow tests. Impact: the previous D11
  is superseded; documentation must not call `make test` the CI gate.

- Observation: `crates/wireframe-verification` still depends only on
  `stateright` (`crates/wireframe-verification/Cargo.toml:11`). It now has a
  `make test-verification` target (`Makefile:279-280`), which no workflow runs.
  Impact: D5 stands.

- Observation: `scripts/doctest-benchmark.sh:13` still defaults to `src`.
  Impact: `make test-doc`, not `make doctest-benchmark`, is the doctest
  evidence.

- Observation: a compiled async-proptest precedent exists at
  `tests/server_shutdown.rs:144-185`, and another at
  `src/connection/tests/vec_delivery.rs:126-141`.
  `tests/advanced/interaction_fuzz.rs` is still compiled by no target. Impact:
  follow `tests/server_shutdown.rs`; reporting the dead file is outwith this
  plan.

- Observation: `wireframe_testing` already exports
  `wait_for_listener_release` (`wireframe_testing/src/lib.rs:107-108`,
  `integration_helpers.rs:79`). Impact: the previous revision's planned
  `await_port_released` helper is unnecessary.

- Observation: `unused_listener()` binds `"localhost:0"`, which may resolve
  to `::1` (`wireframe_testing/src/integration_helpers.rs:52`). Impact: assert
  `is_loopback()`.

- Observation: `client_pair.rs` is 387 lines (Constraint 4). Impact: EP-M3
  must shrink it, which the deletions in Stage D do.

- Observation (historical, 2026-08-23): `JoinHandle::abort()` did not stop a
  Wireframe server, because aborting the supervisor neither cancelled its
  workers' token nor stopped its tracker. Resolved upstream by #656 (a drop
  guard in the supervisor) and #657 (`ServerShutdown`). Retained because it
  explains why this plan depends on `ServerShutdown` rather than on task abort.

## Decision log

D1 to D22 were accepted by leynos on 2026-11-22, by instruction to mark all
outstanding decisions accepted; D17 to D22 were introduced by that day's
re-derivation. D18 and D20 were reopened on 2026-11-24, when their premise was
disproved, and accepted again the same day with D23, which makes that premise
true.

- Decision (D23): release the listener while a request is in flight by changing
  the server (option 1), delivered as PRE-1 (#974). Accepted by leynos on
  2026-11-24; options 2 and 3 are rejected. Without that change the listener
  stays bound until the request finishes (Table 5). That contradicts D18 and
  D20, and makes EP-M3 regress a reachable behaviour of `WireframePair`,
  breaching Constraint 1. Affected: RFC-5.3, RFC-5.5, INV-4, D17, D18, D20,
  EP-M3.
  - Option 1, accepted: change the server to drop its own listener handle
    once the accept loops are spawned. One line, and every probe in Table 5
    then passes. It lies in `src/`, outwith Constraint 6, so it has its own issue,
    #974, and needs its own pull request. `drained()` still waits for the
    request, so D18 and D20 become true as written.
  - Option 2, rejected: drive `run_with_shutdown` in a harness-owned task and
    abort it after a grace period, as the pair does today. No server change, but
    it abandons D17 and restores the detached cleanup the re-derivation removed.
  - Option 3, rejected: accept the weaker guarantee, amend RFC 0001 and
    Constraint 1, and document that a stuck request keeps the port bound until
    the runtime ends.
  Date/Author: 2026-11-24, planning agent.

- Decision (D1): scope to RFC §5.2, §5.3, §5.5, §5.6, §6, §7.1 except its
  connector bullets, and §7.5. The connector, the protocol-generic proof, and
  `trybuild` go to 17.3.4. Known cost: RFC §5.5 has the pair call the connector
  helper, so 17.3.4 rewrites `spawn_wireframe_pair`'s body again and re-runs
  INV-8. Date/Author: 2026-08-23, planning agent. Accepted 2026-11-22.

- Decision (D2): introduce a public `ServerHarnessOptions`, resolving RFC §11
  question 1 in the affirmative, now with two fields. Rationale: without an
  injectable drain bound the timeout path in RFC §7.1 is testable only with
  five-second sleeps, and the pair cannot express its unbounded waits. Both
  fields are `Option<Duration>` so the pair passes `None`. `Copy` and `Eq` are
  deliberately not derived, as permanent commitments a future field could
  break. The previous third field, a `Drop` grace period, is withdrawn with
  D20. Date/Author: 2026-08-23, revised 2026-11-22. Accepted 2026-11-22.

- Decision (D3): `shutdown()` is idempotent in effect and stable in result.
  Every call issues the idempotent `stop()`; once the drain has concluded,
  every later call returns an equivalent result, because `drained()` itself
  does (AXIOM-3); after a timeout, a later call re-checks without blocking
  (D18). `shutdown_outcome()` exposes the record without awaiting. This
  replaces the previous revision's "return `Ok(())` on every later call, and
  lend the error through `shutdown_error()`", which existed only because a
  joined `JoinHandle` cannot be joined twice and `ServerError` is not `Clone`.
  `drained()` rebuilds an equivalent error on every call, so the typed error is
  simply returned again and `shutdown_error()` is withdrawn. This matches RFC
  §5.3's "returns the outcome of the terminal state already reached" more
  closely than the previous design did. Date/Author: 2026-08-23, revised
  2026-11-22. Accepted 2026-11-22.

- Decision (D4): split RFC §5.6's `lifecycle.rs` into private submodules
  `handle.rs`, `lifecycle.rs`, `options.rs`, and `outcome.rs` under one
  `pub mod server_harness`. Rationale: the 400-line limit, and keeping the file
  layout out of the public API. Date/Author: 2026-08-23, revised 2026-11-22.
  Accepted 2026-11-22.

- Decision (D5): withdraw the Stateright model. Rationale: a model in
  `crates/wireframe-verification` could not reference `wireframe_testing`;
  Stateright cannot prove an exhaustive search when the depth bound truncates
  it; and RFC §7.5 says a model checker is not warranted in the first release.
  With `ServerShutdown` the case is stronger still: the harness has no
  concurrent protocol to model. Date/Author: 2026-08-23. Accepted 2026-11-22.

- Decision (D6): do not use Kani, Verus, or `insta`. Reasoning in
  `Methods deliberately not used`. Date/Author: 2026-08-23. Accepted 2026-11-22.

- Decision (D7): put integration, property, and behavioural tests in the root
  `tests/` tree, and pure-function unit tests in `#[cfg(test)]` modules inside
  `wireframe_testing`. Rationale: the BDD infrastructure, the pair suites, and
  the closest precedent (`tests/server_shutdown.rs`) all live in root `tests/`,
  and CI's coverage action runs them; in-crate unit tests keep the pure
  functions `pub(crate)` rather than `#[doc(hidden)] pub`, and
  `make test-wireframe-testing` runs them in CI. The previous rationale, that
  companion-crate tests were invisible to `make test`, no longer holds.
  Date/Author: 2026-08-23, revised 2026-11-22. Accepted 2026-11-22.

- Decision (D9): Table 1 is the compatibility obligation, cell by cell, and
  applies to reachable paths. Rationale: an early draft claimed the pair's
  startup failures were uniformly `TestError::Msg`, which was false; a blanket
  re-wrap would have broken the typed `Io` and `Client` paths. Unreachable
  cells may change only where their producing machinery is deleted (D21).
  Date/Author: 2026-08-23, revised 2026-11-22. Accepted 2026-11-22.

- Decision (D12): keep three free spawn functions rather than a builder.
  Rationale: RFC §5.2 names two of them, and the options variant is the minimum
  addition. Recommendation for 17.3.4: introduce a builder then, rather than a
  fourth function at the argument-count ceiling. Date/Author: 2026-08-23.
  Accepted 2026-11-22.

- Decision (D13): `shutdown` takes `&mut self`, not `self`. Rationale: RFC
  §5.3 requires a later call after a cancelled shutdown future, impossible if
  the handle moved into it, and `WireframePair::shutdown(&mut self)` is frozen
  by Constraint 1. `&mut self` is needed only to update the outcome record.
  Date/Author: 2026-08-23. Accepted 2026-11-22.

- Decision (D17): the harness wraps `WireframeServer::spawn()` and its
  `ServerShutdown`, rather than spawning `run_with_shutdown` in its own task
  with a `oneshot` and a `JoinHandle`. Rationale: `ServerShutdown` already
  provides an idempotent stop, a repeatable terminal observation, a
  listener-release guarantee, and runtime-free drop cancellation (facts 4 to
  8), with its own test suite; reimplementing them in the harness would
  duplicate the server's contract and its bugs. ADR 0014's outstanding
  decisions anticipate exactly this wrapper. Affected: RFC §5.2 step 4, §5.3,
  §6. Date/Author: 2026-11-22, planning agent. Accepted 2026-11-22.

- Decision (D18): on an elapsed drain bound the harness does not abort
  anything, and a later call re-checks with a zero bound. Rationale: there is
  no task handle to abort. The listener is released when the accept loops exit,
  given PRE-1 (AXIOM-7); only in-flight connection tasks may outlive the bound.
  A zero-bound re-check satisfies RFC §5.3's "rather than blocking again", and
  reports the real outcome if the drain has since finished, which is more
  accurate than replaying a stale timeout. Affected: RFC §5.3 bullets 4 and 6,
  §5.3's repeated-call list. Date/Author: 2026-11-22. Accepted 2026-11-22;
  reopened and accepted again 2026-11-24, subject to PRE-1.

- Decision (D19): identify the failing stage by operation and variant (Table
  3) rather than by error text. Rationale: `TestError` has no field for
  context, so stage text and a distinct typed variant cannot coexist without a
  new `TestError` variant in `src/testkit/result.rs`, which Constraint 6
  excludes and which RFC §11 question 2 reserves for 17.3.4 to decide. Table 3
  gives callers the stage without parsing text, which is the purpose of RFC
  §6's requirement. Affected: RFC §6 bullet 7. Date/Author: 2026-11-22.
  Accepted 2026-11-22.

- Decision (D20): `RunningWireframeServer` has no `impl Drop`. Dropping its
  `ServerShutdown` cancels the server synchronously through the control's own
  drop guard, so `Drop` needs no runtime, no detached task, no grace period,
  and no abort, and behaves identically inside and outside a runtime. Affected:
  RFC §5.2's grace period, §5.3's `Drop` paragraph, §7.1's drop bullet. Given
  PRE-1, the listener is released even while a request is in flight.
  Date/Author: 2026-11-22. Accepted 2026-11-22; reopened and accepted again
  2026-11-24, subject to PRE-1.

- Decision (D21): accept the type changes on unreachable paths that follow
  from D17. A supervisor panic now surfaces as
  `ServerError::AbnormalTermination { source: None, .. }` rather than a
  `JoinError`, so the new API's "panic" and "server error" classes are
  distinguished by `source` rather than by `TestError::Join` versus
  `TestError::Server`; and the pair's unreachable address-readback cell becomes
  `TestError::Io`. Every reachable *error* cell of Table 1 is unchanged.
  Affected: RFC §6 bullet 8. Date/Author: 2026-11-22. Accepted 2026-11-22.

- Decision (D22): the pair passes `None` for both bounds. Rationale: today
  the pair awaits readiness and the join without bounds (`client_pair.rs:318`,
  `:152`), and RFC §5.5 forbids adding a failure mode existing callers do not
  expect. Date/Author: 2026-11-22. Accepted 2026-11-22.

Superseded or discharged decisions, retained by identifier so earlier
references resolve:

- D8 (abort does not stop the server; escalated 2026-08-23): discharged by
  #656 and #657, both merged. Accepted as resolved 2026-11-22.
- D10 (lint does not reach `wireframe_testing`): superseded by #813 and #856.
- D11 (add CI `timeout-minutes`): superseded; `main` has them (#857).
- D14 (depend on #656 and #657): discharged; both merged.
- D15 (carry a known-expiry table for ADR 012): superseded by this
  re-derivation, which re-derived every expiring finding.
- D16 (2026-11-21 rebase bookkeeping): superseded by this re-derivation.

## Outcomes & retrospective

EP-M1 to EP-M4 are implemented. `wireframe_testing::server_harness` provides
the handle, options, pure outcome functions, and three spawn helpers;
`spawn_wireframe_pair` delegates to it; RFC 0001 carries a dated revision note
for D17 to D21; and roadmap 17.3.3 is ticked. The Stage E amendment list is
discharged against RFC 0001's new `Revision note: 2026-11-25` section, which
also records the form that was implemented: the supervisor drops its own
listener immediately after spawning the accept loops, before readiness, so the
socket's lifetime is exactly the accept loops' lifetime.

What changed against the plan's estimates: the integration cases were split
across `server_harness_lifecycle.rs` and `server_harness_in_flight.rs` exactly
as the plan permitted, and the pure-function tests live in `outcome.rs` rather
than a sibling file because they fit the budget. `client_pair.rs` shrank from
387 to 307 lines, with its error mapping and cancellation tests in sibling
modules. EP-M0's "PRE-1 landed on `main`" is the main unmet item: PRE-1 is
delivered as prerequisite PR #977, which this branch is based on, but this
assignment carries no merge authority. The residual evidence gaps recorded
below are the other unmet items, and the code-size breach is an open escalation.

Two lessons. First, `mapsplice replace` rewrote unrelated headings in
`docs/roadmap.md` by escaping their parentheses; per its own skill guidance
that churn was rejected, and the single checkbox change was applied directly
with the inline links preserved verbatim. Second, the property test found its
own bug: after a completed shutdown, a further `shutdown()` poll resolves
immediately rather than pending, so a "cancelled shutdown" operation has to
accept a ready result. That case is now pinned in the committed
`.proptest-regressions` file.

Assessment-driven evidence, 2026-11-26:

- Pair in-flight negative control: with PRE-1's listener drop replaced by a
  comment in `src/server/runtime/server/mod.rs`,
  `drop_with_request_in_flight_ releases_listener` failed with
  `server listener at <addr> remained bound for one second`; it passes with the
  drop in place.
- INV-5 negative control: with `shutdown()` skipping `stop()` when no outcome
  was recorded, five property cases and witnesses failed with
  `shutdown failed: deadline has elapsed` or
  `the final drain never completed at <addr>`. The property body and witness
  `case_2` (empty trace, terminal `Drop`) passed: dropping the handle cancels
  the server without `stop()`, so a drop-ending trace cannot witness the
  missing stop. The third named witness is the terminal-`Shutdown` one; the
  idle-peer witness belongs to INV-6, not INV-5.
- INV-1 to INV-3 mutant controls: letting `TimedOut` overwrite `Completed` made
  `merge_outcome_is_monotone` fail; returning `TestError::Msg` for a server
  error made `classify_drain_distinguishes_every_class` fail; returning the
  configured bound after a timeout made
  `drain_bound_never_blocks_after_timeout` fail on its assertion. The runtime
  half, `shutdown_bound_is_reported`, asserts the repeated call within 50 ms
  against a 300 ms configured bound.
- INV-4 seeded fault: `std::mem::forget(control.clone())` in `start_bound` made
  both `drop_inside_runtime_releases_listener` cases and both
  `drop_outside_runtime_releases_listener` cases fail; the leak keeps the
  listener bound.
- INV-6 control: the INV-5 skip-stop mutant also made
  `connect_schedules_leave_nothing_behind` fail, which is its negative control.
- INV-8 control: removing both pair error re-wraps made
  `startup_mapping_rewraps_only_server_errors` and
  `shutdown_mapping_separates_the_panic_class` fail.
- INV-7 has no mutant transcript: a seeded shared-listener fault cannot be
  expressed through the public API without fabricating addresses. Its
  non-vacuity is the distinct-address assertion plus the generated selector.
- Cancelled startup: `cancelled_preparing_startup_releases_listener` blocks
  `WireframeApp` preparation in a public middleware transform with a known
  listener, polls the startup future once, drops it without crossing the
  barrier, and requires the listener to be refused. A second test,
  `client_pair::startup_cancellation_tests::cancelled_pair_startup_releases_
  listener`,
  does the same through the pair's private startup core, so the pair-specific
  path is covered too. Both pass.
- INV-5 classification counts by trace class were not recorded; the property
  generates `Shutdown` and `CancelledShutdown` uniformly over a `0..=5` trace
  with a uniform terminal `Shutdown` or `Drop`, for 16 cases per run.

Evidence gaps, recorded rather than claimed:

- The pre-delegation pair characterization result was not recorded: the
  characterization case was written after the delegation, so it has never run
  against the old pair implementation.
- The ten consecutive full `make test` runs from the flakiness check were not
  performed. Each full run takes about 90 minutes on this host, and the two
  full runs made during implementation both passed with no intermittent
  failure. The INV-5 evidence is the property's 16 generated cases plus its
  three named witnesses; the idle-peer witness belongs to INV-6.

**Tolerance breach, escalated 2026-11-26.** Measured against the prerequisite
branch `426edda1`, the delivered diff adds 2,364 net Rust and feature-file
lines across 27 files, above both the `Code` tolerance of 1,500 and the 26-file
tolerance. The documentation is 249 net lines after excluding this plan, within
its 450 budget. The overage is concentrated in the integration, behavioural,
and property tests and in the harness's exhaustive unit tests: the plan's
estimate of about 1,250 code lines assumed fewer cases and did not price the
assessment-driven strengthening (bounded polling, streamed in-flight cases, the
outcome check on every trace step, a panicking-factory regression, and the
single-definition macro). The work is implemented and verified; the breach is
recorded here for the reviewer or operator to accept or to require scope
reduction rather than being silently absorbed.

Status remains `APPROVED` rather than `COMPLETE` until PRE-1 is merged to
`main`, because EP-M0's wording makes that part of the milestone. The delivered
branch has the code, tests, documentation, RFC amendment, and roadmap tick. The
evidence gaps and the tolerance breaches recorded above are still open, so not
everything the plan asks for is complete.

Lesson recorded early, from the 2026-11-22 re-derivation: when `main` moves
under a plan, re-verify the plan's claims, not only its links. Two rebases
passed every gate while the plan still described a server API that no longer
existed and a file that had since been created. The re-derivation then repeated
the fault in another form: it asserted two runtime behaviours from partial
source reading. A claim about runtime behaviour needs a measurement or a named
test before a decision rests on it.

## Artefacts and notes

Reserved for transcripts. At minimum, capture:

1. The Stage A auto-trait probe result.
2. The nine distinct red failures after Stage B0.
3. The green `test result: ok` line for each new test target.
4. One negative-control or seeded-fault result per invariant, INV-1 to INV-8.
5. The INV-5 classification counts.
6. The ten full `make test` runs from the flakiness check.

EP-M0 evidence, 2026-11-25, on `dcce5481` (PRE-1) rebased into this branch:

- `make check-fmt`: `144 files left unchanged.`
- `make lint`: `lint-doc` finished clean; `lint-clippy` clean; `lint-whitaker`
  `EXIT=0` with `src/server/runtime/server/mod.rs` at exactly 400 lines, so
  `module_max_lines` is observed rather than inferred; `lint-python-docs`
  `RESULT: PASSED (minimum: 100.0%, actual: 100.0%)`.
- `make typecheck`: `Finished dev profile ... in 16m 23s`.
- `make test`: 112 `test result: ok` suites, 0 `FAILED`, 0 ignored, including
  the three new `server_listener_release` cases.
- `make markdownlint`: `Summary: 0 issues in 0 files`.
- `make nixie`: `All diagrams validated successfully!`

## Revision history

- 2026-08-23: first draft around a harness-owned `JoinHandle` and `oneshot`
  sender. An expert design review withdrew a Stateright model in favour of an
  enumerated pure transition table, corrected the pair-compatibility premise,
  and found that `JoinHandle::abort()` did not stop a Wireframe server. That
  escalation was resolved by raising #656 and #657 upstream.
- 2026-10-30: rebased; #656, #641, #642, and #657 confirmed merged; the plan
  recorded that its design needed re-deriving.
- 2026-11-21: rebased; recorded 17.3.5, the workspace-wide gates, and the
  documentation relocation.
- 2026-11-22: re-derived against `a7eecaff`. The harness now wraps
  `ServerShutdown` (D17). Withdrawn: the `Phase`/`Op`/`Step` transition table,
  the detached-cleanup `Drop`, the `Abandoned` outcome, the drop-grace option,
  the diagnostics module and its stage strings, `shutdown_error()`, and the
  `await_port_released` helper. Citations were refreshed from source, but two
  runtime claims (D18's prompt release and AXIOM-6) were not measured and
  proved false; the mandatory `Progress` section was also lost. A false
  observation about `docs/repository-layout.md` was corrected. All decisions
  were marked accepted. Effect on remaining work: smaller code budget (1 500
  lines, 26 files, down from 1 800 and 28), no configuration changes, and, as
  then believed, no remaining upstream dependency.
- 2026-11-24: rebased onto #962 and stacked. Measured the in-flight listener
  and zero-timeout behaviour; withdrew AXIOM-6; reopened D18 and D20; raised
  D23; restored `Progress`; moved the readiness cases in-crate; removed a race
  from the bound test; reconciled EP-M3 with Stage D; refreshed shifted
  citations. Effect on remaining work: blocked on D23.
- 2026-11-24: D23 option 1 accepted. Added PRE-1 as an upstream prerequisite of
  EP-M1 and EP-M3, with its scope and acceptance; added AXIOM-7 as what PRE-1
  supplies; restored D18 and D20 subject to it; added in-flight cases to the
  drop and shutdown-bound tests and a pair characterization case; moved the
  tolerance base to the branch beneath. Effect on remaining work: no open
  decision; EP-M1 waits for PRE-1 and for authorization to implement.
- 2026-11-25: PRE-1 raised as #974, to land before EP-M1 rather than beneath the
  plan's own branch; recorded that #962 merged.
