# Exercise interleaved high- and low-priority push queues

This Execution Plan (ExecPlan) is a living document. The sections
`Constraints`, `Tolerances`, `Risks`, `Progress`, `Surprises & Discoveries`,
`Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work
proceeds.

Status: COMPLETE

This document must be maintained in accordance with `AGENTS.md` at the
repository root, including all quality gates, commit conventions, and code
style requirements defined therein.

## Purpose / big picture

The wireframe server's connection actor uses a biased `tokio::select!` loop
that polls a shutdown token, high-priority push queue, low-priority push queue,
multi-packet channel, and response stream — in that order. A `FairnessConfig`
mechanism prevents high-priority traffic from starving low-priority frames by
forcing a yield after a configurable burst threshold or time slice. Both
priority queues share a single `leaky_bucket::RateLimiter` token bucket,
meaning push throughput is capped globally regardless of which queue a producer
uses.

All of this machinery already exists and passes basic tests. What is missing is
a comprehensive test suite that exercises interleaved concurrent traffic on
both queues and proves three properties:

1. Fairness: low-priority frames are eventually delivered even during sustained
   high-priority bursts, at the threshold configured by `FairnessConfig`.
2. Rate-limit symmetry: the shared rate limiter enforces identical throughput
   caps whether tokens are consumed by high-priority pushes, low-priority
   pushes, or an interleaved mix of both.
3. Completeness: no frames are lost when both queues carry traffic
   simultaneously under various fairness and rate-limit configurations.

Observable outcome: running `make test` passes, including new unit tests in
`tests/interleaved_push_queues.rs` and new Behaviour-Driven Development (BDD)
scenarios in `tests/features/interleaved_push_queues.feature`. The existing
test suite remains green. The roadmap entry 10.3.2 is marked as done.

## Constraints

- No new external crate dependencies may be added.
- Existing public API signatures in `src/push/`, `src/connection/`, and
  `src/fairness.rs` must not change.
- All code must pass `make check-fmt`, `make lint`, `make test`, and
  `make markdownlint`.
- Documentation must use en-GB-oxendict spelling per `AGENTS.md`.
- No single source file may exceed 400 lines per `AGENTS.md`.
- BDD tests must use `rstest-bdd` 0.5.0.
- The `Packet` impl for `u8` in `src/connection/test_support.rs` is the
  established frame type for connection actor tests; reuse it.

## Tolerances (exception triggers)

- Scope: if implementation requires changes to more than 15 files or 400 lines
  of code (net), stop and escalate.
- Interface: if an existing public API signature must change, stop and
  escalate.
- Dependencies: if a new external crate dependency is required, stop and
  escalate.
- Iterations: if tests still fail after 5 attempts at a fix, stop and
  escalate.
- Ambiguity: if "symmetrical rate limits" or "fairness" requires a definition
  beyond what the existing `FairnessConfig` and shared `RateLimiter` provide,
  stop and present options.

## Risks

- Risk: Virtual-time tests using `tokio::time::pause()` and
  `tokio::time::advance()` can be sensitive to scheduling order, causing
  flakiness. Severity: medium. Likelihood: low. Mitigation: use `#[serial]`
  where timing is critical; keep virtual time advances large relative to the
  rate limiter's 10 ms poll interval; follow the pattern established in
  `tests/connection_actor_fairness.rs`.

- Risk: BDD step wording may collide with existing step definitions from other
  feature files. Severity: low. Likelihood: low. Mitigation: use
  scenario-specific prefixes (e.g. "interleaved") in step text and review
  existing steps before authoring.

- Risk: New test file may push the BDD module wiring close to the 400-line
  limit. Severity: low. Likelihood: low. Mitigation: the BDD entry point
  (`tests/bdd/mod.rs`) is currently 29 lines; adding one more fixture module is
  well within budget.

## Progress

- [x] Stage A: Unit tests — interleaved push queue test file. 8 tests
  passing.
- [x] Stage B: BDD behavioural tests — feature file, fixture, steps,
  scenarios. 4 BDD scenarios passing (98 total BDD tests green).
- [x] Stage C: Documentation — update design doc, users guide, roadmap.
- [x] Stage D: Validation — full quality gate pass. All four gates
  (`check-fmt`, `lint`, `test`, `markdownlint`) exit 0.

## Surprises & discoveries

- Observation: BDD step functions that use `tokio::time::pause()` must
  use a `current_thread` runtime, not the default multi-thread runtime created
  by `tokio::runtime::Runtime::new()`. Evidence: the `rate_limit_symmetry` BDD
  scenario panicked with "`time::pause()` requires the `current_thread` Tokio
  runtime". Impact: the step function for the rate-limit scenario uses
  `tokio::runtime::Builder::new_current_thread()` instead of `Runtime::new()`.
  Other steps that do not use virtual time can continue to use the default
  runtime.

