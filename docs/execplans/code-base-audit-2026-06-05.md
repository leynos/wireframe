# Address code-base audit findings

This ExecPlan (execution plan) is a living document. The sections `Constraints`,
`Tolerances`, `Risks`, `Progress`, `Surprises & Discoveries`, `Decision Log`,
and `Outcomes & Retrospective` must be kept up to date as work proceeds.

Status: IN PROGRESS

## Purpose / big picture

This plan addresses the refactoring concerns found in the 2026-06-05 code-base
audit. The work improves maintainability by moving repeated send, encoding,
builder-transition, deserialization-policy, pool-lock, pool-lease, workspace,
example-bootstrap, benchmark-helper, and fragment-helper patterns behind
focused reusable modules. A reviewer can observe success by reading the smaller
call sites, running the project gates, and seeing behavioural tests continue to
exercise externally observable client, app, workspace, example, benchmark, and
fragment workflows.

The implementation deliberately preserves public behaviour. The goal is to make
existing behaviours easier to maintain, not to redesign Wireframe's external
API.

## Constraints

- Keep the branch named `code-base-audit-2026-06-05` and track
  `origin/code-base-audit-2026-06-05`.
- Do not start implementation until this draft plan has been reviewed and
  explicitly approved.
- Preserve public API signatures unless the plan is updated and the user
  explicitly approves the public API change.
- Use existing repository patterns before adding new abstractions.
- Keep every code file at or below the repository's 400-line limit.
- Use `rstest` for new unit-style tests and `rstest-bdd` for behavioural tests
  where a change affects externally observable workflow behaviour.
- Use `googletest` assertions and `pretty_assertions` in new or substantially
  revised tests where they improve failure diagnostics.
- Use `insta` snapshot tests only where multivariant output format consistency
  is relevant. None of the currently planned refactors changes formatted
  output, so no snapshot tests are planned unless implementation reveals such a
  case.
- Use `proptest`, Kani, or Verus only when the change introduces a new invariant
  over input ranges, states, orderings, or transitions. The planned work is
  mostly structure-preserving extraction, so no new proof or model-checking
  milestone is planned unless the implementation adds new decision logic.
- Validate each major milestone with `make check-fmt`, `make lint`, and
  `make test`, using `tee` to write logs under `/tmp`.
- Run `coderabbit review --agent` after deterministic gates pass for each major
  milestone. If rate-limited, run `vsleep $(shuf -i 15-30 -n 1)m` before
  retrying.
- Commit after each gated milestone. Use a file-based commit message via
  `git commit -F`, not `git commit -m`.
- Update `docs/developers-guide.md` whenever a reusable concern or pattern is
  introduced or clarified by this work.

## Tolerances (exception triggers)

- Scope: if the implementation of a single milestone requires touching more
  than 15 files or 500 net lines, stop, document the reason, and ask for
  direction.
- Interface: if a public API signature must change, stop and ask for explicit
  approval.
- Behaviour: if any behavioural test needs its expected behaviour changed
  rather than preserved, stop and ask for explicit approval.
- Validation: if `make check-fmt`, `make lint`, or `make test` fails for a
  reason unrelated to the current milestone and cannot be isolated within one
  hour of investigation, stop and ask for direction.
- CodeRabbit: if CodeRabbit reports a concern that conflicts with a repository
  requirement, document the conflict in `Decision Log` and ask for direction
  before ignoring it.
- Dependency: if adding `googletest`, `pretty_assertions`, or `insta` causes
  dependency or lint fallout beyond the touched tests, stop and ask whether to
  continue broadening the dependency change.

## Risks

- Refactoring async send paths can subtly change hook, timing, span, or error
  ordering. Mitigation: keep helpers thin, preserve call order exactly, and add
  unit and BDD coverage around success and failure paths.
- Refactoring app outbound encoding can change frame wrapping, flush behaviour,
  or error mapping. Mitigation: keep transport-specific send operations at the
  edge and test raw stream, framed stream, serialization failure, and send
  failure paths.
- Reworking `PooledClientLease` can affect recycle-on-error behaviour.
  Mitigation: replace the macro with a helper that returns the same error
  values, and add tests for successful dispatch and failed checkout/dispatch.
- Adding `wireframe_testing` explicitly to workspace members can broaden
  workspace gates. Mitigation: run the full requested gates after the manifest
  change and update `docs/developers-guide.md` to reflect the new semantics.
- Splitting test helpers can disrupt many integration tests through module path
  churn. Mitigation: introduce a facade module that preserves current import
  names, then move implementation into submodules behind that facade.

