# Define derived memory budget defaults from `buffer_capacity` (8.3.5)

This ExecPlan (execution plan) is a living document. The sections
`Constraints`, `Tolerances`, `Risks`, `Progress`, `Surprises & Discoveries`,
`Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work
proceeds.

Status: COMPLETE

## Purpose / big picture

Roadmap item `8.3.5` closes the last gap in the three-tier per-connection
memory budget protection model. Today, when a user does not call
`.memory_budgets(...)` on `WireframeApp`, all three budget tiers are completely
disabled:

- **Per-frame enforcement** (8.3.2) -- no per-message, per-connection, or
  in-flight budget checks.
- **Soft-limit pacing** (8.3.3) -- no read throttling under memory pressure.
- **Hard-cap abort** (8.3.4) -- no connection termination on budget breach.

The `memory_budgets` field remains `None`, and every check short-circuits to a
no-op. An application using fragmentation and message assembly but without
explicit budgets has zero memory protection on inbound assembly paths.

After this change, sensible memory budgets are derived automatically from
`buffer_capacity` (the codec's `max_frame_length()`) whenever the user has not
set explicit budgets. The derivation follows the same pattern already used for
fragmentation defaults in `src/app/builder_defaults.rs`. Derived budgets are
computed lazily at connection time in the inbound handler, not stored on the
builder. Changing `buffer_capacity` automatically adjusts derived budgets;
explicit `.memory_budgets(...)` always takes precedence.

Success is observable when:

- An application using `WireframeApp::new()` with default settings (no
  `.memory_budgets(...)` call) has memory budget enforcement active, with
  budgets derived from the 1024-byte default frame length.
- Changing `buffer_capacity` changes the derived budgets proportionally.
- Explicitly calling `.memory_budgets(...)` overrides the derived defaults.
- All three tiers (per-frame enforcement, soft-limit pacing, hard-cap abort)
  function with derived budgets.
- `rstest` unit tests validate the `default_memory_budgets()` derivation
  function and the `resolve_effective_budgets()` integration helper.
- `rstest-bdd` v0.5.0 scenarios validate that the three-tier protection model
  activates with derived budgets and that explicit budgets override them.
- ADR-002 records the implementation decisions.
- The users guide explains that budgets are derived automatically.
- `docs/roadmap.md` marks `8.3.5` as complete.

## Constraints

- Scope is strictly roadmap item `8.3.5`: derived default budgets from
  `buffer_capacity`.
- Do not regress budget enforcement (8.3.2), soft-limit pacing (8.3.3), or
  hard-cap abort (8.3.4) semantics.
- Preserve runtime behaviour when `memory_budgets` IS explicitly configured.
  Explicit budgets must always take precedence over derived defaults, per the
  ADR-002 precedence rule (explicit > global caps > derived defaults).
- Keep public API changes to zero. The derivation is an internal mechanism. If
  a public API change is required, stop and escalate.
- Do not add new external dependencies.
- Keep all modified or new source files at or below the 400-line repository
  guidance.
- `src/app/inbound_handler.rs` is currently 392 lines. Changes must keep it at
  or below 400.
- Use en-GB-oxendict spelling in all comments and documentation.
- Follow the behaviour-driven development (BDD) 4-file pattern (feature,
  fixture, steps, scenarios) with globally unique step text using a
  `derived-budget` prefix.
- Validate with `rstest` unit tests and `rstest-bdd` v0.5.0 behavioural tests.
- Follow testing guidance from `docs/rust-testing-with-rstest-fixtures.md`,
  `docs/reliable-testing-in-rust-via-dependency-injection.md`, and
  `docs/rstest-bdd-users-guide.md`.
- Record implementation decisions in ADR-002.
- Update `docs/users-guide.md` for any consumer-visible behavioural change.
- Mark roadmap item `8.3.5` done only after all quality gates pass.

## Tolerances (exception triggers)

- Scope: if implementation requires changes to more than 16 files or more than
  600 net lines of code, stop and escalate.
- Interface: if a new public builder method, changed public function signature,
  or new public type is required, stop and escalate.
- Dependencies: if any new crate is required, stop and escalate.
- Semantics ambiguity: if the derived multipliers produce budgets that conflict
  with fragmentation defaults in a way that breaks existing tests, stop and
  present options with trade-offs.
- Iterations: if the same failing gate persists after 3 focused attempts, stop
  and escalate.
- Time: if any single stage exceeds 4 hours elapsed effort, stop and escalate.
- File length: if `src/app/inbound_handler.rs` would exceed 400 lines, extract
  additional logic into `src/app/frame_handling/` helpers before continuing.
- File length: if `src/app/builder_defaults.rs` would exceed 400 lines, stop
  and restructure.

## Risks

- Risk: `src/app/inbound_handler.rs` is currently 392 lines (8 lines from the
  400-line cap). Wiring in derived-budget resolution must fit within this
  headroom. Severity: high. Likelihood: high. Mitigation: resolve effective
  budgets via a one-line call to a new `resolve_effective_budgets()` helper in
  `backpressure.rs`, then replace two inline `self.memory_budgets` references
  with `Some(effective_budgets)`. Net change is approximately +2 lines (392 to
  394). The helper lives in `backpressure.rs` (131 lines, ample room).

- Risk: BDD step-text collisions with existing `memory_budgets_steps.rs`,
  `memory_budget_backpressure_steps.rs`, or `memory_budget_hard_cap_steps.rs`.
  Severity: medium. Likelihood: medium. Mitigation: all new step phrases use a
  `derived-budget` prefix (e.g., "a derived-budget app with buffer capacity
  512").

- Risk: the multiplier choice for `bytes_per_connection` and `bytes_in_flight`
  may be too generous or too restrictive for certain workloads. Severity: low.
  Likelihood: low. Mitigation: choose multipliers that align with fragmentation
  defaults (which use `16x` for max message size). Document the rationale and
  note that explicit `.memory_budgets(...)` is available for tuning.

- Risk: `pub(super)` visibility on `default_memory_budgets()` may not be
  reachable from `frame_handling::backpressure`. Severity: low. Likelihood:
  low. Mitigation: confirmed that the existing `default_fragmentation()`
  function uses `pub(super)` and is already imported from
  `frame_handling::assembly.rs` via `crate::app::builder_defaults::`. The same
  access path works for the new function.

## Progress

- [x] Drafted ExecPlan for roadmap item `8.3.5`.
- [x] Stage A: add `default_memory_budgets()` and
  `resolve_effective_budgets()` with unit tests.
- [x] Stage B: integrate derived defaults into inbound read path.
- [x] Stage C: add behavioural tests (`rstest-bdd` v0.5.0).
- [x] Stage D: documentation and roadmap updates.
- [x] Stage E: quality gates and final verification.

## Surprises & discoveries

- BDD scenario 1 (derived budgets enforce per-connection limit) required
  careful frame-count tuning. With `buffer_capacity=512`, derived
  `per_connection=32768`. Sending frames with 400-byte bodies means each frame
  consumes ~400 bytes of budget. The connection budget is breached around frame
  82, but the `DeserFailureTracker` requires 10 consecutive failures before
  closing the connection. This means 95 frames are needed (frames 82-91 produce
  10 failures, triggering abort). Initial attempts with 20 and 90 frames caused
  test hangs.

- `tokio::time::pause()` is required in the BDD fixture even though no
  explicit time assertions are made. The soft-limit pacing tier inserts a
  `sleep(5ms)` when pressure is detected. With paused time, tokio's
  auto-advance fires instantly when all tasks await timers (during
  `block_on(server)`). Without paused time, the real 5ms sleeps accumulate and
  combine with the read-timeout loop to cause hangs.

- `inbound_handler.rs` grew from 392 to 396 lines (plan estimated 394).
  The `resolve_effective_budgets` call plus its import added 4 net lines rather
  than 2, but the file remains within the 400-line cap.

## Decision log

- Decision: place `default_memory_budgets()` in `src/app/builder_defaults.rs`
  alongside `default_fragmentation()`. Rationale: the two functions are
  structural twins -- both derive configuration from `frame_budget` using a
  multiplier pattern. Co-locating them makes the parallel intent visible.
  `builder_defaults.rs` is currently 22 lines, leaving ample headroom.
  Date/Author: 2026-02-26 / plan phase.

- Decision: resolve effective budgets once in `process_stream` (Option B from
  design analysis) rather than duplicating derivation across
  `new_message_assembly_state()` and `evaluate_memory_pressure()`. Rationale:
  avoids dual derivation, keeps resolution explicit in one place. The
  `let effective_budgets = ...` binding replaces two raw `self.memory_budgets`
  references. Net line impact on `inbound_handler.rs` is approximately +2 lines
  (392 to 394). Date/Author: 2026-02-26 / plan phase.

- Decision: introduce `resolve_effective_budgets()` as a thin helper in
  `src/app/frame_handling/backpressure.rs` rather than inlining the
  `Option::unwrap_or_else` in `process_stream`. Rationale: keeps
  `inbound_handler.rs` under the line cap and co-locates budget resolution with
  budget evaluation. `backpressure.rs` is at 131 lines with ample room.
  Date/Author: 2026-02-26 / plan phase.

- Decision: `default_memory_budgets()` returns `MemoryBudgets` (not
  `Option<MemoryBudgets>`). Rationale: `clamp_frame_length()` guarantees
  `frame_budget >= 64`, so multiplied values are always non-zero. Returning a
  non-optional type makes the function's infallibility clear. Date/Author:
  2026-02-26 / plan phase.

- Decision: use the following multipliers for derived defaults:
  - `bytes_per_message` = `frame_budget * 16` (aligned with fragmentation's
    `DEFAULT_MESSAGE_SIZE_MULTIPLIER`).
  - `bytes_per_connection` = `frame_budget * 64` (allows 4 concurrent message
    assemblies at max message size).
  - `bytes_in_flight` = `frame_budget * 64` (same as per-connection; the
    aggregate cap is a single value).
  Rationale: `bytes_per_message` matches fragmentation's `max_message_size` so
  the two guards agree. `bytes_per_connection` at 64x provides headroom for
  multiple concurrent assemblies without being so large as to be meaningless.
  Setting `bytes_in_flight` equal to `bytes_per_connection` simplifies the
  mental model. For the default 1024-byte frame, this yields: per_message = 16
  KiB (kibibytes), per_connection = 64 KiB, in_flight = 64 KiB. Date/Author:
  2026-02-26 / plan phase.

- Decision: `buffer_capacity()` and `with_codec()` do NOT need to clear
  derived defaults because derivation is lazy (computed at runtime from
  `codec.max_frame_length()`), not stored on the builder. Rationale: mirrors
  how `default_fragmentation()` works -- it is called at runtime in
  `new_message_assembly_state()` when the stored `fragmentation` is `None`.
  Changing the codec automatically changes the input to the derivation
  function. Date/Author: 2026-02-26 / plan phase.

## Outcomes & retrospective

All acceptance criteria met. All quality gates green.

- `default_memory_budgets()` added to `src/app/builder_defaults.rs` (grew
  from 22 to 129 lines including tests).
- `resolve_effective_budgets()` added to
  `src/app/frame_handling/backpressure.rs` (grew from 131 to 145 lines).
- `inbound_handler.rs` wired with derived defaults (grew from 392 to 396
  lines, within 400-line cap).
- 3 BDD scenarios pass: derived enforcement, within-limits delivery, and
  explicit override.
- 8 unit tests pass: 5 for `default_memory_budgets()`, 3 for
  `resolve_effective_budgets()`.
- ADR-002 updated with 8.3.5 implementation decisions.
- Users guide expanded with "Derived budget defaults" section.
- Roadmap item 8.3.5 marked complete.
- No regressions: full test suite (371 unit + 135 BDD) passes.

Retrospective:

- The frame-count tuning for the BDD enforcement scenario was the main
  implementation challenge. The interaction between derived budget thresholds,
  the `DeserFailureTracker` 10-failure limit, and `tokio::time::pause()`
  auto-advance required three iterations to get right. Documenting the math in
  the surprises section will help future tests.
- The plan estimated +2 lines for `inbound_handler.rs` but actual was +4.
  The discrepancy came from the `use` import line and a blank line for
  readability. Still well within the 400-line cap.
- The lazy derivation decision proved correct: no builder changes were needed
  for `buffer_capacity()` or `with_codec()`, matching the
  `default_fragmentation()` pattern exactly.

## Context and orientation

### Memory budget types

`src/app/memory_budgets.rs` (101 lines) defines `MemoryBudgets`, a `Copy`
struct with three `BudgetBytes(NonZeroUsize)` fields:

- `message_budget` (accessor: `bytes_per_message()`) -- max bytes per
  single logical message.
- `connection_window` (accessor: `bytes_per_connection()`) -- max bytes
  buffered across all assemblies on one connection.
- `assembly_bytes` (accessor: `bytes_in_flight()`) -- max bytes across
  in-flight assemblies.

`BudgetBytes` wraps `NonZeroUsize` and provides `new()`, `get()`, `as_usize()`,
plus `From` conversions.

### Builder field and explicit configuration

`src/app/builder/core.rs` stores `memory_budgets: Option<MemoryBudgets>` on
`WireframeApp`. Default is `None`. The builder method
`WireframeApp::memory_budgets(budgets)` in `src/app/builder/config.rs` sets it
to `Some(budgets)`.

Type-changing operations (`with_codec()` in `src/app/builder/codec.rs`,
`serializer()`) preserve the `memory_budgets` field via `RebuildParams`.

### `buffer_capacity` and frame budget

`buffer_capacity(capacity)` in `src/app/builder/codec.rs` (lines 76-81) clamps
the value to `[64, 16 MiB]` via `clamp_frame_length()`, creates a new
`LengthDelimitedFrameCodec`, and clears `fragmentation` to `None`.

`LengthDelimitedFrameCodec::default()` uses `max_frame_length: 1024`.

### Derivation template: `default_fragmentation()`

`src/app/builder_defaults.rs` (22 lines) contains `default_fragmentation()`,
which takes `frame_budget: usize`, clamps it, multiplies by
`DEFAULT_MESSAGE_SIZE_MULTIPLIER` (16) to get `max_message_size`, and builds a
`FragmentationConfig`. This is the exact pattern `8.3.5` follows.

### Inbound read path

`src/app/inbound_handler.rs` (392 lines) contains
`WireframeApp::process_stream()`. Two call sites use `self.memory_budgets`:

1. Line 270-276: `frame_handling::new_message_assembly_state(self.fragmentation,
   requested_frame_length,
   self.memory_budgets)` -- creates `MessageAssemblyState`. When budgets is `
   None`, no aggregate enforcement occurs.

2. Line 281-284: `frame_handling::evaluate_memory_pressure(
   message_assembly.as_ref(),
   self.memory_budgets)` -- evaluates pressure. When budgets is `None
   `, returns `Continue` unconditionally.

Both call sites pass the raw `Option<MemoryBudgets>`. Task 8.3.5 resolves this
to an always-`Some` value by deriving defaults when the field is `None`.

### Assembly module

`src/app/frame_handling/assembly.rs` (289 lines) contains
`new_message_assembly_state()` which threads budgets into
`MessageAssemblyState::with_budgets(...)`. When `memory_budgets` is `None`, it
falls through to `MessageAssemblyState::new()` which internally calls
`with_budgets(..., None, None)` -- disabling all aggregate budget enforcement.

### Backpressure module

`src/app/frame_handling/backpressure.rs` (131 lines) contains
`evaluate_memory_pressure()` and `apply_memory_pressure()`. Both short-circuit
on `None` budgets. The `active_aggregate_limit_bytes()` helper computes
`min(bytes_per_connection, bytes_in_flight)`.

### Re-exports

`src/app/frame_handling/mod.rs` (30 lines) re-exports `pub(crate)` items from
`backpressure` and `assembly`. Adding `resolve_effective_budgets` to the
re-export list requires one additional line.

### BDD test infrastructure

The project uses a 4-file BDD pattern with `rstest-bdd` v0.5.0:

1. Feature file: `tests/features/<name>.feature` (Gherkin syntax).
2. Fixture: `tests/fixtures/<name>.rs` (world struct + helpers).
3. Steps: `tests/steps/<name>_steps.rs` (`given`/`when`/`then` macros).
4. Scenarios: `tests/scenarios/<name>_scenarios.rs` (bindings).

Module registrations: `tests/fixtures/mod.rs`, `tests/steps/mod.rs`,
`tests/scenarios/mod.rs`. Step text must be globally unique across all step
files. The fixture parameter name in step functions must match the fixture
function name exactly.

Reference implementation: `tests/fixtures/memory_budget_hard_cap.rs` (345
lines) demonstrates the world struct pattern with `tokio::runtime::Runtime`,
`tokio::io::duplex`, frame sending helpers, and assertion methods.

## Plan of work

### Stage A: add `default_memory_budgets()` and `resolve_effective_budgets()` with unit tests

**A1.** Add `default_memory_budgets()` to `src/app/builder_defaults.rs`.

Below the existing `default_fragmentation()` function, add three constants and
one function:

```rust
const DEFAULT_MESSAGE_BUDGET_MULTIPLIER: usize = 16;
const DEFAULT_CONNECTION_BUDGET_MULTIPLIER: usize = 64;
const DEFAULT_IN_FLIGHT_BUDGET_MULTIPLIER: usize = 64;

