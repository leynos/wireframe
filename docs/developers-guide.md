# Wireframe developers' guide

This guide defines the architectural vocabulary used across Wireframe source,
rustdoc, and user-facing documentation. Treat it as the naming contract for new
APIs and refactors.

## GitHub Actions runner placement

Namespace is retired. Each lane now sits where the kind of work decides, rather
than all four repository-owned lanes sharing one profile.

| Workflow                 | Job                 | Trigger            | Runner                | Ceiling |
| ------------------------ | ------------------- | ------------------ | --------------------- | ------- |
| `ci.yml`                 | `build-test`        | pull request, push | `ubicloud-standard-4` | 30 min  |
| `coverage-main.yml`      | `coverage-upload`   | push               | `ubicloud-standard-4` | 20 min  |
| `advanced-tests.yml`     | `advanced`          | schedule           | `ubuntu-latest`       | 60 min  |
| `delayed-pr-comment.yml` | `delay_and_comment` | dispatch           | `ubuntu-latest`       | none    |

*Table 1: Where each repository-owned lane runs, and its ceiling.*

Pull-request, push and tag lanes move to Ubicloud, because those are the ones a
developer waits on: `build-test` waited 461 seconds for a GitHub-hosted runner
on 2026-09-16 to do 556 seconds of work. Scheduled and delayed-comment lanes
stay GitHub-hosted, where public-repository minutes are free and nobody is
blocked by the wait.

`build-test` also serves forks, which cannot obtain an Ubicloud runner, so it
carries the fork fallback:

```yaml
runs-on: >-
  ${{ github.event.pull_request.head.repo.fork
  && 'ubuntu-latest' || 'ubicloud-standard-4' }}
```

The continuation line sits at the same indent as the line above it. A
more-indented continuation in a folded scalar keeps its line break, putting a
newline inside the expression; GitHub evaluates the broken value and the job
runs, so a green run is not evidence that the scalar is well formed.

### Why CI can leave GitHub-hosted Linux now

This guide previously recorded that CI had to stay on GitHub-hosted Linux
"because Whitaker's prebuilt `cargo-dylint` does not verify on the shared
Ubuntu 22.04 profile". That reason was real, and it was specific to the image
rather than to the provider. `whitaker-installer` requires glibc 2.39;
Namespace's shared profile is Ubuntu 22.04, which carries glibc 2.35.

`ubicloud-standard-4` is Ubuntu 24.04, which carries glibc 2.39, so the
constraint is satisfied rather than waived. repovec-appliance runs the same
`whitaker-installer` 0.2.6 on Ubicloud, green, and so does this lane. Should a
lane ever need the `-ubuntu-2204` variant, the old constraint returns with it.

The Whitaker installer cache key now includes `runner.environment`, because
this lane runs on two environments and the cached artefact is a compiled
binary. Both images are Ubuntu 24.04 today, so the key would not yet collide;
it is keyed now because the lane gained a second environment in the change that
could later give it a second glibc.

### Why four vCPU and not two

Both Ubicloud lanes are `ubicloud-standard-4`. That is a measurement, not a
preference for the larger shape.

`tests/compile_error.rs` is a trybuild case: it spawns its own cargo build of
the whole dependency tree, and it does so while the other 987 tests in the
suite are running beside it. On a four-vCPU runner it takes 64.5 seconds of the
suite's 65.4. On `ubicloud-standard-2` it was still running when nextest killed
it at 180.0 seconds, which is that tool's default 60-second slow-timeout times
three, and the coverage step failed at exit 100 with 987 of 988 tests passed.

Halving the cores does not halve that test. Its siblings are competing for the
same two, so the test that most wants a whole machine is the one that gets
least of one. Both lanes carry the shape because both run the same
`generate-coverage` over the same workspace; `coverage-upload`'s 240-second
sample was taken on Namespace's shared profile, which is four vCPU, so two was
never a size it had been measured at.