## Progress

- [x] 2026-06-05: Loaded `execplans`, `leta`, and `rust-router`.
- [x] 2026-06-05: Renamed the local branch to
  `code-base-audit-2026-06-05`.
- [x] 2026-06-05: Drafted this ExecPlan.
- [x] 2026-06-05: Ran deterministic gates for the draft plan and applied two
  trivial CodeRabbit punctuation fixes.
- [x] 2026-06-05: Resolved CodeRabbit's wording feedback by using Oxford
  `-ize` spelling and documenting the explicit filename exception.
- [x] Push branch and create draft pull request for plan review.
- [x] Create GitHub follow-up issues for audit findings not implemented in this
  plan.
- [x] Receive explicit approval to implement this plan.
- [x] Milestone 1: centralize client send pipeline logic.
- [x] 2026-06-05: Milestone 1 passed `make check-fmt`, `make lint`,
  `make test`, targeted ExecPlan Markdown lint, and CodeRabbit review with
  zero findings.
- [x] Milestone 2: centralize app outbound encoding and inbound pipeline
  failure policy.
- [x] 2026-06-05: Milestone 2 passed `make check-fmt`, `make lint`, and
  `make test`; CodeRabbit review findings have been fixed except repeated
  `-ise` spelling requests that conflict with repository Oxford `-ize` style.
- [x] 2026-06-05: Milestone 2 rerun passed `make check-fmt`, `make lint`,
  `make test`, targeted Markdown lint, and CodeRabbit review with only the
  documented Oxford-spelling conflict findings remaining.
- [x] 2026-06-05: Milestone 2 final rerun passed `make check-fmt`,
  `make lint`, `make test`, and targeted Markdown lint after the added
  response-helper tests.
- [x] Milestone 3: route app builder transitions through one rebuild path.
- [x] 2026-06-05: Milestone 3 passed `make check-fmt`, `make lint`,
  `make test`, targeted Markdown lint, and CodeRabbit review with zero
  findings.
- [x] Milestone 4: refactor pool lock recovery, builder parts construction, and
  pooled lease dispatch.
- [x] 2026-06-05: Milestone 4 passed focused pool BDD coverage, `make
  check-fmt`, `make lint`, `make test`, and Markdown lint before CodeRabbit
  review.
- [x] 2026-06-05: Fixed CodeRabbit's Milestone 4 request for direct
  `lock_or_recover` tests, then reran `make check-fmt`, `make lint`,
  `make test`, and Markdown lint successfully.
- [x] 2026-06-05: Fixed CodeRabbit's follow-up Milestone 4 requests for
  `lock_or_recover` contract documentation, poison-recovery observability, and
  stronger poison assertions, then reran `make check-fmt`, `make lint`,
  `make test`, and Markdown lint successfully.
- [x] 2026-06-05: Resolved CodeRabbit's remaining Milestone 4 spelling finding
  by changing the helper module comment to avoid the contested
  `Synchronization`/`Synchronisation` word entirely, then reran
  `make check-fmt`, `make lint`, `make test`, and Markdown lint successfully.
- [x] 2026-06-05: Fixed CodeRabbit's Milestone 4 encapsulation and metric
  findings by narrowing `lock_or_recover` to `pub(super)` and adding
  `wireframe_pool_bookkeeping_poison_recoveries_total`, then reran
  `make check-fmt`, `make lint`, `make test`, and Markdown lint successfully.
- [x] 2026-06-05: Fixed CodeRabbit's Milestone 4 module-policy documentation
  request and created issue #539 for broader scheduler/slot poison recovery
  integration coverage, then reran `make check-fmt`, `make lint`, `make test`,
  and Markdown lint successfully.
- [x] 2026-06-05: Fixed CodeRabbit's Milestone 4 observability assertion
  request by extending the poison-recovery unit test to assert the warning text
  and `wireframe_pool_bookkeeping_poison_recoveries_total`, then reran focused
  pool-sync tests, `make check-fmt`, `make lint`, `make test`, and Markdown
  lint successfully.
- [x] 2026-06-05: Milestone 4 CodeRabbit rerun passed with zero findings.
- [x] Milestone 5: update workspace membership and documentation.
- [x] 2026-06-05: Milestone 5 passed focused workspace-manifest integration
  and BDD tests, `make check-fmt`, `make lint`, `make test`, and Markdown
  lint before CodeRabbit review.
