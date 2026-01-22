# Migration Plan: Cucumber to rstest-bdd v0.4.0

**Branch**: `migrate-from-cucumber-to-rstest-bdd`

**Duration**: 9 weeks (phased incremental migration)

**Status**: Planning

**Last Updated**: 2026-01-22

## Executive Summary

Migrate Wireframe's 14 Cucumber-based BDD test suites (~3,941 lines of
world code, ~1,330 lines of steps, 60+ scenarios) to rstest-bdd v0.4.0.
The migration leverages rstest-bdd's new async scenario support while
maintaining test coverage through parallel execution during migration.

**Key Strategy**: Async scenarios with sync steps calling async helpers
via `tokio::task::block_in_place`.

## Current State Analysis

### Infrastructure Inventory

- **14 World structs** across 15+ files (~3,941 lines total)
- **14 .feature files** with 60+ scenarios
- **~1,330 lines** of async step definitions
- **100% async steps** (Cucumber framework requirement)
- **Complex async operations**: TCP servers, client connections, actor
  processing, timeout handling

### World Complexity Classification

#### Tier 1 - Simple (115-200 lines)

- `CorrelationWorld` (115 lines): Simple state + 2 async methods
- `RequestPartsWorld` (~150 lines): Basic state validation

#### Tier 2 - Medium (200-400 lines)

- `PanicWorld`, `MultiPacketWorld`, `StreamEndWorld`,
  `MessageAssemblerWorld`, `CodecStatefulWorld`

#### Tier 3 - High Complexity (400+ lines)

- `ClientMessagingWorld` (302 lines): Server spawning, client
  connections, envelope handling
- `ClientLifecycleWorld`, `ClientPreambleWorld` (~400 lines): Lifecycle
  hooks, callbacks
- `MessageAssemblyWorld`, `CodecErrorWorld`, `FragmentWorld`
  (multi-file, 11 scenarios)

## Implementation Big Picture

### Async Handling Model

rstest-bdd v0.4.0 supports **async scenarios** with **sync step
definitions**:

```rust
// Scenario function is async
#[scenario(path = "tests/features/client_messaging.feature",
           name = "Client sends envelope")]
#[tokio::test(flavor = "current_thread")]
async fn client_sends_envelope_scenario(world: ClientMessagingWorld) {}

// Steps are sync, call async helpers
#[when("the client sends the envelope")]
fn when_client_sends_envelope(world: &mut ClientMessagingWorld)
    -> TestResult
{
    tokio::task::block_in_place(|| {
        tokio::runtime::Handle::current().block_on(
            world.send_envelope()
        )
    })
}
```

**Why this works**: `#[tokio::test(flavor = "current_thread")]` creates
async runtime, `block_in_place` allows sync steps to await,
current-thread flavor avoids `Send` bounds with `&mut` fixtures.

### World-to-Fixture Conversion

**Use `&mut Fixture` when**:

- Simple owned fields mutated directly
- Complex objects with Drop semantics
- Direct ownership desired

**Use `Slot<T>` when**:

- Optional state populated conditionally
- Late-bound values (set during test)
- State reset between steps needed
- Mix of required + optional state

**Example Pattern**:

```rust
use rstest_bdd::{Slot, ScenarioState};
use rstest_bdd_macros::ScenarioState;

#[derive(Debug, ScenarioState)]
pub struct ClientMessagingWorld {
    // Slots for optional/late-bound state
    addr: Slot<SocketAddr>,
    server: Slot<JoinHandle<()>>,
    client: Slot<WireframeClient<...>>,
    envelope: Slot<Envelope>,

    // Direct fields for always-present state
    sent_correlation_ids: Vec<u64>,

    // Slots for conditional outcomes
    response: Slot<Envelope>,
    last_error: Slot<ClientError>,
}

#[fixture]
fn client_messaging_world() -> ClientMessagingWorld {
    // ScenarioState auto-derives Default
    ClientMessagingWorld::default()
}
```

### Feature File Changes