The alternative would have been to give that one test a longer deadline. This
repository has no nextest configuration at all, so that means new machinery
introduced to make a smaller runner tolerable, which is the wrong direction:
the cost is what should move, not the victim's deadline.

### Ceilings, and the one lane that has none

A per-minute runner bills until something stops it, so GitHub's six-hour
default is the expensive failure mode. A ceiling close to the measured work is
the other one, because it cancels the run at the moment an overrun becomes
interesting and discards the log that would explain it.

Two ceilings are measured: `build-test` at 556 seconds and `coverage-upload` at
240 seconds, both on four-vCPU runners. Two are judgements, and the guide says
so rather than implying otherwise. `advanced` has failed every night since at
least 2026-09-09 and its last green run was 2025-10-04 at 32 seconds, so 60
minutes is generous enough not to mask the repair when it lands.

`delay_and_comment` declares no ceiling, deliberately. Its entire duration is a
`sleep` of the caller's `delay_minutes` input, so any fixed ceiling cancels a
legitimate longer delay: it would fire exactly when the delay became
interesting, which inverts the failure a ceiling exists to prevent. The lane is
GitHub-hosted, so the six-hour default costs nothing.
`ci_runner_placement_test.py` asserts that absence with its reason, so it reads
as a decision rather than as the gap the neighbouring test looks for.

### The contract

`tests/workflow_contracts/ci_runner_placement_test.py` replaces
`namespace_runners_test.py`. It sits beside two modules it imports, split by
role rather than by size: `runner_placement_policy.py` holds the reviewed
decisions and the reason for each, and `runner_placement_reader.py` holds the
machinery that derives facts from the workflow tree. A reviewer who wants to
know what was decided reads the policy module alone.

The contract reads every job's `runs-on` from the parsed document and fails on
an embedded line break; compares the fork guard and both arms against exact
strings, so a sibling field cannot stand in for `head.repo.fork`; pins
placement by `(workflow, job id)` coordinate and compares that set against the
tree's jobs in both directions; pins each ceiling by value and requires one on
every job that can select any `ubicloud-` label; asserts the delayed lane has
none; asserts the two reusable callers declare no runner; and compares
`.github/actionlint.yaml` against the labels in use in both directions. That
last comparison is what retires `namespace-profile-default` from the registry
rather than leaving it behind to authorize a runner family nobody uses.

It also checks that the reviewed tables are total against each other, which is
what stops a lane escaping review by being left out of one of them. Every other
assertion iterates a table, and an iterated table cannot report what was never
put in it: a placed job with no ceiling entry would pass both ceiling checks,
because one iterates the ceiling table and the other reads only `ubicloud-`
lanes, and a second expression-checked coordinate would have no runner
assertion at all.

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

The upload passes the repository secret `CS_ACCESS_TOKEN` directly to the
upload action's required `access-token` input, without binding it in a job
environment variable. Never hard-code the token in workflow or source files,
and never log it. No pull-request workflow names the project or the token; the
changed-line gate a reviewer sees on a pull request is CodeScene's own check
against what this workflow published, so this upload must succeed before that
gate can evaluate anything.

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

### Where CodeScene may appear

The CodeScene command-line tool is installed from a URL at job time and is not
pinned to a version this repository chose: the shared action selects the
archive from a committed manifest and verifies its digest, so the artefact is
pinned, but what that artefact talks to is not. The tool calls CodeScene's API
and refuses to run when the answer changes shape, and that has happened twice.
Its output format moved, and more recently thirteen projects stopped returning
a gates configuration at all, so the changed-line gate fails with "received
project-config isn't valid" for a reason no change in the repository could have
caused.

So the tool runs in exactly one place: `coverage-main.yml`, on push to main. A
failure there delays a coverage report. It cannot block a merge.

A pull-request lane may still generate coverage, because the ratchet is
repository-owned and runs offline with no network dependency. What it may not
do is any of these six, each of which fails
`tests/workflow_contracts/ci_codescene_placement_test.py`:

| Forbidden in a pull-request lane                      | Why it is read                                         |
| ----------------------------------------------------- | ------------------------------------------------------ |
| a step or job whose `uses:` names CodeScene           | the obvious form                                       |
| a `run:` step invoking `cs-coverage`                  | the same hazard without an action to notice            |
| `CS_ACCESS_TOKEN` at any scope                        | a lane holding the token is one line from using it     |
| `codescene.io` anywhere                               | `curl` needs neither the action nor the tool           |
| `secrets: inherit` into another repository's workflow | forwards the token unnamed, to a document nobody reads |
| a call to this repository's workflow by `@ref`        | runs a revision the contract has not read              |

*Table 2: What the CodeScene placement contract refuses.*

The third row is what makes the contract worth having. Deleting the step but
leaving the token in the job environment looks clean in a diff and leaves the
hazard in place, so the whole document is walked for the name rather than the
three scopes that are meant to carry it: a `run` body, an action input, an
`env` value under any key and a named `secrets:` forwarding are all found. The
fifth row closes the one route the walk cannot see, since `secrets: inherit`
names nothing. Inheriting into a workflow in this repository is permitted,
because that workflow is in the closure described below and read like any other.

A further test guards the other direction. Without it the rule could be
satisfied by deleting coverage reporting altogether, which is compliance by
amputation, so the publisher is asserted to exist, to have exactly its reviewed
triggers, not to be startable by a pull request, and to state `mode: upload`
rather than inherit it. The trigger set is compared whole: a `pull_request`
added beside `push` would make the publisher a pull-request lane, and any other
addition or removal changes what it is for unreviewed. Stating the mode means
the publisher cannot quietly become the pull-request check gate.

`pull_request_target`, `merge_group`, `workflow_run`, `pull_request_review`,
`pull_request_review_comment` and `issue_comment` count as pull-request
triggers here. `pull_request_target` runs on a pull request with write
permissions, which makes it more dangerous than `pull_request`, not less.
`merge_group` runs the checks a pull request needs to leave the merge queue, so
a red one blocks the merge. `workflow_run` runs after a pull-request workflow,
with the repository's secrets. A review, a review comment or a comment on the
pull request each start a workflow for it. `workflow_dispatch` does not count:
a dispatch is not a pull request.

### The pull-request lane is a closure, not a list

The prohibitions apply to every workflow a pull request can reach, not only to
those carrying a pull-request trigger. A workflow declaring only
`workflow_call` has no pull-request trigger, yet a pull-request job that calls
it runs it on that pull request, and hands it the token with `secrets: inherit`.

The contract therefore follows `jobs.<id>.uses`, transitively, with a visited
set so two reusable workflows calling each other cannot hang it. A call is
local when it resolves to a file directly under `.github/workflows/` once its
prefix is stripped. That is matched by shape rather than by a list of
spellings, with two prefixes stripped: `./`, the documented form, and `$/`.
Accepting a spelling GitHub might refuse only widens the set the prohibitions
run over; missing one GitHub accepts hides a workflow from all of them.

A call to this repository's workflows at a ref, whether written with the
qualified name (`leynos/wireframe/.github/workflows/x.yml@main`) or with a
local prefix and a ref (`./.github/workflows/x.yml@main`,
`$/.github/workflows/x.yml@main`), is refused rather than followed. GitHub runs
it at the named ref, not at the pull request's head, so the file the closure
would read is not the file that runs, and the named revision could hold a
CodeScene step every clause passes over. A workflow that needs this
repository's reusable workflow calls it with `./`. For the same reason
`secrets: inherit` into such a call counts as inheriting into a document the
contract cannot read.

References to other repositories are not followed: their content is not in this
tree. That is why `secrets: inherit` into one is refused outright rather than
traced. This repository has no local reusable workflows today, so the closure
equals the roots; the traversal is here so that adding one does not silently
take it out of scope.

### The publisher's two silent failure modes