- [x] 2026-06-05: Milestone 5 CodeRabbit review passed with zero findings.
- [x] Milestone 6: extract example bootstrap and benchmark helper wiring.
- [x] 2026-06-05: Milestone 6 passed focused codec benchmark helper tests,
  example compile checks, benchmark BDD tests, `make check-fmt`, `make lint`,
  `make test`, and Markdown lint before CodeRabbit review.
- [x] 2026-06-05: Milestone 6 CodeRabbit review passed with zero findings.
- [ ] Milestone 7: split fragment test helpers behind a compatibility facade.
- [ ] Run final gates, CodeRabbit review, push, and update the pull request.

## Surprises & Discoveries

- `cargo metadata` reports `wireframe_testing` as a workspace member even
  though the root manifest currently lists only `.` and
  `crates/wireframe-verification` in `[workspace].members`. The existing
  developers' guide documents this as a nuance. This plan will replace that
  nuance with explicit membership.
- `googletest`, `pretty_assertions`, and `insta` are not currently declared in
  the root manifest. Milestone 2 added `googletest` and `pretty_assertions` as
  dev-dependencies for the new focused response and outbound encoding tests.
- Global `make fmt` currently fails on unrelated pre-existing Markdown lint
  violations outside this plan file. Targeted Markdown lint for this plan file
  passes.
- `cargo fmt --workspace` is not supported by the installed Cargo formatter
  wrapper. Use `cargo fmt --all` or the repository's `make check-fmt` target
  for Rust formatting in this worktree.
- Moving inbound frame construction into `build_dispatchable_envelope` pushed
  `src/app/inbound_handler.rs` over the 400-line module limit. Extracting the
  public response-sending methods to `src/app/outbound_response.rs` brought the
  inbound handler back to 326 lines and improved its separation of concerns.
- The local `gh issue comment 538` command cannot add the zero-copy migration
  note because the GitHub integration reports `Resource not accessible by
  integration`. ADR 005 and this ExecPlan now carry the migration detail.

## Decision Log

- 2026-06-05: Use an approval gate before implementation. The user asked for
  `execplans`, and the skill requires draft review before execution. The branch
  and draft pull request may be prepared before approval, but code changes must
  wait.
- 2026-06-05: Keep public APIs stable. The audit concerns are maintainability
  issues, and there is no requirement to change user-visible contracts.
- 2026-06-05: Treat CodeRabbit as a milestone reviewer after deterministic
  gates, not as a substitute for formatting, linting, or tests.
- 2026-06-05: Accept CodeRabbit's two draft-plan punctuation findings because
  they improve readability without changing scope.
- 2026-06-05: Restore `centralize` in milestone labels because the repository
  uses en-GB-oxendict spelling, which favours `-ize` forms.
- 2026-06-05: Decline CodeRabbit's filename-pattern finding because the user
  explicitly required the plan path
  `docs/execplans/code-base-audit-2026-06-05.md`.
- 2026-06-05: Implement the client send-pipeline extraction as a private
  `WireframeClient::serialize_and_send` method in `src/client/send_pipeline.rs`.
  The helper accepts a span-construction callback taking `TracingConfig` and
  the final frame byte length, so callers keep correlation-specific span data
  without borrowing the client twice.
- 2026-06-05: Introduce `src/app/outbound_encoding.rs` as the shared
  serialization-to-codec-frame helper for app outbound paths. Keep raw-stream
  and framed transport writes in `src/app/outbound_response.rs` and
  `src/app/codec_driver.rs`, respectively.
- 2026-06-05: Split inbound dispatch preparation into
  `WireframeApp::build_dispatchable_envelope`, with decode failures delegated
  to `DeserFailureTracker`. Preserve the counter reset point after decode,
  reassembly, and message assembly have all succeeded.
- 2026-06-05: Accept CodeRabbit's buffer-capacity concern in
  `send_response`. Since `FrameCodec` has no generic encoded-overhead API and
  `F::Frame` need not be cloneable for an encoding probe, remove the
  payload-length preallocation rather than guessing overhead.
- 2026-06-05: Decline CodeRabbit's request to change `serialize` and
  `serialization` comments to `serialise` and `serialisation` in
  `src/app/outbound_response.rs`. The repository requires en-GB-oxendict, which
  favours `-ize` forms, and the surrounding app comments consistently use
  `serialize`/`serialization`.
- 2026-06-05: Accept CodeRabbit's bound-tightening finding for
  `send_response_framed_with_codec`; the method only sends through a framed
  sink, so `W: AsyncWrite + Unpin` is sufficient.