**NONE REQUIRED** - rstest-bdd uses same Gherkin parser as Cucumber.
All existing `.feature` files are 100% compatible.

## Phase Breakdown

### Phase 0: Foundation (Week 1)

**Objective**: Set up parallel infrastructure without disrupting
existing tests.

**Tasks**:

1. Add rstest-bdd dependencies to `Cargo.toml`:

   ```toml
   [dev-dependencies]
   rstest-bdd = "0.4.0"
   rstest-bdd-macros = { version = "0.4.0",
                         features = ["compile-time-validation"] }
   ```

2. Create directory structure:

   ```text
   tests/
     bdd/              # NEW: rstest-bdd tests
       mod.rs
       fixtures/
         mod.rs
       steps/
         mod.rs
       scenarios/
         mod.rs
     cucumber.rs       # KEEP: existing runner
     features/         # KEEP: shared .feature files
     worlds/           # KEEP: existing Cucumber worlds
     steps/            # KEEP: existing Cucumber steps
   ```

3. Update `Cargo.toml` test configuration:

   ```toml
   [[test]]
   name = "bdd"
   path = "tests/bdd/mod.rs"
   required-features = ["advanced-tests"]

   [[test]]
   name = "cucumber"
   path = "tests/cucumber.rs"
   required-features = ["advanced-tests", "cucumber-tests"]
   ```

4. Update Makefile:

   ```makefile
   test-bdd: ## Run rstest-bdd tests only
       RUSTFLAGS="-D warnings" $(CARGO) test --test bdd \
           --all-features $(BUILD_JOBS)

   test-cucumber: ## Run Cucumber tests only
       RUSTFLAGS="-D warnings" $(CARGO) test --test cucumber \
           --features cucumber-tests $(BUILD_JOBS)

   test: test-bdd test-cucumber ## Run all tests
   ```

**Validation**: `make test-cucumber` still works, `make test-bdd` runs
(empty at first).

**Commit**: "Set up parallel rstest-bdd infrastructure"

### Phase 1: Pilot Migration - Simple Worlds (Weeks 2-3)

**Objective**: Validate approach with 2 simple worlds, establish
conversion patterns.

**Selected Worlds**:

1. `CorrelationWorld` (115 lines, 3 scenarios)
2. `RequestPartsWorld` (~150 lines, basic validation)

**Per-World Steps**:

1. Convert World struct → fixture
2. Migrate step definitions (remove `async`, add `block_in_place`)
3. Create scenario tests with `#[scenario]` + `#[tokio::test]`
4. Run and validate against Cucumber

**Example - CorrelationWorld**:

```rust
// tests/bdd/fixtures/correlation.rs
use rstest::fixture;

#[derive(Debug, Default)]
pub struct CorrelationWorld {
    expected: Option<u64>,
    frames: Vec<Envelope>,
}

#[fixture]
pub fn correlation_world() -> CorrelationWorld {
    CorrelationWorld::default()
}

// Methods stay async
impl CorrelationWorld {
    pub fn set_expected(&mut self, expected: Option<u64>) {
        self.expected = expected;
    }

    pub async fn process(&mut self) -> TestResult {
        // ... existing async code
    }

    pub fn verify(&self) -> TestResult {
        // ... existing sync code
    }
}
```

```rust
// tests/bdd/steps/correlation_steps.rs
use rstest_bdd_macros::{given, when, then};

#[given(expr = "a correlation id {int}")]
fn given_cid(world: &mut CorrelationWorld, id: u64) {
    world.set_expected(Some(id));
}

#[when("a stream of frames is processed")]
fn when_process(world: &mut CorrelationWorld) -> TestResult {
    tokio::task::block_in_place(|| {
        tokio::runtime::Handle::current().block_on(
            world.process()
        )
    })
}

#[then(expr = "each emitted frame uses correlation id {int}")]
fn then_verify(world: &mut CorrelationWorld, id: u64)
    -> TestResult
{
    if world.expected() != Some(id) {
        return Err("mismatched expected correlation id".into());
    }
    world.verify()
}
```