The upload step's condition is
`steps.codescene-token.outputs.available == 'true' && github.ref == 'refs/heads/main'`,
compared whole rather than searched for parts. A containment test accepts this
expression, which holds both halves and is true everywhere, so it would pass
the one expression it exists to refuse:

```yaml
if: (github.ref == 'refs/heads/main' || true) && (env.CS_ACCESS_TOKEN != '' || true)
```

Appending `|| github.event_name == 'workflow_dispatch'` is the same defeat in
another form: every conjunct is still present and all of them become optional.
The equality comparison refuses both, and it admits no extra conjunct, so an
`||` hidden behind one
(`… && github.actor != 'x' || github.event_name == 'workflow_dispatch'`) fails
it too; no separate `||` rule is needed.

The token is bound in no `env` anywhere in `coverage-main.yml`. A step with id
`codescene-token` runs exactly one command, with no `if:`:

```sh
echo "available=${{ secrets.CS_ACCESS_TOKEN != '' }}" >> "$GITHUB_OUTPUT"
```

The expression is evaluated to `true` or `false` before the shell runs, so the
step writes the answer without holding the token, and the upload's condition
reads that output. The upload passes
`access-token: ${{ secrets.CS_ACCESS_TOKEN }}` directly: the uploader is a
composite action that hands its step's `env` to nested artefact and cache
steps, and a job-scoped token would be readable by the tests that generate
coverage. A guard on `env.CS_ACCESS_TOKEN != ''`, the earlier shape, passes
with its binding deleted, and the upload then skips forever with nothing
failing. The contract pins the command, its lack of a condition and of an
`env`, its position before the upload, and the input, and refuses the token in
any `env` on the job.

The uploader is pinned to a full commit SHA, at or after shared-actions
`a5765019`: from there it selects the CodeScene CLI from a committed manifest,
where earlier revisions install "latest", which no longer resolves. Per the pin
policy below, the contract asserts the SHA's shape and the absence of the
retired `installer-checksum`, not the specific revision.

Dependabot's automerge merges with the workflow's `GITHUB_TOKEN`, and a push
made that way starts no workflow, so an automerged dependency bump never runs
the publisher: a known exception, tracked in shared-actions issue #518. If the
publisher ever gains a `workflow_dispatch`, a dispatch that replaces a pending
push run in the concurrency group uploads the dispatched commit's coverage,
which leaves the baseline one commit behind the trunk until the next push; the
same issue tracks it.

The ref test is not redundant with the trigger. `push.branches` is `[main]` and
there is no `workflow_dispatch`, so `github.ref` cannot currently be anything
else. It is what keeps the trigger honest: the moment either changes, a
dispatch from a feature branch would publish that branch's coverage as the
trunk's, because CodeScene accepts an upload for the analysed branch whatever
the payload came from. Both are pinned so neither moves without the other being
reconsidered.

The publisher also declares a concurrency group keyed on the ref alone,
`coverage-main-${{ github.ref }}`, with `cancel-in-progress: false`; the
contract compares the group whole, so naming the event in it fails. Without a
group, two pushes in quick succession upload at once and the baseline is set by
whichever finishes last. With one, GitHub keeps a single pending run per group,
so among triggered runs (push, and dispatch where the workflow allows it) a
newer one replaces an older pending run and the newest baseline wins. A manual
"Re-run jobs" on an older main run is an operator action, not a trigger: it
keeps that run's commit, so it republishes that commit's coverage and baseline
until the next push supersedes them. Cancelling would instead abandon a running
upload and its baseline write.

### What must remain

Everything above forbids something, so all of it is satisfied by a repository
that measures no coverage at all. One test says what must stay: `ci.yml` is
still started by `pull_request` and still runs `generate-coverage` with
`with-ratchet` and `publish-artefact: 'false'` (the ratchet reads the report;
nothing else does), exactly once, under exactly the reviewed condition
`github.event_name == 'pull_request'`. The condition is pinned by value rather
than merely permitted, because presence is not reachability: `if: false` leaves
the step in the file, where every other check still sees it, and runs it never.