- 2026-06-05: Accept CodeRabbit's matching bound-tightening finding for
  `send_response_framed`; the length-delimited framed response path also only
  sends.
- 2026-06-05: Accept CodeRabbit's request to document the future zero-copy
  serializer migration. Create tracking issue
  <https://github.com/leynos/wireframe/issues/538> and keep the current
  `Bytes::from(Vec<u8>)` conversion until the public serializer contract can
  change deliberately.
- 2026-06-05: Mirror the zero-copy migration TODO on the length-delimited
  `send_response_framed` conversion so both app outbound conversion sites
  reference issue #538.
- 2026-06-05: Accept CodeRabbit's request to document why
  `send_response_framed` bypasses `encode_message_frame`; the
  `LengthDelimitedCodec` path intentionally sends raw serialized messages and
  lets `framed.send` perform length-prefix encoding.
- 2026-06-05: Accept CodeRabbit's request for a concrete zero-copy serializer
  deferral note. Update ADR 005, add an in-code comment at
  `encode_message_frame`, and keep issue #538 as the owner for changing the
  public serializer output contract. The audit milestone deliberately retains
  `Vec<u8>` output to preserve source compatibility and avoid turning a
  refactor into an API migration.
- 2026-06-05: Decline repeated CodeRabbit requests to change
  `serialization` comments to `serialisation` in
  `src/app/outbound_encoding.rs` and `src/app/outbound_response.rs` for the
  same repository-style reason already recorded above.
- 2026-06-05: Accept CodeRabbit's request for direct tests around
  `encode_message_frame` and the public framed response send helpers. Add
  lightweight test-only serializers, codecs, and writers to cover success,
  serialization failure, codec encode failure, I/O failure, large payloads, and
  codec wrapper variation.
- 2026-06-05: Document the app inbound and outbound helper boundaries in
  `docs/developers-guide.md`, because milestone 2 introduces reusable internal
  patterns that future app refactors should follow.
- 2026-06-05: Accept CodeRabbit's assertion-style cleanup in
  `outbound_encoding` tests and replace the boolean matcher with a concise
  googletest `err(pat!(...))` matcher.
- 2026-06-05: Partially accept CodeRabbit's broader public response-helper
  coverage request. Existing tests already covered success, serialization,
  write, framed-send, and large-payload paths; add the missing focused cases
  for codec-encoder failure, flush failure, and raw serialized payload output
  on the length-delimited framed path.
- 2026-06-05: Decline CodeRabbit's follow-up request to duplicate the public
  response-helper integration coverage inside a new
  `src/app/outbound_response.rs` `#[cfg(test)]` module. `tests/response.rs`
  already exercises `send_response`, `send_response_framed_with_codec`,
  `length_codec`, and `send_response_framed` through the public API, including
  serialization, codec-encode, write, flush, framed-send, raw-payload, and
  large-payload paths. The requested direct
  `LengthDelimitedCodec::max_frame_length()` assertion is not available through
  Tokio's public API, so the existing large-payload behavioural checks are the
  observable contract.
- 2026-06-05: Implement the app connection-state type transition with
  `WireframeApp::rebuild_with_connection_state`. The helper keeps field
  movement centralised and clears teardown because a teardown hook registered
  for old state `C` cannot be type-correct for new state `C2`. Document that
  teardown must be registered after setup when both hooks are required.
- 2026-06-05: Start milestone 4 by extracting
  `client::pool::sync::lock_or_recover`, adding
  `WireframeClientBuilder::into_parts()`, and replacing
  `PooledClientLease`'s dispatch macro with a closure-based async helper.
  Document the pool synchronization and client-build-parts patterns in
  `docs/developers-guide.md`.
- 2026-06-05: Implement milestone 4 with a private pool synchronization helper
  instead of duplicated poison recovery, route single-client and pooled-client
  connection construction through `WireframeClientBuilder::into_parts()`, and
  make pooled lease methods call `PooledClientLease::dispatch_on_connection`.
  The closure helper uses `AsyncFnOnce` so the borrowed managed connection can
  be awaited without boxing or weakening the recycle-on-error flow. Document
  the lock, builder-parts, and lease-dispatch patterns in
  `docs/developers-guide.md`.
- 2026-06-05: Accept CodeRabbit's Milestone 4 test request and add direct
  `lock_or_recover` coverage for normal acquisition and poison recovery. Use
  the googletest harness directly for these tests because `expect_that!`
  requires a googletest context, while `verify_that!(...)?` inside an
  `rstest` returning `Result` conflicts with the repository's strict
  `panic_in_result_fn` lint when the test intentionally poisons a lock.
