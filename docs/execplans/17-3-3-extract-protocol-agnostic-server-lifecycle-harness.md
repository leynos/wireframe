# Extract a protocol-agnostic server lifecycle harness (17.3.3)

This ExecPlan (execution plan) is a living document. The sections
`Constraints`, `Tolerances`, `Risks`, `Progress`, `Surprises & discoveries`,
`Decision log`, `Outcomes & retrospective`, `Conformance basis`, and
`Verification plan` must be kept up to date as work proceeds.

Status: DRAFT

## Purpose / big picture

Today the only way to get a running Wireframe server inside a test is
`wireframe_testing::spawn_wireframe_pair`. That helper builds the server for
you and hands back a client whose type is fixed to
`WireframeClient<BincodeSerializer, RewindStream<TcpStream>, ()>`. A downstream
crate that speaks a different protocol — one with its own frame codec, its own
typed handshake preamble, or its own client type — cannot use it. Such a crate
must instead hand-roll listener reservation, readiness waiting, task ownership,
shutdown signalling, and defensive cleanup. That is precisely the code most
likely to produce port races, orphaned Tokio tasks, and hanging test suites.

After this change, a test author can configure a `WireframeServer` however they
like — any serializer, any codec, any preamble, any worker count — hand the
still-unbound server to `wireframe_testing::spawn_wireframe_server`, and get
back a `RunningWireframeServer`: a small, non-generic handle that knows the
bound loopback address and can be stopped exactly once, safely, repeatedly, and
without hanging.

Observable success, stated as behaviour a person can check:

1. `cargo test --all-targets --all-features` passes, and the new integration
   test `server_harness_lifecycle::spawn_then_connect_round_trip` fails before
   the change and passes after it.
2. A test can call `handle.shutdown().await` twice in a row and the second call
   returns `Ok(())` without blocking, without re-signalling, and without
   joining a second time.
3. A test can drop a `RunningWireframeServer` without ever calling `shutdown`,
   inside or outside a Tokio runtime, and the test process still exits; nothing
   hangs and no listener stays bound.
4. Running the existing suites `tests/client_pair_harness.rs`,
   `tests/features/client_pair_harness.feature`, and
   `wireframe_testing/tests/integration_helpers.rs` unchanged still passes:
   `spawn_wireframe_pair` and `spawn_wireframe_pair_default` keep their exact
   signatures and their existing `TestError::Msg` startup-failure shape.
5. `cargo test -p wireframe-verification --all-features` reports the new
   bounded state-machine model as satisfying its properties, and deliberately
   breaking one model transition makes it fail.

## Context and orientation

Read this section first if you have never worked in this repository.

### What Wireframe is

Wireframe is a Rust library for building servers and clients that speak custom
binary protocols over TCP. The root package is `wireframe` (manifest
`Cargo.toml` at the repository root, sources under `src/`). A companion package
`wireframe_testing` (manifest `wireframe_testing/Cargo.toml`, sources under
`wireframe_testing/src/`) ships helpers that make Wireframe applications easy
to test. A third package `wireframe-verification`
(`crates/wireframe-verification/`) holds formal models; it is `publish = false`
and exists purely to host bounded model checks.

### Vocabulary used throughout this plan

- **Frame**: one length-delimited (or codec-defined) unit of bytes on the wire.
- **Envelope**: Wireframe's built-in framed message type, `wireframe::app::Envelope`.
- **Preamble**: an optional typed handshake value exchanged once, before any
  framed traffic, at the start of a connection.
- **Serializer**: the strategy that turns a Rust value into payload bytes; the
  default is `wireframe::serializer::BincodeSerializer`.
- **Codec**: the strategy that splits a byte stream into frames; the default is
  `LengthDelimitedFrameCodec`.
- **Typestate**: a marker type parameter that encodes a compile-time phase.
  `WireframeServer<F, T, S, ...>` uses `S = Unbound` before it owns a listener
  and `S = Bound` afterwards. `Unbound` and `Bound` live in `src/server/mod.rs`
  and the `ServerState` trait that admits them is *sealed*, so no new typestate
  can be added from outside the root crate.
- **Readiness signal**: a `tokio::sync::oneshot::Sender<()>` handed to the
  server; the server fires it once, after it has spawned all of its accept-loop
  worker tasks.
- **Harness**: test-only scaffolding that owns a server's lifetime.

### The relevant existing code

`wireframe_testing/src/client_pair.rs` (379 lines) is the whole of the current
lifecycle machinery. Reading it is the single most useful preparation for this
work. It contains:

- `WireframePair { addr: SocketAddr, running: Option<Running> }`, the public
  handle, with a handwritten `Debug` that redacts `running`.
- `Running { client: Option<WireframeClient<...>>, shutdown_tx:
  Option<oneshot::Sender<()>>, handle: Option<JoinHandle<Result<(),
  ServerError>>> }`, the private live-resource bundle.
- `spawn_wireframe_pair(app_factory, configure_client)` at line 274, whose
  `configure_client` callback must return the *same*
  `WireframeClientBuilder<BincodeSerializer, (), ()>` type it receives. This is
  the constraint that makes type-changing builder methods such as
  `with_preamble` and `on_connection_setup` impossible to use.
- `spawn_wireframe_pair_default(app_factory)` at line 371, which simply passes
  the identity closure.
- `WireframePair::shutdown` at line 125: close the client, send the shutdown
  signal, then perform an **unbounded** `handle.await`. It deliberately leaves
  `self.running` populated until the join completes, so that a cancelled
  `shutdown` future still leaves recoverable state for `Drop`.
- `Drop for WireframePair` at line 160: send the shutdown signal, then call
  `spawn_bounded_shutdown(handle, Duration::from_millis(100))`.
- `spawn_bounded_shutdown` at line 183: if `tokio::runtime::Handle::try_current()`
  succeeds, spawn a detached task that races the join against
  `tokio::time::sleep(timeout)` and aborts the server task only if the sleep
  wins; outside a runtime, abort immediately.
- `PendingServer` at line 205: an RAII guard holding
  `Option<(oneshot::Sender<()>, JoinHandle<...>)>` whose `Drop` repeats the same
  signal-then-`spawn_bounded_shutdown` pattern, used to clean up if the client
  fails to connect.

The bounded-shutdown machinery therefore already exists twice in one file. RFC
0001 §5.6 asks for it to be consolidated once, in a new `server_harness`
module.

### Server-side API this harness must drive

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

The methods the harness drives are:

```rust
// src/server/config/binding.rs:172, on the Unbound typestate
pub fn bind_existing_listener(
    self,
    std_listener: StdTcpListener,
) -> Result<BoundServer<F, T, Ser, Ctx, E, Codec>, ServerError>;

// src/server/config/binding.rs:209, on the Bound typestate
pub fn local_addr(&self) -> Option<SocketAddr>;

// src/server/config/mod.rs:135, on either typestate
pub fn ready_signal(self, tx: tokio::sync::oneshot::Sender<()>) -> Self;

// src/server/runtime.rs:142, on the Bound typestate only
pub async fn run_with_shutdown<S>(self, shutdown: S) -> Result<(), ServerError>
where
    S: Future<Output = ()> + Send;
```

Note that `local_addr` returns an `Option` even once bound, because reading the
address from the underlying listener can itself fail.

The `where` clause on the impl block that provides `run_with_shutdown`
(`src/server/runtime.rs:29`) is exactly:

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

Copy that clause verbatim onto the new helpers. Do not add default type
parameters and do not narrow any bound; the entire point of this item is that
callers keep their own serializer, context, packet, codec, and preamble
choices.

`run_with_shutdown` does not itself require `S: 'static`, but `tokio::spawn`
does. Construct the shutdown future as an owned `async move { let _ =
shutdown_rx.await; }` block so it is trivially `Send + 'static`, exactly as
`client_pair.rs:301` already does.

### Error types

`TestError` and `TestResult` are defined in the *root* crate at
`src/testkit/result.rs` and re-exported by
`wireframe_testing/src/integration_helpers.rs:18`. `TestError` is
`#[non_exhaustive]` and already carries every variant this work needs:
`Msg(String)`, `Server(#[from] wireframe::server::ServerError)`, `Join(#[from]
tokio::task::JoinError)`, and `Timeout(#[from] tokio::time::error::Elapsed)`.
No new variant is required. `TestResult<T = ()> = Result<T, TestError>`.

`ServerError` (`src/server/error.rs`) is `#[non_exhaustive]` with variants
`Bind(io::Error)` and `Accept(io::Error)`; any `match` on it needs a wildcard
arm.

### Where tests live in this repository

