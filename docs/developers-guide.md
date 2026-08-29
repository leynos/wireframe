# Wireframe developers' guide

This guide defines the architectural vocabulary used across Wireframe source,
rustdoc, and user-facing documentation. Treat it as the naming contract for new
APIs and refactors.

## Layer model and glossary

| Layer                 | Canonical term | Primary types                                    | Description                                                                                                                |
| --------------------- | -------------- | ------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------- |
| Transport framing     | Frame          | `FrameCodec::Frame`, `LengthDelimitedFrameCodec` | Physical wire unit read from or written to the socket.                                                                     |
| Routing envelope      | Envelope       | `Packet`, `Envelope`, `PacketParts`              | Routable wrapper that carries route id, optional correlation id, and payload bytes (`Packet` names the trait abstraction). |
| Domain payload        | Message        | `message::Message`, extractor `Message<T>`       | Typed application data encoded into envelope payload bytes.                                                                |
| Transport subdivision | Fragment       | `FragmentHeader`, `Fragmenter`, `Reassembler`    | Size-limited chunk used when a payload exceeds frame budget.                                                               |

## Naming invariants

- Frame terms belong to codec and socket boundaries only.
- Envelope terms belong to routing and middleware request/response wrappers;
  packet is an alias reserved for trait-level abstractions (`Packet`,
  `PacketParts`).
- Message terms belong to typed payload encode/decode concerns.
- Fragment terms belong to transport splitting and reassembly.
- Correlation identifiers are cross-layer metadata and may appear on frame,
  packet, and message-adjacent APIs when protocol metadata requires it.

## Public byte-container model

[ADR 008](adr-008-zero-copy-public-byte-container.md) accepts `bytes::Bytes`,
or a transparent project wrapper over `bytes::Bytes`, as Wireframe's stable
public byte representation for packet, envelope, serializer, middleware, and
hook payload hand-offs.

Read-only packet and routing paths should preserve shared byte storage and
cheap cloning. Middleware and client hook mutation must go through an explicit
edit-on-demand workflow, copying only when an edit needs unique mutable
storage. Do not introduce new public `Vec<u8>` payload contracts unless the
[ADR 009 compatibility policy](adr-009-vec-u8-migration-rollout.md) for the
zero-copy rollout explicitly calls for them.

## Actor and codec-driver boundary

[ADR 010](adr-010-transport-frame-boundary-for-zero-copy.md) accepts the
runtime boundary for zero-copy transport framing. The connection actor remains
packet-oriented and works with `Envelope` or another packet-shaped type; it
does not become responsible for codec frame buffers.

The codec driver owns the `packet -> bytes -> transport frame` transition,
including serialization and `FrameCodec::wrap_payload`. Protocol hooks remain
packet-oriented and run before serialization. The known gap is narrower:
`before_send` does not yet fire for app-router responses routed through
`FramePipeline`; roadmap item `11.2.1` owns that closure.

## Runtime ownership model

Three proposed decision records govern runtime ownership and task-lifetime
boundaries for Epic 635:

- [ADR 011](adr-011-runtime-ownership-and-task-lifetime-boundaries.md)
  defines ownership rules R1-R7 (shared roots, borrowed handles, sole-owner
  moves, and actor coordination) with mechanical lint enforcement for the
  grep-able rules.
- [ADR 012](adr-012-prepared-application-and-connection-runtime.md) splits
  `WireframeApp` into a registration builder, an immutable `PreparedApp`
  template shared by connection tasks, and a per-connection `ConnectionRuntime`.
- [ADR 013](adr-013-client-pool-scheduler-and-slot-ownership.md) gives the
  client-pool scheduler a single persistent owner task and index-based slot
  leases beneath one `PoolCore` root.

The first implementation slice is now in place: consuming
`WireframeApp::prepare().await` returns an immutable `PreparedApp` or a typed
`PrepareError`. Preparation consumes route and middleware registrations and
builds each route chain once. Connection tasks borrow the prepared route table,
so a single prepared application can serve multiple connections without
repeating middleware transforms. `WireframeApp` remains the registration
builder, and its direct connection methods are compatibility APIs.

