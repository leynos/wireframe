# RFC 0002: Model-checking the connection actor's write loop

## Preamble

- RFC number: 0002
- Status: Proposed
- Created: 2026-09-23
- Audience: Wireframe maintainers
- Related: issue #683;
  [the Loom lane plan][plan], [multi-layered testing strategy §4.2][strategy],
  and [formal verification methods in Wireframe][formal]

[plan]: ../execplans/plan-the-loom-lane-repair.md
[strategy]: ../multi-layered-testing-strategy.md
[formal]: ../formal-verification-methods-in-wireframe.md

## Summary

The scheduled **Advanced Tests** lane now runs Loom models, and they check the
parts of the write loop's surroundings that Loom can schedule: the dead-letter
drop counter and log mutex that concurrent producers share through
`PushHandle`, and the active-connection gauge every actor's guard moves. The
write loop itself, a `tokio::select!(biased; ...)` over Tokio channels and a
cancellation token, is outside Loom's reach, and so are the handle's `Arc`, its
rate limiter and the session registry. This RFC sets out how to verify what
remains, recommends extracting the write loop's scheduling decision into a pure
function checked by Kani and refined against the Stateright model, and records
the smaller follow-ups the Loom work found.

## Problem

`docs/multi-layered-testing-strategy.md` §4.2 aimed Loom at the write loop's
`select!` logic, and two documents claimed the Loom models checked queue-full
behaviour. Neither was achievable. Loom schedules only the threads it spawns
and the primitives it supplies, and every primitive the write loop uses comes
from Tokio. Tokio swaps in Loom internally only for its own test suite, so a
downstream `--cfg loom` build gets its standard implementations. An assertion
made through them passes or fails regardless of the interleaving Loom chose,
which is coverage that cannot fail.

The user ruled on 2026-09-23 that the lane refocuses on the synchronization
Loom can model, that the channel assertions go, and that the rest is proposed
here rather than built in the same change.

## Current state

What checks the write loop and its producers today:

| Property                                                | Checked by                                                                | Can it fail for a scheduling reason? |
| ------------------------------------------------------- | ------------------------------------------------------------------------- | ------------------------------------ |
| Active-connection gauge under concurrent actors         | Loom models, `crates/wireframe-loom/tests/connection_gauge.rs`            | yes                                  |
| Dead-letter drop counting and reset under concurrency   | Loom models, `crates/wireframe-loom/tests/push_dlq.rs`                    | yes                                  |
| Queue-full errors from `try_push`                       | deterministic test, `tests/push.rs::try_push_respects_policy`             | no, and it need not                  |
| Strict ordering `shutdown > high > low > stream`        | deterministic tests `strict_priority_order`, `shutdown_signal_precedence` | only for the one schedule they run   |
| Fairness yielding after a burst of high-priority frames | `tests/connection_actor_fairness.rs`                                      | only for the schedules they run      |
| Abstract actor state: terminator uniqueness, progress   | Stateright `PlaceholderConnectionModel`, `crates/wireframe-verification`  | yes, over the model, not the code    |
| Mixed push and stream inputs                            | Proptest, `tests/advanced/interaction_fuzz.rs`                            | partly: its strategy orders inputs   |

*Table 1: What verifies the write loop today.*

The gap is the join between the last two rows and the code. The Stateright
model is a placeholder, whose own documentation says it will be refined toward
the real `ConnectionActor`, and nothing ties its transitions to what
`next_event` actually does.

## Goals and non-goals

Goals:

- verify the write loop's scheduling decision over every combination of
  source availability, fairness state and run state, not only the ones a test
  happens to construct;
- tie the Stateright model to that decision, so the model cannot drift from
  the code it describes;
- close the smaller gaps the Loom work found.

Non-goals:

- running the Tokio runtime under Loom, or substituting a Loom-visible channel
  for Tokio's, which the ruling set aside: the tested object and the shipped
  object would then differ exactly where it matters;
- changing the write loop's behaviour.

## Proposed design

### 1. Extract the scheduling decision

`ConnectionActor::next_event` does two things at once: it decides which source
may be polled next, from `EventAvailability`, `ActorState` and the fairness
tracker, and then it awaits that source through `tokio::select!`. The first is
a pure function of plain values; only the second needs Tokio.

Split out:

```rust
pub(crate) fn eligible_sources(
    availability: EventAvailability,
    state: &ActorState,
    should_yield_to_low: bool,
) -> SourceOrder;
```