- Root-package integration tests: `tests/*.rs`. These are compiled by `make
  test` (`cargo test --all-targets --all-features`). The root package
  dev-depends on `wireframe_testing`, `rstest`, `rstest-bdd`, `proptest`,
  `googletest`, and `pretty_assertions`.
- Behaviour-driven tests: Gherkin features in `tests/features/*.feature`, World
  fixtures in `tests/fixtures/*.rs`, step definitions in `tests/steps/*.rs`,
  and scenario bindings in `tests/scenarios/*.rs`. They are compiled into the
  `bdd` test target (`Cargo.toml` `[[test]] name = "bdd"`, `required-features =
  ["advanced-tests"]`), which `--all-features` enables.
- Companion-crate tests: `wireframe_testing/tests/*.rs` and `#[cfg(test)]`
  modules inside `wireframe_testing/src/`. **These are not run by `make
  test`**, because the root manifest sets `default-members = ["."]`. Wiring
  them into the gate is roadmap item 17.3.5, not this one. This plan therefore
  places every behaviour that must be gated today in the root `tests/` tree,
  and runs the companion-crate command explicitly at each milestone.

### Skills and documents to load before starting

Load these skills at the start of the session, before touching code:

- `leta` — semantic code navigation. Run `leta workspace add .` once, then use
  `leta show <symbol>`, `leta refs <symbol>`, and `leta calls --to/--from` in
  preference to grep and whole-file reads.
- `rust-router` — routes to the smallest useful Rust skill. From it you will
  most likely want `rust-async-and-concurrency` (task ownership, cancellation),
  `rust-types-and-apis` (the generic boundary and typestate), `rust-errors`
  (the `TestError` mapping), and `rust-unit-testing` (rstest and googletest
  shape).
- `proptest` — strategies, shrinking, and the discipline of keeping generated
  cases cheap.
- `kani` and `verus` — load them so you can *justify* not using them here; see
  `Verification plan` for the reasoning.
- `execplans` — the rules this document obeys.
- `en-gb-oxendict` — all prose and comments use Oxford British spelling.
- `firecrawl` — for any external documentation lookup.

Read these repository documents:

- `docs/rfcs/0001-protocol-agnostic-test-harness-lifecycle.md` — the governing
  design. Sections 5.2, 5.3, 5.5, 5.6, 6, 7.1, and 7.5 are in scope here.
- `docs/wireframe-testing-crate.md` — the companion crate's design document.
  The section `## In-process server/client pair harness` is what this work sits
  beside.
- `docs/execplans/17-3-2-in-process-server-and-client-pair-test-harness.md` —
  the predecessor plan; useful for house style and for why the pair harness is
  shaped the way it is.
- `docs/documentation-style-guide.md` — 80-column prose, 120-column code,
  sentence-case headings, captions on every table and figure, en-GB-oxendict
  spelling, and the ADR and RFC templates.
- `docs/developers-guide.md` — `## Test infrastructure and framework`,
  `## Quality gates`, `## Cargo workspace semantics`, and `## Roadmap editing
  with mapsplice`.
- `docs/users-guide.md` — `## Conceptual model and vocabulary` for terminology,
  and `## Client runtime` for the vocabulary the harness docs must match.
- `docs/rust-testing-with-rstest-fixtures.md` — fixture composition.
- `docs/rstest-bdd-users-guide.md` — step functions are synchronous; fixture
  names must match step parameter names exactly.
- `docs/rust-doctest-dry-guide.md` — doctests are a gate; only public API
  carries executable examples; `make doctest-benchmark` enforces a runnable
  ratio of at least 70 per cent.
- `docs/hardening-wireframe-a-guide-to-production-resilience.md` and
  `docs/the-road-to-wireframe-1-0-feature-set-philosophy-and-capability-maturity.md`
  — background on why bounded, diagnosable failure is preferred to hanging.
- `docs/generic-message-fragmentation-and-re-assembly-design.md` and
  `docs/multi-packet-and-streaming-responses-design.md` — background on the
  frame and codec vocabulary; not directly modified by this work.
- `docs/reliable-testing-in-rust-via-dependency-injection.md` — the argument
  for injecting timeouts rather than hard-coding them, which this plan follows.

## Conformance basis

Upstream artefacts, at the revisions present in the working tree:

- `docs/rfcs/0001-protocol-agnostic-test-harness-lifecycle.md`, Status
  Proposed, Created 2026-07-18. This is the governing technical design. There
  is **no** separate Terms of Reference document for this work; do not invent
  one.
- `docs/roadmap.md` item 17.3.3 (the scope of this plan), with 17.3.4 and
  17.3.5 explicitly out of scope.
- `AGENTS.md` (repository engineering standards) and
  `docs/documentation-style-guide.md` (documentation standards).
- `clippy.toml`: `cognitive-complexity-threshold = 9`,
  `too-many-arguments-threshold = 4`, `too-many-lines-threshold = 70`,
  `excessive-nesting-threshold = 4`. These are unusually tight and directly
  shape how the implementation must be decomposed.

Traceability. Identifiers prefixed `RFC-` refer to RFC 0001 sections, `EP-M`
to milestones in this plan, and `INV-` to invariants in `Verification plan`.

```plaintext
ROADMAP-17.3.3 -> RFC-5.2 -> EP-M1 -> tests/server_harness_lifecycle.rs::spawn_then_connect_round_trip
ROADMAP-17.3.3 -> RFC-5.2 -> EP-M1 -> tests/server_harness_lifecycle.rs::caller_listener_keeps_its_address
ROADMAP-17.3.3 -> RFC-5.3 -> INV-1 -> EP-M2 -> tests/server_harness_lifecycle_props.rs::repeated_shutdown_converges
ROADMAP-17.3.3 -> RFC-5.3 -> INV-2 -> EP-M2 -> tests/server_harness_lifecycle_props.rs::shutdown_then_drop_converges
ROADMAP-17.3.3 -> RFC-5.3 -> INV-3 -> EP-M3 -> crates/wireframe-verification/tests/server_harness_lifecycle.rs
ROADMAP-17.3.3 -> RFC-5.3 -> INV-4 -> EP-M3 -> crates/wireframe-verification/tests/server_harness_lifecycle.rs
ROADMAP-17.3.3 -> RFC-6   -> INV-5 -> EP-M2 -> tests/server_harness_lifecycle_props.rs::parallel_handles_are_independent
ROADMAP-17.3.3 -> RFC-5.5 -> EP-M4 -> tests/client_pair_harness.rs (unchanged, still green)
ROADMAP-17.3.3 -> RFC-5.5 -> EP-M4 -> tests/client_pair_harness.rs::startup_failure_is_reported_as_msg
ROADMAP-17.3.3 -> RFC-5.6 -> EP-M1 -> wireframe_testing/src/server_harness/
```

RFC sections deliberately **not** discharged here, with their owning roadmap
item:

- RFC §5.4 (`spawn_wireframe_server_and_connect`) → roadmap 17.3.4.
- RFC §7.2 (protocol-generic proof) → roadmap 17.3.4.
- RFC §7.3 (`trybuild` coverage) → roadmap 17.3.4.
- RFC §7.4 (companion-crate quality gates, issue #578) → roadmap 17.3.5.

## Constraints

Hard invariants. Violating one requires escalation, not a workaround.

1. `spawn_wireframe_pair` and `spawn_wireframe_pair_default` keep their exact
   signatures, their return type `TestResult<WireframePair>`, and their
   observable behaviour, including that startup failures surface as
   `TestError::Msg`. `WireframePair` stays concrete; it must not gain a generic
   parameter. This is a genuine external-compatibility obligation: RFC §3.1
   commits to it, and the published `wireframe_testing` 0.3.0 crate is the
   named consumer contract.
2. Do not add default type parameters to the new helpers, and do not narrow any
   bound below the `where` clause quoted in `Context and orientation`. Any
   valid `WireframeServer` configuration must be accepted.
3. `ready_signal` is reserved for the harness. Callers must not be able to set
   it and have the harness silently overwrite it; document the reservation.
4. No source file may exceed 400 lines (`AGENTS.md`). Respect `clippy.toml`:
   cognitive complexity 9, at most 4 function arguments, at most 70 lines per
   function, nesting depth at most 4.
5. Test-only lifecycle API stays in `wireframe_testing`. Do not move it into
   the root crate's public surface, and do not modify `src/server/` or
   `src/client/` behaviour.
6. Do not touch the Makefile or CI targets that gate `wireframe_testing`; that
   is roadmap 17.3.5. The one Makefile change this plan does make is an
   additive `test-verification` target plus its CI step, needed so the
   bounded-model evidence in EP-M3 is reproducible; see `Decision log`.
7. Every new module begins with a `//!` module comment. Every new public item
   carries a `///` Rustdoc comment with `# Errors` where it returns `Result`
   and a runnable or justified `no_run` `# Examples` block.
8. All prose, comments, and identifiers use en-GB-oxendict spelling, except
   where an external API name forces otherwise.
9. Red-Green-Refactor. Every behavioural change lands with its test written
   first and observed failing for the intended reason.
10. No new third-party dependency in the root package. `proptest`,
    `googletest`, `pretty_assertions`, `rstest`, and `rstest-bdd` are already
    dev-dependencies; `stateright` is already a dependency of
    `crates/wireframe-verification`.

## Tolerances (exception triggers)

Stop and escalate when any of these is reached. Do not work around them.

- **Scope**: more than 14 files changed, or more than 1 200 net added lines
  across the whole plan.
- **Interface**: the public surface must grow beyond the items listed in
  `Interfaces and dependencies`, or any existing public signature in
  `wireframe`, `wireframe_testing`, or `wireframe-verification` must change.
- **Dependencies**: any new entry in any `[dependencies]` or
  `[dev-dependencies]` table.
- **Constraint 1**: any change, however small, to the observable behaviour of
  `spawn_wireframe_pair` or `spawn_wireframe_pair_default`.
- **Iterations**: a single test still failing after four corrective attempts.
- **Flakiness**: any new test failing intermittently more than once in ten
  consecutive runs of `make test`. Loopback port exhaustion and timing
  sensitivity are the expected causes; reduce generated case counts rather than
  adding sleeps.
- **Model budget**: the Stateright check in EP-M3 exceeding
  `VerificationBounds::default()` (depth 8, 5 000 states) or taking longer than
  60 seconds.
- **Time**: any milestone exceeding four hours of wall-clock work.
- **Ambiguity**: two readings of RFC 0001 that would produce materially
  different public API. Present both with trade-offs.

## Risks

- Risk: readiness fires when accept-loop workers have been *spawned*, not when
  `accept()` is actually being polled (`src/server/runtime.rs:165`). A client
  that connects immediately after readiness may therefore race the first
  `accept()`.
  Severity: medium. Likelihood: medium.
  Mitigation: this is pre-existing behaviour that the pair harness already
  tolerates in practice, because the kernel queues the connection in the
  listen backlog once the listener is bound — and the listener is bound before
  the task is spawned. Do not add a sleep. If a connect-after-ready flake does
  appear, record it in `Surprises & discoveries` and escalate rather than
  papering over it; the correct fix is upstream in `src/server/runtime.rs`.

- Risk: `tokio::time::error::Elapsed`, `tokio::task::JoinError`, and
  `wireframe::server::ServerError` are none of them `Clone`, so a terminal
  failure cannot be returned verbatim from a second `shutdown()` call.
  Severity: medium. Likelihood: certain.
  Mitigation: the design records a `Copy` classification (`ShutdownOutcome`)
  alongside the retained failure text, returns the genuine typed error on the
  first call, and returns a clearly-marked replay `TestError::Msg` afterwards.
  See `Decision log` entry D3; this is a proposed clarification to RFC §5.3 and
  must be accepted before EP-M1 completes.

- Risk: property tests that spin a Tokio runtime and real loopback sockets per
  generated case are slow and can exhaust file descriptors.
  Severity: medium. Likelihood: medium.
  Mitigation: cap generated cases at 16 per property, cap generated handle
  pools at 4, build one `current_thread` runtime per case, and assert every
  handle is stopped before the case returns. Follow the existing in-repo
  idiom at `tests/advanced/interaction_fuzz.rs:109`.

- Risk: `proptest!` does not accept `async fn`, and `prop_assert!` returns
  early with `Err(TestCaseError)`.
  Severity: low. Likelihood: certain.
  Mitigation: use the documented workaround — an ordinary `#[test] fn` body
  that builds a runtime and calls `block_on` on an `async` block returning
  `Result<(), TestCaseError>`, then applies `?`. Upstream issue
  `proptest-rs/proptest#179` confirms there is no better option without adding
  a dependency, which Constraint 10 forbids.

- Risk: the tight `clippy.toml` thresholds (70 lines per function, cognitive
  complexity 9, four arguments) make the natural shape of a lifecycle function
  unlintable.
  Severity: medium. Likelihood: high.
  Mitigation: plan the decomposition up front — a separate module for the
  handle, the state machine, the bounded-join helper, and the spawn entry
  points — rather than discovering it during lint fixes. Group timeouts into
  `ServerHarnessOptions` so no function takes more than three arguments.

- Risk: `crates/wireframe-verification` is not run by any Makefile or CI
  target today, so a model added there would be unexecuted evidence.
  Severity: high. Likelihood: certain.
  Mitigation: add the additive `test-verification` target and CI step
  (Constraint 6). If the reviewer rejects that addition, EP-M3 must be dropped
  and its invariants downgraded to proptest-only coverage, with the residual
  gap recorded here.

- Risk: `wireframe_testing`'s existing doctests do not compile (issue #578), so
  `cargo test -p wireframe_testing --doc` fails wholesale and cannot confirm
  that *new* doctests are sound.
  Severity: low. Likelihood: certain.
  Mitigation: validate the new module's doctests in isolation with a filtered
  invocation, `cargo test -p wireframe_testing --doc --all-features
  server_harness`, and record the transcript. Repairing #578 belongs to 17.3.5.

- Risk: `tests/advanced/interaction_fuzz.rs` is referenced by no `[[test]]`
  target and matches no Cargo auto-discovery rule, so the async-proptest
  precedent it provides is itself never compiled.
  Severity: low. Likelihood: certain.
  Mitigation: treat it as a style reference only; verify the idiom compiles in
  the new, properly wired `tests/server_harness_lifecycle_props.rs` target.
  Report the dead target in `Surprises & discoveries`; fixing it is outwith
  this plan.

## Verification plan

### Axioms (assumed, not verified here)

- **AXIOM-1**: Tokio's `oneshot` channel delivers at most one value, and
  `JoinHandle::await` resolves exactly once with either the task's output or a
  `JoinError`. Third-party contract; not re-verified.
- **AXIOM-2**: `tokio::time::timeout` resolves with `Err(Elapsed)` no earlier than
  the supplied duration when the inner future has not completed.
- **AXIOM-3**: `WireframeServer::run_with_shutdown` returns once its shutdown
  future resolves and its worker tracker drains, and fires `ready_tx` exactly
  once before awaiting shutdown (`src/server/runtime.rs:165-188`). This is a
  repository-owned interface, so EP-M1's rstest cases exercise it against the
  real server rather than a stub.
- **AXIOM-4**: `std::net::TcpListener::bind("localhost:0")` yields a listener
  on a free ephemeral loopback port, and handing it to `bind_existing_listener`
  transfers ownership without releasing the port. This is the property that
  eliminates the classic reserve-then-release race; EP-M1 exercises it directly
  by asserting the address is preserved.
- **AXIOM-5**: `tokio::runtime::Handle::try_current()` returns `Err` exactly when
  no runtime is entered on the current thread.

### Invariants and lemmas

**INV-1 — single consumption.** Across the entire life of one
`RunningWireframeServer`, the shutdown signal is sent at most once and the
server task is joined-or-aborted at most once. After the handle reaches a
terminal state it holds neither a shutdown sender nor a join handle.

- Method: property test over generated operation traces, plus an exhaustive
  bounded state-machine model check.
- Rationale: the statement quantifies over sequences of operations, not over
  data, so example tests can only sample it. The model check makes the
  quantification exhaustive within bounds; the property test keeps the real
  implementation honest against the model.
- Domain: property test generates a call count in `1..=4` for `shutdown`, each
  followed by an outcome assertion. Model check explores every interleaving of
  `Shutdown`, `CancelShutdown`, and `Drop` up to depth 8.
- Artefact: `tests/server_harness_lifecycle_props.rs::repeated_shutdown_converges`
  and `crates/wireframe-verification/src/lifecycle_model/properties.rs`.
- Evidence: `cargo test --all-targets --all-features` and `make
  test-verification`. Before EP-M1 the property test does not compile because
  the type does not exist; that is the red state.
- Non-vacuity: the generated count range excludes zero, so at least one
  `shutdown` always runs and the antecedent is inhabited. A negative control
  removes the `state.take()` in the `Live` arm so the sender is re-sent; the
  property must then fail with a "signal sent twice" counter-example. Record
  that counter-example transcript in `Artefacts and notes`.

**INV-2 — terminal convergence.** Any finite sequence of `shutdown` calls
followed by `Drop` leaves the same terminal state as a single successful
`shutdown` followed by `Drop`: task joined or aborted, listener released, no
orphaned task.

- Method: property test over generated traces ending in `Drop`; model check for
  exhaustiveness.
- Rationale: as INV-1; this is the ordering-sensitive half of idempotence.
- Domain: property test generates `0..=3` `shutdown` calls before dropping, so
  both "drop after shutdown" and "drop without shutdown" are covered.
- Artefact: `tests/server_harness_lifecycle_props.rs::shutdown_then_drop_converges`.
- Evidence: as INV-1.
- Non-vacuity: the generator includes zero so the drop-without-shutdown class
  is reached; assert with a classification counter that both classes occur
  across the run. Observing terminal state after `Drop` uses an external
  witness — attempting a fresh `TcpStream::connect` to the recorded address
  must fail — because the handle itself is gone. Negative control: make `Drop`
  skip the signal when the state is `Live`; the connect must then still
  succeed and the property must fail.

**INV-3 — no orphaned task.** Every path out of the `Live` state either joins
the server task or aborts it. No path drops the join handle without doing one
of the two.

- Method: exhaustive bounded state-machine model check.
- Rationale: this is a reachability property over a small, closed transition
  system. Enumerating it exhaustively is cheap and is strictly stronger than
  sampling. It cannot be checked directly on the real type, because "the handle
  was dropped without joining" is not observable from inside a test.
- Domain: `crates/wireframe-verification` Stateright model with states `Live`,
  `Signalled`, `Stopped`, actions `Shutdown`, `CancelShutdown`, `Drop`,
  `TaskCompletes`, `TaskPanics`, `GraceElapses`; bounds
  `VerificationBounds::default()` (depth 8, 5 000 states).
- Artefact: `crates/wireframe-verification/src/lifecycle_model/`, checked by
  `crates/wireframe-verification/tests/server_harness_lifecycle.rs`.
- Evidence: `make test-verification`.
- Non-vacuity: assert the checker reports a non-zero explored-state count and
  that each of `Live`, `Signalled`, and `Stopped` is reached; a model whose
  initial state is already terminal would explore one state and must be
  treated as a failure. Negative control: add a transition from `Live` straight
  to `Stopped` that discards the task without joining or aborting; the property
  must report a counter-example trace.

**INV-4 — bounded termination.** From any reachable state, every operation
completes in bounded time: `shutdown` either joins within its timeout or aborts
and returns a timeout failure, and `Drop` returns immediately, delegating to a
detached task that itself terminates within the grace period.

- Method: bounded state-machine model check for the transition argument, plus
  one rstest example per timeout path using an injected zero-or-tiny timeout.
- Rationale: this is the property that distinguishes this harness from
  `WireframePair`'s unbounded join, and it is the reason the suite cannot hang.
  A liveness argument over transitions needs the model; the constant-selection
  logic needs a real-clock example.
- Domain: model — every state has at least one enabled action leading towards
  `Stopped`, and `Stopped` is absorbing. Examples —
  `ServerHarnessOptions::default().with_shutdown_timeout(Duration::from_millis(1))`
  against a task that has been made to outlive the bound.
- Artefact: `crates/wireframe-verification/src/lifecycle_model/properties.rs`
  and `tests/server_harness_lifecycle.rs::shutdown_timeout_is_reported`.
- Evidence: `make test-verification` and `make test`.
- Non-vacuity: the timeout example must actually time out — assert
  `handle.shutdown_outcome() == Some(ShutdownOutcome::TimedOut)` and not merely
  that an error was returned. Negative control: raise the injected timeout to
  five seconds and confirm the same test then reports
  `ShutdownOutcome::Completed`, proving the assertion is sensitive to the bound
  rather than always failing.

**INV-5 — handle isolation.** Concurrently spawned handles bind distinct
listener addresses and share no mutable state; stopping one has no effect on
another.

- Method: property test over a generated pool size.
- Rationale: the property concerns a set of independently generated resources,
  which is a range, not a fixed case. A bounded model would abstract away the
  very thing under test — real OS port allocation.
- Domain: pool size generated in `2..=4`; assert all local addresses are
  pairwise distinct, shut down a generated index, then assert every other
  handle still accepts a connection.
- Artefact: `tests/server_harness_lifecycle_props.rs::parallel_handles_are_independent`.
- Evidence: `make test`.
- Non-vacuity: the shut-down index is generated, so the first, last, and middle
  positions are all reachable; assert the surviving-handle count is at least
  one so the post-condition is never trivially empty. Negative control:
  temporarily give every handle the same listener; the distinct-address
  assertion must fail.

**INV-6 — pair compatibility.** `spawn_wireframe_pair` and
`spawn_wireframe_pair_default` produce the same observable behaviour after the
refactor as before, including the `TestError::Msg` shape of startup failures.

- Method: the existing example-based and behavioural suites, unchanged, plus
  one new pinning test.
- Rationale: this is a finite, enumerable compatibility surface. A property
  test would add nothing; a `trybuild` type-inference pin is genuinely useful
  but belongs to 17.3.4 per RFC §7.3.
- Domain: the four existing cases in `tests/client_pair_harness.rs`, the two
  Gherkin scenarios in `tests/features/client_pair_harness.feature`, the case
  in `wireframe_testing/tests/integration_helpers.rs`, and the new pin.
- Artefact: `tests/client_pair_harness.rs::startup_failure_is_reported_as_msg`.
- Evidence: `make test`, plus `cargo test -p wireframe_testing --all-targets
  --all-features`.
- Non-vacuity: the pin must assert `matches!(err, TestError::Msg(_))` *and*
  that the message names the failing stage; a test that only asserts `is_err()`
  would pass even if the variant silently became `TestError::Timeout`. Negative
  control: change the pair wrapper to propagate the typed variant; the pin must
  fail.

### Methods deliberately not used

- **Kani** is not used. Kani sequentialises concurrency and does not model task
  interleaving (`kani` skill, "What Kani detects and what it does not"). Every
  invariant here is about orderings of concurrent operations, which is exactly
  what Kani cannot see. The repository's tool for orderings is Stateright, and
  it is already installed in `crates/wireframe-verification`. Using Kani would
  produce a passing harness that proves nothing about the property of interest
  — the definition of vacuous verification.
- **Verus** is not used. Verus is for unbounded induction over pure functions
  and for contractual lemmas. This change introduces no such lemma: the state
  machine is finite and closed, so bounded exhaustive checking is not an
  approximation of the truth here but is the truth. Reaching for a deductive
  prover would be ceremony, and the RFC (§7.5) says as much.
- **`insta` snapshots** are not used. `insta` is not a dependency of this
  repository, and there is no multivariant output format in this change; the
  only formatted output is error text, which RFC §7.3 explicitly says to assert
  semantically with `googletest::contains_substring` rather than by
  whole-string match.

### Residual gaps

- The bounded model abstracts the OS. It proves the handle's transition system
  is sound; it does not prove Tokio implements AXIOM-1 or that the kernel implements
  AXIOM-4. Those remain axioms, exercised by the real-socket rstest cases.
- Model depth 8 admits traces of at most eight actions. Sequences longer than
  that are covered only by the property tests, whose generated call counts stop
  at four. Longer adversarial sequences are not covered; record this and revisit
  if a defect escapes.
- The connect-then-shutdown schedule named in RFC §7.5 bullet three is covered
  by INV-2's trace generator extended with a `Connect` operation; it is not a
  separate invariant.

## Interfaces and dependencies

At the end of EP-M4 the following must exist, exactly. Nothing else may be
added to the public surface without breaching the interface tolerance.

In `wireframe_testing/src/server_harness/mod.rs`:

```rust
//! Protocol-agnostic server lifecycle harness.

pub use self::{
    handle::{RunningWireframeServer, ShutdownOutcome},
    lifecycle::{spawn_wireframe_server, spawn_wireframe_server_on, spawn_wireframe_server_with_options},
    options::{LifecycleStage, ServerHarnessOptions},
};
```

In `wireframe_testing/src/server_harness/options.rs`:

```rust
/// Stage of the harness lifecycle that a failure occurred in.
///
/// The stage name appears in every failure message so a caller can tell a
/// bind failure from a readiness timeout without matching on private state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum LifecycleStage { Bind, Readiness, Shutdown, Join, AbortAfterTimeout }

/// Timeout policy for a [`RunningWireframeServer`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ServerHarnessOptions { /* private fields */ }

impl Default for ServerHarnessOptions {
    /// Five seconds for readiness, five seconds for the shutdown join, and a
    /// 100-millisecond grace period for defensive `Drop` cleanup.
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

In `wireframe_testing/src/server_harness/handle.rs`:

```rust
/// How a [`RunningWireframeServer`] reached its terminal state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ShutdownOutcome { Completed, ServerFailed, TaskFailed, TimedOut }

/// A bound, ready, running Wireframe server owned by a test.
pub struct RunningWireframeServer { /* private fields */ }

impl RunningWireframeServer {
    /// The loopback address the server is accepting on.
    #[must_use] pub const fn local_addr(&self) -> SocketAddr;
    /// `true` once the handle has reached a terminal state.
    #[must_use] pub fn is_stopped(&self) -> bool;
    /// The terminal classification, or `None` while the handle is still live.
    #[must_use] pub fn shutdown_outcome(&self) -> Option<ShutdownOutcome>;
    /// Stop the server and join its task.
    ///
    /// # Errors
    ///
    /// Returns `TestError::Server` if the server task returned an error,
    /// `TestError::Join` if it panicked or was cancelled, and
    /// `TestError::Timeout` if the join exceeded the configured bound. A call
    /// made after a failed shutdown replays the retained diagnosis as
    /// `TestError::Msg`; use [`Self::shutdown_outcome`] to match on the
    /// classification instead of parsing that text.
    pub async fn shutdown(&mut self) -> TestResult<()>;
}

impl Debug for RunningWireframeServer { /* redacts live resources */ }
impl Drop for RunningWireframeServer { /* bounded, non-blocking safety net */ }
```

In `wireframe_testing/src/server_harness/lifecycle.rs`:

```rust
pub async fn spawn_wireframe_server<F, T, Ser, Ctx, E, Codec>(
    server: WireframeServer<F, T, Unbound, Ser, Ctx, E, Codec>,
) -> TestResult<RunningWireframeServer>
where /* the verbatim clause from Context and orientation */;

pub async fn spawn_wireframe_server_on<F, T, Ser, Ctx, E, Codec>(
    server: WireframeServer<F, T, Unbound, Ser, Ctx, E, Codec>,
    listener: std::net::TcpListener,
) -> TestResult<RunningWireframeServer>
where /* as above */;

pub async fn spawn_wireframe_server_with_options<F, T, Ser, Ctx, E, Codec>(
    server: WireframeServer<F, T, Unbound, Ser, Ctx, E, Codec>,
    listener: Option<std::net::TcpListener>,
    options: ServerHarnessOptions,
) -> TestResult<RunningWireframeServer>
where /* as above */;
```

Re-exported from `wireframe_testing/src/lib.rs` alongside the existing
`client_pair` re-exports, keeping the crate's flat import style.

In `crates/wireframe-verification/src/lifecycle_model/`: a Stateright `Model`
following the shape already used by `connection_model/` — `action.rs`,
`state.rs`, `model.rs`, `properties.rs`, `mod.rs` — checked through the
existing `harness::assert_model_properties`.

No new dependency in any manifest.

## Milestones and plateaus

### EP-M0 — orientation and red scaffolding

- Outcome: the repository is unchanged except for this plan, and the reader has
  confirmed the baseline is green.
- Requirements: none discharged; establishes the baseline for every later
  conformance check.
- Acceptance evidence: `make check-fmt`, `make lint`, and `make test` all pass
  on an unmodified tree, transcripts captured under `/tmp`.
- Conformance check: confirm RFC 0001 has not been amended since this plan was
  written; if it has, re-read §5.2, §5.3, §5.5, and §5.6 before continuing.
- Recovery: nothing to undo.
- Remaining gaps: everything.
- Compatibility decision: none required.

### EP-M1 — `RunningWireframeServer` exists and is proven by example

- Outcome: `wireframe_testing::server_harness` exists with the handle, the
  options type, and the three spawn helpers. `client_pair.rs` is **untouched**.
  The repository has two independent lifecycle implementations at this point,
  which is intentional and matches RFC §8 stage 1; it is not compatibility
  machinery, because both are live implementations and one is removed in EP-M4.
- Requirements: RFC §5.2, §5.3, §5.6, §6, §7.1.
- Acceptance evidence: `tests/server_harness_lifecycle.rs` passes with cases
  for round trip, caller-supplied listener address preservation, startup
  failure before readiness, readiness timeout cleanup, idempotent shutdown,
  cancelled-shutdown-then-drop, drop inside a runtime, drop outside a runtime,
  and shutdown timeout classification. The BDD feature
  `tests/features/server_harness_lifecycle.feature` passes.
- Conformance check: the `where` clause matches the quoted RFC bounds exactly;
  no default type parameter was introduced; no file exceeds 400 lines; `make
  lint` is clean including whitaker.
- Recovery: the whole milestone is additive; `git revert` of its commits
  restores EP-M0.
- Remaining gaps: `WireframePair` still has its own copy of the machinery;
  no property or model coverage yet.
- Compatibility decision: none; the new module has no consumers yet.

### EP-M2 — invariants hold across generated traces

- Outcome: the property suite exists and passes, and each property has been
  seen to fail against its negative control.
- Requirements: RFC §7.5; INV-1, INV-2, INV-5.
- Acceptance evidence: `tests/server_harness_lifecycle_props.rs` passes under
  `make test`; the three recorded negative-control transcripts show the
  expected counter-examples.
- Conformance check: generated case counts stay within the flakiness tolerance;
  ten consecutive `make test` runs show no intermittent failure.
- Recovery: additive; revert the property test file.
- Remaining gaps: INV-3 and INV-4 are still only sampled, not exhausted.
- Compatibility decision: none.

### EP-M3 — the lifecycle state machine is exhaustively checked

- Outcome: `crates/wireframe-verification/src/lifecycle_model/` models the
  handle's transition system and `make test-verification` checks it.
- Requirements: INV-3, INV-4.
- Acceptance evidence: `make test-verification` passes and reports a non-zero
  explored-state count with all three states reached; the negative-control
  mutation produces a counter-example trace, recorded in `Artefacts and notes`.
- Conformance check: the model's transitions correspond one-for-one with the
  arms of the real `HarnessState` transition function; if the implementation
  gains a state, the model must gain it too. Note the correspondence in a
  comment on both sides.
- Recovery: additive; revert the model crate changes and the Makefile target.
- Remaining gaps: `WireframePair` still duplicated.
- Compatibility decision: none.

### EP-M4 — the pair façade delegates, and the duplication is gone

- Outcome: `client_pair.rs` builds its default server, calls
  `spawn_wireframe_server_with_options` with an unbounded shutdown timeout, and
  keeps its own client-close-then-shutdown ordering. `spawn_bounded_shutdown`
  and `PendingServer` no longer exist in `client_pair.rs`; the single
  implementation lives in `server_harness`.
- Requirements: RFC §5.5, §5.6; INV-6.
- Acceptance evidence: every pre-existing pair test passes unmodified, the new
  `startup_failure_is_reported_as_msg` pin passes, and `cargo test -p
  wireframe_testing --all-targets --all-features` passes.
- Conformance check: `WireframePair` is still concrete; its two constructors
  still have byte-identical signatures (confirm with `git diff` on the
  signature lines); the unbounded join is preserved.
- Recovery: this milestone changes existing behaviour, so it is the one to
  revert first if a regression appears. `git revert` restores the EP-M3
  plateau, in which both implementations coexist and everything is green.
- Remaining gaps: documentation.
- Compatibility decision: **required**. The named consumer is the published
  `wireframe_testing` 0.3.0 crate and any downstream test suite using
  `spawn_wireframe_pair`. RFC §3.1 and §5.5 commit to source compatibility, so
  the façade is a real requirement and not compatibility theatre. Note that the
  façade is *retained existing API*, not a new shim.

### EP-M5 — documentation, RFC amendment, and roadmap

- Outcome: the design record matches the code, and roadmap 17.3.3 is ticked.
- Requirements: `AGENTS.md` documentation obligations.
- Acceptance evidence: `make markdownlint`, `make nixie`, and `make
  doctest-benchmark` pass; `docs/contents.md` lists nothing new unless an ADR
  was added; the roadmap checkbox is `[x]`.
- Conformance check: every deviation recorded in `Decision log` has a
  corresponding amendment in RFC 0001; the plan status moves to `COMPLETE` only
  once none remains unaccepted.
- Recovery: documentation-only; revert freely.
- Remaining gaps: 17.3.4 and 17.3.5.
- Compatibility decision: none.

## Plan of work

### Stage A — understand and propose (no code changes)

Load the skills listed in `Context and orientation`. Run `leta workspace add .`.
Read `wireframe_testing/src/client_pair.rs` in full — all 379 lines — and
`src/server/runtime.rs:29-198`. Run `make check-fmt`, `make lint`, and `make
test` to establish a green baseline and keep the transcripts.

Confirm the three design points that this plan resolves beyond the RFC's
literal text, and escalate if any is unacceptable: the `ServerHarnessOptions`
type (D2), the `ShutdownOutcome` replay contract (D3), and the additive
`test-verification` Makefile target (D5). All three are in `Decision log`.

Stage A ends when the baseline is green and the three decisions are accepted.

### Stage B — red tests

Write, in this order, and observe each failing for the intended reason before
writing any production code.

First, `tests/server_harness_lifecycle.rs`. It will not compile, because
`wireframe_testing::server_harness` does not exist. That is the intended red
state for a new module; note the exact compiler error in the transcript so a
later reader can distinguish "not written yet" from "written and broken".

Cases, one `#[rstest]` per bullet, all `#[tokio::test]`-driven except the two
that must run outside a runtime:

1. `spawn_then_connect_round_trip` — build a default echo server with
   `wireframe_testing::integration_helpers::echo_app_factory`, spawn it, connect
   a plain `tokio::net::TcpStream`, and assert the harness reports a loopback
   address in `127.0.0.0/8` with a non-zero port.
2. `caller_listener_keeps_its_address` — reserve a listener with
   `unused_listener()`, record its `local_addr()`, pass it to
   `spawn_wireframe_server_on`, and assert `handle.local_addr()` equals the
   recorded address.
3. `startup_failure_before_readiness_is_reported` — construct a server whose
   app factory returns an error, and assert the spawn fails. Assert the typed
   variant and, with `googletest::matchers::contains_substring`, that the
   message names the stage.
4. `readiness_timeout_cleans_up` — use
   `ServerHarnessOptions::default().with_readiness_timeout(Duration::from_millis(1))`
   against a readiness receiver that cannot fire. Because a real
   `WireframeServer` always signals promptly, drive this case through the
   internal seam instead: expose `pub(crate) async fn await_readiness` in
   `lifecycle.rs` and unit-test it from a `#[cfg(test)]` module inside
   `wireframe_testing`. Record in `Surprises & discoveries` that this one case
   is not visible to `make test` until 17.3.5, and run it explicitly with
   `cargo test -p wireframe_testing`.
5. `shutdown_is_idempotent` — call `shutdown().await` twice; assert the first
   returns `Ok(())`, the second returns `Ok(())`, and `shutdown_outcome()` is
   `Some(ShutdownOutcome::Completed)` after both.
6. `cancelled_shutdown_then_drop_cleans_up` — obtain the shutdown future, poll
   it exactly once with `futures::poll!` on a pinned future, drop the future,
   then drop the handle, then assert a fresh `TcpStream::connect` to the
   recorded address fails.
7. `drop_inside_runtime_returns_immediately` — measure that `drop(handle)`
   returns in well under the grace period, then await a short interval and
   assert the address is no longer connectable.
8. `drop_outside_runtime_aborts` — construct the handle inside a runtime,
   move it out via `std::thread::spawn`, and drop it there; assert the test
   process does not hang. Use `#[test]`, not `#[tokio::test]`.
9. `shutdown_timeout_is_reported` — with a one-millisecond shutdown timeout
   against a server made to outlast it, assert `shutdown_outcome()` is
   `Some(ShutdownOutcome::TimedOut)` and the error is `TestError::Timeout`.
   Then, as the non-vacuity control described in INV-4, re-run the same body
   with a five-second timeout and assert `Completed`. Express the pair as one
   `#[rstest]` with two `#[case]`s.

Second, the behavioural specification. Create
`tests/features/server_harness_lifecycle.feature` with exactly this content:

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
    Then a client can no longer open a connection to that address
```

Add `tests/fixtures/server_harness_lifecycle.rs` holding a
`ServerHarnessLifecycleWorld` that owns a `tokio::runtime::Runtime`, an
`Option<RunningWireframeServer>`, the recorded `SocketAddr`, and the recorded
stop results. Follow the shape of
`tests/fixtures/client_pair_harness.rs` exactly, including its `Drop` that
blocks on cleanup. Step definitions in
`tests/steps/server_harness_lifecycle_steps.rs` are synchronous and delegate to
world methods, per `docs/rstest-bdd-users-guide.md`. Scenario bindings go in
`tests/scenarios/server_harness_lifecycle_scenarios.rs`. Register all four new
files in `tests/fixtures/mod.rs`, `tests/steps/mod.rs`, and
`tests/scenarios/mod.rs`. The fixture parameter name in every step must be
`server_harness_lifecycle_world`, matching the `#[fixture]` function name; a
mismatch is a compile-time error under
`strict-compile-time-validation`.

Third, `tests/server_harness_lifecycle_props.rs`, with the three properties
from INV-1, INV-2, and INV-5. Use the in-repo async-proptest idiom:

```rust
proptest! {
    #![proptest_config(ProptestConfig { cases: 16, ..ProptestConfig::default() })]

    #[test]
    fn repeated_shutdown_converges(calls in 1usize..=4) {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| TestCaseError::fail(e.to_string()))?;
        runtime.block_on(async move {
            // ... spawn, call shutdown `calls` times, assert with prop_assert! ...
            Ok(())
        })?;
    }
}
```

Note the shape: `proptest!` cannot take an `async fn` (upstream issue
`proptest-rs/proptest#179`), so the body is synchronous, builds a
`current_thread` runtime, and `block_on`s an `async` block that returns
`Result<(), TestCaseError>` so `prop_assert!` works and `?` propagates. Do not
use `.unwrap()` or `.expect()`; the whitaker lint suite forbids `expect` outwith
test contexts and clippy denies `unwrap_used`.

Fourth, `tests/client_pair_harness.rs::startup_failure_is_reported_as_msg`,
which pins INV-6. It must pass *before* EP-M4 as well as after, since it
describes today's behaviour.

Stage B ends when every new test fails or fails to compile for a reason you
have written down, and no pre-existing test has changed.

### Stage C — implementation

Create `wireframe_testing/src/server_harness/` with four files, each under 400
lines and each function under 70 lines:

`options.rs` holds `LifecycleStage`, `ServerHarnessOptions`, its `Default`, its
consuming `with_*` builders, its getters, and a small
`fn stage_message(stage: LifecycleStage, detail: &str) -> String` that produces
the stage-labelled text RFC §6 requires. Keeping message construction in one
place is what makes the `contains_substring` assertions stable.

`handle.rs` holds `RunningWireframeServer`, the retained failure text, `Debug`,
`Drop`, the accessors, and the private state enum:

```rust
enum HarnessState {
    Live { shutdown_tx: oneshot::Sender<()>, task: JoinHandle<Result<(), ServerError>> },
    Signalled { task: JoinHandle<Result<(), ServerError>> },
    Stopped(ShutdownOutcome),
}
```

The state machine
is the heart of this change; write it as an explicit `match` over the taken
state so that every arm is visible in one place and so it corresponds one-for-one
with the Stateright model. Transitions:

- `shutdown` on `Live`: take the state, send the signal, install `Signalled`,
  then join. Installing `Signalled` *before* awaiting is what makes a cancelled
  shutdown recoverable, exactly as `client_pair.rs:118` explains for the pair.
- `shutdown` on `Signalled`: join without re-signalling.
- `shutdown` on `Stopped(Completed)`: return `Ok(())`.
- `shutdown` on `Stopped(other)`: return the replayed `TestError::Msg`.
- `Drop` on `Live`: send the signal, then delegate to the bounded cleanup.
- `Drop` on `Signalled`: delegate to the bounded cleanup.
- `Drop` on `Stopped`: nothing.

`shutdown.rs` holds the single copy of the bounded-join and detached-cleanup
machinery lifted from `client_pair.rs:183-203`, generalized to take a
`Duration` and an optional bound, plus the mapping from a
`Result<Result<(), ServerError>, JoinError>` and a timeout into a
`(ShutdownOutcome, Option<TestError>)` pair.

`lifecycle.rs` holds the three spawn entry points and the internal
`await_readiness` seam. `spawn_wireframe_server` calls `unused_listener()` and
delegates; `spawn_wireframe_server_on` delegates with default options;
`spawn_wireframe_server_with_options` does the work in this order: bind the
listener, read `local_addr` (mapping `None` to a `Bind`-stage failure), create
the readiness and shutdown channels, install the readiness sender, spawn the
task, guard the spawned task with the same RAII pattern `PendingServer` uses so
that an early return cannot orphan it, await readiness under the bound, and
finally construct the handle.

`mod.rs` holds the module documentation and the re-exports. Its module comment
must state the reservation of `ready_signal` (Constraint 3) and must carry one
runnable `# Examples` doctest showing spawn, connect, and shutdown. Add the
re-exports to `wireframe_testing/src/lib.rs`.

Run the focused tests after each file, not only at the end.

### Stage C2 — the bounded model

Add `crates/wireframe-verification/src/lifecycle_model/` mirroring
`connection_model/`: `action.rs` (the action enum), `state.rs` (the abstract
state plus a "task joined" and "task aborted" ledger), `model.rs` (the
`stateright::Model` impl), `properties.rs` (INV-1, INV-3, INV-4 as
`Property::always` and `Property::eventually` predicates), and `mod.rs`. Wire
the check through `wireframe_verification::harness::assert_model_properties`
in `crates/wireframe-verification/tests/server_harness_lifecycle.rs`, following
`crates/wireframe-verification/tests/connection_actor.rs`.

Add to the Makefile, immediately after the `test-doc` target:

```make
test-verification: ## Run the bounded formal models
	RUSTFLAGS="-D warnings" $(CARGO) test -p wireframe-verification --all-features $(BUILD_JOBS)
```

Add a matching step to the `build-test` job in `.github/workflows/ci.yml`,
immediately after the existing `Lint` step:

```yaml
      - name: Bounded model checks
        run: make test-verification
```

### Stage D — the pair delegates

Rewrite `spawn_wireframe_pair`'s body to build the default server exactly as it
does today, call
`spawn_wireframe_server_with_options(server, Some(listener), ServerHarnessOptions::default().with_shutdown_timeout(None))`,
then connect the default client and store the resulting
`RunningWireframeServer` inside `Running`. Delete `spawn_bounded_shutdown` and
`PendingServer` from `client_pair.rs` and use the `server_harness` copies.
`WireframePair::shutdown` keeps closing the client first and keeps its
unbounded join, which the `None` shutdown timeout preserves. Re-wrap the
harness's typed startup failures into `TestError::Msg` so INV-6 holds.

Run the full pre-existing suite. Any change in a pre-existing test's expected
output is a Constraint 1 breach: stop and escalate.

### Stage E — documentation

1. Add a `## Protocol-agnostic server lifecycle harness` section to
   `docs/wireframe-testing-crate.md`, immediately after
   `## In-process server/client pair harness`, with subsections mirroring the
   existing one: `### Public API`, `### Lifecycle`, `### Timeout policy`,
   `### Usage`, `### Rationale`. Update the `## Crate layout` bullet list to
   mention `src/server_harness/`.
2. Add a short subsection to `docs/users-guide.md` under `## Running servers`
   pointing consumers at the harness for their own test suites, and stating the
   `ready_signal` reservation and the difference between explicit `shutdown`
   and defensive `Drop`.
3. Add a subsection to `docs/developers-guide.md` under `## Test infrastructure
   and framework` recording the internal conventions this work establishes: the
   async-proptest idiom, the requirement that the Stateright model track the
   real state machine, and the new `make test-verification` target. Also add it
   to `## Quality gates`.
4. Amend `docs/rfcs/0001-protocol-agnostic-test-harness-lifecycle.md`: resolve
   open question §11.1 in favour of `ServerHarnessOptions`, record the
   `ShutdownOutcome` replay contract in §5.3, add `handle.rs`, `options.rs`,
   and `shutdown.rs` to the §5.6 module list, and append a revision note. Leave
   §11.2 open; it belongs to 17.3.4.
5. Decide whether the replay contract also warrants an ADR. The default answer
   is no, because the RFC is the governing artefact for this feature and
   amending it is clearer than an ADR that qualifies it. Record whichever way
   the design review lands in `Decision log`. If an ADR is added, it is
   `docs/adr-011-server-lifecycle-shutdown-outcome-contract.md`, follows the
   Status/Date/Context template in `docs/documentation-style-guide.md`, and
   gains a line in `docs/contents.md`.
6. Tick roadmap item 17.3.3 from `- [ ]` to `- [x]`, preserving its inline
   links verbatim. Use `mapsplice` per `docs/developers-guide.md` §"Roadmap
   editing with mapsplice"; the roadmap must not use footnote references.
7. Add a `CHANGELOG.md` entry if the repository's changelog covers unreleased
   test-support changes; check the existing file's conventions first.

## Concrete steps

Run everything from the repository root. Every gate is piped through `tee` so
the full output survives truncation.

```bash
export LOG_PREFIX="/tmp/17-3-3-$(git branch --show-current)"
set -o pipefail && make check-fmt 2>&1 | tee "${LOG_PREFIX}-check-fmt.log"
set -o pipefail && make lint      2>&1 | tee "${LOG_PREFIX}-lint.log"
set -o pipefail && make test      2>&1 | tee "${LOG_PREFIX}-test.log"
```

Expected on a green baseline, at the tail of the test log:

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
set -o pipefail && cargo test -p wireframe_testing --all-targets --all-features 2>&1 \
  | tee "${LOG_PREFIX}-testing-crate.log"
set -o pipefail && cargo test -p wireframe_testing --doc --all-features server_harness 2>&1 \
  | tee "${LOG_PREFIX}-testing-doc.log"
```

The second command is filtered to `server_harness` on purpose: the crate's
pre-existing doctests do not compile (issue #578), so an unfiltered run cannot
tell you whether *your* doctest is sound. Record both the filtered pass and the
unfiltered pre-existing failure.

Bounded model check, from EP-M3 onwards:

```bash
set -o pipefail && make test-verification 2>&1 | tee "${LOG_PREFIX}-verification.log"
```

Documentation gates, at EP-M5:

```bash
set -o pipefail && make markdownlint      2>&1 | tee "${LOG_PREFIX}-markdownlint.log"
set -o pipefail && make nixie             2>&1 | tee "${LOG_PREFIX}-nixie.log"
set -o pipefail && make doctest-benchmark 2>&1 | tee "${LOG_PREFIX}-doctest-bench.log"
```

Flakiness check, once at the end of EP-M2:

```bash
for i in $(seq 1 10); do
  set -o pipefail && cargo test --test server_harness_lifecycle_props --all-features \
    2>&1 | tee "${LOG_PREFIX}-props-run-${i}.log" | tail -3
done
```

Commit after each milestone, and gate each commit with `make check-fmt`, `make
lint`, and `make test` before committing. Delegate full gate runs to the
`scrutineer` sub-agent rather than running them inline, and read the cited log
on failure rather than re-running.

## Validation and acceptance

### Red-Green-Refactor evidence to record

- **Red**: `cargo test --test server_harness_lifecycle --all-features` fails to
  compile with `unresolved import wireframe_testing::server_harness`. Paste the
  first ten lines of that error.
- **Green**: the same command reports `test result: ok` with nine cases passing
  once Stage C lands.
- **Refactor**: after the Stage C tidy-up and again after Stage D, both the
  focused command and `make test` pass.

### BDD evidence

`cargo test --test bdd --all-features server_harness` fails before Stage C with
a scenario-not-found or compile error, and reports three scenarios passing
after.

### Verification evidence

For each of INV-1 through INV-6, record: the command, the passing result, the
negative-control mutation applied, and the counter-example or failure the
control produced. An invariant whose negative control still passes has not been
verified — treat that as a failure and return to `Verification plan`.

For EP-M3, additionally record Stateright's explored-state count and confirm it
is within `VerificationBounds::default()`.

### Quality criteria

- Tests: `make test` passes with no new failures and no `#[ignore]`.
  `cargo test -p wireframe_testing --all-targets --all-features` passes.
  `make test-verification` passes.
- Verification: INV-1 through INV-6 all discharged with non-vacuity evidence.
- Lint and format: `make check-fmt` and `make lint` clean, including the
  whitaker Dylint pass. If `no_expect_outside_tests` fires, consult the
  `addressing-whitaker-findings` skill and check whether `main` is red before
  concluding it is your change.
- Documentation: `make markdownlint`, `make nixie`, and `make
  doctest-benchmark` pass.
- Performance: the new tests add no more than 20 seconds to `make test` on the
  reference machine. If they do, cut generated case counts.
- Security: none applicable; this is test-only scaffolding on loopback.

## Idempotence and recovery

Every stage is re-runnable. The gates are pure reads. The only destructive
operations are file edits, all of which are under version control.

EP-M1 through EP-M3 are purely additive: reverting their commits restores the
previous plateau with everything green. EP-M4 is the only milestone that
changes existing behaviour; if a regression surfaces later, revert EP-M4 alone
and the repository returns to the EP-M3 plateau, in which the new harness and
the old pair implementation coexist and both work. That coexistence is a
deliberate, temporary duplication of live code, not a compatibility shim.

If a test hangs, it is almost certainly an unjoined task or an unreleased
listener. Reproduce with `cargo test --test <target> -- --nocapture
--test-threads=1` and inspect which handle never reached `Stopped`. Do not add
a sleep.

Leave no stray processes or bound ports behind. If a run is interrupted, check
with `ss -ltn | grep 127.0.0.1` before re-running.

## Progress

- [ ] EP-M0 Orientation and green baseline.
- [ ] EP-M1 `RunningWireframeServer`, options, spawn helpers, rstest and BDD
      coverage.
- [ ] EP-M2 Property suite for INV-1, INV-2, INV-5, with negative controls.
- [ ] EP-M3 Stateright lifecycle model for INV-3 and INV-4, plus the
      `test-verification` target and CI step.
- [ ] EP-M4 `WireframePair` delegates; duplicated machinery removed; INV-6
      pinned.
- [ ] EP-M5 Design document, users' guide, developers' guide, RFC amendment,
      roadmap tick.

## Surprises & discoveries

- Observation: `tests/advanced/interaction_fuzz.rs` is not wired into any
  `[[test]]` target and does not match Cargo's auto-discovery rules, so the
  repository's only async-proptest example is never compiled.
  Evidence: no reference to `interaction_fuzz` in `Cargo.toml` or anywhere
  under `tests/`; Cargo discovers `tests/*.rs` and `tests/*/main.rs` only.
  Impact: treat that file as a style reference, not as proof the idiom
  compiles. Verify the idiom in the new, properly wired property-test target.

- Observation: `crates/wireframe-verification` is a workspace member but not a
  default member, and no Makefile target or CI job runs it, so the existing
  placeholder Stateright model is unexecuted.
  Evidence: `Cargo.toml:15` `default-members = ["."]`; no `-p
  wireframe-verification` in the Makefile or `.github/workflows/`.
  Impact: motivates the additive `test-verification` target in EP-M3. Without
  it the model would be evidence nobody runs.

- Observation: `docs/repository-layout.md` is referenced by `AGENTS.md` and by
  `docs/documentation-style-guide.md` but does not exist.
  Evidence: the path is absent from the working tree.
  Impact: none for this work; do not attempt to create it. Recorded so a reader
  does not waste time looking.

- Observation: the readiness signal fires after accept-loop worker tasks are
  spawned, not after `accept()` is first polled.
  Evidence: `src/server/runtime.rs:165-188`.
  Impact: shapes the wording of the harness documentation — "ready" means
  "workers launched onto a bound listener", and the listen backlog is what
  makes an immediate connect safe.

## Decision log

- Decision (D1): scope this plan to RFC §5.2, §5.3, §5.5, §5.6, §6, §7.1, and
  §7.5 only. The connector (§5.4), the protocol-generic proof (§7.2), and
  `trybuild` (§7.3) go to roadmap 17.3.4; the companion-crate gates and issue
  #578 (§7.4) go to 17.3.5.
  Rationale: those are the boundaries the roadmap itself draws, and RFC §8
  sequences them the same way. Pulling 17.3.4 forward would make EP-M4's
  compatibility check harder to interpret.
  Date/Author: 2026-08-23, planning agent.

- Decision (D2): introduce a public `ServerHarnessOptions` type in the first
  release, resolving RFC open question §11.1 in the affirmative.
  Rationale: RFC §5.2 permits an options type "if the implementation spike
  proves that fixed defaults are insufficient". They are. With fixed five-second
  timeouts, the readiness-timeout and shutdown-timeout paths named in RFC §7.1
  can only be exercised by five-second sleeps, and the non-vacuity control for
  INV-4 — showing the assertion is sensitive to the bound — is impossible.
  Injectable timeouts also let the `WireframePair` façade express its required
  unbounded join as `with_shutdown_timeout(None)` rather than needing a second
  code path. The type is deliberately three fields and no builder ceremony, per
  the RFC's warning that timeout configuration must not become a second server
  builder.
  Date/Author: 2026-08-23, planning agent. **Requires acceptance before EP-M1.**

- Decision (D3): `shutdown()` returns the genuine typed `TestError` on its
  first failing call and a clearly-marked replay `TestError::Msg` on later
  calls, with a `Copy` `ShutdownOutcome` accessor for matching.
  Rationale: RFC §5.3 asks a repeated call to "report the same timeout failure",
  and RFC §6 asks callers to distinguish failures "without parsing message
  text". Neither `tokio::time::error::Elapsed`, `tokio::task::JoinError`, nor
  `wireframe::server::ServerError` is `Clone` or publicly constructible, so
  returning the identical typed value twice is not expressible in safe Rust.
  The classification accessor satisfies the intent of both sections exactly,
  and is more useful than a reconstructed error because it is `Eq` and
  matchable. The alternative — storing `Arc<TestError>` and changing the return
  type — would breach Constraint 1's spirit and the RFC's own signature.
  Impacts: RFC §5.3 and §6 need an amendment recording the contract; this is a
  proposed deviation under the ExecPlan skill's rules.
  Date/Author: 2026-08-23, planning agent. **Requires acceptance before EP-M1.**

- Decision (D4): split RFC §5.6's `lifecycle.rs` into `lifecycle.rs`,
  `handle.rs`, `shutdown.rs`, and `options.rs`.
  Rationale: `clippy.toml` caps functions at 70 lines and cognitive complexity
  at 9, and `AGENTS.md` caps files at 400. The RFC's two-file split cannot hold
  the state machine, the bounded-join machinery, the three entry points, and
  their doctests inside those limits. The module boundary the RFC actually
  cares about — protocol-neutral lifecycle in `server_harness`, default-client
  façade in `client_pair` — is unchanged.
  Date/Author: 2026-08-23, planning agent.

- Decision (D5): add a `test-verification` Makefile target and a matching CI
  step, even though gate changes are otherwise reserved for 17.3.5.
  Rationale: EP-M3's model is the primary evidence for INV-3 and INV-4. Without
  a target that runs it, that evidence would be unreproducible and would rot,
  exactly as `wireframe_testing`'s doctests did. The change is additive, touches
  a different package from 17.3.5's, and fixes a pre-existing gap in which the
  placeholder connection model is also never executed.
  Alternative considered and rejected: adding `stateright` to the root
  package's dev-dependencies so the model runs under `make test`. That breaches
  Constraint 10 and duplicates the purpose of `crates/wireframe-verification`.
  Alternative considered and rejected: dropping EP-M3 and relying on proptest
  alone. That leaves INV-3, which is not observable from inside a test, with no
  verification at all.
  Date/Author: 2026-08-23, planning agent. **Requires acceptance before EP-M3.**

- Decision (D6): do not use Kani, Verus, or `insta`.
  Rationale: recorded in `Verification plan` under "Methods deliberately not
  used". In summary — Kani sequentialises concurrency and so cannot see the
  ordering properties at issue; Verus addresses unbounded induction, which this
  finite state machine does not need; `insta` is not a dependency and there is
  no multivariant output to snapshot.
  Date/Author: 2026-08-23, planning agent.

- Decision (D7): place the gated behavioural coverage in the root package's
  `tests/` tree rather than in `wireframe_testing/tests/`.
  Rationale: `default-members = ["."]` means the companion crate's own targets
  are invisible to `make test` until 17.3.5. Tests that do not run are not
  evidence. The root package already dev-depends on `wireframe_testing`,
  `proptest`, `googletest`, and `rstest-bdd`, and the existing pair harness sets
  the precedent with `tests/client_pair_harness.rs`. The one exception is the
  `await_readiness` seam test, which needs crate-internal access; it is run
  explicitly and its limited visibility is recorded.
  Date/Author: 2026-08-23, planning agent.

## Outcomes & retrospective

To be completed at EP-M5. Before setting the status to `COMPLETE`, reconcile
every entry in `Surprises & discoveries` and `Decision log` against RFC 0001
and confirm that D2, D3, and D5 have been accepted and that the RFC amendment
in Stage E step 4 has landed. An unaccepted deviation blocks completion.

## Artefacts and notes

Reserved for transcripts. At minimum, capture:

1. The red compile error for `tests/server_harness_lifecycle.rs`.
2. The green `test result: ok` line for each new test target.
3. One negative-control counter-example per invariant, INV-1 through INV-6.
4. Stateright's explored-state count from `make test-verification`.
5. The filtered `-p wireframe_testing --doc ... server_harness` pass alongside
   the unfiltered pre-existing failure, so a later reader can see that issue
   #578 was not made worse.
