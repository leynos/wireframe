# Extract a protocol-agnostic server lifecycle harness (17.3.3)

This ExecPlan (execution plan) is a living document. The sections
`Constraints`, `Tolerances`, `Risks`, `Progress`, `Surprises & discoveries`,
`Decision log`, `Outcomes & retrospective`, `Conformance basis`, and
`Verification plan` must be kept up to date as work proceeds.

Status: BLOCKED

This plan is blocked on the architecture deviation recorded as decision D8:
`JoinHandle::abort()` does not stop a Wireframe server, so RFC 0001's
abort-after-timeout cleanup guarantee is not implementable as written. See
`Escalation: abort does not stop the server`. That deviation must be accepted,
along with decisions D2, D3, and D5, before EP-M1 begins.

## Purpose / big picture

Today the only way to get a running Wireframe server inside a test is
`wireframe_testing::spawn_wireframe_pair`. That helper builds the server for
you and hands back a client whose type is fixed to
`WireframeClient<BincodeSerializer, RewindStream<TcpStream>, ()>`. A downstream
crate that speaks a different protocol — one with its own frame codec, its own
typed handshake preamble, or its own client type — cannot use it, and must
hand-roll listener reservation, readiness waiting, task ownership, shutdown
signalling, and defensive cleanup. That is the code most likely to produce port
races, orphaned Tokio tasks, and hanging test suites.

After this change a test author configures a `WireframeServer` however they
like — any serializer, any codec, any preamble, any worker count — hands the
still-unbound server to `wireframe_testing::spawn_wireframe_server`, and gets
back a `RunningWireframeServer`: a small, non-generic handle that knows the
bound address and can be stopped safely and repeatedly.

Observable success, stated as behaviour a person can check:

1. `make test` passes, and the new integration test
   `server_harness_lifecycle::spawn_then_connect_round_trip` fails before the
   change and passes after it.
2. A test can call `handle.shutdown().await` twice and the second call returns
   `Ok(())` immediately, without re-signalling and without joining again.
   `handle.shutdown_outcome()` reports the same classification both times, and
   `handle.shutdown_error()` still lends the original typed failure if there
   was one.
3. A test can drop a `RunningWireframeServer` without ever calling `shutdown`,
   inside or outside a Tokio runtime, and the test process still exits.
4. The existing suites `tests/client_pair_harness.rs`,
   `tests/features/client_pair_harness.feature`, and
   `wireframe_testing/tests/integration_helpers.rs` pass unmodified:
   `spawn_wireframe_pair` and `spawn_wireframe_pair_default` keep their exact
   signatures and their exact per-path error variants.
5. `cargo test --test server_harness_lifecycle_props` enumerates every
   `(Phase, Op)` pair of the lifecycle transition function and every operation
   sequence up to length six, and reports them exhausted.

## Constraints

Hard invariants. Violating one requires escalation, not a workaround.

1. `spawn_wireframe_pair` and `spawn_wireframe_pair_default` keep their exact
   signatures, their return type `TestResult<WireframePair>`, and their exact
   per-path failure variants. `WireframePair` stays concrete and must not gain
   a generic parameter. This is a real external-compatibility obligation:
   `wireframe_testing` is published at 0.3.0, and RFC §3.1 commits to it.

   The variants to preserve are *not* uniformly `TestError::Msg`. Verified
   against the tree:

   <!-- markdownlint-disable MD013 -->

   | Failure path | Source | Variant today |
   | --- | --- | --- |
   | listener reservation | `client_pair.rs:286` `unused_listener()?` | `TestError::Io` |
   | binding | `client_pair.rs:289` `bind_existing_listener(..)?` | `TestError::Server` |
   | address readback | `client_pair.rs:292` `local_addr()` is `None` | `TestError::Msg` |
   | readiness, three arms | `client_pair.rs:316-324` | `TestError::Msg` |
   | client connect | `client_pair.rs:330` `builder.connect(addr).await?` | `TestError::Client` |
   | shutdown join | `client_pair.rs:146-150` | `TestError::Msg` |

   <!-- markdownlint-enable MD013 -->

   *Table 1: the failure variants `spawn_wireframe_pair` and
   `WireframePair::shutdown` produce today, each of which EP-M3 must preserve
   individually.*

2. Do not add default type parameters to the new helpers, and do not narrow any
   bound below the `where` clause quoted in `Context and orientation`. Every
   valid `WireframeServer` configuration must be accepted.
3. `ready_signal` must be *documented* as reserved for the harness. It cannot
   be *enforced*: `ready_signal` is an unconditional setter
   (`src/server/config/mod.rs:121-135`) and `WireframeServer` exposes no
   accessor that would let the harness detect a caller-set sender. A caller who
   sets it gets a receiver that never fires. Record the footgun; do not pretend
   it is prevented.
4. No source file may exceed 400 lines (`AGENTS.md`). Per-file budgets are in
   `Plan of work`. Note that the strict `clippy.toml` thresholds are *not*
   enforced on this code — see decision D10 — so this is authoring discipline
   verified by review, not by a gate.
5. `Drop`, and everything it calls synchronously, must be panic-free: no
   `unwrap`, no `expect`, no indexing, no locking, no fallible arithmetic. A
   panic in `Drop` during unwind aborts the process with no test output.
6. Test-only lifecycle API stays in `wireframe_testing`. Do not modify
   `src/server/` or `src/client/` behaviour.
7. Do not touch the Makefile or CI test targets; that is roadmap 17.3.5. The
   one CI change this plan makes is adding `timeout-minutes` to the existing
   jobs, because today a hung test burns the six-hour GitHub default. See
   decision D11.
8. Every new module begins with a `//!` comment. Every new public item carries
   a `///` comment with `# Errors` where it returns `Result`.
9. All prose, comments, and identifiers use en-GB-oxendict spelling.
10. Red-Green-Refactor, with each test failing on its own assertion rather than
    on a shared compile error. See `Plan of work`, Stage B0.
11. No new entry in any `[dependencies]` or `[dev-dependencies]` table.

## Tolerances (exception triggers)

Stop and escalate when any of these is reached.

- **Scope**: more than 28 files changed, or more than 1 800 net added lines.
  These numbers are derived from the file list in `Plan of work`, not guessed;
  the first draft of this plan set them at 14 and 1 200 and breached both
  before its second milestone.
- **Interface**: the public surface must grow beyond
  `Interfaces and dependencies`, or any existing public signature must change.
- **Dependencies**: any new manifest dependency entry.
- **Constraint 1**: any change to a cell of Table 1.
- **Iterations**: a single test still failing after four corrective attempts.
- **Flakiness**: any new test failing more than once in ten consecutive runs of
  the full `make test`, run as `make test` — not as an isolated focused target,
  which removes the very contention that surfaces timing races.
- **Case duration**: any property case exceeding its ten-second internal
  timeout. That is a hang, not slowness; do not raise the bound.
- **Time**: any milestone exceeding four hours of wall-clock work.
- **Ambiguity**: two readings of RFC 0001 that would produce materially
  different public API.

## Progress

- [ ] EP-M0 Escalations accepted; green baseline recorded.
- [ ] EP-M1 `RunningWireframeServer`, options, spawn helpers, rstest and BDD
      coverage.
- [ ] EP-M1a Throwaway delegation spike, then revert.
- [ ] EP-M2 Exhaustive transition-table tests and the property suite.
- [ ] EP-M3 `WireframePair` delegates; duplicated machinery removed.
- [ ] EP-M4 Documentation, RFC amendment, roadmap tick.

Timestamp each completion as `- [x] (2026-08-23 14:05Z) …` so the four-hour
per-milestone tolerance can be checked.

## Escalation: abort does not stop the server

**This blocks EP-M1 and requires an explicit decision.**

RFC 0001 §5.3 says explicit shutdown "bounds the join and aborts the task only
after timeout", and §6 requires every cleanup path to "join the server task".
Both assume that aborting the spawned `run_with_shutdown` future tears the
server down. It does not.

`run_with_shutdown` is a supervisor, not the server. It creates a
`CancellationToken` and a `TaskTracker` locally, then spawns one `accept_loop`
task per worker, giving each its own token clone and its own `Arc` to the
listener:

```rust
// src/server/runtime.rs:157-181
let shutdown_token = CancellationToken::new();
let tracker = TaskTracker::new();
for _ in 0..workers {
    let listener = Arc::clone(&listener);
    let token = shutdown_token.clone();
    tracker.spawn(accept_loop(listener, factory, AcceptLoopOptions { shutdown: token, .. }));
}
```

The token is cancelled in exactly one place, the graceful arm of the `select!`
at `src/server/runtime.rs:191`. Aborting the supervisor never reaches it, and
neither type cleans up on drop:

- `tokio-util-0.7.17/src/task/task_tracker.rs:56`: "Note that unlike
  `JoinSet`, dropping a `TaskTracker` does not abort the tasks."
- `tokio-util-0.7.17/src/sync/cancellation_token.rs:125-129`: `Drop` only calls
  `decrease_handle_refcount`; it does not cancel.

An accept loop terminates only on `handles.shutdown.cancelled()`
(`src/server/runtime/accept.rs:213`). So after `abort()`, every accept loop is
still running and still holding an `Arc` to the listener. **The port stays
bound for the remaining lifetime of the runtime.**

This is not a defect this plan introduces. `WireframePair::Drop` already relies
on the same ineffective abort (`wireframe_testing/src/client_pair.rs:167` and
`:226`). It is a pre-existing defect that RFC 0001 turns into a guarantee, and
that this plan would have propagated into a public API and a verification
claim.

### Blast radius

Inside `#[tokio::test]` the runtime is torn down at the end of the test and
takes the accept loops with it, so the leak is invisible. The pattern where it
bites is a long-lived runtime: the behaviour-driven world at
`tests/fixtures/client_pair_harness.rs` owns a `tokio::runtime::Runtime` for a
whole feature file and drives it with synchronous steps. A downstream crate
adopting this harness in that shape — which RFC §2 names mxd as the first
candidate to do — accumulates bound listeners until `unused_listener()` starts
failing in unrelated tests.

### Options

1. **Report the leak instead of hiding it.** Keep the bounded join, but when
   the bound elapses stop claiming cleanup: name the terminal state
   `Abandoned` rather than `TimedOut`, emit a `log::warn!` naming the stage and
   the address, and document that the listener stays bound until the runtime is
   dropped. No upstream change. Honest, cheap, and it makes the failure
   greppable in CI output.