```rust
// tests/bdd/scenarios/correlation_scenarios.rs
use rstest_bdd_macros::scenario;
use crate::fixtures::correlation::*;

#[scenario(path = "tests/features/correlation_id.feature",
           name = "Streamed frames reuse the request correlation id")]
#[tokio::test(flavor = "current_thread")]
async fn streamed_frames_correlation(
    correlation_world: CorrelationWorld
) {}

#[scenario(
    path = "tests/features/correlation_id.feature",
    name = "Multi-packet responses reuse the request correlation id"
)]
#[tokio::test(flavor = "current_thread")]
async fn multi_packet_correlation(
    correlation_world: CorrelationWorld
) {}

#[scenario(
    path = "tests/features/correlation_id.feature",
    name = "Multi-packet responses clear correlation ids without \
           a request id"
)]
#[tokio::test(flavor = "current_thread")]
async fn no_correlation(correlation_world: CorrelationWorld) {}
```

**Validation**:

```bash
# Compare outputs
cargo test --test cucumber correlation
cargo test --test bdd correlation

# Both should pass all scenarios
```

**Commits**:

- "Migrate CorrelationWorld to rstest-bdd"
- "Migrate RequestPartsWorld to rstest-bdd"

### Phase 2: Medium Complexity Worlds (Weeks 4-5)

**Selected Worlds** (in order):

1. `PanicWorld` (server spawning pattern)
2. `MultiPacketWorld` (channel operations)
3. `StreamEndWorld` (actor processing)
4. `CodecStatefulWorld` (codec state)

**Focus**: Server lifecycle, channels, actors.

**Server Spawning Pattern**:

```rust
#[derive(ScenarioState)]
pub struct PanicWorld {
    // Spawned in step, not fixture
    server: Slot<PanicServer>,
}

#[fixture]
fn panic_world() -> PanicWorld {
    PanicWorld::default()  // Empty slot
}

#[given("a panic server")]
fn given_panic_server(world: &mut PanicWorld) -> TestResult {
    tokio::task::block_in_place(|| {
        tokio::runtime::Handle::current().block_on(async {
            let server = PanicServer::spawn().await?;
            world.server.set(server);
            Ok(())
        })
    })
}
```

**Commits**: One per world (4 commits).

### Phase 3: Complex Worlds - Client & Messaging (Weeks 6-7)

**Selected Worlds** (in order):

1. `ClientRuntimeWorld` (simpler client)
2. `ClientMessagingWorld` (server + client + envelope handling)
3. `ClientLifecycleWorld` (lifecycle hooks)
4. `ClientPreambleWorld` (preamble exchange)

**Focus**: Multi-step async sequences, server + client coordination,
callbacks.

**Multi-Async Step Pattern**:

```rust
#[given("an envelope echo server")]
fn given_echo_server(world: &mut ClientMessagingWorld) -> TestResult {
    tokio::task::block_in_place(|| {
        tokio::runtime::Handle::current().block_on(async {
            world.start_echo_server().await?;
            world.connect_client().await
        })
    })
}
```

**Commits**: One per world (4 commits).

### Phase 4: Specialized Worlds (Week 8)

**Selected Worlds**:

1. `MessageAssemblerWorld` (header parsing)
2. `MessageAssemblyWorld` (multiplexing)
3. `CodecErrorWorld` (multi-module structure)
4. `FragmentWorld` (multi-file, 11 scenarios)

**Focus**: Multi-file structures, high scenario counts.

**Multi-File Pattern** (FragmentWorld):

```rust
// tests/bdd/fixtures/fragment/
//   mod.rs         - Main world struct
//   reassembly.rs  - Helper types

pub mod reassembly;
use reassembly::*;

#[derive(Debug, ScenarioState)]
pub struct FragmentWorld {
    // ... fields
}

#[fixture]
pub fn fragment_world() -> FragmentWorld {
    FragmentWorld::default()
}
```

**Commits**: One per world (4 commits).

