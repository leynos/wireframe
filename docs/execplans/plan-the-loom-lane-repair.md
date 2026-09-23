# Make the Loom lane run the models, and make the models able to fail

This ExecPlan (execution plan) is a living document. The sections `Constraints`,
`Tolerances`, `Risks`, `Progress`, `Surprises & discoveries`, `Decision log`,
`Outcomes & retrospective`, `Conformance basis`, and `Verification plan` must
be kept up to date as work proceeds.

Status: IN PROGRESS. Delivered in this pull request as one change of achievable
checks plus an RFC, on the user's ruling of 2026-09-23 (D-1).

The ruling: refocus the lane on the synchronization around the connection
actor's write loop that Loom can model, drop the assertions that run through
Tokio channels Loom cannot schedule, correct the two design documents'
queue-full claims, and propose the rest in an RFC. That RFC is
[RFC 0002](../rfcs/0002-model-checking-the-write-loop.md). The milestones below
record what this change delivers and what it hands to the RFC.

## Purpose / big picture

The scheduled **Advanced Tests** lane exists to model-check the push queue's
behaviour under concurrent producers. It has never done so. It fails while
compiling the library, before any model runs, and has failed every night since
at least 2026-09-09.

Repairing only that compile failure would turn the lane green while checking
almost nothing, because two further defects mean the models do not exercise the
state they assert about. A green lane that establishes nothing is worse than a
red one, because a red lane advertises its own uselessness and a green one does
not.

After this work a reader can see:

- the lane running the four push-queue models rather than failing to compile;
- a statement in the developers' guide naming exactly which synchronization
  primitives the models schedule and which they do not, so nobody has to
  re-derive it;
- every assertion in the model file proved able to fail, by a recorded
  mutation that the model rejects for the intended reason.

Until the last of those, the lane's honest description is "a compile check for
the `cfg(loom)` configuration", and the guide will say so in those words.

### Terms used here

**Loom** is a library that runs a piece of concurrent code many times, choosing
a different interleaving of threads each time, to find orderings that break an
assertion. It can only do that for threads it created and for synchronization
primitives it supplies, such as `loom::sync::Mutex`. A primitive it did not
supply is invisible: Loom cannot pause a thread inside it, so orderings
involving it are never explored.

**A model** is one closure passed to `loom::model`, which Loom runs repeatedly
under different schedules.

**`cfg(loom)`** is a compile-time configuration flag. The lane sets it with
`RUSTFLAGS="--cfg loom"`, which builds a different version of the crate in
which Loom's primitives replace the standard ones.

**A dead-letter queue**, abbreviated DLQ, is where this crate sends frames it
had to discard because a queue was full.

## Context and orientation

Assume no prior knowledge of this repository.

The lane is `.github/workflows/advanced-tests.yml`. It runs nightly at 17:00
UTC and its only command is:

```bash
RUSTFLAGS="--cfg loom" cargo test --features advanced-tests
```

The models live in one file, `tests/advanced/concurrency_loom.rs`, declared in
`Cargo.toml` as the `concurrency_loom` test target. It contains four models,
all exercising `PushHandle`:

- `concurrent_drops_reset_dlq_counter`, parameterized over two priorities,
- `concurrent_queue_full_errors_are_reported`,
- `dlq_probe_ignores_absent_channel`,
- `dlq_probe_reports_zero_when_dlq_idle`.

The subject is `src/push/queues/`, principally `handle.rs`. `PushHandle` is a
cloneable producer handle wrapping `PushHandleInner`, which holds two Tokio
channels for high- and low-priority frames, an optional Tokio channel for the
DLQ, an optional `leaky_bucket::RateLimiter`, a counter `dlq_drops`, and a mutex
`dlq_last_log` guarding the time of the last log line.

`handle.rs` already carries the conventional pattern for a Loom build: a private
`sync` module aliasing types differently under each configuration.

The test helpers are in `src/test_helpers.rs` and
`src/test_helpers/pool_client.rs`. The latter is a TCP client helper.

## Constraints

- `PushHandle` and `PushQueues` are public API. Their signatures under
  `cfg(not(loom))` must not change, and none did.
- The pinned toolchain is `nightly-2026-03-26`, and Loom stays at `0.7.2`.
- No production behaviour changes. The only source edit is a configuration
  predicate on `test_helpers::pool_client`.
- No Tokio channel is substituted under the model (D-1): the tested object and
  the shipped object must not differ where the models look.
- The scheduled lane stays GitHub-hosted, per the estate placement rule.

## Tolerances (exception triggers)