returning the ordered list the `select!` arms are guarded by, with `biased`
reducing to "the first ready source in this order". The fairness decision is an
input, not something the function consults: today `FairnessTracker` is applied
in `after_high`'s opportunistic drain, not in `next_event`, so the caller passes
`should_yield_to_low` from the tracker and the Kani harness ranges over both
values. Keeping it explicit is what lets Kani and Stateright share the function
without either modelling the tracker's clock. `next_event` keeps its `select!`,
now guarded by the extracted result, so the shipped path is the one verified.

### 2. Verify the decision exhaustively with Kani

The inputs are a handful of booleans and small counters, so a Kani harness can
cover every combination rather than a sample. Properties:

- shutdown, when active, is always first;
- high precedes low unless the fairness tracker says to yield;
- a yield is followed by a low-priority poll when one is available;
- a response and a multi-packet channel are never both eligible;
- `Idle` is returned exactly when nothing is eligible.

The Kani smoke targets already exist in the Makefile as stubs (`make kani`,
roadmap 15.3.1), so this is their first real harness.

### 3. Refine the Stateright model against the same function

Replace the placeholder transition relation's source choice with a call to
`eligible_sources`, so the model explores the actor's real decisions over
abstract queues. Stateright then checks the protocol-level properties it
already states (single terminator, progress for each output kind, shutdown
racing an active output) against the code's own ordering.

### 4. Smaller follow-ups found by the Loom work

- **The drop counter's reset was not exact under three producers. Fixed.**
  A scratch model with three concurrent drops and a threshold of two failed:
  the producer that reached 2 reported 2 and then stored zero, while increments
  it never saw left the counter at 3, so the log over-reported. The user ruled
  to fix it with an atomic swap-and-report: the reporter takes the count with
  `swap(0)` and reports exactly what it took, with no separate load and store.
  `every_drop_is_reported_or_still_counted` in
  `crates/wireframe-loom/tests/push_dlq.rs` asserts that reported plus
  remaining equals the three drops on every interleaving. It failed on the old
  code (reported 2 plus remaining 3) and passes with the swap.
- **Handle and registry lifetimes are outside Loom.** Loom 0.7.2 has no
  `Weak`: `loom::sync::Arc` offers no `downgrade`, and the session registry
  keeps `Weak<PushHandleInner>` from `Arc::downgrade`, in a `DashMap` whose
  shard locks are not Loom primitives either. Widening the handle's `Arc` to
  Loom's would therefore break the registry rather than model it. Options: a
  Loom release with `Weak`, a Stateright model of registration, expiry and
  upgrade, or leaving lifetime to the deterministic registry tests.
- **The configuration boundary has no contract.** Issue #683 was a module
  admitted under `cfg(loom)` while it imported modules compiled out there. A
  source-reading contract, parsing the module tree with `syn` and checking that
  every import edge's effective predicate implies its target's, was specified
  in the execution plan's V-2 and is deferred to this RFC to keep the Loom
  change achievable. It would add `syn` as a direct dev-dependency. In the
  meantime the Loom build compiles the library with `pool` and `test-support`
  on, which catches the #683 regression itself but no other import edge.
- **The lane has no executed-count guard.** A model file compiled without
  `--cfg loom` is empty, and `cargo test` passes with zero tests. The workflow
  contract holds the target to `--cfg loom`, which covers today's command; a
  guard failing the run when fewer than the expected number of models executed
  would cover the rest.

## Alternatives considered

- **A Loom-visible channel under `cfg(loom)`.** This makes the write loop
  schedulable by Loom, at the cost of a second channel implementation used only
  in tests, and it would verify that implementation rather than Tokio's. Set
  aside by the ruling for that reason.
- **Loom over the whole actor with a mock connection**, as §4.2 once
  sketched. The mock does not exist, and the actor would still be built from
  Tokio primitives, so the model could not be written.
- **Proptest alone.** Broad, but it samples schedules rather than covering
  them, and its current strategy orders inputs. It stays as a complement.

## Open questions

- Is an approximate drop counter acceptable for the diagnostic it feeds?
- Should `eligible_sources` live in `src/connection/` or in a
  `connection::schedule` module shared with the verification crate?

## Recommendation

Adopt §1 to §3 as the write loop's verification path, in that order, since each
depends on the one before. Take the counter decision in §4 first, because it
decides whether a Loom model can assert an exact count. Take the other
follow-ups when their subjects next change.