### Phase 5: Validation & Cleanup (Week 9)

**Tasks**:

1. **Comprehensive comparison**:

   ```bash
   cargo test --test cucumber > cucumber-output.txt 2>&1
   cargo test --test bdd > bdd-output.txt 2>&1
   # Compare scenario counts, all should pass
   ```

2. **Enable strict validation**:

   ```toml
   rstest-bdd-macros = { version = "0.4.0",
       features = ["strict-compile-time-validation"] }
   ```

3. **Performance check**:

   ```bash
   hyperfine 'cargo test --test cucumber' \
             'cargo test --test bdd'
   # Should be within 10-20%
   ```

4. **Remove Cucumber infrastructure**:
   - Delete `tests/cucumber.rs`
   - Delete `tests/worlds/`
   - Delete `tests/steps/`
   - Remove `cucumber = "0.21.1"` from `Cargo.toml`
   - Update Makefile: `test` → `test-bdd` only

5. **Rename structure** (optional cleanup):

   ```bash
   mv tests/bdd/fixtures tests/fixtures
   mv tests/bdd/steps tests/steps
   mv tests/bdd/scenarios tests/scenarios
   # Update imports
   ```

**Commits**:

- "Enable strict compile-time validation"
- "Remove Cucumber infrastructure"
- "Rename bdd structure to standard layout"

## Migration Progress Tracking

| Phase | Worlds | Scenarios | Status         | Completion |
| ----- | ------ | --------- | -------------- | ---------- |
| 0     | -      | -         | Complete       | 2026-01-22 |
| 1     | 2      | 6         | In Progress    | 1/2 done   |
| 2     | 4      | 15        | Not Started    | -          |
| 3     | 4      | 20        | Not Started    | -          |
| 4     | 4      | 19+       | Not Started    | -          |
| 5     | -      | -         | Not Started    | -          |

**Total**: 14 worlds, 60+ scenarios

## Risk Mitigation

### Risk 1: Async Boundary Issues

**Mitigation**: Test `block_in_place` pattern in Phase 1 before
widespread adoption. Keep complex `ClientMessagingWorld` for Phase 3
after validation.

**Contingency**: Create helper async functions if `block_in_place` has
issues.

### Risk 2: Server Spawning Conflicts

**Mitigation**: Use `Slot<Server>` pattern, not direct fixture spawn.
Test in Phase 2 with `PanicWorld`.

### Risk 3: Fragment.feature Complexity (11 scenarios)

**Mitigation**: Migrate in Phase 4 after patterns proven. Can use
`scenarios!` macro if individual tests become verbose.

### Risk 4: Compile-Time Validation False Positives

**Mitigation**: Start with `compile-time-validation` (warnings only),
enable strict mode in Phase 5.

### Risk 5: Migration Timeline Slippage

**Mitigation**: Strict phase boundaries. Parallel execution allows
partial migration. Can pause after any phase.

## Critical Files

### Phase 0 (Foundation)

1. `Cargo.toml` - Dependencies, test targets
2. `tests/bdd/mod.rs` - New test module root
3. `Makefile` - Test targets

### Phase 1 (Pilot)

1. `tests/bdd/fixtures/correlation.rs` - First fixture
2. `tests/bdd/steps/correlation_steps.rs` - First steps
3. `tests/bdd/scenarios/correlation_scenarios.rs` - First scenarios
4. `tests/bdd/fixtures/request_parts.rs`
5. `tests/bdd/steps/request_parts_steps.rs`
6. `tests/bdd/scenarios/request_parts_scenarios.rs`

### Phase 2 (Medium Complexity)

Panic, MultiPacket, StreamEnd, CodecStateful (fixtures, steps,
scenarios) - 12 files total

### Phase 3 (Complex)

ClientRuntime, ClientMessaging, ClientLifecycle, ClientPreamble - 12
files total

### Phase 4 (Specialized)

MessageAssembler, MessageAssembly, CodecError, Fragment - 12 files
total

### Phase 5 (Cleanup)