pub(super) fn default_memory_budgets(frame_budget: usize) -> MemoryBudgets {
    let frame_budget = clamp_frame_length(frame_budget);
    let per_message = NonZeroUsize::new(
        frame_budget.saturating_mul(DEFAULT_MESSAGE_BUDGET_MULTIPLIER),
    )
    .unwrap_or(NonZeroUsize::MIN);
    let per_connection = NonZeroUsize::new(
        frame_budget.saturating_mul(DEFAULT_CONNECTION_BUDGET_MULTIPLIER),
    )
    .unwrap_or(NonZeroUsize::MIN);
    let in_flight = NonZeroUsize::new(
        frame_budget.saturating_mul(DEFAULT_IN_FLIGHT_BUDGET_MULTIPLIER),
    )
    .unwrap_or(NonZeroUsize::MIN);
    MemoryBudgets::new(
        BudgetBytes::new(per_message),
        BudgetBytes::new(per_connection),
        BudgetBytes::new(in_flight),
    )
}
```

Add imports for `MemoryBudgets` and `BudgetBytes` from
`crate::app::memory_budgets`. The `unwrap_or(NonZeroUsize::MIN)` fallback
avoids the forbidden `.unwrap()` lint while being effectively unreachable since
`clamp_frame_length` guarantees `>= 64`.

The file grows from 22 to approximately 55 lines (including a
`#[cfg(test)] mod tests` block).