- **Scope**: this change touches the configuration predicate, a new models
  package, the lane, its contract, the Makefile and documentation. Anything
  wider, such as widening the `sync` alias or extracting the scheduling
  decision, belongs to RFC 0002.
- **Dependencies**: no new external dependency. The models package uses Loom,
  rstest and Tokio at the versions the workspace already resolves. The `syn`
  exception the previous draft declared is withdrawn with V-2 (D-4).
- **Defects found**: a model that exposes a behaviour question in the subject
  is recorded and escalated, not fixed here. One was (see
  `Surprises & discoveries`).

## Risks

- **R-1**: The models pass while exploring one schedule. Mitigated: replacing
  the counter's `fetch_add` with a separate load and store is rejected, which
  only an explored interleaving can show (V-4, L1).
- **R-2**: A model finds a real defect in the push queue. It did, in a scratch
  three-producer model; recorded and referred to RFC 0002 rather than fixed.
- **R-3**: The lane's command needed narrowing. Answered by EP-M1: after the
  library repair, `wireframe_testing` failed to build under `cfg(loom)`, a
  separate target-selection failure. Resolved without editing the harness by
  moving the models to their own package (D-5).
- **R-4**: The lane goes green while running nothing. Mitigated by the
  workflow contract (V-1); an executed-count guard is proposed in RFC 0002.

## Progress

- [x] (2026-09-17) Read-only diagnosis, and this plan drafted.
- [x] (2026-09-18 to 2026-09-22) Four review rounds on the plan.
- [x] (2026-09-23) D-1 ruled by the user: refocus, drop the channel
  assertions, correct the documents, one change plus an RFC.
- [x] (2026-09-23) EP-M1: baseline reproduced, and R-3 answered.
- [x] (2026-09-23) EP-M2: configuration repair, models package, lane and
  contract.
- [x] (2026-09-23) EP-M3: models rewritten onto Loom-visible state and
  mutation-proved.
- [x] (2026-09-23) EP-M4: documents corrected, guide statement, RFC 0002.
- [ ] The first scheduled run of the lane on `main`, recorded here once it has
  run.

## Surprises & discoveries

Three findings, from the read-only diagnosis. The first was already recorded in
issue #683; the second and third were not, and they change what the work is.

- **Observation**: The helper's configuration predicate admits a build in
  which the types it imports do not exist. Evidence: `src/test_helpers.rs` gates
  `pub mod pool_client` on `#[cfg(feature = "pool")]`. `src/lib.rs` gates
  `pub mod client` on `#[cfg(not(loom))]`, and `tokio::net` is
  `#![cfg(not(loom))]` inside Tokio. `Cargo.toml` line 60 declares a self
  dev-dependency
  `wireframe = { path = ".", features = ["test-support", "pool", "testkit"] }`,
  so feature unification enables `pool` in the library build even though the
  lane requests only `advanced-tests`. Runs 35143307834, 35017357350 and
  34895037905 each fail with the same eight errors and exit 101. Impact: the
  models never run.

  An earlier draft claimed a second layer, that the `bdd_pool` target would
  also fail to compile because feature unification selects it. **That claim is
  withdrawn.** `tests/bdd_pool/mod.rs` opens with
  `#![cfg(all(not(loom), feature = "pool"))]`, so under `--cfg loom` it is an
  empty crate that compiles cleanly: selected, and harmless. Whether the lane's
  command needs narrowing at all is therefore an open question that EP-M1's
  reproduction answers, not something this plan asserts. The library failure is
  the only one there is evidence for.

- **Observation**: Almost nothing the models touch is instrumented.
  Evidence: the `cfg(loom)` arm of the `sync` module in
  `src/push/queues/handle.rs` re-exports `std::sync::{Arc, Weak}`, aliasing only
  `Mutex` and `AtomicUsize` to Loom. The models clone the handle into every
  thread they spawn, so every reference-count operation is outside the schedule.
  `PushHandleInner` holds `tokio::sync::mpsc::Sender` for all three channels,
  and the models construct `tokio::sync::mpsc::channel` inside `loom::model`.
  `try_push` is a synchronous path over `tokio::sync::mpsc::try_send`. Impact:
  the `QueueFull` assertions in `concurrent_queue_full_errors_are_reported`
  exercise Tokio's channel under three threads Loom believes it controls. The
  Loom-visible surface of the whole subject is one `AtomicUsize` and one
  `Mutex`.

