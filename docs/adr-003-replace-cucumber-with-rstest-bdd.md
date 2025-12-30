# ADR 001: replace Cucumber with rstest-bdd for behavioural tests

## Status

Proposed (2025-12-21)

## Deciders

- Wireframe maintainers

## Context

Wireframe's behavioural tests currently use the `cucumber` crate, with a
bespoke test runner (`tests/cucumber.rs`), `World` types, and step modules.
This approach provides Gherkin coverage but sits outside the default Rust test
harness, depends on an async runner, and duplicates fixture concepts already in
use elsewhere. The suite also has to keep a separate execution path in
Continuous Integration (CI), which increases maintenance overhead when evolving
test infrastructure.

The `rstest-bdd` framework (v0.2.0) offers a Behaviour-Driven Development (BDD)
layer that integrates directly with `rstest` fixtures and `cargo test`, keeping
behavioural tests in the same execution environment as unit and integration
tests. It reuses familiar fixtures, supports compile-time validation of step
coverage, and keeps Gherkin feature files as the source of truth for behaviour
specifications. The `rstest-bdd` user's guide documents the current feature set
and its limitations, which must be accounted for in a migration plan.
[^rstest-bdd-guide]

## Decision

Migrate Wireframe's behavioural testing from `cucumber` to `rstest-bdd` v0.2.0,
keeping Gherkin feature files but replacing the Cucumber runner, worlds, and
step bindings with `rstest-bdd` scenarios and fixtures.

## Rationale

- Align behavioural tests with the standard Rust test harness so `cargo test`
  runs all suites consistently.
- Reuse `rstest` fixtures for dependency injection, reducing bespoke world
  scaffolding and aligning with the existing testing conventions.
- Improve feedback with optional compile-time validation of missing steps and
  better integration with standard Rust tooling.
- Reduce the maintenance burden of a dedicated Cucumber runner, async runtime
  configuration, and bespoke test wiring.
- Adopt a framework that already has internal documentation and guidance in
  this repository, keeping institutional knowledge in one place.

## Approach

- Keep existing `.feature` files, with minor edits only where `rstest-bdd`
  syntax differs from current usage.
- Replace Cucumber world structs with `rstest` fixtures and, where needed,
  `rstest-bdd` `ScenarioState` and `Slot` helpers for per-scenario state.
- Convert Cucumber step macros (`given`, `when`, `then`) to their `rstest-bdd`
  equivalents, using placeholder captures and fixture injection rather than
  `World` arguments.
- Bind scenarios via `#[scenario]` macros that point at the same feature files,
  allowing `cargo test` to run them alongside other tests.
- Enable `rstest-bdd-macros` compile-time validation and tighten to
  strict validation once all step definitions are local to the Wireframe
  workspace.
- Use explicit async boundaries for current async steps, such as helper layers
  that `block_on` async operations inside a shared test runtime fixture, while
  keeping async-heavy scenarios on Cucumber until support lands.
- Remove the Cucumber runner and dependencies only after parity is established,
  and behavioural coverage is fully represented in the new suite.

## Consequences

- Behavioural tests will run under the standard `cargo test` runner, simplifying
  CI and local development workflows.
- Step definitions will move from async Cucumber `World` methods to synchronous
  functions, which may require refactors where steps currently perform async
  work.
- The migration will temporarily run two frameworks in parallel, increasing the
  short-term maintenance load until decommissioning is complete.
- `rstest-bdd` async step support is on the roadmap but not yet available, so
  async behaviour must be refactored or deferred.
- Migration timing should align with the landing of async support to avoid
  unnecessary rewrites of async-heavy steps.
- `rstest-bdd` currently lacks support for wildcard `*` steps and other
  advanced features; any existing scenarios using unsupported constructs must
  be rewritten or deferred until upstream support is available.

## Roadmap

### Phase 1: Establish the rstest-bdd foundation

- Step: Add the new toolchain and skeleton tests.
  - Task: Add `rstest-bdd` and `rstest-bdd-macros` v0.2.0 as dev-dependencies
    with `compile-time-validation` enabled.
  - Task: Create a `tests/bdd/` module layout and a minimal smoke scenario that
    runs under `cargo test`.
  - Task: Document the new test layout and invocation in the testing guides.

### Phase 2: Migrate existing scenarios

- Step: Port core behavioural coverage from Cucumber to `rstest-bdd`.
  - Task: Convert each Cucumber world into a `rstest` fixture or
    `ScenarioState` struct, and move shared helpers into `wireframe_testing` as
    needed.
  - Task: Translate step definitions to `#[given]`, `#[when]`, and `#[then]`
    functions, keeping the same feature file wording wherever possible.
  - Task: Ensure each migrated scenario passes under `cargo test` and that
    feature coverage matches the existing suite.

### Phase 3: Retire Cucumber

- Step: Remove the legacy runner and dependencies once parity is confirmed.
  - Task: Delete `tests/cucumber.rs`, Cucumber-specific world modules, and the
    `cucumber` dev-dependency.
  - Task: Update CI pipelines to drop the Cucumber test target and rely on
    `cargo test` for behavioural coverage.
  - Task: Replace or retire documentation that refers to Cucumber, including
    guides and examples in `docs/`.

## Mitigation and rollback

- If `rstest-bdd` limitations block parity, keep the affected scenarios on the
  Cucumber runner and document the gap in the migration tracker.
- Continue running both suites until the missing capabilities land or a
  suitable alternative is agreed, then resume the migration work.

[^rstest-bdd-guide]: See [rstest-bdd user's guide](rstest-bdd-users-guide.md).