### The retired CodeScene digest variable

`CODESCENE_CLI_SHA256` fed the uploader's `installer-checksum` input. The
uploader takes its digest from a committed manifest now, so the input is gone
and the variable feeds nothing. `get-codescene-sha.yml`, which existed only to
refresh it, is deleted, and no workflow may mention the name. That test reads
every workflow rather than the pull-request closure, because a refresher on a
schedule or a dispatch is exactly the shape it is meant to catch and neither is
reachable from a pull request.

The repository variable itself can be deleted; nothing reads it.

### How the readings are proved

The reading machinery lives in
`tests/workflow_contracts/codescene_placement_reader.py`, and every reader
takes its documents as an argument. The reviewed values live in
`codescene_placement_policy.py`, the publisher's clauses in
`codescene_publisher_test.py`, the classification of `uses:` calls in
`workflow_calls.py`, and loading in `workflow_loader.py`, which the runner
placement contract shares. The repository's own workflows are all written the
one way the first reader understood, so a reading that mishandles another shape
passes against them either way.
`tests/workflow_contracts/codescene_placement_reader_test.py` therefore drives
each reading with constructed trees:

- the closure reaches a `workflow_call` probe that curls CodeScene's API with
  an inherited token, in both local call spellings, and stays out of a reusable
  workflow nothing calls;
- a call to this repository by `@ref` is recognized, and kept narrow: a local
  call, another repository, a repository whose name merely begins the same way
  and this repository's own actions are not;
- workflows are loaded through a strict `SafeLoader` that refuses a duplicated
  mapping key, because PyYAML otherwise keeps the last `runs-on` or `env` and
  says nothing;
- `on:` is read as a scalar, a sequence or a mapping, under both the quoted
  string key and YAML 1.1's boolean `True`; a workflow declaring both is
  refused, since GitHub merges them; and any other shape is refused rather than
  read as "no triggers", which would let the workflow escape every clause; and
- a `.YML` extension is read like `.yml`.

Each reading was mutated alone and restored from a copy while writing the
contract, and each mutation failed at least one test that names what it broke.

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

## Development builds

The repository pins `nightly-2026-03-26` with `rustfmt`, `clippy`, and
`rust-analyzer`. The toolchain also needs `rustc-codegen-cranelift-preview` for
the optional development backend. Native Linux development builds use `mold` as
the linker; CI provisions the component and linker before running the standard
debug gates.

The Makefile keeps the development configuration in
`tools/dev-fast/config.toml`, outside Cargo's automatic configuration paths.
Standard debug Make targets select it explicitly: `build`, `test`, `test-bdd`,
`test-doc`, `lint`'s rustdoc and Clippy commands, and `typecheck`. The
`dev-build` target runs the configured Cargo build directly, even when a
library artefact already exists; `dev-test` aliases `test`.

The fragment configures the backend only. It deliberately declares no
`[target.<...>]` table, so a direct `--config tools/dev-fast/config.toml`
invocation behaves the same on every machine. `mold` selection cannot live in
the fragment: Cargo compares a target-specific `cfg` against the *compilation
target*, not the host, so a Linux `cfg` would also select `mold` for a
non-Linux host cross-compiling to Linux. Only Make can test the host, so Make
carries the linker flag in `RUSTFLAGS`. Make adds it when the host is Linux
*and* the effective Cargo target is Linux, and recognizes standard Linux target
triples by `-unknown-linux-`. An explicit native Linux target therefore retains
`mold`, while non-Linux targets, including Android, do not. For a custom JSON
or non-standard Linux target, set `CARGO_BUILD_TARGET_OS=Linux`. Non-Linux
hosts never select `mold`, including when cross-compiling to Linux.

The fragment selects Cranelift for the development profile; Cargo test commands
(`make test`, `make test-bdd`, and `make test-doc`) use the LLVM test profile
described below. Installing the Cranelift component alone does not change the
backend.