**A2.** Add unit tests for `default_memory_budgets()` in a
`#[cfg(test)] mod tests` block at the bottom of `src/app/builder_defaults.rs`:

1. `default_budgets_use_expected_multipliers` -- with the default 1024-byte
   frame: asserts `bytes_per_message == 16_384`,
   `bytes_per_connection == 65_536`, `bytes_in_flight == 65_536`.
2. `default_budgets_scale_with_frame_budget` -- with a 4096-byte frame:
   asserts proportional scaling.
3. `default_budgets_clamp_minimum_frame_budget` -- with a 10-byte input
   (below `MIN_FRAME_LENGTH` of 64): asserts values are based on clamped 64.
4. `default_budgets_clamp_maximum_frame_budget` -- with a value above
   `MAX_FRAME_LENGTH`: asserts values are based on clamped 16 MiB.
5. `default_budgets_message_budget_aligns_with_fragmentation` -- confirms
   that `bytes_per_message` equals
   `frame_budget * DEFAULT_MESSAGE_SIZE_MULTIPLIER`, the same multiplier
   fragmentation uses.

**A3.** Add `resolve_effective_budgets()` to
`src/app/frame_handling/backpressure.rs`:

```rust
/// Resolve the effective memory budgets for one connection.
///
/// Returns the explicit budgets if configured, or derives sensible
/// defaults from `frame_budget` using the same multiplier pattern as
/// fragmentation defaults.
#[must_use]
pub(crate) fn resolve_effective_budgets(
    explicit: Option<MemoryBudgets>,
    frame_budget: usize,
) -> MemoryBudgets {
    explicit.unwrap_or_else(|| default_memory_budgets(frame_budget))
}
```