2. **Fix it upstream.** Let `run_with_shutdown` accept a caller-supplied
   `CancellationToken`, or return a handle exposing one, so the harness can
   cancel and then drain the tracker. This is the only option that makes
   abort-after-timeout mean what RFC §5.3 says. It changes `src/server/`'s
   public API, breaching Constraint 6, and is plausibly its own roadmap item.
3. **Accept silently.** Rejected: it would make the no-orphan invariant vacuous,
   and a harness whose purpose is diagnosable cleanup must not lie about
   cleaning up.

### Recommendation

Take option 1 for 17.3.3 and raise option 2 as a separate roadmap item. Option
1 keeps this item's scope intact, removes a false guarantee from the public
documentation, and turns a silent leak into a logged one. Option 2 is the real
fix, but it is server-side work rather than test-harness work.

Either way RFC 0001 §5.3 and §6 need amending, because their current wording
describes behaviour no implementation can provide.

## Risks

- Risk: readiness fires when accept-loop workers have been *spawned*, not when
  `accept()` is being polled (`src/server/runtime.rs:184-188`).
  Severity: medium. Likelihood: medium.
  Mitigation: the listener is bound before the task is spawned, so the kernel
  queues the connection in the listen backlog; the existing pair harness relies
  on this today. Do not add a sleep. If a connect-after-ready flake appears,
  escalate — the fix is upstream, and sanding it down by reducing generated
  cases would suppress exactly the signal that detects a readiness regression.

- Risk: a hung test has no timeout anywhere in the stack. `proptest`'s
  `max_shrink_time` defaults to unlimited, there is no nextest configuration,
  and `.github/workflows/ci.yml` sets no `timeout-minutes`, so GitHub's
  360-minute default applies. libtest buffers per-test output, so the log ends
  at `running N tests` with no indication of which test hung.
  Severity: high. Likelihood: medium.
  Mitigation: three layers. Wrap every property case body in
  `tokio::time::timeout(Duration::from_secs(10), …)` *inside* the `block_on`,
  so a hang becomes a shrinkable, printable failure; set
  `max_shrink_time: 30_000`; and add `timeout-minutes` to the CI jobs (D11).
  The internal timeout must not use `ServerHarnessOptions`, because those are
  the thing under test.

- Risk: the property tests default to `available_parallelism()` workers per
  server, so a pool of four handles is 96 accept-loop tasks on this machine and
  16 on a four-vCPU runner — a machine-dependent cost that makes the flakiness
  protocol unreproducible.
  Severity: medium. Likelihood: high.
  Mitigation: pin `.workers(1)` in every harness test, as
  `spawn_wireframe_pair` already does at `client_pair.rs:288`.

- Risk: the plan's new module lands outwith every automated lint. `make lint`
  runs `cargo clippy --all-targets --all-features` with no `-p` or
  `--workspace`, and `default-members = ["."]`; `wireframe_testing` has no
  `[lints]` table, so even a manual `-p` run gets none of the root package's
  `pedantic`, `unwrap_used = "deny"`, or `missing_docs` configuration.
  Severity: medium. Likelihood: certain.
  Mitigation: recorded as decision D10. Treat the limits as authoring
  discipline verified in review; hoisting `[workspace.lints]` belongs to
  17.3.5, which owns companion-crate gating.

- Risk: the CodeScene and Codecov patch-coverage ratchet (`codecov.yml`,
  `patch: target: 80%`) sees ~400 new lines in a module whose error arms are
  deliberately unexercised.
  Severity: medium. Likelihood: medium.
  Mitigation: keep unreachable arms to a minimum, extract the pure functions
  so they are directly testable, and if the ratchet still fails, report it
  rather than adding hollow tests.

- Risk: `proptest!` does not accept `async fn`, and `prop_assert!` returns
  early with `Err(TestCaseError)`.
  Severity: low. Likelihood: certain.
  Mitigation: use the documented workaround — an ordinary `#[test] fn` that
  builds a runtime and calls `block_on` on an `async` block returning
  `Result<(), TestCaseError>`, then applies `?`. Upstream issue
  `proptest-rs/proptest#179` confirms there is no better option without a new
  dependency, which Constraint 11 forbids.

- Risk: `wireframe_testing`'s existing doctests do not compile (issue #578), so
  `cargo test -p wireframe_testing --doc` fails wholesale.
  Severity: low. Likelihood: certain.
  Mitigation: validate the new module's doctests with a filtered invocation and
  record the transcript. Note that `make doctest-benchmark` cannot help:
  `scripts/doctest-benchmark.sh:13` defaults its search root to `src`, the root
  crate only, so it never sees `wireframe_testing/src/server_harness/`.
  Repairing #578 belongs to 17.3.5.

## Context and orientation

Read this section first if you have never worked in this repository.

### What Wireframe is

Wireframe is a Rust library for building servers and clients that speak custom
binary protocols over TCP. The root package is `wireframe` (`Cargo.toml` at the
repository root, sources under `src/`). A companion package
`wireframe_testing` (`wireframe_testing/Cargo.toml`) ships test helpers.

### Vocabulary

- **Frame**: one codec-defined unit of bytes on the wire.
- **Envelope**: Wireframe's built-in framed message type.
- **Preamble**: an optional typed handshake value exchanged once, before any
  framed traffic.
- **Serializer**: turns a Rust value into payload bytes; default
  `BincodeSerializer`.
- **Codec**: splits a byte stream into frames; default
  `LengthDelimitedFrameCodec`.
- **Typestate**: a marker type parameter encoding a compile-time phase.
  `WireframeServer` uses `Unbound` before it owns a listener and `Bound`
  afterwards. The `ServerState` trait admitting them is *sealed*, so no third
  typestate can be added from outside the root crate.
- **Readiness signal**: a `tokio::sync::oneshot::Sender<()>` the server fires
  once, after spawning its accept-loop workers.
- **Plateau**: a milestone end state that is correct, coherent, and safe to
  stop at if later work is postponed.
- **Non-vacuity**: evidence that a passing check *could* have failed had the
  implementation been wrong. A test whose precondition is never satisfied, or
  whose assertion holds regardless of the code, is vacuous and proves nothing.
- **Whitaker**: a Dylint lint suite installed by CI and run by `make lint`. Its
  `no_expect_outside_tests` rule forbids `.expect()` outwith test contexts.
- **Nixie**: the Mermaid-diagram validator run by `make nixie`.
- **Mapsplice**: the structural editor used for `docs/roadmap.md`. Its grammar
  rejects footnote references, so the roadmap cites with inline links.

### The existing code

`wireframe_testing/src/client_pair.rs` (379 lines) is the whole of the current
lifecycle machinery. Read it in full; it is the single most useful preparation
for this work. It contains `WireframePair`, the private `Running` bundle,
`spawn_wireframe_pair` (line 274), `spawn_wireframe_pair_default` (line 371),
an unbounded `shutdown` (line 125), a defensive `Drop` (line 160),
`spawn_bounded_shutdown` (line 183), and the `PendingServer` RAII guard (line
205). The bounded-shutdown machinery already appears twice in that one file;
RFC §5.6 asks for it to appear once, in a new `server_harness` module.

### Server-side API this harness drives

Verified against the current tree. The declaration at `src/server/mod.rs:188`
is:

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
// src/server/config/binding.rs:172, on the Unbound typestate
pub fn bind_existing_listener(
    self,
    std_listener: StdTcpListener,
) -> Result<BoundServer<F, T, Ser, Ctx, E, Codec>, ServerError>;

// src/server/config/binding.rs:210, on the Bound typestate
pub fn local_addr(&self) -> Option<SocketAddr>;

// src/server/config/mod.rs:135
pub fn ready_signal(self, tx: tokio::sync::oneshot::Sender<()>) -> Self;

// src/server/runtime.rs:142, on the Bound typestate only
pub async fn run_with_shutdown<S>(self, shutdown: S) -> Result<(), ServerError>
where
    S: Future<Output = ()> + Send;
```

The `where` clause on the impl block providing `run_with_shutdown`
(`src/server/runtime.rs:29-36`) is exactly:

```rust
impl<F, T, Ser, Ctx, E, Codec> WireframeServer<F, T, Bound, Ser, Ctx, E, Codec>
where
    F: AppFactory<Ser, Ctx, E, Codec>,
    T: Preamble,
    Ser: Serializer + FrameMetadata<Frame = Envelope> + Send + Sync + 'static,
    Ctx: Send + 'static,
    E: Packet,
    Codec: FrameCodec,
    Envelope: DecodeWith<Ser> + EncodeWith<Ser>,
```

Copy that clause verbatim onto the new helpers.

**You will not need to add `Send + 'static` bounds.** `tokio::spawn` requires
them, but every supertrait already supplies them: `AppFactory: Send + Sync +
Clone + 'static` (`src/server/mod.rs:122`), `Preamble: … + Send + Sync +
'static` (`src/preamble.rs:23`), `Packet: … + Send + Sync + 'static`
(`src/app/envelope.rs:62`), `FrameCodec: Send + Sync + Clone + 'static`
(`src/codec.rs:63`), plus `Ctx: Send + 'static` and `Ser: … + 'static` in the
clause above. Construct the shutdown future as an owned `async move { let _ =
shutdown_rx.await; }` block so it is trivially `Send + 'static`, exactly as
`client_pair.rs:301` does.

### Facts that shape the design

Each of these was verified against the tree and each one changed a decision.

1. **`run_with_shutdown` cannot fail.** It destructures, spawns workers, fires
   `ready_tx`, `select!`s, drains the tracker, and returns `Ok(())`
   (`src/server/runtime.rs:142-198`). `accept_loop` returns `()`
   (`src/server/runtime/accept.rs:149`) and `ServerError::Accept` is
   constructed nowhere in `src/`. App-factory failures are `warn!`-logged per
   connection (`src/server/connection_spawner.rs:99-108`). So
   `ShutdownOutcome::ServerFailed` is unreachable today; its arm exists only
   because the task's type is `JoinHandle<Result<(), ServerError>>`.
2. **A panicking `Clone` is the reachable pre-readiness fault.** `AppFactory`
   requires `Clone`, and `run_with_shutdown` calls `factory.clone()` once per
   worker before firing `ready_tx`; the default worker count is at least one.
   A test factory whose `Clone` panics therefore yields `JoinError` and
   `ShutdownOutcome::Panicked`.