Release builds, coverage generation, verification commands, and Whitaker run
without the development fragment. Direct Cargo commands also leave it
unselected unless the caller passes `--config tools/dev-fast/config.toml`. This
keeps ordinary Cargo use on the repository's configured default backend while
making the faster backend an explicit choice for routine debug work.

## Cranelift

The Make test targets still select the development fragment, but the test
profile overrides Cranelift with LLVM on the pinned `nightly-2026-03-26`
toolchain. This exception follows reproduced test failures: with Cranelift,
`make test` aborted in
`client::pool::sync::tests::lock_or_recover_reads_poisoned_mutex` with
`SIGABRT` and `fatal runtime error: failed to initiate panic, error 5`. The
test passed with the LLVM test-profile override. Under Cranelift, a metrics
doctest in `make test-doc` failed to link because AWS-LC symbols were
undefined; all doctests passed with the LLVM override.

The explicit fragment remains selected by the Make test targets, but their
tests are not Cranelift-accelerated. Development builds, lint, and typecheck
continue to use Cranelift. This is a pinned-toolchain exception to the Netsuke
source fragment; reproduce both failures and reassess the override when the
toolchain pin changes.

## Cargo workspace semantics

Wireframe now uses a hybrid root manifest: the repository root `Cargo.toml`
contains both `[package]` and `[workspace]`.

The workspace explicitly lists the root package, the internal verification
crate, and the testing helper crate, while keeping only the root package as a
default member:

- `members = [".", "crates/wireframe-verification", "wireframe_testing"]`
- `default-members = ["."]`

Plain root-level commands such as `cargo build`, `cargo check`, `cargo test`,
and `cargo clippy` retain their existing ergonomics and continue to target the
main `wireframe` package by default. The Makefile validation targets are the
workspace-wide exception: they pass `--workspace` so the root, verification,
and testing helper crates are checked together.

Use `make test-verification` to run `cargo test -p wireframe-verification`.
`make kani`, `make kani-full`, and `make verus` are tool-free placeholders
until their named roadmap work activates them.

Use `cargo test -p wireframe_testing` when changing shared test fixtures,
observability helpers, codec drivers, or other support APIs. Use
`cargo test -p wireframe` when a change should stay limited to the published
library.

### Workspace-wide validation and private-item documentation

The standard Makefile gates cover all supported workspace members and targets:

- `make test` selects `tools/dev-fast/config.toml` and runs:

  ```text
  RUSTFLAGS="$(DEV_WARNING_FLAGS)" $(CARGO) $(DEV_FAST) test --workspace \
    --all-targets --all-features $(BUILD_JOBS)
  ```