1. `Cargo.toml` - Remove cucumber dependency
2. `tests/cucumber.rs` - DELETE
3. `tests/worlds/` - DELETE (directory)
4. `tests/steps/` - DELETE (old Cucumber steps)

## Verification

### Per-Phase Validation

After each phase:

- [ ] All migrated scenarios pass: `cargo test --test bdd`
- [ ] Cucumber still works: `cargo test --test cucumber`
- [ ] No compile warnings
- [ ] Output matches Cucumber behavior
- [ ] Commit gateways pass (lint, format)

### Final Validation (Phase 5)

- [ ] All 60+ scenarios passing
- [ ] Strict compile-time validation enabled
- [ ] No undefined steps
- [ ] No unused step definitions
- [ ] Performance within 10-20% of Cucumber
- [ ] Cucumber infrastructure removed
- [ ] CI pipeline updated
- [ ] Documentation updated

## Helper Utilities

Create shared async helper:

```rust
// tests/bdd/async_helpers.rs
/// Execute an async closure within the current tokio runtime.
pub fn run_async<F, T>(f: F) -> T
where
    F: std::future::Future<Output = T>,
{
    tokio::task::block_in_place(|| {
        tokio::runtime::Handle::current().block_on(f)
    })
}

// Usage in steps:
#[when("server starts")]
fn when_server_starts(world: &mut ServerWorld) -> TestResult {
    run_async(world.start_server())
}
```

## Success Criteria

- [ ] All 14 worlds migrated to rstest-bdd fixtures
- [ ] All 60+ scenarios passing under `cargo test`
- [ ] Cucumber infrastructure removed
- [ ] Strict compile-time validation enabled
- [ ] No test coverage gaps
- [ ] Performance comparable to Cucumber
- [ ] Clean CI pipeline (single test command)
- [ ] Team onboarded to rstest-bdd patterns

## Lessons Learned

### Phase 1: CorrelationWorld Migration (Completed)

**CRITICAL DISCOVERY**: The async scenario approach outlined in the plan does
NOT work with rstest-bdd v0.4.0's current implementation.

**The Problem**:

- Scenarios marked with `#[tokio::test(flavor = "current_thread")] async fn`
  create a tokio runtime
- Steps are sync and must remain sync (documented limitation)
- Attempting to call `tokio::runtime::Handle::current().block_on()` from
  within a step fails with "Cannot start runtime within runtime"
- Attempting to create `Runtime::new()` in a step also fails when the
  scenario itself is async

**The Solution**:

- Scenarios must be **sync functions** (remove `async fn` and
  `#[tokio::test]`)
- Steps remain sync and create their own `Runtime::new()` for async
  operations
- Pattern: `let rt = tokio::runtime::Runtime::new()?; rt.block_on(async_fn())`

**Updated Async Pattern**:

```rust
// Scenario: SYNC (not async!)
#[scenario(path = "tests/features/correlation_id.feature",
           name = "Streamed frames reuse the request correlation id")]
fn streamed_frames_correlation(correlation_world: CorrelationWorld) {
    let _ = correlation_world;
}

// Step: Sync, creates own runtime
#[when("a stream of frames is processed")]
fn when_process(correlation_world: &mut CorrelationWorld) -> TestResult {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(correlation_world.process())
}
```

**Impact on Migration Plan**:

- All references to `#[tokio::test(flavor = "current_thread")]` in scenario
  examples should be removed
- All scenario functions are sync, not async
- `tokio::task::block_in_place` is NOT needed and won't work
- Each async step creates its own `Runtime::new()` and uses `block_on()`

**Validation**: CorrelationWorld migration complete with all 3 scenarios
passing, verified against Cucumber output.

## References

- [rstest-bdd User's Guide](../rstest-bdd-users-guide.md)
- [ADR-003: Replace Cucumber with
  rstest-bdd](../adr-003-replace-cucumber-with-rstest-bdd.md)
- [Plan Agent Output](https://claude.ai) - Agent ID: a9eb419
