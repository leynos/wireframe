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
- [ ] Milestone 2: centralize app outbound encoding and inbound pipeline
  failure policy.
- [ ] Milestone 3: route app builder transitions through one rebuild path.
- [ ] Milestone 4: refactor pool lock recovery, builder parts construction, and
  pooled lease dispatch.
- [ ] Milestone 5: update workspace membership and documentation.
- [ ] Milestone 6: extract example bootstrap and benchmark helper wiring.
- [ ] Milestone 7: split fragment test helpers behind a compatibility facade.
- [ ] Run final gates, CodeRabbit review, push, and update the pull request.

## Surprises & Discoveries

- `cargo metadata` reports `wireframe_testing` as a workspace member even
  though the root manifest currently lists only `.` and
  `crates/wireframe-verification` in `[workspace].members`. The existing
  developers' guide documents this as a nuance. This plan will replace that
  nuance with explicit membership.
- `googletest`, `pretty_assertions`, and `insta` are not currently declared in
  the root manifest. Tests will add only the dependencies that are used by
  implemented changes.
- Global `make fmt` currently fails on unrelated pre-existing Markdown lint
  violations outside this plan file. Targeted Markdown lint for this plan file
  passes.
- `cargo fmt --workspace` is not supported by the installed Cargo formatter
  wrapper. Use `cargo fmt --all` or the repository's `make check-fmt` target
  for Rust formatting in this worktree.

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

This section is empty while the plan is in draft. During implementation, record
what changed, which validation commands passed, CodeRabbit outcomes, issue
links, and any deviations from the original plan.