Add import for `default_memory_budgets` from `crate::app::builder_defaults`.
The file grows from 131 to approximately 145 lines.

**A4.** Add `resolve_effective_budgets` to the `pub(crate) use` re-export in
`src/app/frame_handling/mod.rs`.

**A5.** Add unit tests for `resolve_effective_budgets()` in
`src/app/frame_handling/backpressure_tests.rs`:

1. `resolve_returns_explicit_budgets_when_configured` -- given
   `Some(budgets)`, returns those exact budgets regardless of `frame_budget`.
2. `resolve_returns_derived_budgets_when_none` -- given `None` and
   `frame_budget=1024`, returns the expected derived values.
3. `resolve_derived_budgets_change_with_frame_budget` -- given `None` and
   different `frame_budget` values, returns proportionally different budgets.

Go/no-go: run `cargo test --lib builder_defaults` and
`cargo test --lib frame_handling`. All tests pass.

### Stage B: integrate derived defaults into inbound read path

**B1.** In `src/app/inbound_handler.rs`, in `process_stream()`, after
`let requested_frame_length = codec.max_frame_length();` (line 260), add:

```rust
let effective_budgets = frame_handling::resolve_effective_budgets(
    self.memory_budgets,
    requested_frame_length,
);
```

**B2.** Change line 274 from `self.memory_budgets,` to
`Some(effective_budgets),`.