- **Observation**: Those two instrumented fields are on a path no model
  reaches, and one assertion is therefore unfalsifiable. Evidence: `dlq_drops`
  and `dlq_last_log` are written only inside `route_to_dlq`, in the branch
  taken when the DLQ channel's `try_send` returns `Full` or `Closed`.
  `concurrent_drops_reset_dlq_counter` builds the DLQ with capacity 4 and drops
  two frames, so both sends succeed and the branch is not entered. The reset
  the assertion names, `dlq_drops.store(0, ...)` in `log_dlq_drop`, is
  unreachable from every model in the file. Impact:
  `assert_eq!(probe.dlq_drop_count(), 0, "counter should reset after
  reaching the logging threshold")`
  passes because the counter was never incremented, not because it reset. No
  interleaving Loom can choose makes it fail. Repairing only the compile
  failure would turn the lane green on this.

Found while implementing, on 2026-09-23:

- **Observation**: the build failure has changed since the diagnosis. On the
  pinned toolchain,
  `RUSTFLAGS="--cfg loom" cargo test --features advanced-tests --no-run` failed
  with five errors, all in `src/test_helpers/pool_client.rs` (`E0432` and
  `E0433` on `crate::client` and `tokio::net`), not eight. Evidence: the local
  log kept with this change.
- **Observation**: R-3 has an answer. With `pool_client` gated, the library
  builds and `wireframe_testing` fails instead, with seven errors in
  `client_pair.rs` and `integration_helpers.rs`: the root package's test build
  compiles its dev-dependencies, and that harness needs the TCP client and
  server. Impact: the models moved to their own package (D-5).
- **Observation**: the drop counter's reset is not exact. A scratch model with
  three concurrent drops and a threshold of two failed, with Loom reporting a
  final count of 3 where a lossless reset would leave 1. Impact: none on this
  change's models, which use two producers; referred to RFC 0002 as a behaviour
  decision.
- **Observation**: the write loop has no Loom-visible synchronization of its
  own. `next_event` selects over Tokio receivers and a `tokio_util`
  cancellation token, and Tokio uses Loom internally only for its own tests.
  Impact: the Loom-visible part of the write loop's path is the push handle's
  shared counter and mutex, which is where the models now look.

## Decision log

- **D-1 (RULED 2026-09-23)**: what happens to the message channel under the
  model. The user ruled: refocus the lane on the write loop's synchronization
  where Loom can model it, drop the assertions that run through Tokio channels
  Loom cannot schedule, correct the two documents' queue-full claims, and keep
  it to one change of achievable checks plus an RFC. This is route (b) of the
  earlier draft, and it retires route (a), the Loom-visible channel.
  Consequence: `concurrent_queue_full_errors_are_reported` is removed, and so
  are the channel-receive assertions in the other models.
- **D-2 (superseded)**: the plan-only pull request. The ruling asked for the
  checks in the same change.
- **D-3**: the nightly cost of a red lane is not addressed; the lane is
  GitHub-hosted and scheduled.
- **D-4 (superseded)**: V-2's `syn`-based source contract is deferred to RFC
  0002. It is achievable, but it is a checker of its own, and the ruling asked
  for one change of achievable checks. The configuration repair is covered by
  V-1 in the meantime.
- **D-5**: the models live in their own package, `crates/wireframe-loom`.
  Rationale: the root package's test build compiles `wireframe_testing`, whose
  TCP harness cannot exist under `cfg(loom)`, and its self dev-dependency turns
  on `pool` by unification. Gating the harness module by module would spread
  `cfg(loom)` through a test-support crate for a configuration it never serves;
  a package whose only dependencies are the library, Loom, rstest and Tokio's
  `sync` feature needs neither.
- **D-6**: handle and registry lifetimes are not modelled. Loom 0.7.2 has no
  `Weak` (`loom::sync::Arc` has no `downgrade`), and the session registry is
  built on `Weak<PushHandleInner>` from `Arc::downgrade`, inside a `DashMap`
  whose locks are not Loom primitives. Widening the handle's `Arc` to Loom's
  would break the registry rather than model it. Referred to RFC 0002.
- **D-7**: the models assert on Loom primitives only: the drop counter, and
  the active-connection gauge. The Tokio channels are used to reach the state
  under test and never asserted on.
- **D-8 (2026-09-23, lead review of the shape)**: the active-connection gauge
  is modelled. Under `cfg(loom)` the gauge is a `loom::lazy_static!` atomic,
  since Loom's atomics have no `const` constructor, and
  `wireframe::connection::LoomConnectionGuard` exposes the actor's own guard to
  the models, the same pattern as `PushHandle::probe`. Neither exists in an
  ordinary build.

## Outcomes & retrospective