3. **Read the address before binding.** `bind_to_listener` calls
   `set_nonblocking` then `TcpListener::from_std`
   (`src/server/config/binding.rs:77-79`) — the same file descriptor, so the
   address is unchanged. Calling `listener.local_addr()?` on the std listener
   *before* handing it over removes the `Option::None` case entirely and
   preserves the real `io::Error`, which `local_addr()` on the bound server
   discards with `.ok()`.
4. **`unused_listener()` binds the string `"localhost:0"`**
   (`wireframe_testing/src/integration_helpers.rs:44`), which resolves through
   `ToSocketAddrs` and may yield `::1`. Assert `addr.ip().is_loopback()`, never
   an IPv4 range.
5. **`shutdown` takes `&mut self`,** so the borrow checker already excludes
   concurrent operations on one handle. The lifecycle is a sequential decision
   table, not a concurrency problem. This is why the verification strategy is
   exhaustive enumeration of a pure transition function rather than a
   concurrency model checker.
6. **`TestError::Server` is `#[cfg(not(loom))]`** (`src/testkit/result.rs:41`).
   Irrelevant today, since `wireframe_testing` is never built under
   `--cfg loom`, but do not assume the variant is unconditional.
7. **`ci.yml` does not run `make test`.** Test execution happens inside the
   shared `generate-coverage` action, on pull requests only. Do not write
   documentation claiming `make test` is the CI gate.
8. **`echo_app_factory` takes an argument**: `echo_app_factory(&counter)` where
   `counter: &Arc<AtomicUsize>` (`wireframe_testing/src/integration_helpers.rs`).

### Where tests live

- Root-package integration tests, `tests/*.rs`, are compiled by `make test`
  (`cargo test --all-targets --all-features`). The root package dev-depends on
  `wireframe_testing`, `rstest`, `rstest-bdd`, `proptest`, `googletest`, and
  `pretty_assertions`.
- Behaviour-driven tests: features in `tests/features/`, worlds in
  `tests/fixtures/`, steps in `tests/steps/`, bindings in `tests/scenarios/`.
  They compile into the `bdd` target, gated on `advanced-tests`, which
  `--all-features` enables.
- Companion-crate tests are **not** run by `make test`, because
  `default-members = ["."]`. Wiring them in is roadmap 17.3.5. This plan
  therefore puts everything that must be gated today in the root `tests/` tree
  and runs the companion-crate command explicitly at each milestone.

### Skills and documents

Load `leta` (run `leta workspace add .`), `rust-router` and from it
`rust-async-and-concurrency`, `proptest`, `execplans`, and `en-gb-oxendict`.
Load `firecrawl` for any external lookup.

Read `docs/rfcs/0001-protocol-agnostic-test-harness-lifecycle.md` (§5.2, §5.3,
§5.5, §5.6, §6, §7.1, §7.5 are in scope), `docs/wireframe-testing-crate.md`
(the `## In-process server/client pair harness` section), and
`docs/documentation-style-guide.md`. Consult
`docs/rstest-bdd-users-guide.md` before writing the BDD suite — step functions
are synchronous and fixture names must match step parameter names exactly —
and `docs/rust-doctest-dry-guide.md` before writing doctests. The predecessor
plan `docs/execplans/17-3-2-in-process-server-and-client-pair-test-harness.md`
explains why the pair harness has its current shape.

`kani` and `verus` are deliberately not used; the reasoning is in
`Verification plan` and needs no further reading.

## Conformance basis

Upstream artefacts at the revisions in the working tree:

- `docs/rfcs/0001-protocol-agnostic-test-harness-lifecycle.md`, Status
  Proposed, Created 2026-07-18. The governing technical design. There is **no**
  separate Terms of Reference document; do not invent one.
- `docs/roadmap.md` item 17.3.3, with 17.3.4 and 17.3.5 out of scope.
- `AGENTS.md` and `docs/documentation-style-guide.md`.

```plaintext
ROADMAP-17.3.3 -> RFC-5.2 -> EP-M1 -> tests/server_harness_lifecycle.rs::spawn_then_connect_round_trip
ROADMAP-17.3.3 -> RFC-5.2 -> EP-M1 -> tests/server_harness_lifecycle.rs::caller_listener_keeps_its_address
ROADMAP-17.3.3 -> RFC-5.3 -> EP-M1 -> tests/server_harness_lifecycle.rs::panicking_factory_is_reported
ROADMAP-17.3.3 -> RFC-5.3 -> EP-M1 -> tests/server_harness_lifecycle.rs::shutdown_is_idempotent
ROADMAP-17.3.3 -> RFC-5.3 -> EP-M1 -> tests/server_harness_lifecycle.rs::cancelled_shutdown_then_drop
ROADMAP-17.3.3 -> RFC-5.3 -> EP-M1 -> tests/server_harness_lifecycle.rs::drop_inside_runtime
ROADMAP-17.3.3 -> RFC-5.3 -> EP-M1 -> tests/server_harness_lifecycle.rs::drop_outside_runtime
ROADMAP-17.3.3 -> RFC-5.3 -> EP-M1 -> tests/server_harness_lifecycle.rs::shutdown_bound_is_reported
ROADMAP-17.3.3 -> RFC-6   -> EP-M1 -> wireframe_testing/src/server_harness/diagnostics.rs (stage strings)
ROADMAP-17.3.3 -> RFC-5.3 -> INV-1 -> EP-M2 -> tests/server_harness_lifecycle_props.rs::every_transition_is_total
ROADMAP-17.3.3 -> RFC-5.3 -> INV-2 -> EP-M2 -> tests/server_harness_lifecycle_props.rs::all_traces_converge
ROADMAP-17.3.3 -> RFC-6   -> INV-3 -> EP-M2 -> wireframe_testing/src/server_harness/state.rs (Step::disposes_task)
ROADMAP-17.3.3 -> RFC-5.3 -> INV-4 -> EP-M2 -> tests/server_harness_lifecycle_props.rs::repeated_shutdown_converges
ROADMAP-17.3.3 -> RFC-6   -> INV-5 -> EP-M2 -> tests/server_harness_lifecycle_props.rs::parallel_handles_are_independent
ROADMAP-17.3.3 -> RFC-5.5 -> INV-6 -> EP-M3 -> wireframe_testing/src/client_pair.rs (map_pair_error unit tests)
ROADMAP-17.3.3 -> RFC-5.6 -> EP-M3 -> wireframe_testing/src/server_harness/ (single copy of the machinery)
```

RFC sections deliberately not discharged here:

- §5.4 (`spawn_wireframe_server_and_connect`) → 17.3.4.
- §7.1's last two bullets (connector-failure cleanup, simultaneous connector
  and cleanup failure) → 17.3.4, because both presuppose the connector.