- 2026-06-05: Accept CodeRabbit's follow-up request to make
  `lock_or_recover`'s recovery policy explicit and observable. The helper now
  documents that it is only for pool bookkeeping state, warns through
  `tracing::warn` before recovering, and the poison test proves the spawned
  thread panicked and the mutex entered the poisoned state before recovery.
- 2026-06-05: Resolve CodeRabbit's Oxford-spelling conflict on the module-level
  `Synchronization` wording by replacing the sentence with "Pool lock helper
  shared across client pool internals." This preserves the repository's
  documented Oxford `-ize` style without leaving a repeated review finding.
- 2026-06-05: Accept CodeRabbit's follow-up request to keep the pool poison
  policy local to the pool module and observable through metrics. Change
  `lock_or_recover` from `pub(crate)` to `pub(super)` and add
  `metrics::inc_pool_bookkeeping_poison_recoveries()` backed by
  `wireframe_pool_bookkeeping_poison_recoveries_total`.
- 2026-06-05: Accept CodeRabbit's request for module-level policy text around
  `lock_or_recover`. Defer the deeper scheduler/slot poison recovery
  integration test to issue #539 because it needs a deliberate way to simulate
  panic-interrupted private scheduler or slot bookkeeping without weakening
  production encapsulation.
- 2026-06-05: Accept CodeRabbit's request to assert `lock_or_recover`
  observability side effects. The poison-recovery unit test now runs the
  recovery call under `metrics::with_local_recorder`, captures warning output
  with a scoped `tracing_subscriber` writer, and asserts both the warning text
  and the `wireframe_pool_bookkeeping_poison_recoveries_total` increment.
- 2026-06-05: Start milestone 5 by adding `wireframe_testing` to explicit
  workspace members. Update direct workspace-manifest tests, BDD fixture/steps,
  feature text, scenario wrappers, and `docs/developers-guide.md` so the helper
  crate is no longer described as a Cargo metadata nuance.
- 2026-06-05: Implement milestone 5 by adding `wireframe_testing` to
  `[workspace].members`, asserting its package id in direct and BDD workspace
  manifest tests, and updating command guidance in `docs/developers-guide.md`.
  The root package remains the only default workspace member, so plain Cargo
  commands retain their root-crate ergonomics.
- 2026-06-05: Start milestone 6 by extracting example TCP server runtime
  bootstrap into `examples/support/runtime_bootstrap.rs` and moving codec
  benchmark helpers into `wireframe_testing::codec_benchmarks`. This keeps
  example-specific app construction local while replacing bench/test `#[path]`
  coupling to `tests/common` benchmark helper files with a stable helper crate
  module.
- 2026-06-05: Implement milestone 6 by routing `ping_pong` and `packet_enum`
  through `examples/support/runtime_bootstrap.rs`, moving codec benchmark
  helpers into `wireframe_testing::codec_benchmarks`, and updating benches,
  direct tests, and BDD fixtures to import helpers from the helper crate. Add
  `PayloadClass::is_empty()` because moving the helper into public
  `wireframe_testing` API made `PayloadClass::len()` subject to
  `len_without_is_empty`.

## Implementation Plan

### Milestone 1: centralize client send pipeline logic

Create an internal helper near the client runtime modules, likely in
`src/client/send_pipeline.rs` or an existing client helper module. The helper
serializes an `EncodeWith<S>` value, invokes before-send hooks, sends the frame
through the framed transport, emits timing events, and invokes the error hook
on serialization or transport failure. It must allow callers to supply the span
or span-construction data so `send`, `send_envelope`, and `call_streaming`
retain their distinct tracing metadata.

Update `src/client/runtime.rs`, `src/client/messaging.rs`, and
`src/client/streaming.rs` to use the helper. Keep correlation-specific logic in
the caller: `send_envelope` and `call_streaming` still decide whether to reuse
or generate a correlation identifier before sending.

Add or update `rstest` unit tests for serialization failure, before-send hook
mutation, send failure, and successful send. Add or update `rstest-bdd`
coverage only where existing client messaging or streaming scenarios can
observe the refactor through public behaviour.

After the milestone, run:

```sh
make check-fmt 2>&1 | tee /tmp/check-fmt-wireframe-code-base-audit-2026-06-05.out
make lint 2>&1 | tee /tmp/lint-wireframe-code-base-audit-2026-06-05.out
make test 2>&1 | tee /tmp/test-wireframe-code-base-audit-2026-06-05.out
coderabbit review --agent
```