The lane now runs six Loom test functions from `make test-loom`, eight model
executions once the two parameterized dead-letter functions run for both
priorities, and each assertion is shown able to fail. What they verify is small
and stated: the dead-letter drop counter and log mutex under two concurrent
producers (six executions), and the active-connection gauge under two
concurrent actors (two). The write loop's own ordering is not Loom's to check,
and the documents that said otherwise are corrected.

Lesson: the question to ask first of a Loom lane is not "does it compile" but
"which of the subject's primitives are Loom's". Here the answer was two fields,
and every assertion outside them was coverage that could not fail.

## Conformance basis

**Correction, 2026-09-22.** An earlier version of this section said there was
no design document for this lane and that the plan's only upstream artefacts
were the issue and the code. That was wrong, and it was wrong because the
search was for an architecture decision record and a Terms of Reference rather
than for the subject. Two existing documents govern what Loom is for here:

- `docs/multi-layered-testing-strategy.md` §4.2, "Concurrency Fuzzing with
  `loom`". It names the **target area** as the connection actor write loop, the
  `select!(biased; ...)` logic, sketches a model spawning concurrent
  high-priority and low-priority producers against a `MockConnection`, and sets
  the measurable objective: all permutations for two to three concurrent
  producers, with no data race and no deadlock.
- `docs/formal-verification-methods-in-wireframe.md`, the "Keep Loom" section
  and its opening survey. It assigns Loom "real synchronization interleavings
  over concrete concurrent code", assigns abstract protocol-state interleavings
  to Stateright, and describes the current models as exploring `PushQueues`
  interleavings and checking "dead-letter queue accounting and queue-full
  behaviour under concurrent producers".

Two things follow, and neither is this plan's to decide.

**The design's target is not what the lane models.** §4.2 names the write loop's
`select!` arm ordering. The models in `tests/advanced/concurrency_loom.rs`
model the push queues instead. The `MockConnection` its example uses does not
exist in the tree. So the lane is not a partial implementation of that design;
it is a different, smaller thing that grew beside it.

**The design asserts coverage that finding two shows is absent.** Both
documents state that the models check queue-full behaviour. They do not: the
`QueueFull` assertions run through a `tokio::sync::mpsc` channel that Loom
cannot schedule, so no interleaving Loom chooses affects them. D-1's route (b)
therefore removes an assertion two documents claim exists, which makes route
(b) a documentation change as well as a test change, and the guide statement in
EP-M3 has to say which document it corrects.

That conflict is referred alongside D-1 rather than resolved here. This plan
repairs a lane that does not run; deciding whether the lane should instead be
rebuilt against §4.2's write-loop target is a larger question with a design
document behind it, and answering it inside a repair would be deciding it by
implication.

- Issue #683, "Fix pool-client test-helper cfg leakage blocking daily Loom
  execution", opened 2026-09-08, which records finding one. Findings two and
  three are new here and should be added to that issue or to this plan's
  successor, not left only in a review comment.
- The models' own stated intent, in the docstring of
  `tests/advanced/concurrency_loom.rs`: "loom explores interleavings to ensure
  DLQ accounting and queue-full errors remain deterministic under concurrent
  producers." Finding two shows the shipped configuration does not meet that
  claim, so either the claim or the configuration must move. D-1 is exactly
  that choice.

Traceability, such as it is:

```plaintext
#683 -> EP-M2 -> lane compiles and models execute
finding two -> D-1 -> EP-M3 -> docs/developers-guide.md scheduling statement
finding three -> EP-M4 -> every assertion mutation-proved
```

**Resolved in this change (2026-09-23).** Both documents are corrected, per the
ruling: `multi-layered-testing-strategy.md` §4.2 now states what Loom can and
cannot see and points the write loop's ordering at the deterministic tests, the
Stateright model and RFC 0002; the formal-verification document's three Loom
claims now say dead-letter accounting and withdraw queue-full. The asynchronous
outbound messaging design's Loom paragraph and the roadmap entry are brought
into line too.

## Verification plan

**V-1: the lane runs the models, under `--cfg loom`, bounded.** Artefact:
`tests/workflow_contracts/loom_lane_test.py`, reading the workflow and the
target through `make --dry-run`, tokenized with shell punctuation. Evidence:
five tests pass. Non-vacuity: eleven mutations, each failing the test that
names it, and one equivalent rewrite passing:

```plaintext
W1 echo instead of make                  test_the_lane_calls_the_loom_target
W2 chained after the target              test_the_lane_calls_the_loom_target
W3 step timeout dropped                  test_the_loom_step_is_bounded
W4 old unscoped command restored         test_the_lane_calls_the_loom_target
M1 compile only (--no-run)               test_the_target_runs_the_models_rather_than_compiling_them
M2 loom configuration dropped            test_the_target_selects_the_loom_configuration
M3 wrong package                         test_the_target_runs_the_models_rather_than_compiling_them
M4 status piped away                     test_the_target_runs_the_models_rather_than_compiling_them
M5 status chained away, no space         test_the_target_runs_the_models_rather_than_compiling_them
M6 preemption bound zeroed               test_the_target_bounds_loom_exploration
M7 preemption bound dropped              test_the_target_bounds_loom_exploration
N1 bound spelled through a variable      <none: equivalent, passes>
```

**V-2 (deferred to RFC 0002)**: the configuration-boundary contract.

**V-3: the guide states what Loom schedules.** Artefact: the "Loom models"
section of `docs/developers-guide.md`, naming each field of `PushHandleInner`
and whether Loom schedules it. A review checkpoint, not a test, for the reason
the earlier draft gave.

**V-4: every model assertion can fail.** `make test-loom` passes six test
functions and eight model executions: four functions and six executions over
the dead-letter accounting (two functions parameterized over both priorities),
and two over the active-connection gauge. Each mutation below is applied to
`src/push/queues/handle.rs` alone and restored from a copy:

```plaintext
L1 fetch_add as a separate load and store   concurrent_drops_are_all_counted (both),
                                            the_counter_resets_at_the_logging_threshold (both)
L2 increment suppressed                     concurrent_drops_are_all_counted (both)
L3 reset suppressed                         the_counter_resets_at_the_logging_threshold (both)
L4 every drop counted, sent or not          drops_into_a_dead_letter_queue_with_room_count_nothing, and the four above
L5 counted without a dead-letter queue      drops_without_a_dead_letter_queue_count_nothing
```

And four of `src/connection/counter.rs`, against
`crates/wireframe-loom/tests/connection_gauge.rs`:

```plaintext
G1 increment as a separate load and store   concurrent_guards_return_the_gauge_to_zero, live_guards_are_all_counted
G2 decrement as a separate load and store   concurrent_guards_return_the_gauge_to_zero
G3 decrement suppressed                     concurrent_guards_return_the_gauge_to_zero, live_guards_are_all_counted
G4 increment suppressed                     concurrent_guards_return_the_gauge_to_zero, live_guards_are_all_counted
```

G2 fails one model only, and correctly: `live_guards_are_all_counted` drops its
guards on one thread, so there is no interleaving for a lost decrement to hide
in.

L1 is the evidence that the models explore interleavings: a lost update is
visible only on a schedule where the two producers' reads and writes
interleave. The increment and the reset are proved separately, with a threshold
of three and of two, so a suppressed increment cannot hide behind a suppressed
reset as it could in the model this replaces.

## Plan of work

Stage A, understand and propose, is the diagnosis and the plan's four review
rounds. Stage B, red first, is EP-M1's reproduction. Stages C and D are the
milestones below, each validated before the next, with the repository's gates
run at the end.

## Milestones and plateaus

**EP-M1: reproduce and bound. Done.** The build failure reproduced on the
pinned toolchain (five errors, `pool_client` only). With the library repaired,
`wireframe_testing` failed next, which answered R-3.

**EP-M2: the configuration boundary, and models that run. Done.**
`test_helpers::pool_client` and its re-exports are gated on `not(loom)` as well
as `pool`. The models moved to `crates/wireframe-loom` (D-5). `make test-loom`
runs them, `advanced-tests.yml` runs only that target with a 30-minute step
timeout, and V-1's contract holds both.

**EP-M3: models that can fail. Done.** The gauge models (D-8) were added on the
lead's review of the shape; handle and registry lifetimes were not, for the
reason in D-6. `concurrent_queue_full_errors_are_reported` is removed (D-1).
The dead-letter models now reach `route_to_dlq`'s error branch by filling a
one-frame dead-letter queue first, prove the increment and the reset separately
for both priorities, and keep the two zero-count cases as the narrowness half:
a drop the dead-letter queue accepted, and no dead-letter queue at all. V-4's
table is filled.

**EP-M4: the statement, the documents and the RFC. Done.** The guide section
(V-3), the corrected documents (`Conformance basis`), and RFC 0002 for the
write loop, the `Arc` seam, the counter's reset, V-2 and an executed-count
guard.

Remaining gaps: everything in RFC 0002, and four dead-code warnings the library
emits under `cfg(loom)`: server metrics helpers and `ServerCancellationReason`,
whose only callers are in the `cfg(not(loom))` server module. They predate this
change and do not fail the lane, which does not deny warnings; gating them on
`not(loom)` is a small follow-up. The one warning on the Loom surface itself,
an undocumented field of `PushHandleProbe`, is fixed here.