- §7.2 (protocol-generic proof) and §7.3 (`trybuild`) → 17.3.4.
- §7.4 (companion-crate gates, issue #578) → 17.3.5.

## Verification plan

### Axioms (assumed, not verified here)

- **AXIOM-1**: Tokio's `oneshot` delivers at most one value, and
  `JoinHandle::await` resolves exactly once with the task's output or a
  `JoinError`.
- **AXIOM-2**: `tokio::time::timeout` resolves `Err(Elapsed)` no earlier than
  the supplied duration when the inner future has not completed.
- **AXIOM-3**: `run_with_shutdown` fires `ready_tx` exactly once before
  awaiting shutdown, and returns once its shutdown future resolves and its
  worker tracker drains (`src/server/runtime.rs:165-197`). Repository-owned, so
  EP-M1's rstest cases exercise it against the real server.
- **AXIOM-4**: binding `"localhost:0"` yields a listener on a free ephemeral
  loopback port, and `bind_existing_listener` transfers the same file
  descriptor without releasing it.
- **AXIOM-5**: `Handle::try_current()` returns `Err` exactly when no runtime is
  entered on the current thread.
- **AXIOM-6** (from the escalation): aborting the supervisor does **not** stop
  the accept loops. Everything below treats abandonment, not cleanup, as the
  outcome of an elapsed bound.

### The design decision that makes verification tractable

Because `shutdown` takes `&mut self`, the handle's lifecycle is sequential. The
implementation therefore separates the *decision* from the *effect*:

```rust
// wireframe_testing/src/server_harness/state.rs — pure, total, no I/O
pub enum Phase { Live, Signalled, Stopped(ShutdownOutcome) }
pub enum Op { Shutdown, Drop }
pub enum Step { SignalThenJoin, JoinOnly, ReportTerminal, DetachedCleanup, Nothing }

pub const fn plan(phase: &Phase, op: Op) -> (Phase, Step);

impl Step {
    /// Every step must either dispose of the server task or provably not hold one.
    pub const fn disposes_task(self) -> bool {
        match self {
            Self::SignalThenJoin | Self::JoinOnly | Self::DetachedCleanup => true,
            Self::ReportTerminal | Self::Nothing => false,
        }
    }
}
```

`handle.rs` calls `plan()` and executes the returned `Step`; it contains no
lifecycle decisions of its own. This is what makes the verification
non-vacuous: the thing enumerated is the thing that runs. An earlier draft
proposed a separate Stateright model in `crates/wireframe-verification`; that
crate cannot depend on `wireframe_testing`, so the model could only ever be a
transcribed copy whose correspondence was enforced by a comment. See D5.

### Invariants

**INV-1 — totality and monotonicity.** `plan` is total over `Phase × Op`, and
`Stopped` is absorbing: no `(Stopped(o), _)` transition yields a phase other
than `Stopped(o)`.

- Method: exhaustive enumeration. Both domains are finite and small.
- Rationale: with a closed finite domain, enumeration is not a sample of the
  truth — it is the truth. Nothing weaker is warranted, and nothing stronger is
  available.
- Domain: all `Phase` values, including one `Stopped(o)` per `ShutdownOutcome`,
  crossed with both `Op` values. Twelve pairs.
- Artefact: `tests/server_harness_lifecycle_props.rs::every_transition_is_total`.
- Evidence: `make test`.
- Non-vacuity: assert the enumeration visited twelve pairs, so a generator bug
  that silently produced none fails. Negative control: add a `Stopped → Live`
  edge; the absorption assertion must fail.

**INV-2 — trace convergence.** Every operation sequence of length up to six
ending in `Drop` leaves the same terminal phase as a single `Shutdown` followed
by `Drop`.

- Method: exhaustive fold of `plan` over all `Op` sequences up to length six —
  126 traces.
- Rationale: covers ordering exhaustively at negligible cost, and subsumes what
  the earlier draft asked a bounded model checker to do.
- Artefact: `tests/server_harness_lifecycle_props.rs::all_traces_converge`.
- Evidence: `make test`.
- Non-vacuity: assert both the "drop after shutdown" and "drop without
  shutdown" classes are present in the enumeration. Negative control: make the
  `Signalled` arm re-signal; convergence must fail.

**INV-3 — no orphaned task.** No reachable `Step` discards the server task
without joining or abandoning it.

- Method: exhaustive `match` in `Step::disposes_task`, plus an enumeration
  asserting every `Step` reachable from a non-`Stopped` phase disposes of the
  task.
- Rationale: this is the invariant that is *not* observable from a test. Making
  it an exhaustive `match` makes it a compile-time obligation — adding a `Step`
  variant without classifying it is a compile error. That is stronger than any
  assertion, and it cannot drift.
- Artefact: `wireframe_testing/src/server_harness/state.rs` plus
  `tests/server_harness_lifecycle_props.rs::every_transition_is_total`.
- Evidence: `make test`, and a deliberate `cargo build` failure.
- Non-vacuity: add a `Step` variant without a `disposes_task` arm and confirm
  the crate fails to compile; record that transcript.

**INV-4 — idempotence in the running system.** On a real handle, repeated
`shutdown()` converges to one joined task, and the reported classification does
not change between calls.

- Method: property test over a generated call count.
- Rationale: INV-1 and INV-2 verify the *decision*; this verifies that the
  execution of each `Step` matches it against a real server, real sockets, and
  a real runtime.
- Domain: call count generated in `1..=4`; 16 cases.
- Artefact: `tests/server_harness_lifecycle_props.rs::repeated_shutdown_converges`.
- Evidence: `make test`.
- Non-vacuity: the range excludes zero, so at least one `shutdown` always runs.
  Assert with `prop_assert_eq!` on `shutdown_outcome()` rather than
  `prop_assert!` on a boolean, so the shrunk counter-example prints actual
  against expected. Negative control: remove the phase update in the `Live`
  arm so the signal is re-sent; the property must fail.

**INV-5 — handle isolation.** Concurrently spawned handles bind distinct
addresses and share no state; stopping one does not affect another.

- Method: property test over a generated pool size.
- Rationale: concerns a set of independently allocated OS resources, which no
  abstraction can model.
- Domain: pool size generated in `2..=4`, each server pinned to
  `.workers(1)`; 16 cases.
- Artefact: `tests/server_harness_lifecycle_props.rs::parallel_handles_are_independent`.
- Evidence: `make test`.
- Non-vacuity: the shut-down index is generated, so first, middle, and last
  positions are all reachable; assert at least one handle survives. Negative
  control: give every handle the same listener; the distinct-address assertion
  must fail.

**INV-6 — pair compatibility.** Every cell of Table 1 still holds after EP-M3.

- Method: the existing suites unchanged, plus exhaustive unit tests of the
  extracted error-mapping function.
- Rationale: a finite, enumerable surface. The compatibility obligation cannot
  be tested end-to-end, because none of the `Msg`-producing paths is reachable
  through the public API — `run_with_shutdown` cannot fail, readiness always
  fires, and `local_addr()` never returns `None` once the address is read from
  the std listener. Extracting the mapping into a pure function makes the
  obligation testable with constructed inputs.
- Domain: `pub(crate) fn map_pair_shutdown_error(Result<Result<(), ServerError>,
  JoinError>) -> TestResult<()>` exercised with `Ok(Ok(()))`, a constructed
  `ServerError::Bind(io::Error::other(..))`, and a real `JoinError` obtained
  from `tokio::spawn(async { panic!() }).await.unwrap_err()`.
- Artefact: `#[cfg(test)]` module in `wireframe_testing/src/client_pair.rs`,
  plus the four existing cases in `tests/client_pair_harness.rs`.
- Evidence: `cargo test -p wireframe_testing --all-targets`, and `make test`.
- Non-vacuity: each case asserts the variant *and* the message prefix, so a
  silent switch from `Msg` to `Join` fails. Negative control: return the typed
  variant instead of wrapping; every case must fail.

### Methods deliberately not used

- **Kani** sequentialises concurrency and cannot see task interleaving. It is
  also the wrong shape for a finite decision table that plain enumeration
  covers exhaustively at compile-and-run cost of milliseconds.
- **Verus** addresses unbounded induction over pure functions. `plan` has a
  twelve-element domain; there is no induction to do.
- **Stateright** was in an earlier draft and was withdrawn; see D5.
- **`insta`** is not a dependency and there is no multivariant output to
  snapshot. Error text is asserted semantically with `contains_substring`, per
  RFC §7.3.

### Residual gaps

- `ShutdownOutcome::ServerFailed` is unreachable through a real server and has
  no runtime test. It is exercised only through `map_pair_shutdown_error`'s
  constructed input.
- The `LifecycleStage::Bind` path is reachable only if `set_nonblocking` or
  `TcpListener::from_std` fails, which cannot be forced from a test. Documented
  as defensive.
- Per AXIOM-6, an elapsed bound abandons rather than cleans up. No test asserts
  that the listener is released on that path, because it is not.
- If `Drop` runs while the runtime is shutting down, the detached cleanup task
  is cancelled before it executes and the server task is neither joined nor
  abandoned by us; it dies with the runtime. Benign in-process, but INV-3's
  guarantee is about the `Step` we choose, not about the runtime honouring it.

## Interfaces and dependencies

At the end of EP-M3 the following exists, exactly. The submodules are
**private**; only `server_harness` itself is `pub`, so the file split stays a
private implementation detail.

```rust
// wireframe_testing/src/server_harness/mod.rs
//! Protocol-agnostic server lifecycle harness.

mod diagnostics;
mod handle;
mod lifecycle;
mod options;
mod state;

pub use self::{
    handle::{RunningWireframeServer, ShutdownOutcome},
    lifecycle::{spawn_wireframe_server, spawn_wireframe_server_on, spawn_wireframe_server_with_options},
    options::ServerHarnessOptions,
};
```

```rust
// options.rs
/// Timeout policy for a [`RunningWireframeServer`].
#[derive(Clone, Debug, PartialEq)]
pub struct ServerHarnessOptions { /* private fields */ }

impl Default for ServerHarnessOptions {
    /// Five seconds for readiness, five seconds for the shutdown join, and a
    /// one-second grace period for defensive `Drop` cleanup.
    fn default() -> Self;
}

impl ServerHarnessOptions {
    #[must_use] pub const fn with_readiness_timeout(self, timeout: Duration) -> Self;
    /// `None` waits indefinitely, matching `WireframePair::shutdown`.
    #[must_use] pub const fn with_shutdown_timeout(self, timeout: Option<Duration>) -> Self;
    #[must_use] pub const fn with_drop_grace(self, grace: Duration) -> Self;
    #[must_use] pub const fn readiness_timeout(&self) -> Duration;
    #[must_use] pub const fn shutdown_timeout(&self) -> Option<Duration>;
    #[must_use] pub const fn drop_grace(&self) -> Duration;
}
```

```rust
// handle.rs
/// How a [`RunningWireframeServer`] reached its terminal state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ShutdownOutcome {
    /// The server task returned and was joined.
    Completed,
    /// The server task panicked or was cancelled.
    Panicked,
    /// Reserved. The server task returned an error. Unreachable today, because
    /// `run_with_shutdown` has no failure path; retained because the task's
    /// type admits one.
    ServerFailed,
    /// The join bound elapsed. Per the escalation, the server was **not**
    /// stopped: its accept loops survive and the listener stays bound until
    /// the runtime is dropped.
    Abandoned,
}

/// A bound, ready, running Wireframe server owned by a test.
///
/// Dropping this handle stops the server, so binding it to `_` discards a
/// server you probably meant to keep.
#[must_use]
pub struct RunningWireframeServer { /* private fields */ }

impl RunningWireframeServer {
    /// The address the server was bound to.
    #[must_use] pub const fn local_addr(&self) -> SocketAddr;

    /// The terminal classification, or `None` while the handle is still live.
    #[must_use] pub fn shutdown_outcome(&self) -> Option<ShutdownOutcome>;

    /// The typed failure that produced the terminal state, if any.
    ///
    /// Lent rather than returned by value, because none of `Elapsed`,
    /// `JoinError`, or `ServerError` is `Clone`. This is how a caller
    /// distinguishes failures without parsing message text.
    #[must_use] pub fn shutdown_error(&self) -> Option<&TestError>;

    /// Stop the server and join its task. Idempotent.
    ///
    /// # Errors
    ///
    /// On its first call from a live handle, returns `TestError::Join` if the
    /// task panicked or was cancelled, `TestError::Server` if it returned an
    /// error, or `TestError::Timeout` if the join bound elapsed. Once the
    /// handle is terminal this method is a no-op returning `Ok(())`; query
    /// [`Self::shutdown_outcome`] and [`Self::shutdown_error`] for the
    /// terminal status.
    pub async fn shutdown(&mut self) -> TestResult<()>;
}

impl Debug for RunningWireframeServer { /* redacts live resources */ }
impl Drop for RunningWireframeServer { /* panic-free, bounded, non-blocking */ }
```

```rust
// lifecycle.rs — the where clause is elided here; it is the verbatim block
// quoted in Context and orientation.
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

Crate-internal, not re-exported: `state::{Phase, Op, Step, plan}` are `pub` for
the root-package enumeration tests but `#[doc(hidden)]`, and
`diagnostics::{LifecycleStage, stage_message}` are `pub(crate)`.
`LifecycleStage` stays crate-private in 17.3.3 because nothing public produces
or consumes it; 17.3.4 should promote it when it gains a `Connect` variant and
a public producer.

The three re-exports are added to `wireframe_testing/src/lib.rs` alongside the
existing `client_pair` re-exports.

No new dependency in any manifest.

### Exact diagnostic strings

`stage_message` is the single site of message construction, and the strings are
part of the contract that the `contains_substring` assertions rely on:

```rust
pub(crate) fn stage_message(stage: LifecycleStage, detail: &str) -> String {
    format!("wireframe_testing server harness: {} stage failed: {detail}", stage.label())
}
```

with `label()` an exhaustive `match` returning `"bind"`, `"readiness"`,
`"shutdown"`, `"join"`, or `"abandon-after-timeout"`. A parameterized rstest
asserts that every variant renders a distinct, non-empty label, so adding a
variant without a label fails.

## Milestones and plateaus

### EP-M0 — escalations accepted, green baseline

- Outcome: D2, D3, D5, D8, and D11 accepted or amended; `make check-fmt`,
  `make lint`, and `make test` recorded green on an unmodified tree.
- Acceptance evidence: transcripts under `/tmp`; a note in `Decision log`
  recording who accepted what.
- Conformance check: confirm RFC 0001 has not changed since this plan was
  written.
- Recovery: nothing to undo.
- Compatibility decision: none.

### EP-M1 — the harness exists and is proven by example

- Outcome: `wireframe_testing::server_harness` exists with the handle, the
  options type, the pure transition function, and the three spawn helpers.
  `client_pair.rs` is untouched, so two live implementations coexist. That is
  intentional and matches RFC §8 stage 1; it is not compatibility machinery,
  because both are real implementations and one is deleted in EP-M3.
- Requirements: RFC §5.2, §5.3, §6, §7.1 (excluding its two connector bullets).
- Acceptance evidence: `tests/server_harness_lifecycle.rs` passes with the
  eight cases listed in Stage B, and the three scenarios in
  `tests/features/server_harness_lifecycle.feature` pass.
- Conformance check: the `where` clause matches verbatim; no default type
  parameter added; no file over its budget; submodules are private.
- Recovery: additive; revert the milestone's commits.
- Remaining gaps: no exhaustive or property coverage; the pair still duplicates
  the machinery, so RFC §5.6 is **not** discharged here.
- Compatibility decision: none; the new module has no consumers yet.

### EP-M1a — delegation spike, then revert

- Outcome: a throwaway commit wires `spawn_wireframe_pair` to the new helper,
  runs `tests/client_pair_harness.rs` and the pair BDD scenarios, and is then
  reverted. Nothing is kept but the knowledge.
- Rationale: EP-M2 invests substantial test code in a design whose only real
  consumer arrives in EP-M3. If the delegation is blocked — most plausibly on
  the Table 1 error shapes — it is far cheaper to discover that now.
- Acceptance evidence: a note in `Surprises & discoveries` recording whether
  the delegation was clean, and the revert commit.
- Recovery: the revert is the recovery.
- Compatibility decision: none; the spike is discarded.

### EP-M2 — the invariants are discharged

- Outcome: the exhaustive transition tests and the three property tests exist
  and pass, and each has been seen to fail against its negative control.
- Requirements: RFC §7.5; INV-1 through INV-5.
- Acceptance evidence: `make test` green; five recorded negative-control
  transcripts, including the deliberate compile failure for INV-3.
- Conformance check: ten consecutive full `make test` runs with no
  intermittent failure; every property case bounded by its internal
  ten-second timeout.
- Recovery: additive.
- Remaining gaps: the pair still duplicates the machinery.
- Compatibility decision: none.

### EP-M3 — the pair delegates and the duplication goes

- Outcome: `client_pair.rs` builds its default server, calls
  `spawn_wireframe_server_with_options` with an unbounded shutdown timeout, and
  keeps its own client-close-then-signal ordering. `spawn_bounded_shutdown` and
  `PendingServer` are gone from `client_pair.rs`; one copy lives in
  `server_harness`. RFC §5.6 is discharged here.
- Requirements: RFC §5.5, §5.6; INV-6.
- Acceptance evidence: every pre-existing pair test passes unmodified; the
  `map_pair_shutdown_error` unit tests pass; `cargo test -p wireframe_testing
  --all-targets` passes.
- Conformance check: `WireframePair` still concrete; both constructor
  signatures byte-identical under `git diff`; every cell of Table 1 unchanged;
  auto-trait assertions still hold.
- Recovery: the only milestone that changes existing behaviour, so revert this
  first if a regression appears. Reverting restores the EP-M2 plateau, in which
  both implementations coexist and everything is green.
- Remaining gaps: documentation.
- Compatibility decision: **required**. The named consumer is the published
  `wireframe_testing` 0.3.0 crate and any downstream suite using
  `spawn_wireframe_pair`. The façade is retained existing API, not a new shim.

### EP-M4 — documentation, RFC amendment, roadmap

- Outcome: the design record matches the code and roadmap 17.3.3 is ticked.
- Acceptance evidence: `make markdownlint` and `make nixie` pass; the roadmap
  checkbox is `[x]`.
- Conformance check: every deviation in `Decision log` has a corresponding RFC
  amendment. The plan moves to `COMPLETE` only when none remains unaccepted.
- Recovery: documentation-only.
- Compatibility decision: none.

## Plan of work

### Per-file budget

`AGENTS.md` caps files at 400 lines. Planned budgets, which together with the
test files give roughly 1 500 net added lines across roughly 26 files:

<!-- markdownlint-disable MD013 -->

| File | Budget | Contents |
| --- | --- | --- |
| `server_harness/mod.rs` | 60 | module docs, private `mod`s, re-exports, one runnable doctest |
| `server_harness/state.rs` | 120 | `Phase`, `Op`, `Step`, `plan`, `disposes_task` |
| `server_harness/options.rs` | 130 | `ServerHarnessOptions`, `Default`, builders, getters |
| `server_harness/diagnostics.rs` | 90 | `LifecycleStage`, `label`, `stage_message` |
| `server_harness/handle.rs` | 260 | the handle, `Debug`, `Drop`, accessors, step execution |
| `server_harness/lifecycle.rs` | 220 | three entry points plus `bind_and_address`, `await_readiness` |

<!-- markdownlint-enable MD013 -->

*Table 2: per-file line budgets. `handle.rs` is the file at risk; if it
approaches 400, move step execution into a sibling module rather than letting
it grow.*

### Stage A — understand and propose (no code changes)

Load the skills. Run `leta workspace add .`. Read `client_pair.rs` in full and
`src/server/runtime.rs:29-198`. Record a green baseline. Obtain acceptance for
D2, D3, D5, D8, and D11. Stage A ends when the baseline is green and the
escalations are resolved.

### Stage B0 — compiling stubs, so each test can fail on its own assertion

Land `server_harness/` with every public item present and every body
`todo!()`, plus the real `state.rs` (which is pure and trivial). Nothing else.
This exists so that Stage B's tests fail on their own assertions rather than
all sharing one `unresolved import` error — the first draft of this plan made
that mistake and its red evidence covered thirteen behaviours with a single
compiler message.

### Stage B — red tests

Write these and observe each failing for its own reason.

`tests/server_harness_lifecycle.rs`, one `#[rstest]` per bullet, all
`#[tokio::test]` except case 7. Pin `.workers(1)` on every server.

1. `spawn_then_connect_round_trip` — build an echo server with
   `echo_app_factory(&counter)` where `counter: Arc<AtomicUsize>`, spawn it,
   connect a `tokio::net::TcpStream`, and assert `handle.local_addr().ip()
   .is_loopback()` with a non-zero port. Do not assert an IPv4 range.
2. `caller_listener_keeps_its_address` — reserve with `unused_listener()`,
   record `local_addr()`, pass it to `spawn_wireframe_server_on`, assert the
   handle reports the recorded address.
3. `panicking_factory_is_reported` — use a test factory whose `Clone` panics.
   `run_with_shutdown` calls `factory.clone()` before firing `ready_tx`, so the
   task panics pre-readiness. Assert the spawn fails with `TestError::Join` and
   that the message contains `"readiness"` via
   `googletest::matchers::contains_substring`. Note: an app factory that
   *returns* an error cannot be used here — those errors are logged per
   connection and never reach startup.
4. `shutdown_is_idempotent` — call `shutdown().await` twice; assert both return
   `Ok(())`, and that `shutdown_outcome()` is `Some(ShutdownOutcome::Completed)`
   after each.
5. `cancelled_shutdown_then_drop` — pin the shutdown future, poll it once with
   `futures::poll!`, drop the future, drop the handle, then
   `await_port_released(addr, deadline)`.
6. `drop_inside_runtime_returns_immediately` — assert `drop(handle)` returns in
   well under the grace period, then `await_port_released(addr, deadline)`.
7. `drop_outside_runtime` — build the runtime in a `#[test] fn`, move the
   handle to a `std::thread::spawn`, drop it there. Assert the *effect*, not
   merely that nothing hangs: keep the runtime alive across the assertion and
   record what actually happens to the listener. Per AXIOM-6 this path abandons
   rather than cleans up, so the honest assertion is that the handle reports
   `Abandoned` before it is dropped, and the test documents that the port
   survives until runtime teardown.
8. `shutdown_bound_is_reported` — one `#[rstest]` with two `#[case]`s. Hold a
   connection open against a handler that sleeps for 200 ms, then shut down
   with a 1 ms bound (expect `Abandoned` and `TestError::Timeout`) and with a
   5 s bound (expect `Completed`). The 200 ms figure is what makes both cases
   deterministic: comfortably above the short bound, comfortably below the
   long one. This pair is INV-4's clock-side non-vacuity control.

Add one shared helper, used by cases 5, 6, and the third BDD scenario, which
removes the plan's only sleep and its only timing race:

```rust
/// Poll until nothing accepts on `addr`, or the deadline passes.
async fn await_port_released(addr: SocketAddr, deadline: Duration) -> TestResult<()>;
```

It retries `TcpStream::connect` on a short interval and returns an error naming
the address if the deadline elapses. Note the caveat: on a busy run the port
may be re-bound by another test, so treat a successful connect as inconclusive
rather than as proof the server is alive, and assert `shutdown_outcome()`
alongside.

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

  Scenario: Dropping the harness without stopping it releases the port
    Given a configured Wireframe echo server that has not been bound
    When the server lifecycle harness starts the server
    And the harness is dropped without being stopped
    Then the address stops accepting connections within the grace period
```

The third scenario's `Then` is deliberately worded as a bounded wait, because
`Drop` schedules detached cleanup and returns immediately; asserting an
instantaneous refusal would be a race. The step calls `await_port_released`.

Add `tests/fixtures/server_harness_lifecycle.rs` with a
`ServerHarnessLifecycleWorld` holding a `tokio::runtime::Runtime`, an
`Option<RunningWireframeServer>`, the recorded address, and the recorded stop
results. **Declare the handle field before the runtime field**, or write an
explicit `Drop` that clears the handle first: fields drop in declaration order,
and a handle dropped after its runtime finds no runtime to schedule cleanup on.
`tests/fixtures/client_pair_harness.rs` gets this right with an explicit `Drop`
that blocks on cleanup; follow it. Steps in
`tests/steps/server_harness_lifecycle_steps.rs` are synchronous and delegate to
world methods; the fixture parameter name must be
`server_harness_lifecycle_world` throughout, matching the `#[fixture]` function
name, because `strict-compile-time-validation` turns a mismatch into a compile
error. Bindings go in
`tests/scenarios/server_harness_lifecycle_scenarios.rs`. Register all four in
the corresponding `mod.rs` files.

`tests/server_harness_lifecycle_props.rs` holds the two exhaustive
enumerations (INV-1, INV-2) as plain `#[rstest]` loops over `Phase × Op` and
over `Op` sequences, and the three property tests (INV-3's runtime half, INV-4,
INV-5). The property idiom:

```rust
proptest! {
    #![proptest_config(ProptestConfig { cases: 16, max_shrink_time: 30_000, ..ProptestConfig::default() })]

    #[test]
    fn repeated_shutdown_converges(calls in 1usize..=4) {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| TestCaseError::fail(e.to_string()))?;
        runtime.block_on(async move {
            tokio::time::timeout(Duration::from_secs(10), async {
                // spawn with .workers(1), call shutdown `calls` times,
                // assert with prop_assert_eq! on shutdown_outcome()
                Ok(())
            })
            .await
            .map_err(|_| TestCaseError::fail("case exceeded 10 s"))?
        })?;
    }
}
```

`proptest!` cannot take an `async fn`, so the body is synchronous, builds a
`current_thread` runtime, and `block_on`s an `async` block returning
`Result<(), TestCaseError>` so `prop_assert!` works and `?` propagates. The
ten-second internal timeout is what turns a hang into a printable failure; it
must not be expressed with `ServerHarnessOptions`, which is the thing under
test. Use `prop_assert_eq!` on `shutdown_outcome()` rather than `prop_assert!`
on a boolean, so a shrunk counter-example prints actual against expected. Do
not use `unwrap` or `expect`.

Commit the `tests/server_harness_lifecycle_props.proptest-regressions` file
that proptest writes on failure, so a reproduction replays deterministically.

Stage B ends when every new test fails for a reason you have written down and
no pre-existing test has changed.

### Stage C — implementation

Replace the Stage B0 `todo!()` bodies, file by file, running the focused tests
after each.

`state.rs` is already real from Stage B0. `diagnostics.rs` holds
`LifecycleStage`, its exhaustive `label()`, and `stage_message`.
`options.rs` holds the options type. `handle.rs` holds the handle and executes
the `Step` that `plan()` returns:

```rust
pub async fn shutdown(&mut self) -> TestResult<()> {
    let phase = self.phase.take_current();          // leaves a placeholder, never moves out of self
    let (next, step) = state::plan(&phase, Op::Shutdown);
    self.phase.set(next);                            // installed BEFORE awaiting, so a cancelled
                                                     // future leaves recoverable state
    match step {
        Step::SignalThenJoin => { self.signal(); self.join_bounded().await }
        Step::JoinOnly => self.join_bounded().await,
        Step::ReportTerminal | Step::Nothing => Ok(()),
        Step::DetachedCleanup => Ok(()),             // unreachable for Op::Shutdown
    }
}
```

Installing the next phase before awaiting is what makes a cancelled shutdown
recoverable, mirroring the reasoning at `client_pair.rs:118`. Because the type
implements `Drop`, the state machine must go through `Option::take` or
`mem::replace`; it can never move out of `self`.

`join_bounded` maps the join result onto a terminal `ShutdownOutcome` and
retains the `TestError` for `shutdown_error()` to lend. On an elapsed bound it
sets `Abandoned`, calls `abort()`, emits

```rust
log::warn!(
    "{}",
    stage_message(LifecycleStage::AbandonAfterTimeout,
                  &format!("server at {addr} did not stop within {bound:?}; its accept loops \
                            survive and the listener stays bound until the runtime is dropped"))
);
```

and returns `TestError::Timeout`. `wireframe_testing` already depends on `log`
and `logtest`, so a `logtest`-based test asserts the warning fires. This is the
only diagnostic available on the `Drop` path, where no error can be returned to
anyone.

`Drop` calls `plan(&phase, Op::Drop)`, executes `DetachedCleanup` or `Nothing`,
and does nothing else. It must satisfy Constraint 5: no `unwrap`, no `expect`,
no locking. Note that the accessors take `&self` and `shutdown` takes
`&mut self`, so no lock is needed; if one appears, a panicking test body would
poison it and `Drop` would double-panic.

`lifecycle.rs` holds three named helpers so
`spawn_wireframe_server_with_options` stays small: `bind_and_address` (read
`local_addr()` from the **std** listener first, then bind — this removes the
`Option::None` case and preserves the real `io::Error`), `await_readiness`
(bounded, with the pending-server RAII guard held across it), and
`diagnose_early_exit`. `spawn_wireframe_server` calls `unused_listener()` and
delegates; `spawn_wireframe_server_on` delegates with default options.

Add the three re-exports to `wireframe_testing/src/lib.rs`.

### Stage D — the pair delegates

Extract `pub(crate) fn map_pair_shutdown_error` from `WireframePair::shutdown`
and unit-test it exhaustively (INV-6). Then rewrite `spawn_wireframe_pair`'s
body to build the default server as today, call
`spawn_wireframe_server_with_options(server, Some(listener),
ServerHarnessOptions::default().with_shutdown_timeout(None))`, connect the
default client, and store the `RunningWireframeServer` in `Running`. Delete
`spawn_bounded_shutdown` and `PendingServer` from `client_pair.rs`.

Three details that are easy to get wrong:

- **Declare `server` before `client` in `Running`**, or `take()` explicitly in
  `Drop`. Today `Drop` signals the server before the client socket closes;
  naive field ordering would invert that.
- **Preserve every cell of Table 1 individually.** Do not blanket-wrap into
  `TestError::Msg` — three of the six paths are already typed, and wrapping
  them would itself be the breach.
- **Add auto-trait assertions** to `tests/client_pair_harness.rs`. Swapping
  `Running`'s fields changes what `Send`, `Sync`, and `Unpin` are inferred from,
  and those are public API. `static_assertions` is not a dependency, so write
  them by hand:

  ```rust
  const _: fn() = || {
      fn assert_send<T: Send>() {}
      fn assert_sync<T: Sync>() {}
      assert_send::<WireframePair>();
      assert_sync::<WireframePair>();
  };
  ```

Run the full pre-existing suite. Any change to a pre-existing test's expected
output is a Constraint 1 breach: stop and escalate.

### Stage E — documentation

1. Add `## Protocol-agnostic server lifecycle harness` to
   `docs/wireframe-testing-crate.md`, after
   `## In-process server/client pair harness`, with `### Public API`,
   `### Lifecycle`, `### Timeout policy`, `### Usage`, and `### Rationale`.
   Update the `## Crate layout` bullet list. State plainly that an elapsed
   bound abandons rather than stops the server.
2. Add a subsection to `docs/users-guide.md` under `## Running servers`
   (line 1460) covering the `ready_signal` reservation, the difference between
   explicit `shutdown` and defensive `Drop`, and the abandonment caveat.
3. Add a subsection to `docs/developers-guide.md` under
   `## Test infrastructure and framework` (line 394) recording the
   async-proptest idiom, the pure-transition-function verification pattern, the
   `await_port_released` helper, and the rule that lifecycle behaviour the root
   package can observe stays in root `tests/` while crate-internal seams stay in
   `#[cfg(test)]` — including that 17.3.5 does not move them.
4. Amend RFC 0001: resolve §11.1 in favour of `ServerHarnessOptions`; record
   the `shutdown_error` lending contract and strict idempotence in §5.3; correct
   §5.5's claim that pair startup failures are uniformly `TestError::Msg`;
   correct §5.3 and §6 for the abandonment reality per D8; add `state.rs`,
   `diagnostics.rs`, `handle.rs`, and `options.rs` to §5.6; note that §5.5's
   "call the new server-and-connector helper" is deferred to 17.3.4. Append a
   revision note. Leave §11.2 open.
5. Do **not** add an ADR. The RFC is the governing artefact for this feature and
   amending it is clearer than an ADR that qualifies it. If the reviewer
   disagrees, the file is
   `docs/adr-011-server-lifecycle-shutdown-contract.md`, follows the
   Status/Date/Context template, and gains a line in `docs/contents.md`.
6. Tick roadmap 17.3.3 with `mapsplice`, preserving its inline links verbatim.
7. Check `CHANGELOG.md`'s conventions and add an entry if it covers unreleased
   test-support changes.

## Concrete steps

Run everything from the repository root. Note that the focused commands set
`RUSTFLAGS="-D warnings"` to match `make test`: alternating between the two
fingerprints costs about twenty seconds of rebuild each way.

```bash
export LOG_PREFIX="/tmp/17-3-3-$(git branch --show-current)"
export RUSTFLAGS="-D warnings"
set -o pipefail && make check-fmt 2>&1 | tee "${LOG_PREFIX}-check-fmt.log"
set -o pipefail && make lint      2>&1 | tee "${LOG_PREFIX}-lint.log"
set -o pipefail && make test      2>&1 | tee "${LOG_PREFIX}-test.log"
```

Expected at the tail of a green test log:

```plaintext
test result: ok. <N> passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

Focused loops during Stages B to D:

```bash
set -o pipefail && cargo test --test server_harness_lifecycle --all-features 2>&1 \
  | tee "${LOG_PREFIX}-lifecycle.log"
set -o pipefail && cargo test --test server_harness_lifecycle_props --all-features 2>&1 \
  | tee "${LOG_PREFIX}-props.log"
set -o pipefail && cargo test --test bdd --all-features server_harness 2>&1 \
  | tee "${LOG_PREFIX}-bdd.log"
set -o pipefail && cargo test --test client_pair_harness --all-features 2>&1 \
  | tee "${LOG_PREFIX}-pair.log"
```

Companion-crate checks, which `make test` does not cover until 17.3.5:

```bash
set -o pipefail && cargo test -p wireframe_testing --all-targets 2>&1 \
  | tee "${LOG_PREFIX}-testing-crate.log"
set -o pipefail && cargo test -p wireframe_testing --doc server_harness 2>&1 \
  | tee "${LOG_PREFIX}-testing-doc.log"
```

`--all-features` is omitted deliberately: `wireframe_testing` has no
`[features]` table, so the flag is a no-op there. The doc run is filtered to
`server_harness` because the crate's pre-existing doctests do not compile
(issue #578), so an unfiltered run cannot tell you whether your doctest is
sound. Record both the filtered pass and the unfiltered pre-existing failure,
so a reader can see #578 was not made worse.

Flakiness check, once at the end of EP-M2, using the **full** suite rather than
a focused target — an isolated loop removes the contention that surfaces
timing races:

```bash
for i in $(seq 1 10); do
  set -o pipefail && make test 2>&1 | tee "${LOG_PREFIX}-full-run-${i}.log" | tail -3
done
```

Documentation gates, at EP-M4:

```bash
set -o pipefail && make markdownlint 2>&1 | tee "${LOG_PREFIX}-markdownlint.log"
set -o pipefail && make nixie        2>&1 | tee "${LOG_PREFIX}-nixie.log"
```

`make doctest-benchmark` is **not** listed: `scripts/doctest-benchmark.sh:13`
searches `src` only, so it cannot see the new doctests and would be evidence of
nothing.

Commit after each milestone and gate each commit. Delegate full gate runs to
the `scrutineer` sub-agent rather than running them inline, and read the cited
log on failure rather than re-running.

## Validation and acceptance

### Red-Green-Refactor evidence

- **Red**: after Stage B0, each of the eight rstest cases fails on its own
  assertion or on a `todo!()` panic naming the function it reached. Paste the
  eight distinct failure lines. A single shared `unresolved import` error is
  not acceptable red evidence.
- **Green**: `cargo test --test server_harness_lifecycle --all-features`
  reports eight passing cases after Stage C.
- **Refactor**: after the Stage C tidy-up and again after Stage D, both the
  focused command and `make test` pass.

### BDD evidence

`cargo test --test bdd --all-features server_harness` fails before Stage C and
reports three passing scenarios after.

### Verification evidence

For each of INV-1 through INV-6 record the command, the passing result, the
negative-control mutation applied, and the failure it produced. An invariant
whose negative control still passes has not been verified. INV-3's control is a
deliberate compile failure, not a test failure; record the compiler message.

### Quality criteria

- Tests: `make test` green with no new failures and no `#[ignore]`;
  `cargo test -p wireframe_testing --all-targets` green.
- Verification: INV-1 through INV-6 discharged with non-vacuity evidence.
- Lint and format: `make check-fmt` and `make lint` clean. Note per D10 that
  neither reaches `wireframe_testing`; the new module's hygiene rests on review.
  If `no_expect_outside_tests` fires elsewhere, consult the
  `addressing-whitaker-findings` skill and check whether `main` is red first.
- Documentation: `make markdownlint` and `make nixie` pass.
- Performance: the new tests add no more than ten seconds to `make test`, of
  which roughly nine will be compilation of two new test targets. Measured
  reference: an existing full pair-harness cycle — bind, spawn, await
  readiness, connect, round-trip, shut down, join — runs in under one
  millisecond, so the roughly eighty spawns this plan adds cost well under a
  second. If the budget is ever breached the lever is target consolidation,
  **not** reducing generated case counts, which would save milliseconds and
  cost coverage.
- Security: none applicable; test-only scaffolding on loopback.

## Idempotence and recovery

Every stage is re-runnable and every gate is a pure read. EP-M1, EP-M1a, and
EP-M2 are additive; reverting them restores the previous plateau. EP-M3 is the
only milestone that changes existing behaviour, and reverting it alone returns
the repository to the EP-M2 plateau, where the new harness and the old pair
implementation coexist and both work. That coexistence is a deliberate,
temporary duplication of live code, not a compatibility shim.

If a test hangs, it is an unjoined task or an unreleased listener. Reproduce
with `cargo test --test <target> -- --nocapture --test-threads=1` and inspect
which handle never reached `Stopped`. Do not add a sleep; use
`await_port_released`.

If a run is interrupted, check `ss -ltn | grep 127.0.0.1` before re-running.

## Surprises & discoveries

- Observation: `run_with_shutdown` cannot fail. It returns `Ok(())` on every
  path; `accept_loop` returns `()`; `ServerError::Accept` is constructed
  nowhere in `src/`; app-factory failures are logged per connection.
  Evidence: `src/server/runtime.rs:142-198`; `src/server/runtime/accept.rs:149`;
  `src/server/connection_spawner.rs:99-108`; `grep -rn "ServerError::Accept"
  src/` returns nothing.
  Impact: `ShutdownOutcome::ServerFailed` is unreachable; an early draft's
  "app factory returns an error" test was vacuous and is replaced by a
  panicking-`Clone` factory; the only reachable pre-readiness faults are a
  panicking task and a readiness timeout.

- Observation: the reachable pre-readiness fault is a panicking `Clone`.
  `AppFactory` requires `Clone`, `run_with_shutdown` clones once per worker
  before firing `ready_tx`, and the default worker count is at least one.
  Evidence: `src/server/mod.rs:122-139`; `src/server/runtime.rs:166-188`;
  `src/server/config/mod.rs:76-90`.

- Observation: `shutdown` takes `&mut self`, so the borrow checker already
  excludes concurrency on one handle.
  Impact: the lifecycle is a sequential decision table. This is why the
  verification is exhaustive enumeration of a pure function rather than a
  concurrency model checker.

- Observation: `crates/wireframe-verification` cannot depend on
  `wireframe_testing`.
  Evidence: `crates/wireframe-verification/Cargo.toml:7-12` lists only
  `stateright` and `wireframe`.
  Impact: decisive against an earlier draft's Stateright model, which could
  only have been a transcribed copy with correspondence enforced by a comment.

- Observation: `crates/wireframe-verification` is run by no Makefile target or
  CI job, so its existing placeholder model has never executed.
  Evidence: no `-p wireframe-verification` in the Makefile or
  `.github/workflows/`.
  Impact: corroborates the previous entry. A model nobody runs is not evidence.

- Observation: `make lint` never reaches `wireframe_testing`, and the crate has
  no `[lints]` table, so the root package's `pedantic`, `unwrap_used = "deny"`,
  and `missing_docs` settings do not apply to it.
  Evidence: `Makefile:82-85` has no `-p` or `--workspace`; `Cargo.toml:15` sets
  `default-members = ["."]`; `Cargo.toml:108` is `[lints.clippy]`, not
  `[workspace.lints]`.
  Impact: decision D10.

- Observation: `ci.yml` does not run `make test`. Tests execute inside the
  shared `generate-coverage` action, on pull requests only, under
  `cargo-llvm-cov`.
  Evidence: `.github/workflows/ci.yml:67-73`.
  Impact: a hung test appears as a coverage-action failure, not a test failure;
  documentation must not claim `make test` is the CI gate.

- Observation: neither `ci.yml` nor `coverage-main.yml` sets `timeout-minutes`,
  so a hung job runs for GitHub's 360-minute default, and libtest's buffering
  means the log ends at `running N tests` with no indication of which test.
  Impact: decision D11.

- Observation: `scripts/doctest-benchmark.sh:13` defaults its search root to
  `src`, the root crate only.
  Impact: `make doctest-benchmark` cannot see the new doctests and was removed
  from the acceptance evidence.

- Observation: `tests/advanced/interaction_fuzz.rs` is referenced by no
  `[[test]]` target and matches no Cargo auto-discovery rule, so the
  repository's only async-proptest example is never compiled.
  Impact: treat it as a style reference only. Reporting the dead target is
  outwith this plan.

- Observation: `unused_listener()` binds `"localhost:0"`, which may resolve to
  `::1`.
  Evidence: `wireframe_testing/src/integration_helpers.rs:44`.
  Impact: assert `is_loopback()`, never an IPv4 range.

- Observation: `bind_to_listener` reuses the same file descriptor.
  Evidence: `src/server/config/binding.rs:77-79`.
  Impact: reading the address from the std listener before binding removes the
  `Option::None` case and preserves the real `io::Error`.

- Observation: `docs/repository-layout.md` is referenced by `AGENTS.md` and the
  style guide but does not exist.
  Impact: none here; recorded so a reader does not hunt for it.

- Observation: the readiness signal fires after workers are spawned, not after
  `accept()` is first polled.
  Evidence: `src/server/runtime.rs:184-188`.
  Impact: "ready" means "workers launched onto a bound listener"; the listen
  backlog is what makes an immediate connect safe. Document it that way.

## Decision log

- Decision (D8): **ESCALATED, BLOCKING.** `JoinHandle::abort()` does not stop a
  Wireframe server, so RFC §5.3's abort-after-timeout guarantee and §6's
  "every cleanup path joins the server task" are not implementable. Evidence,
  blast radius, and three options are in `Escalation: abort does not stop the
  server`; the recommendation is option 1, report the leak rather than hide it,
  with the upstream fix raised as its own roadmap item.
  Affected identifiers: RFC-5.3, RFC-6, INV-3, Constraint 6, and the
  `ShutdownOutcome` vocabulary (`TimedOut` becomes `Abandoned`).
  Required upstream change: an amendment to RFC 0001 §5.3 and §6 whichever
  option is chosen.
  Date/Author: 2026-08-23, planning agent. **Requires acceptance.**

- Decision (D9): correct the compatibility premise. An earlier draft's claim
  that `spawn_wireframe_pair` startup failures "surface as `TestError::Msg`" is
  false; only two of six paths produce `Msg` today, and
  `WireframePair::shutdown` produces `Msg` for both of its failure modes. The
  obligation is Table 1, cell by cell. RFC §5.5 has the same error and needs
  the same amendment.
  Rationale: a blanket re-wrap in EP-M3 would have broken the `Io`, `Server`,
  and `Client` paths in the name of preserving compatibility.
  Date/Author: 2026-08-23, planning agent.

- Decision (D1): scope to RFC §5.2, §5.3, §5.5, §5.6, §6, most of §7.1, and
  §7.5. The connector (§5.4), its two §7.1 bullets, the protocol-generic proof
  (§7.2), and `trybuild` (§7.3) go to 17.3.4; the companion-crate gates and
  issue #578 (§7.4) go to 17.3.5.
  Rationale: those are the boundaries the roadmap draws and RFC §8 sequences.
  Known cost: RFC §5.5 says the pair should call the connector helper, so
  17.3.4 will rewrite `spawn_wireframe_pair`'s body a second time and re-run
  the INV-6 argument. Accepted, and recorded so 17.3.4 is not surprised.
  Date/Author: 2026-08-23, planning agent.

- Decision (D2): introduce a public `ServerHarnessOptions`, resolving RFC §11.1
  in the affirmative.
  Rationale: RFC §5.2 permits it "if the implementation spike proves that fixed
  defaults are insufficient". They are. With fixed five-second timeouts the
  shutdown-bound path in RFC §7.1 can only be exercised by five-second sleeps,
  and INV-4's non-vacuity control — showing the assertion is sensitive to the
  bound — is impossible. Injectable timeouts also let the pair façade express
  its required unbounded join as `with_shutdown_timeout(None)` rather than
  needing a second code path. Three fields, no builder ceremony, per the RFC's
  warning that timeout configuration must not become a second server builder.
  Rejected alternative: `pub(crate)` timeouts with a `#[cfg(test)]` setter.
  `cfg(test)` is set only when compiling the crate's own harness, and D7 puts
  the gated tests in the root package, where `wireframe_testing` is an ordinary
  dependency. The setter would be invisible to every test that needs it.
  Rejected alternative: `tokio::time::pause()`. Available — the root package
  enables `test-util` — but it interacts badly with real loopback I/O and does
  not give the pair its unbounded join.
  Note: `Copy` and `Eq` are deliberately **not** derived. Both are permanent
  commitments, and a future non-`Copy` field such as a label would make
  removing them a breaking change.
  Date/Author: 2026-08-23, planning agent. **Requires acceptance.**

- Decision (D3): `shutdown()` is strictly idempotent — it performs the work
  once and returns `Ok(())` on every later call — and the terminal status is
  exposed by `shutdown_outcome()` and `shutdown_error()`.
  Rationale: RFC §5.3 asks a repeated call to report the same failure, and §6
  asks callers to distinguish failures without parsing message text. An earlier
  draft tried to satisfy the first by replaying a `TestError::Msg`, which
  contradicted this plan's own stated success criterion that a second call
  returns `Ok(())`, and created a real hazard: the repository's BDD world
  pattern calls `shutdown()` defensively from `Drop`, so a replayed `Err` would
  surface during unwind.
  The key realization is that D3's original premise was too narrow. None of
  `Elapsed`, `JoinError`, or `ServerError` is `Clone`, so the typed error
  cannot be *returned by value* twice — but it can be *lent*.
  `shutdown_error(&self) -> Option<&TestError>` retains the original, gives the
  caller the full `source()` chain, and satisfies §6 properly. `f(f(x)) ==
  f(x)` now holds for both effect and result.
  Rejected alternative: `Result<(), Arc<TestError>>`, which breaks `?` at every
  call site and contradicts RFC §5.2's signature.
  Rejected alternative: a new `Clone` `TestError::Lifecycle` variant in
  `src/testkit/result.rs`. Viable — `TestError` is `#[non_exhaustive]` and
  test-facing — but unnecessary once the error is lent rather than returned,
  and it would widen the root crate's surface for no gain.
  Impacts: RFC §5.3 needs amending to describe lending rather than replay.
  Date/Author: 2026-08-23, planning agent. **Requires acceptance.**

- Decision (D4): split RFC §5.6's `lifecycle.rs` into `state.rs`,
  `diagnostics.rs`, `options.rs`, `handle.rs`, and `lifecycle.rs`, all private
  submodules of a single `pub mod server_harness`.
  Rationale: `AGENTS.md` caps files at 400 lines and the machinery does not fit
  in two. Keeping the submodules private means the split stays an
  implementation detail and can be re-cut later; `client_pair` is `pub mod`, so
  the precedent would otherwise have frozen the file layout as public API. The
  boundary RFC §5.6 cares about — protocol-neutral lifecycle in
  `server_harness`, default-client façade in `client_pair` — is unchanged.
  Date/Author: 2026-08-23, planning agent.

- Decision (D5): **withdraw the Stateright model.** An earlier draft added a
  bounded state-machine model in `crates/wireframe-verification`, plus a
  `test-verification` Make target and a CI step. All three are withdrawn.
  Rationale: four reasons, in descending weight. First, the model could not
  reference the code it modelled — `crates/wireframe-verification` depends on
  `wireframe` and `stateright`, not on `wireframe_testing`, and adding that
  dependency would breach Constraint 11 — so its correspondence would have been
  enforced by a comment in two crates. Second, Stateright's checker cannot
  prove it searched exhaustively: states beyond `target_max_depth` are skipped
  without evaluation, and `is_done()` returns true even when workers stopped on
  the state budget, so a truncated search passes silently. Third, the shared
  `assert_model_properties` returns `()` and cannot yield the explored-state
  count the milestone required as evidence. Fourth, RFC §7.5 explicitly says a
  model checker is not warranted in the first release, and the roadmap asks for
  proptest.
  What replaces it is stronger, not weaker: extracting the lifecycle decision
  into a pure, total `plan(Phase, Op) -> (Phase, Step)` that the real handle
  calls, then enumerating `Phase × Op` and every operation sequence up to
  length six. The correspondence is by construction rather than by comment, the
  enumeration is genuinely exhaustive rather than bounded-and-possibly-
  truncated, and `Step::disposes_task`'s exhaustive `match` makes the
  no-orphaned-task obligation a compile error rather than an assertion.
  Consequences: the Makefile and CI stay untouched (restoring Constraint 7);
  `docs/developers-guide.md:294-296`, which says formal-verification targets
  belong to later roadmap items, is no longer contradicted; and the coverage
  ratchet does not gain a new crate.
  Date/Author: 2026-08-23, planning agent. **Requires acceptance.**

- Decision (D6): do not use Kani, Verus, or `insta`. Reasoning in
  `Verification plan` under "Methods deliberately not used".
  Date/Author: 2026-08-23, planning agent.

- Decision (D7): put gated behavioural coverage in the root package's `tests/`
  tree, not `wireframe_testing/tests/`.
  Rationale: `default-members = ["."]` makes the companion crate's targets
  invisible to `make test` until 17.3.5, and tests that do not run are not
  evidence. The root package already dev-depends on everything needed, and
  `tests/client_pair_harness.rs` sets the precedent. The exceptions are the
  `map_pair_shutdown_error` unit tests and the `logtest` assertion, which need
  crate-internal access; they run under the explicit `-p` command. Note this is
  a permanent boundary, not a scheduling artefact: 17.3.5 does not move them.
  Date/Author: 2026-08-23, planning agent.

- Decision (D10): accept that `make lint` does not reach `wireframe_testing`,
  and treat the size and complexity limits as authoring discipline verified in
  review.
  Rationale: `Makefile:82-85` passes neither `-p` nor `--workspace` and
  `default-members = ["."]`; the crate has no `[lints]` table, so even a manual
  `-p` run gets none of the root package's configuration. The fix — hoisting
  `[workspace.lints]` and adding `lints.workspace = true` — would newly subject
  the crate's existing code to `unwrap_used = "deny"` and `missing_docs`,
  which is a large and unrelated change. It belongs to 17.3.5, which owns
  companion-crate gating.
  Consequence: statements like "clippy denies `unwrap_used` here" are false for
  this crate. Only whitaker's `no_expect_outside_tests` applies, and only via
  the root-package run.
  Date/Author: 2026-08-23, planning agent.

- Decision (D11): add `timeout-minutes` to the `build-test` and coverage jobs
  in `.github/workflows/`, notwithstanding Constraint 7.
  Rationale: nothing in the stack bounds a hung test. `proptest`'s
  `max_shrink_time` is unlimited by default, there is no nextest configuration,
  and no workflow sets a job timeout, so GitHub's 360-minute default applies.
  Because libtest buffers per-test output, the surviving log ends at
  `running N tests` with no indication of which test hung. A harness whose
  purpose is to stop test suites hanging should not ship without this. The
  change is two lines and touches no test target, so it does not encroach on
  17.3.5.
  Date/Author: 2026-08-23, planning agent. **Requires acceptance.**

- Decision (D12): keep the three free spawn functions rather than introducing a
  harness builder now.
  Rationale: RFC §5.2 names exactly `spawn_wireframe_server` and
  `spawn_wireframe_server_on`, and the options variant is the minimum addition
  that makes the timeout paths testable. A `WireframeServerHarness` builder
  with `on_listener`, `with_options`, `spawn`, and later `spawn_and_connect`
  would cap the argument count at two forever and give 17.3.4 a natural slot,
  which is a genuinely better long-term shape. But it is a fourth public type
  introduced ahead of the consumer that needs it. Recommendation for 17.3.4:
  adopt the builder then, and reduce these three functions to thin wrappers,
  rather than adding a fifth function
  (`spawn_wireframe_server_and_connect_with_options`) that would sit on the
  four-argument clippy ceiling.
  Date/Author: 2026-08-23, planning agent.

- Decision (D13): `shutdown` takes `&mut self`, not `self`.
  Rationale: a consuming signature would make double-shutdown a compile error
  and delete most of the state machine, which is attractive. It is rejected
  because RFC §5.3 requires that "after a cancelled shutdown future, a later
  call completes the outstanding join" — impossible if the handle was moved
  into the cancelled future — and because `WireframePair::shutdown(&mut self)`
  is signature-frozen by Constraint 1 and must drive the inner handle through a
  `&mut`. The honest price: the type system cannot prevent use-after-shutdown,
  which is precisely why `shutdown_outcome()` exists as a runtime query.
  Also rejected: `tokio_util::task::AbortOnDropHandle`, whose `Drop` aborts
  immediately, the opposite of the signal-then-grace-then-abandon sequence RFC
  §5.3 requires — and which is behind the `rt` feature, a manifest change.
  Also considered: replacing the `oneshot` with a `CancellationToken`, which
  `tokio-util` exposes ungated. It would remove the shutdown-future adaptor,
  but the server builds its own token internally so it does not fix D8, and it
  would lose the property that dropping the `oneshot` sender closes the channel
  and stops the server. Not worth the churn.
  Date/Author: 2026-08-23, planning agent.

## Outcomes & retrospective

To be completed at EP-M4. Before setting the status to `COMPLETE`, reconcile
every entry in `Surprises & discoveries` and `Decision log` against RFC 0001,
and confirm that D2, D3, D5, D8, and D11 have been accepted and that the RFC
amendments in Stage E step 4 have landed. An unaccepted deviation blocks
completion.

## Artefacts and notes

Reserved for transcripts. At minimum, capture:

1. The eight distinct red failures after Stage B0.
2. The green `test result: ok` line for each new test target.
3. One negative-control result per invariant, INV-1 through INV-6, including
   INV-3's deliberate compile error.
4. The filtered `-p wireframe_testing --doc server_harness` pass alongside the
   unfiltered pre-existing failure.
5. The `logtest` transcript showing the abandonment warning.
6. The ten full `make test` runs from the flakiness check.