Commit the milestone if all gates and CodeRabbit concerns are clear.

### Milestone 2: centralize app outbound encoding and inbound pipeline policy

Extract app outbound encoding into a helper that serializes a message or
envelope and wraps it with the configured `FrameCodec`. Keep raw stream writing
and framed stream sending separate, so transport semantics remain explicit.
Update `src/app/inbound_handler.rs` and `src/app/codec_driver.rs` to use the
helper.

Change `frame_handling::decode_envelope` so it accepts `DeserFailureTracker`
instead of manually incrementing and checking the counter. Then extract the
chained decode, reassemble, and assemble flow from `WireframeApp::handle_frame`
into an internal
`build_dispatchable_envelope(...) -> io::Result<Option<Envelope>>` helper. The
helper must preserve the rule that deserialization failure count resets only
after the full inbound pipeline produces a dispatchable envelope.

Add or update tests for outbound serialization success and failure, framed send
failure, raw stream send failure, recoverable decode failure, threshold decode
failure, and successful reset after a complete inbound pipeline. Add BDD
coverage only where existing inbound message assembly scenarios expose this
behaviour.

Run the same three Makefile gates and `coderabbit review --agent`, then commit.

### Milestone 3: route app builder transitions through one rebuild path

Add a `WireframeApp` rebuild helper for changing the connection-context generic
parameter `C`, modelled on the existing `rebuild_with_params` helper. Refactor
`on_connection_setup` to use it instead of manually constructing `WireframeApp`.

Preserve existing `on_disconnect` behaviour unless analysis proves it is
invalid. If teardown cannot be preserved safely with the current type model,
document the ordering consequence in rustdoc and in `docs/developers-guide.md`.

Add tests that configure setup and teardown in both orders that currently
compile, then assert the resulting lifecycle behaviour. Use `rstest` for the
matrix and BDD only if an existing lifecycle feature already models the
observable setup/teardown workflow.

Run the three Makefile gates and `coderabbit review --agent`, then commit.

### Milestone 4: refactor pool internals

Move duplicated poison-recovery locking from `src/client/pool/scheduler.rs` and
`src/client/pool/slot.rs` into a shared private pool sync module. Replace
`PooledClientLease`'s dispatch macro with a closure-based async helper that
keeps checkout, dispatch, recycle, and error mapping visible in ordinary Rust
control flow. Add `WireframeClientBuilder::into_parts()` and use it from both
connect paths.

Add `rstest` coverage for lock recovery, builder-parts construction, lease
dispatch success, and lease dispatch error/recycle behaviour. Existing pool BDD
scenarios should continue to pass; add a new scenario only if the helper change
exposes an externally observable pool behaviour that is not already covered.

Run the three Makefile gates and `coderabbit review --agent`, then commit.

### Milestone 5: update workspace membership and documentation

Add `wireframe_testing` explicitly to `Cargo.toml` `[workspace].members`. Update
`docs/developers-guide.md` so it no longer describes `wireframe_testing` as a
metadata nuance. The guide should explain when to use default-member commands,
`--workspace`, `-p wireframe-verification`, and `-p wireframe_testing`.

Update or add workspace manifest tests and the existing
`workspace_manifest.feature` scenarios so they assert the explicit membership.
Use `pretty_assertions` and `googletest` assertions in any new Rust tests where
they clarify failure output.

Run `make fmt` for Markdown formatting, then the three Makefile gates and
`coderabbit review --agent`, then commit.

### Milestone 6: extract example bootstrap and benchmark helper wiring

Extract the duplicated runtime bootstrap shape shared by
`examples/ping_pong.rs` and `examples/packet_enum.rs` into
`examples/support/runtime_bootstrap.rs`. Keep example-specific handler,
middleware, and app construction in each example.

Replace bench `#[path]` coupling with a stable shared helper path. Prefer a
normal shared module or existing helper crate over new public production API.
If the cleanest solution is to move helper code into `wireframe_testing`,
document why the helper belongs in test infrastructure rather than production.

Add tests that compile or exercise the helper paths already covered by
`make test`, and use BDD only if the externally observable example or benchmark
contract is represented in existing feature files.

Run the three Makefile gates and `coderabbit review --agent`, then commit.

### Milestone 7: split fragment test helpers

Split `tests/common/fragment_helpers.rs` into smaller modules grouped by
responsibility, such as errors, config, transports, app/spawn helpers, and
assertions. Preserve the existing import surface through a facade module so
call-site churn stays limited.