## Decision log

- Decision: Place all interleaved queue tests in a single new file
  (`tests/interleaved_push_queues.rs`) rather than extending the existing
  `tests/connection_actor_fairness.rs` or `tests/push.rs`. Rationale: the
  existing files cover different concerns (basic fairness and basic queue
  routing respectively); a dedicated file makes the interleaving-specific
  coverage easy to find and keeps each file focused. Date: 2026-02-21.

- Decision: Use `ConnectionActor::run()` as the integration boundary for
  fairness tests rather than testing `FairnessTracker` in isolation. Rationale:
  the tracker is already unit-tested in `src/fairness.rs::tests`; the value of
  10.3.2 is proving that the tracker integrates correctly with the actor's
  `select!` loop and drain logic. Date: 2026-02-21.

## Outcomes & retrospective

### Deliverables

- 8 unit tests in `tests/interleaved_push_queues.rs` covering rate-limit
  symmetry (3 tests), counter-based fairness (1), time-slice fairness (1),
  total throughput cap (1), frame completeness (1), and strict priority (1).
- 4 BDD scenarios in `tests/features/interleaved_push_queues.feature` with
  supporting fixture, steps, and scenario wiring modules.
- Design document updated with Section 13 documenting the interleaved queue
  testing strategy.
- Users guide updated with a note on interleaved push queue validation.
- Roadmap entry 10.3.2 marked as done.

### Metrics

- Files created: 5 (unit tests, feature file, fixture, steps, scenarios).
- Files modified: 6 (3 BDD mod.rs files, design doc, users guide, roadmap).
- Total new test count: 12 (8 unit + 4 BDD).
- All quality gates pass with 0 warnings.

### Retrospective

- The `current_thread` runtime requirement for `tokio::time::pause()` was the
  only unexpected friction. It was caught early during Stage B and documented
  in the Surprises section.
- Clippy's `stable_sort_primitive` lint caught `sort()` on `Vec<u8>` in both
  the unit test and BDD fixture; the fix was trivial (`sort_unstable()`).
- Separating rate-limit tests (queue-level, no actor) from fairness tests
  (actor-level) simplified ownership management and made the test intentions
  clearer.
- The rstest-bdd macro requires step function parameter names to match the
  fixture function name exactly — a detail not obvious from the documentation
  but easily caught by the runtime panic message.

## Context and orientation

### Repository structure (relevant files)

The wireframe crate lives at the repository root. Key paths for this task:

    src/push/queues/mod.rs         — PushQueues, PushQueueConfig,
                                     PushPriority, PushPolicy,
                                     FrameLike, recv() with biased
                                     select!
    src/push/queues/handle.rs      — PushHandle, push_high_priority,
                                     push_low_priority, try_push,
                                     wait_for_permit (rate limiter)
    src/push/queues/builder.rs     — PushQueuesBuilder (fluent API)
    src/fairness.rs                — FairnessConfig,
                                     FairnessTracker, Clock trait
    src/connection/mod.rs          — ConnectionActor, biased
                                     select! loop (next_event),
                                     set_fairness, run()
    src/connection/drain.rs        — after_high, after_low,
                                     try_opportunistic_drain,
                                     process_high, process_low
    src/connection/test_support.rs — Packet impl for u8, ActorHarness

    tests/push.rs                  — Existing push queue unit tests
    tests/connection_actor_fairness.rs
                                   — Existing fairness unit tests
    tests/rate_limiter_regression.rs
                                   — Rate limiter regression test
    tests/support.rs               — builder::<F>() helper

    tests/bdd/mod.rs               — BDD test entry point
    tests/features/                — Gherkin .feature files
    tests/fixtures/                — BDD fixture modules
    tests/steps/                   — BDD step definitions
    tests/scenarios/               — BDD scenario wiring

    wireframe_testing/src/lib.rs   — TestResult, push_expect!,
                                     recv_expect!

    docs/roadmap.md                — Roadmap (10.3.2 to mark done)
    docs/users-guide.md            — Users guide
    docs/multi-packet-and-streaming-responses-design.md
                                   — Design document

### Key types

`PushQueues<F>` (struct, `src/push/queues/mod.rs:71`): holds `high_priority_rx`
and `low_priority_rx` mpsc receivers. The `recv()` method uses a biased
`tokio::select!` preferring high-priority frames.

`PushHandle<F>` (struct, `src/push/queues/handle.rs:54`): clone-safe handle
wrapping an `Arc<PushHandleInner<F>>`. Provides `push_high_priority`,
`push_low_priority` (both async, rate-limited), and `try_push` (synchronous,
policy-controlled).