- `make test-doc` selects `tools/dev-fast/config.toml` and runs:

  ```text
  RUSTFLAGS="$(DEV_WARNING_FLAGS)" $(CARGO) $(DEV_FAST) test --workspace \
    --exclude wireframe_testing --doc --all-features $(BUILD_JOBS)
  ```

  The testing helper's standalone doctests require generic application types
  that snippets cannot infer; [issue #578][issue-578] tracks their repair.
  Remove the exclusion when that issue is resolved.

  [issue-578]: https://github.com/leynos/wireframe/issues/578

- `make typecheck` selects `tools/dev-fast/config.toml` and runs:

  ```text
  RUSTFLAGS="$(DEV_WARNING_FLAGS)" $(CARGO) $(DEV_FAST) check --workspace \
    --all-targets --all-features $(BUILD_JOBS)
  ```

- `make lint` runs these workspace-wide checks:

  ```text
  RUSTFLAGS="$(DEV_WARNING_FLAGS)" RUSTDOCFLAGS="$(RUSTDOC_FLAGS)" \
    $(CARGO) $(DEV_FAST) doc --workspace --no-deps
  RUSTFLAGS="$(DEV_WARNING_FLAGS)" $(CARGO) $(DEV_FAST) clippy $(CLIPPY_FLAGS)
  RUSTFLAGS="-D warnings" $(WHITAKER) --all -- --all-targets --all-features
  ```

The shared `[workspace.lints.clippy]`, `[workspace.lints.rust]`, and
`[workspace.lints.rustdoc]` tables are inherited by each member through
`[lints] workspace = true`. In particular, Clippy's
`missing_docs_in_private_items = "deny"` gate covers private implementation
items in every crate and target, while Rust's `missing_docs = "deny"` continues
to cover the public surface. Add `//!` module documentation and `///` item
documentation that explains the relevant contract, invariant, or test purpose.
If generated or macro-expanded code leaves a genuinely unavoidable gap, use an
item-scoped `#[expect(clippy::missing_docs_in_private_items, reason = "...")]`
with the concrete technical limitation; do not add blanket allows.

Plain root-level commands keep their day-to-day ergonomics because
`default-members = ["."]` leaves the main `wireframe` package as the only
default member; use the Makefile gates or explicit `--workspace` flags for
repository-wide validation.

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

### Formal verification execution targets

Run `make test-verification` to execute the Stateright verification crate with
the repository's ordinary Rust test runner. `make kani`, `make kani-full`, and
`make verus` are deliberately tool-free placeholders until their owned roadmap
work supplies Kani harnesses and Verus proofs. Each writes a `FORMAL-SKIP:`
marker to standard error and succeeds on a clean checkout.

Use `FORMAL_STRICT=1` with any placeholder target to make that skip fail. This
is the tripwire for a CI job that must detect a placeholder left behind after
the target should have been activated. `make formal-pr` combines
`test-verification`, `kani`, and `verus`; `make formal-nightly` replaces the
smoke target with `kani-full`; `make formal` aliases the pull-request gate.

The owning roadmap item must replace the corresponding one-line placeholder
recipe rather than adding an automatic readiness check. Roadmap 15.3.1 owns the
`kani` smoke harnesses, later 15.3.x work owns the full Kani harness set, and
15.5.2 owns `verus/wireframe_proofs.rs`. Kani activation must use the pinned
tooling route, and a change that turns a placeholder into a real tool command
must move or guard its execution test: the default `make test` suite must not
install or invoke Kani or Verus.

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

Run the spelling gate with:

```bash
make spelling
```

The gate enforces en-GB-oxendict spelling across every tracked file. It runs
Typos and a phrase checker that rejects the hyphenated form in favour of
`handwritten`. `make markdownlint` depends on the same gate.

The tracked `typos.toml` is regenerated on every run from the live shared
dictionary and the repository-specific `typos.local.toml` overlay. The
generator is the shared `typos-config-builder` command, pinned by release tag
in the Makefile. It refreshes the untracked `.typos-oxendict-base.toml` cache
only when the authority is newer than the local copy;
`.typos-oxendict-base.json` records refresh metadata. A valid cache remains
usable when the network is unavailable. Because the dictionary is live,
`typos.toml` must never be drift checked in continuous integration.

Never edit `typos.toml` directly; add narrow repository terminology to the
overlay instead. Keep repository exceptions narrow: preserve public APIs,
external tooling keys, formal names, and immutable diagnostics without adding
ordinary bare-word exceptions.

Apart from the inline-code span, which the overlay masks until the shared
dictionary does so itself, use one exact documented pattern per exception
rather than disabling a whole syntax class. The remaining exceptions are
limited to the `PoolServerBehavior` test-server fixture, the former
`BackoffConfig::normalised` public method, exact generic-bound fragments in RFC
0001, immutable en-GB diagnostic fixtures, and Tokio test attributes. Add a new
pattern only when a narrower correction or wording change would alter a public
API, external-tool key, formal name, or deliberately fixed diagnostic.

Eligible tracked files must remain readable UTF-8 text so the gate cannot
silently omit them. Continuous integration installs Nixie 1.1.0 with Python
3.14 and Merman CLI 0.7.0 before validating the repository's Mermaid diagrams
with `make nixie`.