Run fragment transport, partial-frame, memory-budget, and full test gates to
confirm the split is mechanical. Add tests only if the split exposes an
untested helper contract; otherwise rely on existing integration and BDD
coverage because behaviour should not change.

Run the three Makefile gates and `coderabbit review --agent`, then commit.

## Follow-up GitHub issues

Create separate GitHub issues for audit findings that are not implemented by
this plan or that should remain independent of this broad refactor:

- Simplify pool waiter state transitions in `src/client/pool/scheduler.rs`.
- Avoid allocation in `ClientPoolInner::ordered_slots`.
- Enforce feature, scenario, and fixture naming consistency for BDD tests.

If implementation reveals additional deferred concerns, add them to this list
and create issues before marking the plan complete.

## Validation

Every implementation milestone must run the following commands sequentially:

```sh
make check-fmt 2>&1 | tee /tmp/check-fmt-wireframe-code-base-audit-2026-06-05.out
make lint 2>&1 | tee /tmp/lint-wireframe-code-base-audit-2026-06-05.out
make test 2>&1 | tee /tmp/test-wireframe-code-base-audit-2026-06-05.out
coderabbit review --agent
```

Documentation-only edits must additionally run:

```sh
make fmt 2>&1 | tee /tmp/fmt-wireframe-code-base-audit-2026-06-05.out
make markdownlint 2>&1 | tee /tmp/markdownlint-wireframe-code-base-audit-2026-06-05.out
```

The final branch must have a clean `git status --short`, all gates passing,
CodeRabbit concerns cleared, and a pushed draft pull request.

## Outcomes & Retrospective

- 2026-06-05: Milestone 4 validation before CodeRabbit:
  `cargo test --test bdd_pool --features "advanced-tests pool"` passed after
  the lease dispatch helper was rewritten to use `AsyncFnOnce`;
  `cargo fmt --all`, `make check-fmt`, `make lint`, `make test`, and
  `make markdownlint` passed. Logs:
  `/tmp/test-bdd-pool-wireframe-code-base-audit-2026-06-05-m4-rerun4.out`,
  `/tmp/fmt-wireframe-code-base-audit-2026-06-05-m4.out`,
  `/tmp/check-fmt-wireframe-code-base-audit-2026-06-05-m4.out`,
  `/tmp/lint-wireframe-code-base-audit-2026-06-05-m4.out`,
  `/tmp/test-wireframe-code-base-audit-2026-06-05-m4.out`, and
  `/tmp/markdownlint-wireframe-code-base-audit-2026-06-05-m4.out`.
- 2026-06-05: CodeRabbit's first Milestone 4 review requested direct
  `lock_or_recover` tests. After adding and adjusting them, validation passed:
  `/tmp/test-pool-sync-wireframe-code-base-audit-2026-06-05-m4-rerun1.out`,
  `/tmp/check-fmt-wireframe-code-base-audit-2026-06-05-m4-rerun4.out`,
  `/tmp/lint-wireframe-code-base-audit-2026-06-05-m4-rerun4.out`,
  `/tmp/test-wireframe-code-base-audit-2026-06-05-m4-rerun4.out`, and
  `/tmp/markdownlint-wireframe-code-base-audit-2026-06-05-m4-rerun4.out`.
- 2026-06-05: CodeRabbit's second Milestone 4 review requested recovery
  contract documentation, warning-level observability, singular module
  wording, and stronger poison assertions. After applying those changes,
  validation passed:
  `/tmp/check-fmt-wireframe-code-base-audit-2026-06-05-m4-rerun5.out`,
  `/tmp/lint-wireframe-code-base-audit-2026-06-05-m4-rerun5.out`,
  `/tmp/test-wireframe-code-base-audit-2026-06-05-m4-rerun5.out`, and
  `/tmp/markdownlint-wireframe-code-base-audit-2026-06-05-m4-rerun6.out`.
- 2026-06-05: CodeRabbit's third Milestone 4 review requested `-ise` spelling
  for `Synchronization`. The module comment now avoids that word. Validation
  passed:
  `/tmp/check-fmt-wireframe-code-base-audit-2026-06-05-m4-rerun6.out`,
  `/tmp/lint-wireframe-code-base-audit-2026-06-05-m4-rerun6.out`,
  `/tmp/test-wireframe-code-base-audit-2026-06-05-m4-rerun6.out`, and
  `/tmp/markdownlint-wireframe-code-base-audit-2026-06-05-m4-rerun8.out`.