`FairnessConfig` (struct, `src/fairness.rs:12`): `max_high_before_low: usize`
(default 8) and `time_slice: Option<Duration>`.

`FairnessTracker` (struct, `src/fairness.rs:49`): tracks `high_counter` and
`high_start`. `record_high_priority()` increments;
`should_yield_to_low_priority()` checks threshold/time; `reset()` clears
counters.

`ConnectionActor<F, E>` (struct, `src/connection/mod.rs:78`): drives outbound
frame delivery. `run(&mut self, out: &mut Vec<F>)` polls sources in biased
order and appends frames to `out`.

### How fairness works in the connection actor

When a high-priority frame is processed, `after_high()` in
`src/connection/drain.rs:99` calls `self.fairness.record_high_priority()`. If
`should_yield_to_low_priority()` returns true, the actor calls
`try_opportunistic_drain(Low)` which does a non-blocking `try_recv()` on the
low-priority queue. If that succeeds, the frame is emitted and the fairness
counter resets. If the low queue is empty, it tries the multi-packet queue.
This means that during a sustained high-priority burst, a low-priority frame is
interleaved every `max_high_before_low` high frames.

### How rate limiting works

Both priority queues share a single `leaky_bucket::RateLimiter` configured in
`PushQueues::build_with_config` (`src/push/queues/mod.rs:151`). The
`push_with_priority` method in `src/push/queues/handle.rs:93` calls
`wait_for_permit(limiter)` before sending, regardless of priority. This means
any push from either queue consumes a token from the same bucket. The rate
limiter refills at the configured rate per second with burst capacity equal to
the rate.

## Plan of work

### Stage A: Unit tests

Create `tests/interleaved_push_queues.rs` with the following tests. All tests
use `u8` frames and the `ConnectionActor<u8, ()>` pattern.

A1. `rate_limit_symmetric_high_only` — Push N frames via high-priority only
with rate limit R=2. Use `tokio::time::pause()`. After the initial burst of 2,
the third push should block until `time::advance(1s)`. Drain via
`ConnectionActor::run()` and verify all frames arrive.

A2. `rate_limit_symmetric_low_only` — Same as A1 but push via low-priority
only. Verify the same blocking behaviour and frame count. This proves the rate
limiter treats both queues identically.

A3. `rate_limit_symmetric_mixed` — Push 1 high, then attempt 1 low. With rate
R=1, the low push should block because the high push already consumed the
token. Advance time, push again, and verify both arrive. This directly proves
the shared token bucket.

A4. `interleaved_fairness_yields_at_threshold` — Configure
`max_high_before_low = 3`. Pre-load 6 high-priority and 2 low-priority frames.
Run the actor and verify the output sequence is `[H, H, H, L, H, H, H, L]`
(low-priority interleaved every 3 high frames).

A5. `interleaved_all_frames_delivered` — Push a mix of high and low frames
(e.g. 5 high + 5 low) with fairness enabled (`max_high_before_low = 2`). Run
the actor and verify all 10 frames appear in `out`. This proves no frame loss
under interleaving.

A6. `interleaved_time_slice_fairness` — Configure `max_high_before_low = 0`
(counter disabled) with `time_slice = Some(10ms)`. Use virtual time. Push high
frames, advance past the time slice, push a low frame and more high frames.
Verify the low frame appears interleaved (not last).

A7. `rate_limit_interleaved_total_throughput` — With rate R=4 and unlimited
fairness, push 4 high + 4 low frames. The first 4 pushes (regardless of
priority) should succeed; the 5th should block. Advance time and push the
remainder. Verify all 8 frames arrive and that total throughput does not exceed
R per second across both queues combined.

A8. `fairness_disabled_strict_priority` — Configure
`max_high_before_low = 0, time_slice = None` (fairness disabled). Pre-load high
and low frames. Verify all high frames precede all low frames (strict biased
ordering).

### Stage B: BDD behavioural tests

B1. Feature file: create `tests/features/interleaved_push_queues.feature` with
a `@interleaved` tag and four scenarios:

1. "High-priority frames take precedence when fairness is disabled" — proves
   biased select ordering.
2. "Fairness yields to low-priority after burst threshold" — proves
   counter-based fairness.
3. "Rate limiting applies symmetrically across both priority levels" — proves
   shared token bucket.
4. "All frames are delivered when both queues carry traffic" — proves no frame
   loss.

B2. Fixture: create `tests/fixtures/interleaved_push_queues.rs` with an
`InterleavedPushWorld` struct and a `#[fixture]` constructor.