**B3.** Change line 283 from `self.memory_budgets,` to
`Some(effective_budgets),`.

Net change: +2 lines (one `let` binding added, two in-place substitutions).
File grows from 392 to approximately 394 lines.

Go/no-go: run `cargo test --lib` and `make lint`. All pass. Verify
`wc -l src/app/inbound_handler.rs` is 400 or less.

### Stage C: add behavioural tests (`rstest-bdd` v0.5.0)

**C1.** Feature file: `tests/features/derived_memory_budgets.feature`

Three scenarios with `derived-budget` step prefix:

1. **Derived budgets enforce per-frame limits** -- send enough frames to a
   default-budget app (no explicit `.memory_budgets(...)`) to trigger the
   hard-cap abort. Assert connection terminates with an error.
2. **Derived budgets allow frames within limits** -- send frames within the
   derived budget to a default-budget app. Assert payload delivery and no
   connection error.
3. **Explicit budgets override derived defaults** -- configure tight explicit
   budgets on an app that also has a `buffer_capacity`. Send frames that exceed
   the explicit budget (but would be within derived defaults). Assert
   connection terminates.

**C2.** Fixture: `tests/fixtures/derived_memory_budgets.rs`

Follow the `memory_budget_hard_cap.rs` pattern. The world struct
(`DerivedMemoryBudgetsWorld`) supports two startup modes:

- `start_app_derived(buffer_capacity)` -- creates `WireframeApp` with
  `.buffer_capacity(capacity)` and `.enable_fragmentation()` but NO
  `.memory_budgets(...)` call. Derived budgets activate at runtime.
- `start_app_explicit(buffer_capacity, per_message, per_connection,
  in_flight)` -- same but adds `.memory_budgets(…)`.

Reuse the frame-sending helpers and assertion methods from the hard-cap fixture
pattern.

**C3.** Steps: `tests/steps/derived_memory_budgets_steps.rs`

Step definitions with `derived-budget` prefix. The `given` step parses buffer
capacity and optional explicit budgets. The `when` and `then` steps delegate to
world methods.

**C4.** Scenarios: `tests/scenarios/derived_memory_budgets_scenarios.rs`

Bind each feature scenario to the fixture.

**C5.** Register modules in `tests/fixtures/mod.rs`, `tests/steps/mod.rs`,
`tests/scenarios/mod.rs`.

Go/no-go: run `cargo test --test bdd --all-features derived_memory_budgets`.
All scenarios pass. Then run `make lint` and `make test` for full regression.

### Stage D: documentation and roadmap updates

**D1.** ADR-002
(`docs/adr-002-streaming-requests-and-shared-message-assembly.md`): add a new
implementation decisions block after the existing 8.3.4 entry:

- Roadmap item `8.3.5` implements derived default memory budgets.
- When `WireframeApp::memory_budgets(...)` is not called, defaults are derived
  at connection time from `codec.max_frame_length()`.