- 2026-06-05: CodeRabbit's fourth Milestone 4 review requested tighter helper
  visibility and a poison-recovery metric. After applying both changes and
  documenting the metric in `docs/developers-guide.md`, validation passed:
  `/tmp/check-fmt-wireframe-code-base-audit-2026-06-05-m4-rerun7.out`,
  `/tmp/lint-wireframe-code-base-audit-2026-06-05-m4-rerun7.out`,
  `/tmp/test-wireframe-code-base-audit-2026-06-05-m4-rerun7.out`, and
  `/tmp/markdownlint-wireframe-code-base-audit-2026-06-05-m4-rerun10.out`.
- 2026-06-05: CodeRabbit's fifth Milestone 4 review requested module-level
  recovery policy text and either deeper integration coverage or a tracked
  issue. Added module policy text, referenced issue #539 from the unit test
  rationale, and created <https://github.com/leynos/wireframe/issues/539>.
  Validation passed:
  `/tmp/check-fmt-wireframe-code-base-audit-2026-06-05-m4-rerun8.out`,
  `/tmp/lint-wireframe-code-base-audit-2026-06-05-m4-rerun8.out`,
  `/tmp/test-wireframe-code-base-audit-2026-06-05-m4-rerun8.out`, and
  `/tmp/markdownlint-wireframe-code-base-audit-2026-06-05-m4-rerun12.out`.
- 2026-06-05: CodeRabbit's sixth Milestone 4 review requested direct
  observability assertions for poison recovery. Added log and metric checks to
  the unit test. Validation passed:
  `/tmp/test-pool-sync-wireframe-code-base-audit-2026-06-05-m4-rerun3.out`,
  `/tmp/check-fmt-wireframe-code-base-audit-2026-06-05-m4-rerun9.out`,
  `/tmp/lint-wireframe-code-base-audit-2026-06-05-m4-rerun9.out`,
  `/tmp/test-wireframe-code-base-audit-2026-06-05-m4-rerun9.out`, and
  `/tmp/markdownlint-wireframe-code-base-audit-2026-06-05-m4-rerun14.out`.
- 2026-06-05: Milestone 4 CodeRabbit review passed with zero findings:
  `/tmp/coderabbit-wireframe-code-base-audit-2026-06-05-m4-rerun6.out`.
- 2026-06-05: Milestone 5 validation before CodeRabbit:
  `cargo test --test workspace_manifest --all-features`,
  `cargo test --test bdd --all-features workspace_manifest`, `make check-fmt`,
  `make lint`, `make test`, and `make markdownlint` passed. Logs:
  `/tmp/test-workspace-manifest-wireframe-code-base-audit-2026-06-05-m5.out`,
  `/tmp/test-bdd-workspace-manifest-wireframe-code-base-audit-2026-06-05-m5.out`,
  `/tmp/check-fmt-wireframe-code-base-audit-2026-06-05-m5.out`,
  `/tmp/lint-wireframe-code-base-audit-2026-06-05-m5.out`,
  `/tmp/test-wireframe-code-base-audit-2026-06-05-m5.out`, and
  `/tmp/markdownlint-wireframe-code-base-audit-2026-06-05-m5.out`.
- 2026-06-05: Milestone 5 CodeRabbit review passed with zero findings:
  `/tmp/coderabbit-wireframe-code-base-audit-2026-06-05-m5.out`.
- 2026-06-05: Milestone 6 validation before CodeRabbit:
  `cargo test --test codec_performance_benchmark_helpers --all-features`,
  `cargo check --examples --all-features`,
  `cargo test --test bdd --all-features codec_performance_benchmarks`,
  `make check-fmt`, `make lint`, `make test`, and `make markdownlint` passed.
  Logs:
  `/tmp/test-codec-benchmark-helpers-wireframe-code-base-audit-2026-06-05-m6.out`,
  `/tmp/check-examples-wireframe-code-base-audit-2026-06-05-m6.out`,
  `/tmp/test-bdd-codec-benchmarks-wireframe-code-base-audit-2026-06-05-m6.out`,
  `/tmp/check-fmt-wireframe-code-base-audit-2026-06-05-m6-rerun1.out`,
  `/tmp/lint-wireframe-code-base-audit-2026-06-05-m6-rerun1.out`,
  `/tmp/test-wireframe-code-base-audit-2026-06-05-m6-rerun1.out`, and
  `/tmp/markdownlint-wireframe-code-base-audit-2026-06-05-m6.out`.
- 2026-06-05: Milestone 6 CodeRabbit review passed with zero findings:
  `/tmp/coderabbit-wireframe-code-base-audit-2026-06-05-m6.out`.