B3. Steps: create `tests/steps/interleaved_push_queues_steps.rs` with
`#[given]`, `#[when]`, `#[then]` step definitions.

B4. Scenarios: create `tests/scenarios/interleaved_push_queues_scenarios.rs`
with `#[scenario]` macros.

B5. Wiring: update `tests/fixtures/mod.rs`, `tests/steps/mod.rs`, and
`tests/scenarios/mod.rs` to include the new modules.

### Stage C: Documentation

C1. Update `docs/multi-packet-and-streaming-responses-design.md` to add a
subsection documenting the interleaved queue testing strategy and design
decisions.

C2. Update `docs/users-guide.md` — add a note to the "Push queues and
connection actors" section about interleaved push queue validation.

C3. Update `docs/roadmap.md` — change the 10.3.2 line from `- [ ]` to `- [x]`.

### Stage D: Validation

Run all quality gates and verify exit 0.

## Concrete steps

All commands run from the repository root (`/home/user/project`).

Stage A:

    # Create tests/interleaved_push_queues.rs with all unit tests
    # Verify compilation
    cargo check --all-targets --all-features
    # Run just the new tests
    set -o pipefail && cargo test --test interleaved_push_queues \
      2>&1 | tee /tmp/test-stage-a.log

    Expected: 8 tests pass.

Stage B:

    # Create tests/features/interleaved_push_queues.feature
    # Create tests/fixtures/interleaved_push_queues.rs
    # Create tests/steps/interleaved_push_queues_steps.rs
    # Create tests/scenarios/interleaved_push_queues_scenarios.rs
    # Edit tests/fixtures/mod.rs — add pub mod
    # Edit tests/steps/mod.rs — add mod
    # Edit tests/scenarios/mod.rs — add mod
    set -o pipefail && make test-bdd 2>&1 | tee /tmp/test-stage-b.log

    Expected: all BDD scenarios pass, including the 4 new ones.

Stage C:

    # Edit docs/multi-packet-and-streaming-responses-design.md
    # Edit docs/users-guide.md
    # Edit docs/roadmap.md
    make markdownlint

Stage D:

    set -o pipefail && make check-fmt 2>&1 | tee /tmp/gate-fmt.log
    set -o pipefail && make lint 2>&1 | tee /tmp/gate-lint.log
    set -o pipefail && make test 2>&1 | tee /tmp/gate-test.log
    set -o pipefail && make markdownlint 2>&1 | tee /tmp/gate-md.log

    Expected: all four commands exit 0 with no warnings.

## Validation and acceptance

Quality criteria:

- Tests: `make test` passes. New tests include at least 8 unit tests in
  `tests/interleaved_push_queues.rs` and 4 BDD scenarios in
  `tests/features/interleaved_push_queues.feature`.
- Lint: `make lint` exits 0.
- Format: `make check-fmt` exits 0.
- Markdown: `make markdownlint` exits 0.
- Type safety: `cargo check --all-targets --all-features` exits 0.

Quality method:

- Run `make check-fmt && make lint && make test && make markdownlint` and
  verify exit 0.
- Manually verify `docs/roadmap.md` shows 10.3.2 as done.
- Verify `docs/users-guide.md` mentions interleaved push queue validation.

## Idempotence and recovery

All stages are idempotent — re-running any stage overwrites the same files and
re-runs the same checks. No database migrations or destructive operations are
involved.

If a stage fails mid-way, fix the issue and re-run the stage's validation
command. No rollback is needed beyond `git checkout` of affected files.

## Artifacts and notes

Expected unit test names in `tests/interleaved_push_queues.rs`:

    rate_limit_symmetric_high_only
    rate_limit_symmetric_low_only
    rate_limit_symmetric_mixed
    interleaved_fairness_yields_at_threshold
    interleaved_all_frames_delivered
    interleaved_time_slice_fairness
    rate_limit_interleaved_total_throughput
    fairness_disabled_strict_priority

Expected BDD feature scenarios:

    High-priority frames take precedence when fairness is disabled
    Fairness yields to low-priority after burst threshold
    Rate limiting applies symmetrically across both priority levels
    All frames are delivered when both queues carry traffic

## Interfaces and dependencies

No new public types or methods. This task adds tests exercising existing
interfaces:

- `PushQueues::<u8>::builder()` — queue construction
- `PushHandle::push_high_priority()` / `push_low_priority()` — pushing
- `ConnectionActor::new()` — actor construction
- `ConnectionActor::set_fairness()` — fairness configuration
- `ConnectionActor::run()` — actor execution
- `FairnessConfig` — fairness threshold configuration

All types are already exported from `wireframe::connection` and
`wireframe::push`.