- Multipliers: `bytes_per_message = frame_budget * 16` (aligned with
  fragmentation), `bytes_per_connection = frame_budget * 64`,
  `bytes_in_flight = frame_budget * 64`.
- Derivation is lazy (not stored on the builder). Changing `buffer_capacity`
  or swapping codecs adjusts derived defaults automatically.
- Explicit budgets always take precedence (ADR-002 precedence rule 1 > 3).

**D2.** Users guide (`docs/users-guide.md`): expand the "Per-connection memory
budgets" section to explain:

- Budgets are derived automatically from `buffer_capacity` when not set
  explicitly.
- State the default multipliers and resulting values for the 1024-byte
  default (per_message = 16 KiB, per_connection = 64 KiB, in_flight = 64 KiB).
- All three protection tiers are active with derived defaults.
- Calling `.memory_budgets(...)` overrides the derived defaults entirely.
- Changing `buffer_capacity` adjusts derived defaults proportionally.

**D3.** Roadmap (`docs/roadmap.md`): change the `8.3.5` line from `- [ ]` to
`- [x]`.

### Stage E: quality gates and final verification

Run all required gates with `set -o pipefail` and `tee` to log files:

```shell
set -o pipefail
make fmt 2>&1 | tee /tmp/wireframe-8-3-5-fmt.log
make markdownlint MDLINT=/root/.bun/bin/markdownlint-cli2 \
  2>&1 | tee /tmp/wireframe-8-3-5-markdownlint.log
make check-fmt 2>&1 | tee /tmp/wireframe-8-3-5-check-fmt.log
make lint 2>&1 | tee /tmp/wireframe-8-3-5-lint.log
make test 2>&1 | tee /tmp/wireframe-8-3-5-test.log
make nixie 2>&1 | tee /tmp/wireframe-8-3-5-nixie.log
```

No roadmap checkbox update until every gate is green.

## Concrete steps

Run from the repository root (`/home/user/project`).

1. Implement Stage A: `default_memory_budgets()` +
   `resolve_effective_budgets()` + unit tests.
2. Run focused unit tests:

```shell
set -o pipefail
cargo test --lib builder_defaults 2>&1 | tee /tmp/wireframe-8-3-5-unit-a1.log
cargo test --lib frame_handling 2>&1 | tee /tmp/wireframe-8-3-5-unit-a2.log
```

1. Implement Stage B: wire into `process_stream`.
2. Run focused tests and lint:

```shell
set -o pipefail
cargo test --lib 2>&1 | tee /tmp/wireframe-8-3-5-unit-b.log
make lint 2>&1 | tee /tmp/wireframe-8-3-5-lint-b.log
```

1. Verify line count:

```shell
wc -l src/app/inbound_handler.rs
```

1. Implement Stage C: BDD tests.
2. Run targeted BDD scenarios:

```shell
set -o pipefail
cargo test --test bdd --all-features derived_memory_budgets \
  2>&1 | tee /tmp/wireframe-8-3-5-bdd.log
```

1. Implement Stage D: documentation updates.
2. Run full quality gates (Stage E).

## Validation and acceptance

Acceptance criteria:

- Derived-budget behaviour: when `WireframeApp::memory_budgets(...)` is not
  called, memory budgets are derived from `buffer_capacity`. All three
  protection tiers are active.
- Explicit override: when `.memory_budgets(...)` is called, the explicit values
  are used regardless of `buffer_capacity`.
- Multiplier correctness: for the default 1024-byte frame, derived budgets are:
  `bytes_per_message` = 16,384, `bytes_per_connection` = 65,536,
  `bytes_in_flight` = 65,536.
- Proportional scaling: changing `buffer_capacity` changes derived budgets
  proportionally.
- No regressions: all existing budget enforcement (8.3.2), soft-limit (8.3.3),
  and hard-cap (8.3.4) tests continue to pass.
- Unit tests (`rstest`) validate `default_memory_budgets()` and
  `resolve_effective_budgets()`.
- Behavioural tests (`rstest-bdd` v0.5.0) validate derived-budget activation
  and explicit override.
- Design documentation records decisions in ADR-002.
- Users guide explains derived defaults.
- `docs/roadmap.md` marks `8.3.5` done.

Quality criteria:

- tests: `make test` passes.
- lint: `make lint` passes with no warnings.
- formatting: `make fmt` and `make check-fmt` pass.
- markdown: `make markdownlint` passes.
- mermaid validation: `make nixie` passes.

## Idempotence and recovery

All planned edits are additive and safe to rerun. If a step fails:

- Preserve local changes.
- Inspect the relevant `/tmp/wireframe-8-3-5-*.log` file.
- Apply the minimal fix.
- Rerun only the failed command first, then downstream gates.

Avoid destructive git commands. If rollback is required, revert only files
changed for `8.3.5`.

## Artefacts and notes

Expected artefacts after completion:

- Modified: `src/app/builder_defaults.rs` (~55 lines, up from 22).
- Modified: `src/app/frame_handling/backpressure.rs` (~145 lines, up from 131).
- Modified: `src/app/frame_handling/backpressure_tests.rs` (~320 lines, up
  from 268).
- Modified: `src/app/frame_handling/mod.rs` (~31 lines, up from 30).
- Modified: `src/app/inbound_handler.rs` (~394 lines, up from 392).
- New: `tests/features/derived_memory_budgets.feature`.
- New: `tests/fixtures/derived_memory_budgets.rs`.
- New: `tests/steps/derived_memory_budgets_steps.rs`.
- New: `tests/scenarios/derived_memory_budgets_scenarios.rs`.
- Modified: `tests/fixtures/mod.rs`.
- Modified: `tests/steps/mod.rs`.
- Modified: `tests/scenarios/mod.rs`.
- Modified: `docs/adr-002-streaming-requests-and-shared-message-assembly.md`.
- Modified: `docs/users-guide.md`.
- Modified: `docs/roadmap.md`.
- New: `docs/execplans/8-3-5-Define derived defaults based on
  buffer_capacity.md`.
- Gate logs: `/tmp/wireframe-8-3-5-*.log`.

Total: 5 new files (4 test + 1 execplan), 11 modified files = 16 artefacts (at
tolerance boundary).

## Interfaces and dependencies

No new external dependencies are required.

Internal interfaces expected at the end of this milestone:

In `src/app/builder_defaults.rs`:

```rust
/// Multiplier applied to `frame_budget` for the per-message default budget.
/// Aligned with `DEFAULT_MESSAGE_SIZE_MULTIPLIER` used by
/// `default_fragmentation()`.
const DEFAULT_MESSAGE_BUDGET_MULTIPLIER: usize = 16;

/// Multiplier applied to `frame_budget` for the per-connection default
/// budget.
const DEFAULT_CONNECTION_BUDGET_MULTIPLIER: usize = 64;

/// Multiplier applied to `frame_budget` for the in-flight default budget.
const DEFAULT_IN_FLIGHT_BUDGET_MULTIPLIER: usize = 64;

/// Derive sensible memory budgets from the codec frame budget.
///
/// Mirrors the pattern of [`default_fragmentation`], which derives
/// fragmentation settings from the same frame budget. The frame budget is
/// clamped to [`crate::codec::MIN_FRAME_LENGTH`]..=
/// [`crate::codec::MAX_FRAME_LENGTH`] before applying multipliers.
#[must_use]
pub(super) fn default_memory_budgets(frame_budget: usize) -> MemoryBudgets
```

In `src/app/frame_handling/backpressure.rs`:

```rust
/// Resolve the effective memory budgets for one connection.
///
/// Returns the explicit budgets if configured, or derives sensible
/// defaults from `frame_budget` using the same multiplier pattern as
/// fragmentation defaults.
#[must_use]
pub(crate) fn resolve_effective_budgets(
    explicit: Option<MemoryBudgets>,
    frame_budget: usize,
) -> MemoryBudgets
```

In `src/app/inbound_handler.rs::process_stream`:

- Calls `frame_handling::resolve_effective_budgets(self.memory_budgets,
  requested_frame_length)` once and passes `Some(effective_budgets)` to both `
  new_message_assembly_state()` and `evaluate_memory_pressure()`.

In behavioural tests:

- A new `rstest-bdd` world (`DerivedMemoryBudgetsWorld`) validates that
  derived budgets enable all three protection tiers and that explicit budgets
  override derived defaults.