Server factory evaluation and readiness semantics remain unchanged in this
slice. The server-runtime work tracked by issue
[#642](https://github.com/leynos/wireframe/issues/642) will prepare the factory
result before server readiness; connection-local state and the
`ConnectionRuntime` follow in issue
[#643](https://github.com/leynos/wireframe/issues/643). The records remain
proposed, and the review checklist derived from ADR 011's rules lands with
their implementation epic.

### Server supervisor lifecycle

`WireframeServer::run_with_shutdown` owns the server's `CancellationToken` and
`TaskTracker` while it supervises the worker accept loops. A named
`drop_guard_ref()` guard cancels the token when the supervisor future is
dropped, including when its `JoinHandle` is aborted. The accept loops then stop
accepting and release their listener references. This release is eventual
rather than synchronous because the loops must be scheduled to observe the
cancellation.

When the supplied shutdown future resolves, the existing graceful path still
cancels the accept loops and waits for tracked work. The drop guard does not
cancel connection tasks that were already accepted; those tasks continue under
the existing graceful-drain semantics.

The private `SupervisorLifecycle` state starts as `Running` and records one
terminal outcome: `Graceful` when the shutdown future resolves, `Dropped` when
the supervisor frame is abandoned, or `Finished` when tracked work ends before
either cancellation path. Cloned lifecycle handles pass the recorded
cancellation reason to each accept loop, which records its own exit after it
observes cancellation. The supervisor cancellation counter and accept-loop exit
counter (`wireframe_server_supervisor_cancellations_total` and
`wireframe_server_accept_loops_exited_total`) use only the bounded `reason`
values `"graceful"` and `"dropped"`; direct future drops and
`JoinHandle::abort()` therefore have the same `"dropped"` reason. The
corresponding static tracing events are `server_supervisor_cancellation` and
`server_accept_loop_exited`, each with the same `reason` field.

## Allowed aliases and prohibited mixing

| Canonical term | Allowed aliases                     | Avoid in the same context                 |
| -------------- | ----------------------------------- | ----------------------------------------- |
| Frame          | wire frame, transport frame         | packet, envelope, message                 |
| Envelope       | packet (trait abstraction contexts) | frame (unless describing codec transport) |
| Message        | payload type, domain message        | frame, fragment                           |
| Fragment       | fragment chunk                      | packet, message                           |

## API and docs checklist

Use this checklist before merging API naming changes:

- Confirm the identifier name matches the owning layer.
- Ensure rustdoc examples use the same term as the symbol being documented.
- Verify `docs/users-guide.md` and migration notes describe the same meaning.
- Label cross-layer terms (for example, `correlation_id`) explicitly as shared
  metadata.

## App inbound and outbound helper boundaries

Application response paths must keep message serialization and codec frame
wrapping in one place. Use `app::outbound_encoding::encode_message_frame` when
an app path needs to turn an `EncodeWith<S>` message into a
`FrameCodec::Frame`. Callers should keep transport-specific work at the edge:

- Raw stream response methods encode the returned codec frame into a byte
  buffer and write that buffer to `AsyncWrite`.
- Framed response methods send the returned codec frame through the supplied
  framed sink.
- The length-delimited compatibility path intentionally sends the raw
  serialized message to `LengthDelimitedCodec`; do not wrap that payload with
  the app codec first, because `framed.send` supplies the length prefix.

Inbound connection handling should also preserve the phase boundary:
`build_dispatchable_envelope` owns decode, fragment reassembly, message
assembly, and the successful deserialization-counter reset. Individual failure
policy remains in `DeserFailureTracker`, so logging, metrics, and threshold
decisions do not drift across inbound call sites.

Builder methods that change `WireframeApp` type parameters should route through
the shared rebuild helpers in `app::builder::core`. Serializer and codec
transitions use `rebuild_with_params`; connection-state transitions use
`rebuild_with_connection_state`. A teardown hook is typed to the old connection
state, so `on_connection_setup` clears any teardown hook registered before it.
Register teardown after setup when both hooks are required.

Client pool internals should use `client::pool::sync::lock_or_recover` for
poison-tolerant `Mutex` access. Keep the policy local to pool synchronization
code so scheduler and slot state recovery cannot drift. Recovery logs a warning
and increments the `wireframe_pool_bookkeeping_poison_recoveries_total`
counter. Client connection construction should flow through
`WireframeClientBuilder::into_parts()` and `ClientBuildParts`, which keeps
single-client and pooled-client socket setup, preamble exchange, lifecycle
hooks, request hooks, and tracing configuration on the same path. Pooled lease
methods should go through `PooledClientLease::dispatch_on_connection` so
checkout and recycle-on-error policy stay in one place.

## Error surface conventions

Library-facing errors should stay typed and inspectable by default. Use
`NoProtocolError` when an API has no protocol-specific failure payload so the
crate-level `Result<T>` still participates in standard error chaining; reserve
`WireframeError<E>` for protocols with their own source-bearing error type. See
the rustdoc for `wireframe::NoProtocolError` for the public API contract.

## Message sequence validation architecture

Message continuation ordering lives in `src/message_assembler/series.rs`. The
public-facing control point is `validate_and_advance_sequence()`, which keeps
the series state machine readable by delegating to two private helpers:

- `start_sequence_tracking()` handles the first numbered continuation after an
  untracked start. It switches the series into tracked mode and then delegates
  to `advance_sequence_or_overflow()` to derive and record the next expected
  sequence number.
- `advance_tracked_sequence()` handles all later numbered continuations once
  tracking is active. It rejects duplicates, gaps, and sequence overflow before
  calling `advance_sequence_or_overflow()` to advance the expected sequence.

`advance_sequence_or_overflow()` is the shared leaf helper: it calls
`checked_increment()` on the incoming sequence and raises
`MessageSeriesError::SequenceOverflow` only when the counter wraps while more
frames are still expected.

This three-helper split is intentional. `start_sequence_tracking()` isolates
the untracked-to-tracked transition; `advance_tracked_sequence()` owns the
duplicate/gap/out-of-order validation; and `advance_sequence_or_overflow()`
owns the overflow decision. Each piece is independently testable and the
decision tree stays flat and readable during future refactors.

### `fill_buf_with_prefix`

The private helper `fill_buf_with_prefix(buf, prefix, endianness)` in
`src/frame/conversion.rs` copies a validated length-prefix byte slice into the
correct position of an 8-byte staging buffer. For big-endian prefixes it places
the bytes at the high end of the buffer (`buf[8 - size..]`); for little-endian
prefixes it places them at the low end (`buf[..size]`). Callers must guarantee
that `prefix.len()` is one of `{1, 2, 4, 8}`; this invariant is enforced
upstream in `bytes_to_u64`, which is why the helper validates the range before
copying and reports an error when the prefix width falls outside the supported
set.

### `ServerShutdownHandle`

`ServerShutdownHandle` in `wireframe_testing::client_pair` is a type alias for
the tuple `(oneshot::Sender<()>, JoinHandle<Result<(), ServerError>>)` that
`PendingServer` stores between server start-up and explicit shutdown. Naming
the alias keeps `PendingServer`'s field types readable and makes
`PendingServer::take` return a self-documenting `Option<ServerShutdownHandle>`
instead of an opaque inline tuple. Tests that call `WireframePair::shutdown()`
do not interact with this type directly; it is an internal implementation
detail of the pair harness.

## Vocabulary normalization outcome (2026-02-20)

The 2026-02-20 normalization pass aligned docs and rustdoc terminology to this
model and did not rename public symbols. Existing API names (`FrameCodec`,
`Packet`, `Envelope`, `Message`, `Fragment*`) already map cleanly to separate
layers, so the implementation focused on clarifying boundaries rather than
introducing additional breaking changes.

## Quality gates

Use the Makefile targets as the contributor entrypoint for routine validation:

- `make check-fmt` verifies workspace formatting.
- `make lint` runs rustdoc with warnings denied, `cargo clippy`, and
  `whitaker --all -- --all-targets --all-features`.
- `make test` runs the main automated test suite with warnings treated as
  errors.

Install Whitaker through the standalone installer described in the
[Whitaker user's guide](whitaker-users-guide.md) so local linting matches
continuous integration (CI).

### CodeScene coverage baseline

The `Coverage (main)` workflow in `.github/workflows/coverage-main.yml` runs on
pushes to `main`. After the test suite succeeds, it generates a ratcheted LCOV
report and uploads that report to CodeScene. The workflow checks out
`leynos/wireframe`, so CodeScene records the coverage under the repository
identity `github.com/leynos/wireframe`, and targets project `68308` explicitly.

The upload reads `CS_ACCESS_TOKEN` from the repository secret into the job
environment, then passes that value through the upload action's required
`access-token` input. This workflow input is permitted because the value still
comes from the repository secret; never hard-code the token in workflow or
source files, and never log it. The pull-request workflow's CodeScene coverage
check uses the same project and repository identity. It consumes the report
published for `main` as the baseline for its changed-line gate, so the main
workflow must publish successfully before that gate can evaluate a pull request.

## Mutation testing

Scheduled mutation testing runs in CI via
`.github/workflows/mutation-testing.yml` (see
[ADR-007](adr-007-mutation-testing-with-cargo-mutants.md) for the design and
its rationale). Key points for contributors:

- The workflow is a thin caller of the shared reusable workflow
  `leynos/shared-actions/.github/workflows/mutation-cargo.yml` (pinned by
  commit SHA; caller guide in that repository's
  `docs/mutation-cargo-workflow.md`). It is informational only: it never gates
  pull requests, and surviving mutants do not fail the run. Scheduled runs
  execute daily, scoped to Rust files changed in the preceding 25 hours, and
  skip cheaply when nothing changed. Manual dispatch (select the branch in the
  Actions "Run workflow" control) runs full mutations, fanned out across eight
  shards with one merged summary.
- The caller passes `--all-features` so feature-gated tests (e.g. the
  `serializer-serde` bridge round-trips) run against mutants, and excludes the
  example/test-support scaffolding whose survivors are noise — both per issue
  #571: `src/codec/examples.rs`, `src/test_helpers.rs`, `src/test_helpers/**`
  (the module-root glob does not match the directory's submodules),
  `src/connection/test_support.rs`, and `src/**/tests.rs` (cfg(test) companion
  files that cargo-mutants cannot detect as test code, #599). The contract test
  `tests/workflow_contracts/mutation_testing_test.py` (run via
  `make test-workflow-contracts`) pins this exclude list, the `--all-features`
  extra-args, and the shard count.
- Mutants proven equivalent (incapable of changing observable
  behaviour) are annotated in source with `#[cfg_attr(test, mutants::skip)]`
  plus a one-line justification. The attribute comes from the
  [`mutants`](https://docs.rs/mutants) crate, a dev-dependency of no-op
  decorator attributes; keep skips rare, justified, and reserved for stateless
  equivalences — prefer a killing test wherever the mutation is observable.
- [`cargo-mutants`](https://mutants.rs/) is a CI-runtime dependency
  only, installed by the shared workflow at a pinned version; it is not a Cargo
  dependency and is not required locally. To reproduce a run locally, install
  it with `cargo install cargo-mutants` and run, for example,
  `cargo mutants --in-place --all-features --file src/frame/mod.rs`.
- Results appear in the run's merged job summary (caught/missed/timeout
  counts plus a table of surviving mutants per target) and as downloadable
  `mutation-report-*` artefacts containing `mutants.out/` (one per shard on
  full runs).
- Surviving mutants are a test-improvement backlog: triage them for
  equivalent mutations (false survivors) before writing tests. Mutants in
  `wireframe_testing` are mostly false survivors because that crate's logic is
  exercised chiefly by the root crate's suite; treat its table as advisory.

### Workflow contract tests

Because the caller is configuration rather than code, a contract test pins the
shape it must uphold, failing the pull request when the caller drifts —
repointing the pin at a branch, widening the token scope, or dropping a
configuration input — rather than letting the breakage surface only in a
scheduled run. The tests live in
`tests/workflow_contracts/mutation_testing_test.py` and
`tests/workflow_contracts/shared_actions_test.py`, and parse the workflows with
PyYAML. Run them locally with `make test-workflow-contracts`. They validate:

- every `leynos/shared-actions` invocation across the repository workflows
  targets an approved action or reusable workflow path, uses a full
  40-character lowercase hex commit SHA, and shares the same version — the
  value itself is not asserted, so Dependabot bumps it freely (see
  [Workflow pins and Dependabot](#workflow-pins-and-dependabot));
- job permissions are exactly least-privilege (`contents: read`,
  `id-token: write`);
- the workflow-level default token scope is empty (`permissions: {}`);
- `concurrency` serializes runs per ref (`mutation-testing-${{ github.ref }}`)
  without cancelling one in progress; and
- the triggers keep the daily schedule and a plain `workflow_dispatch` with
  no legacy branch input.

A further test pins the `with:` block itself: `extra-args: "--all-features"`
(so feature-gated tests run against mutants, matching the CI baseline),
`shard-count: 8`, and the `exclude-globs` scaffolding list
(`src/test_helpers.rs`, `src/test_helpers/**`, `src/connection/test_support.rs`,
`src/codec/examples.rs`, and `src/**/tests.rs`). It also asserts that
`extra-crate-dirs` is *absent*, guarding the temporary removal of the
`wireframe_testing` companion target until its standalone doctests compile
again (#578); restore that assertion alongside the input when #578 closes.

## Workflow pins and Dependabot

Dependabot owns the upgrade of GitHub Actions and reusable workflows, including
calls into `leynos/shared-actions`. Contract tests that assert a caller's exact
commit SHA create a lockstep dependency: every time Dependabot opens a bump PR,
the test fails until a human edits the pinned constant to match. That defeats
the purpose of automated dependency updates and turns a routine bump into a
manual chore.

Contract tests may still verify the *shape* of a reusable-workflow caller. They
must not verify the specific SHA value.

- Do assert every `leynos/shared-actions` invocation has an approved action or
  reusable-workflow path and is pinned to a full 40-character commit SHA, not a
  mutable branch such as `main` or `rolling`.
- Do assert all `leynos/shared-actions` invocations across all workflows use
  the same commit SHA.
- Do assert the expected `on:` triggers, least-privilege `permissions:`, and
  the inputs the caller relies on.
- Do not hard-code the current SHA value as an expected string. Match it with
  a pattern instead.
- Do not fail a test purely because Dependabot bumped the pinned SHA.

```python
import re

SHA_RE = re.compile(r"^[0-9a-f]{40}$")


def test_uses_pinned_full_sha(caller_step):
    ref = caller_step["uses"].split("@")[-1]
    assert SHA_RE.match(ref), f"expected a 40-hex commit SHA, got {ref!r}"
```

If a workflow's behaviour genuinely depends on a feature only present from a
particular commit onwards, express that as a comment or a changelog note, not
as a test assertion on the SHA string.

## Cargo workspace semantics

Wireframe now uses a hybrid root manifest: the repository root `Cargo.toml`
contains both `[package]` and `[workspace]`.

The workspace explicitly lists the root package, the internal verification
crate, and the testing helper crate, while keeping only the root package as a
default member:

- `members = [".", "crates/wireframe-verification", "wireframe_testing"]`
- `default-members = ["."]`

That means ordinary root-level commands such as `cargo build`, `cargo check`,
`cargo test`, and `cargo clippy` retain their existing ergonomics and continue
to target the main `wireframe` package by default.

Use plain root-level Cargo commands for day-to-day work on the main crate.
Reach for `--workspace` when a task is explicitly meant to cover every current
workspace member, for example repository-wide validation in CI or when a change
also touches `crates/wireframe-verification` or `wireframe_testing`.

Use `cargo test -p wireframe-verification` to exercise the Stateright crate in
isolation. The existing `Makefile` targets still focus on the root `wireframe`
crate because the dedicated formal-verification targets belong to later roadmap
items.

Use `cargo test -p wireframe_testing` when changing shared test fixtures,
observability helpers, codec drivers, or other support APIs exported by the
testing helper crate. Use `cargo test -p wireframe` for the root crate when a
change should stay limited to the published library. Plain root-level commands
keep their day-to-day ergonomics because `default-members = ["."]` leaves the
main `wireframe` package as the only default member.

### Workspace manifest test support

The shared module `tests/common/workspace_manifest_support.rs` keeps the
workspace-contract helpers beside the integration tests that use them. It is a
module, not a library crate, because the code is test-only scaffolding and
should not widen the published crate surface or add another Cargo target.

The support layer uses `cap-std` with the `fs_utf8` feature for
capability-oriented directory access, `camino` for UTF-8-typed paths, and
`serde_json` for structured assertions over `cargo metadata` output.

- `repo_root()` locates the repository root as a `Utf8PathBuf`.
- `repo_dir()` opens that root as a `cap_std::fs_utf8::Dir`.
- `root_manifest()` reads the root `Cargo.toml` into a `String`.
- `run_cargo(args)` runs `cargo` in the repository root and returns UTF-8
  stdout, or an error that includes stderr.
- `cargo_metadata()` wraps `cargo metadata --no-deps --format-version 1`.
- `root_package_id()` wraps `cargo pkgid -- wireframe` and trims trailing
  whitespace.
- `has_manifest_line(manifest, line)` checks for a complete trimmed line rather
  than a substring match.

`WorkspaceManifestWorld` in `tests/fixtures/workspace_manifest.rs` is the
behaviour-driven development (BDD) fixture for these assertions. Extend it by
loading more workspace-state inputs in `load()` and adding focused verification
methods that the step definitions and scenario can reuse.

## Example and benchmark support

TCP server examples that share the standard
`WireframeApp<BincodeSerializer, (), Envelope>` runtime shape should use
`examples/support/runtime_bootstrap.rs` for tracing setup, runtime app
construction, listener binding, connection spawning, shutdown-aware accept
loops, and current-thread Tokio runtime startup. Keep example-specific address
parsing, app construction, handlers, and middleware in the example file.

Codec benchmark helpers live in `wireframe_testing::codec_benchmarks`. Bench
targets, direct unit tests, and BDD fixtures should import the workload matrix,
measurement helpers, fragmentation helpers, and allocation-label helpers from
that module instead of coupling to files under `tests/common` with `#[path]`.

Fragment transport integration tests import `tests/common/fragment_helpers.rs`
as a facade. Keep the public re-export surface stable there, and place helper
implementation details in responsibility-focused modules under
`tests/common/fragment_helpers/`: app construction and spawning, assertions,
fragmentation configuration, envelope building, error types, and framed
transport. New fragment helpers should be added to the smallest matching module
and re-exported only when more than one test binary needs the helper.

## Formal verification tooling

Formal-verification tools are pinned in repository metadata and installed
through concise Makefile entry points. Contributors should use these targets
from the repository root instead of running long `uv tool run` commands by hand:

- `make install-kani` installs the Kani version named in
  `tools/kani/VERSION`.
- `make check-kani-version` verifies that the installed Kani binary matches
  `tools/kani/VERSION`.
- `make install-verus` installs the Verus release named in
  `tools/verus/VERSION` after checking `tools/verus/SHA256SUMS`.
- `make run-verus` runs the proof file selected by `VERUS_PROOF_FILE`, or
  `verus/wireframe_proofs.rs` by default.

The targets delegate to `prover-tools`, supplied by the pinned
`rust-prover-tools` source in `tools/rust-prover-tools/REF`. The Makefile
should stay thin: it constructs the pinned `uv tool run --python 3.14`
invocation and lets `prover-tools` own Kani installation, Kani version checks,
Verus download and checksum verification, Verus binary resolution, and Verus
toolchain handling.

Keep Verus proof files outside the normal Cargo build under `verus/`. The
`run-verus` target is expected to fail with a clear missing-proof-file
diagnostic until later formal-verification roadmap work adds
`verus/wireframe_proofs.rs`.

### Formal tooling test support

The shared module `tests/common/formal_tooling_support.rs` keeps
repository-contract helpers beside the integration tests that use them. It
reads the tool metadata files, extracts Makefile target recipes, and verifies
that those recipes delegate to `prover-tools` rather than embedding installer
commands such as `cargo install`, `curl`, or `rustup toolchain install`.

`FormalToolingWorld` in `tests/fixtures/formal_tooling.rs` is the BDD fixture
for the contributor workflow. Extend it by loading additional repository
metadata in `load()` and adding focused verification methods that scenario
functions and step definitions can reuse.

## Test infrastructure and framework

### rstest and rstest-bdd

The test suite uses [`rstest`](https://crates.io/crates/rstest) for
fixture-based parametric tests and `rstest-bdd` (via `rstest_bdd_macros`) for
behaviour-driven development (BDD) scenarios expressed in Gherkin.

Fixtures are plain Rust functions annotated with `#[fixture]`. Inject them into
tests by listing them as parameters; `rstest` constructs each fixture before
running the test body.

BDD scenarios live in `.feature` files under `tests/features/`. Each file
describes one or more scenarios using the standard Given/When/Then syntax.
Scenario functions are annotated with `#[scenario(path = "…", name = "…")]` and
receive fixture parameters by name.

### trybuild compile-time tests

Compile-time API contracts live in
[`tests/compile_error.rs`](../tests/compile_error.rs). That runner uses
[`trybuild`](https://crates.io/crates/trybuild) to execute small pass and
compile-fail programs under [`tests/ui/`](../tests/ui/).

Use these tests for public trait bounds, default generic parameters, and other
contracts that must fail or succeed at type-check time rather than runtime.
Place new snippets in `tests/ui/`, register them in `tests/compile_error.rs`,
and commit the generated `.stderr` file for compile-fail cases after verifying
that the diagnostics describe the intended contract.

### Feature files and step definitions

Each `.feature` file under `tests/features/` has a corresponding
step-definition module under `tests/steps/`. Step functions are annotated with
`#[given]`, `#[when]`, or `#[then]` and accept a mutable reference to the BDD
world fixture as their first argument.

Add new scenarios by:

1. Writing a new Gherkin scenario in the relevant `.feature` file.
2. Implementing the missing step functions in the corresponding
   `tests/steps/` module.
3. Adding a scenario function in `tests/scenarios/` that names the new
   scenario, injects the fixture, and delegates to a helper that invokes the
   step logic in sequence.

### Fallible test helpers and server-task results

Two distinct `TestResult` aliases exist in the codebase; do not assume they are
the same type.

- `wireframe::testkit::result::TestResult<T = ()>` (re-exported as
  `wireframe_testing::TestResult`) is `Result<T, TestError>`, where `TestError`
  (`src/testkit/result.rs`) collects the typed errors produced across the root
  crate, client and server runtimes, push queues, codecs, and fragmentation.
  Use it for any helper that crosses those boundaries, and in BDD fixtures and
  steps that import it from `wireframe_testing`.
- A module-local `TestResult<T = ()> = Result<T, Box<dyn Error + Send + Sync>>`
  is scoped to individual test modules, for example
  `src/client/tests/helpers.rs` and the corresponding aliases in
  `src/fragment/tests/`. Use it where a helper only needs to erase the error
  type for `?`-propagation within that module and gains nothing from
  `TestError`'s typed variants.

For the in-process server/client pair harness, which also returns
`wireframe_testing::TestResult`, see the
["In-process server/client pair harness"](wireframe-testing-crate.md#in-process-serverclient-pair-harness)
section of `docs/wireframe-testing-crate.md`.

**Fixtures and helpers are not tests.** A fixture or helper arranges state, and
arrangement can fail, so it returns `Result` and propagates failures with `?`.
Only a test body unwraps or asserts, because a failure there becomes the test's
verdict. The whitaker `no_expect_outside_tests` lint enforces this rule, but it
cannot see through proc-macro expansion, so an `rstest` `#[fixture]` function
counts as non-test code even though it exists only to support tests.

**The `finish_server` convention.** A fixture that spawns a server as a
`JoinHandle<TestResult>` exposes an
`async fn finish_server(&mut self) -> TestResult` that takes the handle and
propagates both the `JoinError` and the inner error with `handle.await??`.
`Drop` remains an abort-only fallback for scenarios that fail or are
interrupted before calling `finish_server`, so a server-side failure is never
silently discarded on the happy path. See `ClientLifecycleWorld::finish_server`
in `tests/fixtures/client_lifecycle.rs` and the precedent in
`tests/fixtures/client_preamble.rs`.

**The server-task cleanup contract**, as implemented by
`run_hook_test_with_server` in `src/client/tests/request_hooks_support.rs`: on
success the client is dropped first so the serve loop ends, then the server
task is joined and its result propagated; on failure the task is aborted and
reaped instead, because the client may never have connected and the server may
be parked in `accept`, where joining would hang rather than report the original
failure. The rule: **await the server handle on the success path; reserve
`abort` for the failure path where the aborted task's result is explicitly
discarded.**

**Loopback server helpers** in `src/client/tests/helpers.rs` — `bind_loopback`,
`spawn_listener`, `spawn_frame_server`, and `is_expected_disconnect` — provide
the building blocks for spawning a listener and driving a length-delimited
frame loop. `is_expected_disconnect` classifies I/O errors observed while
serving: an ordinary peer disconnect (`UnexpectedEof`, `ConnectionReset`,
`ConnectionAborted`, or `BrokenPipe`) ends the serve loop normally, because a
client that finishes its work and drops its connection produces exactly one of
those kinds. Every other I/O error is returned from the task instead, so it
surfaces when the caller joins the handle.

### `LoggerHandle::Default` and `ObservabilityHandle::Default`

Both `LoggerHandle` (in `wireframe_testing::logging`) and `ObservabilityHandle`
(in `wireframe_testing::observability`) implement `Default`. The `default()`
method delegates to `new()` in each case, providing a convenient way to acquire
a fresh handle without explicitly calling the constructor.

`LoggerHandle::new()` tolerates a poisoned mutex: if a prior test panicked
while holding the logger lock, `new()` recovers the guard via `into_inner()`
and drains any buffered log records, so the next test starts from a clean state.

## Roadmap editing with mapsplice

[The combined roadmap](roadmap.md) is roadmap-shaped Markdown: `mapsplice`
parses it to append, insert, delete, and replace numbered items, so it must
stay within that tool's grammar. Phases are level-2 headings
(`## 9. Phase title`), steps are level-3 headings (`### 9.2. Step title`), and
tasks are numbered checklist items (`- [ ] 9.2.1. Task title`).

That grammar rejects footnote references, failing with
`unsupported inline node footnoteReference`. Roadmap references must therefore
use inline links instead of the GitHub-flavoured `[^1]` footnotes used
elsewhere in the documentation set: cite a target either as a parenthetical
`(see [target](path))` or by linking an existing phrase. This is a scoped
exception to the footnote rule recorded in the
[documentation style guide](documentation-style-guide.md).

Keep the visible link text consistent: cite ADRs by number (for example
`ADR 0005`) and design documents by a short descriptive name, rather than by
raw filename.

`mapsplice` fails closed when the target does not match the supported grammar,
so previewing an edit to stdout doubles as a grammar check:

```bash
MAPSPLICE_IN_PLACE=false mapsplice append docs/roadmap.md fragment.md >/dev/null
```

Run `make fmt` to reformat and rewrap after editing, then `make markdownlint`.

## Spelling policy

The `make spelling` gate enforces en-GB-oxendict spelling across tracked text.
It runs Typos 1.48.0 and a phrase checker that rejects the hyphenated form in
favour of `handwritten`. `make markdownlint` depends on the same spelling gate.

The tracked `typos.toml` is generated from the shared Oxford dictionary and the
repository-specific `typos.local.toml` overlay. The generator is the focused
`typos-config-builder` command pinned to commit
`b604f198797fdd36a567dd0f8f07b13f9539b241`. It refreshes the untracked
`.typos-oxendict-base.toml` cache only when the authority is newer than the
local copy; `.typos-oxendict-base.json` records refresh metadata.

Use `make spelling-config-write` after changing `typos.local.toml`, and use
`make spelling-config` to check deterministic output. Never edit `typos.toml`
directly. Keep repository exceptions narrow: preserve public APIs, external
tooling keys, formal names, and immutable diagnostics without adding ordinary
bare-word exceptions.

The standalone phrase helper and its tests require Python 3.13 or later and pin
Pathspec 1.1.1 and Hypothesis 6.156.6; Ruff targets Python 3.13 compatibility.
The property suite protects phrase boundaries and source locations across
generated neighbouring characters. Eligible tracked files must remain readable
UTF-8 text so the gate cannot silently omit them. Continuous integration
installs Nixie 1.1.0 with Python 3.14 and Merman CLI 0.7.0 before validating
the repository's Mermaid diagrams with `make nixie`.
