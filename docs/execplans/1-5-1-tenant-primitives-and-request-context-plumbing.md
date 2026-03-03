# Establish tenant primitives and request context plumbing (1.5.1)

This ExecPlan (execution plan) is a living document. The sections
`Constraints`, `Tolerances`, `Risks`, `Progress`, `Surprises & Discoveries`,
`Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work
proceeds.

Status: COMPLETE

## Purpose / big picture

Wireframe is a Rust library for building asynchronous binary protocol servers
and clients. Today it has no concept of tenancy or cross-cutting request
identity. Handlers receive a `MessageRequest` carrying only a peer address and
type-erased shared state — there is no structured way to thread tenant, user,
correlation, causation, or session identifiers through the request pipeline.

After this change a library consumer can:

1. Construct `TenantId`, `TenantSlug`, `UserId`, and `Tenant` domain
   primitives that model multi-tenant identity with one owning user per tenant,
   while keeping the user-versus-tenant identity model separate for future team
   and organisation tenants.
2. Build a `RequestContext` carrying tenant, user, session, correlation, and
   causation identifiers and inject it into the request pipeline via the
   existing `AppDataStore`/extractor mechanism.
3. Extract `RequestContext` in handlers via `FromMessageRequest`, then call
   `require_tenant_id()` or `require_user_id()` to enforce tenant-awareness at
   compile-guided call sites.

Observable success: `make test` passes with new unit tests (rstest) and
behavioural tests (rstest-bdd v0.5.0) exercising all domain primitives and the
`RequestContext` extractor. `make lint` and `make check-fmt` pass. The roadmap
entry is marked done.

## Constraints

Hard invariants. Violation requires escalation, not workarounds.

- No single source file may exceed 400 lines.
- Public APIs must be documented with rustdoc (`///`) including examples.
- Every module must begin with a `//!` module-level comment.
- All code must pass `make check-fmt`, `make lint` (clippy `-D warnings`), and
  `make test`.
- Comments and documentation must use en-GB-oxendict spelling ("-ize" /
  "-yse" / "-our").
- Tests must use `rstest` fixtures; BDD tests use `rstest-bdd` v0.5.0.
- Existing public API signatures must not change (backwards compatibility).
- No new external crate dependencies may be added. All types needed (`u64`,
  `String`, `Arc`, `thiserror`) are already available.
- `docs/users-guide.md` must be updated with the new public API surface.
- `docs/roadmap.md` must be updated upon completion.
- The design document `docs/corbusier-design.md` referenced in the task does
  not exist. Design decisions are captured in this ExecPlan's Decision Log and
  will be recorded in a design section within this document.

## Tolerances (exception triggers)

- Scope: if implementation requires changes to more than 20 files (net new +
  modified), stop and escalate.
- LOC: if net new code exceeds 1200 lines (excluding tests), stop and
  escalate.
- Interface: if any existing public API signature must change (not just adding
  new methods/modules), stop and escalate.
- Dependencies: if a new external crate is required, stop and escalate.
- Iterations: if the same gate fails after 3 focused attempts, stop and
  escalate.
- File size: if any source file would exceed 400 lines, extract into
  submodules before continuing.

## Risks

- Risk: `RequestContext` may couple the framework to a specific multi-tenancy
  model, limiting flexibility for downstream consumers. Severity: medium
  Likelihood: low Mitigation: all `RequestContext` fields are `Option`-wrapped.
  Consumers that do not use tenancy simply ignore the tenant fields. The
  `require_*` methods are opt-in enforcement, not mandatory.

- Risk: introducing `CorrelationId` newtype alongside the existing raw
  `Option<u64>` correlation pattern in `CorrelatableFrame`, `ServiceRequest`,
  and `ServiceResponse` may cause confusion. Severity: medium Likelihood:
  medium Mitigation: document the distinction clearly. `CorrelationId` provides
  `From<u64>` and `as_u64()` for bridge code. Migration of existing APIs to the
  newtype is explicitly deferred.

- Risk: the `ExtractError` enum is `#[non_exhaustive]`, so adding variants is
  safe, but downstream match arms using `_` will silently absorb new errors.
  Severity: low Likelihood: low Mitigation: the `#[non_exhaustive]` attribute
  is intentionally designed for this purpose. Document new variants in the
  changelog.

## Progress

- [x] (2026-03-03) Drafted ExecPlan and obtained approval.
- [x] (2026-03-03) Stage A: tenant domain primitives (`src/tenant/`).
- [x] (2026-03-03) Stage B: cross-cutting identifier newtypes and
  `RequestContext` (`src/context/`).
- [x] (2026-03-03) Stage C: `RequestContext` extractor
  (`src/extractor/context.rs`).
- [x] (2026-03-03) Stage D: unit tests (rstest) integrated in source
  files under `#[cfg(test)]`.
- [x] (2026-03-03) Stage E: BDD behavioural tests (rstest-bdd v0.5.0).
- [x] (2026-03-03) Stage F: documentation updates (users-guide, roadmap,
  prelude).
- [x] (2026-03-03) Stage G: quality gates passed — `make check-fmt`,
  `make lint`, `make test`, `make markdownlint`.

## Surprises & discoveries

- Observation: `MessageRequest` does not implement `Debug`, so BDD
  world structs holding it cannot derive `Debug`. Evidence: compilation error
  when using `#[derive(Debug)]` on `RequestContextWorld`. Impact: required a
  manual `Debug` impl that redacts the `MessageRequest` field. This matches the
  known gotcha for `WireframeApp` (also lacks `Debug`).

- Observation: rstest-bdd matches fixture parameters by name; a
  leading underscore prefix (`_request_context_world`) breaks the match.
  Evidence: `default_context` scenario panicked with "missing fixtures:
  `_request_context_world`". Impact: unused fixture parameters in step
  functions must use `let _ = param;` instead of prefixing with `_`.

- Observation: clippy `struct_field_names` fires when all fields of a
  struct share the same postfix (`_id`). Evidence: `RequestContext` has
  `tenant_id`, `user_id`, `session_id`, `correlation_id`, `causation_id`.
  Impact: added a scoped `#[expect(clippy::struct_field_names)]` with a reason
  string. The `_id` suffix is the domain name, not a naming defect.

## Decision log

- Decision: use `u64` as the backing type for `TenantId`, `UserId`,
  `CorrelationId`, `CausationId`, and `SessionId` rather than UUID. Rationale:
  the codebase has no `uuid` dependency. Adding one would require escalation
  per the tolerance rules. The `ConnectionId(u64)` pattern in `src/session.rs`
  is well-established. For a framework library the backing type should be
  lightweight and serialisation-agnostic. Applications needing UUIDs can wrap
  these types in their domain layer. The `u64` approach also aligns with binary
  protocol efficiency (8 bytes versus 16 for UUID). Date/Author: 2026-03-01 /
  agent.

- Decision: place tenant domain primitives in `src/tenant/` and cross-cutting
  context in `src/context/` as separate modules. Rationale: follows the "group
  by feature" guidance in AGENTS.md. Tenant primitives (`TenantId`,
  `TenantSlug`, `UserId`, `Tenant`) are domain identity types. `RequestContext`
  is a cross-cutting concern carrying identifiers from multiple domains
  (tenant, user, session, tracing). Keeping them separate makes each
  discoverable and avoids overloading either module. Date/Author: 2026-03-01 /
  agent.

- Decision: `RequestContext` is stored in `AppDataStore` and extracted via
  `FromMessageRequest`, following the existing `SharedState<T>` pattern.
  Rationale: the `AppDataStore` already supports type-erased
  `Send + Sync + 'static` values via `DashMap<TypeId, Arc<dyn Any>>`.
  `RequestContext` can be inserted per-request by framework code or protocol
  hooks and extracted by handlers. This requires no changes to existing APIs.
  Date/Author: 2026-03-01 / agent.

- Decision: `RequestContext` extraction returns `Default` (all-`None`) when no
  context was registered, rather than failing. Rationale: this makes
  `RequestContext` extraction infallible in practice. Handlers that require
  specific fields call `require_tenant_id()` etc. after extraction, producing
  clear errors at the point of use rather than at extraction time. This mirrors
  how `ConnectionInfo` is infallible. Date/Author: 2026-03-01 / agent.

- Decision: keep user and tenant as separate identity concepts. `Tenant` has
  an `owner: UserId` field, but `UserId` is not a `TenantId`. Rationale: the
  task requires "preserving a separate user-versus-tenant identity model for
  future team and organisation tenants". The initial model is one owning user
  per tenant, but the type system does not conflate the two identities.
  Date/Author: 2026-03-01 / agent.

- Decision: use explicit tuple structs (not `newt-hype` or `the-newtype`
  crates) for all newtypes. Rationale: neither crate is in the dependency list.
  AGENTS.md says "Prefer explicit tuple structs whenever bespoke validation or
  tailored trait surfaces are required." Each type here needs bespoke
  `Display`, accessors, and potentially different trait impls, so explicit
  structs are appropriate. Date/Author: 2026-03-01 / agent.

- Decision: the design document `docs/corbusier-design.md` referenced in the
  task does not exist. All design decisions are captured in this ExecPlan.
  Rationale: the file is absent from the repository. The task description
  provides sufficient requirements to derive the design. If the design document
  is created later, it should reference this ExecPlan's decision log.
  Date/Author: 2026-03-01 / agent.

## Outcomes & retrospective

All acceptance criteria met. The implementation delivers:

- `TenantId`, `TenantSlug`, `UserId`, `Tenant` in `src/tenant/`
  (primitives.rs: 280 lines including tests).
- `CorrelationId`, `CausationId`, `SessionId` in
  `src/context/identifiers.rs` (245 lines including tests).
- `RequestContext`, `MissingTenantError`, `MissingUserError` in
  `src/context/request_context.rs` (280 lines including tests).
- `FromMessageRequest for RequestContext` in
  `src/extractor/context.rs` (83 lines including tests).
- 4 BDD scenarios in `tests/features/request_context.feature`.
- Documentation in `docs/users-guide.md` and `docs/roadmap.md`.
- `RequestContext` exported via `wireframe::prelude`.

Quality gates: `make check-fmt`, `make lint`, `make test`, and
`make markdownlint` all pass.

Lessons: (1) always use manual `Debug` impls for test world structs holding
framework types that lack `Debug`; (2) never prefix rstest-bdd fixture
parameters with `_` — use `let _ = param` instead; (3) `struct_field_names`
clippy lint fires on identity-heavy structs.

## Context and orientation

Wireframe (`wireframe` crate, `src/lib.rs`) is structured around these key
modules relevant to this task:

**Extractor system** (`src/extractor/`):

- `src/extractor/mod.rs` — re-exports all extractors.
- `src/extractor/trait_def.rs` — defines `FromMessageRequest` trait
  with `from_message_request(req, payload) -> Result<Self, Error>`.
- `src/extractor/request.rs` — `MessageRequest` struct with
  `peer_addr`, `app_data: AppDataStore`, and `body_stream`. Provides
  `state::<T>()` and `insert_state::<T>()` for type-erased storage.
- `src/extractor/connection_info.rs` — `ConnectionInfo` extractor
  (infallible). The canonical pattern for a simple extractor.
- `src/extractor/shared_state.rs` — `SharedState<T>` extractor (fallible,
  returns `ExtractError::MissingState`).
- `src/extractor/error.rs` — `ExtractError` enum (`#[non_exhaustive]`) with
  `MissingState`, `InvalidPayload`, `MissingBodyStream`.

**Identity types** (`src/session.rs`):

- `ConnectionId(u64)` — the canonical newtype pattern in this codebase.
  Derives `Clone, Copy, Debug, PartialEq, Eq, Hash`. Has `From<u64>`, `new()`,
  `as_u64()`, `Display`.

**Type-erased storage** (`src/app_data_store.rs`):

- `AppDataStore` — `DashMap<TypeId, Arc<dyn Any + Send + Sync>>` with
  `insert::<T>()`, `get::<T>()`, `contains::<T>()`, `remove::<T>()`.

**Correlation** (`src/correlation.rs`):

- `CorrelatableFrame` trait with `correlation_id() -> Option<u64>` and
  `set_correlation_id()`. Raw `u64`, no newtype.

**BDD test structure** (4-file pattern per feature):

1. `tests/features/<name>.feature` — Gherkin feature file.
2. `tests/fixtures/<name>.rs` — world struct + rstest fixture function.
3. `tests/steps/<name>_steps.rs` — `#[given]`/`#[when]`/`#[then]` step
   definitions.
4. `tests/scenarios/<name>_scenarios.rs` — `#[scenario]` test bindings.
5. Registration in `tests/fixtures/mod.rs`, `tests/steps/mod.rs`,
   `tests/scenarios/mod.rs`.

**BDD entry point** (`tests/bdd/mod.rs`):

- Loads fixtures, scenarios, and steps via `#[path]` attributes.
- BDD tests require `--features advanced-tests`.

**Key Cargo.toml facts**:

- `serde = "1.0.219"` with `derive` feature (available for `Serialize`,
  `Deserialize`).
- `derive_more = "2.0.1"` with `display`, `from` features.
- `thiserror = "2.0.16"` (for error types).
- No `uuid` crate.
- Strict clippy lints: `unwrap_used`, `expect_used`, `indexing_slicing`,
  `panic_in_result_fn` all denied.

## Plan of work

### Stage A: tenant domain primitives (`src/tenant/`)

Create a new `src/tenant/` module containing `TenantId`, `TenantSlug`,
`UserId`, and `Tenant`.

**New file: `src/tenant/mod.rs`** (~15 lines)

Module declaration and re-exports.

**New file: `src/tenant/primitives.rs`** (~250 lines including unit tests)

Four types:

- `TenantId(u64)` — derives `Clone, Copy, Debug, PartialEq, Eq, Hash`. Has
  `From<u64>`, `new(u64)`, `as_u64()`, `Display`. Serde `Serialize` and
  `Deserialize`.
- `TenantSlug(String)` — derives `Clone, Debug, PartialEq, Eq, Hash`. Has
  `new(impl Into<String>)`, `as_str()`, `From<&str>`, `From<String>`,
  `AsRef<str>`, `Display`. Serde `Serialize` and `Deserialize`.
- `UserId(u64)` — same pattern as `TenantId`.
- `Tenant` — struct with `id: TenantId`, `slug: TenantSlug`, `owner: UserId`.
  Constructor and accessor methods. Serde `Serialize` and `Deserialize`.

**Modify: `src/lib.rs`** — add `pub mod tenant;` (1 line).

Unit tests within `src/tenant/primitives.rs` under `#[cfg(test)]`:

- Round-trip `TenantId` through `From<u64>` and `as_u64()`.
- `TenantSlug` from `&str`, from `String`, `as_str()`, `AsRef<str>`.
- `Tenant` construction and accessor methods.
- `Display` implementations produce expected output.
- `UserId` round-trip and `Display`.
- Equality and hashing for all identity types.

Go/no-go: `cargo check` succeeds. `cargo test --lib tenant` passes.

### Stage B: cross-cutting identifiers and `RequestContext` (`src/context/`)

Create a new `src/context/` module with `CorrelationId`, `CausationId`,
`SessionId`, and `RequestContext`.

**New file: `src/context/mod.rs`** (~15 lines)

Module declaration and re-exports.

**New file: `src/context/identifiers.rs`** (~180 lines including unit tests)

Three identity newtypes following the `TenantId` pattern:

- `CorrelationId(u64)` — links related operations.
- `CausationId(u64)` — traces the triggering operation.
- `SessionId(u64)` — links operations within a logical session (distinct from
  `ConnectionId` which is a transport concern).

Each has `Clone, Copy, Debug, PartialEq, Eq, Hash`, `From<u64>`, `new()`,
`as_u64()`, `Display`, and serde derives.

Unit tests: round-trip and `Display` for each type.

**New file: `src/context/request_context.rs`** (~280 lines including unit tests)

```rust
/// Cross-cutting request context carrying tenant, user, and tracing
/// identifiers.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RequestContext {
    tenant_id: Option<TenantId>,
    user_id: Option<UserId>,
    session_id: Option<SessionId>,
    correlation_id: Option<CorrelationId>,
    causation_id: Option<CausationId>,
}
```

Builder methods (each consuming `self`, returning `Self`):

- `with_tenant_id(self, id: TenantId) -> Self`
- `with_user_id(self, id: UserId) -> Self`
- `with_session_id(self, id: SessionId) -> Self`
- `with_correlation_id(self, id: CorrelationId) -> Self`
- `with_causation_id(self, id: CausationId) -> Self`

Accessors (each returning `Option<T>`):

- `tenant_id(&self) -> Option<TenantId>`
- `user_id(&self) -> Option<UserId>`
- `session_id(&self) -> Option<SessionId>`
- `correlation_id(&self) -> Option<CorrelationId>`
- `causation_id(&self) -> Option<CausationId>`

Enforcement methods:

- `require_tenant_id(&self) -> Result<TenantId, MissingTenantError>`
- `require_user_id(&self) -> Result<UserId, MissingUserError>`

Bridge method:

- `from_raw_correlation(correlation_id: Option<u64>) -> Self` — creates a
  context from the existing raw `Option<u64>` pattern.

Error types (in the same file):

```rust
/// Error indicating a required tenant identifier is missing.
#[derive(Debug, thiserror::Error)]
#[error("tenant identifier is required but not present in request context")]
pub struct MissingTenantError;

/// Error indicating a required user identifier is missing.
#[derive(Debug, thiserror::Error)]
#[error("user identifier is required but not present in request context")]
pub struct MissingUserError;
```

**Modify: `src/lib.rs`** — add `pub mod context;` (1 line).

Unit tests within `src/context/request_context.rs` under `#[cfg(test)]`:

- Default context has all fields `None`.
- Builder chaining sets individual fields.
- `require_tenant_id` returns `Ok` when present, `Err(MissingTenantError)`
  when absent.
- `require_user_id` returns `Ok` when present, `Err(MissingUserError)` when
  absent.
- `from_raw_correlation` bridges `Option<u64>` to `CorrelationId`.
- `Clone` produces equal context.

Go/no-go: `cargo check` succeeds. `cargo test --lib context` passes.

### Stage C: `RequestContext` extractor (`src/extractor/context.rs`)

**New file: `src/extractor/context.rs`** (~70 lines including unit tests)

Implements `FromMessageRequest for RequestContext`. The extraction is
infallible in practice: if no `RequestContext` was registered in the
`AppDataStore`, it returns `RequestContext::default()` (all `None` fields).
Handlers that need specific fields call `require_tenant_id()` etc.

```rust
impl FromMessageRequest for RequestContext {
    type Error = std::convert::Infallible;

    fn from_message_request(
        req: &MessageRequest,
        _payload: &mut Payload<'_>,
    ) -> Result<Self, Self::Error> {
        Ok(req
            .state::<RequestContext>()
            .map(|s| (*s).clone())
            .unwrap_or_default())
    }
}
```

Note: using `Infallible` (not `ExtractError`) because this extraction cannot
fail. This follows the `ConnectionInfo` pattern exactly.

**Modify: `src/extractor/mod.rs`** — add `mod context;` (1 line).

Unit tests within `src/extractor/context.rs` under `#[cfg(test)]`:

- Extraction returns default when no context is registered.
- Extraction returns the stored context when one is registered via
  `insert_state`.

Go/no-go: `cargo check` succeeds. `cargo test --lib extractor::context` passes.

### Stage D: BDD behavioural tests (rstest-bdd v0.5.0)

Create the 4-file BDD test set following the established project pattern.

**New file: `tests/features/request_context.feature`** (~35 lines)

```gherkin
@request_context
Feature: Tenant-aware request context
  The framework provides a cross-cutting request context carrying
  tenant, user, and tracing identifiers through the request pipeline.

  Scenario: Request context carries tenant identifier
    Given a request-context with tenant id 42
    When the request-context is extracted from the message request
    Then the extracted tenant id is 42

  Scenario: Request context carries all cross-cutting identifiers
    Given a request-context with tenant id 1 and user id 2 and correlation id 3
    When the request-context is extracted from the message request
    Then the extracted tenant id is 1
    And the extracted user id is 2
    And the extracted correlation id is 3

  Scenario: Missing tenant identifier fails require check
    Given a request-context without a tenant id
    When require-tenant-id is called
    Then a missing tenant error is returned

  Scenario: Default context has no identifiers
    Given no request-context is registered
    When the request-context is extracted from the message request
    Then all extracted identifiers are absent
```

**New file: `tests/fixtures/request_context.rs`** (~130 lines)

`RequestContextWorld` struct holding:

- `request: MessageRequest`
- `extracted: Option<RequestContext>`
- `tenant_error: Option<MissingTenantError>`

`request_context_world()` rstest fixture function.

Helper methods for context creation, insertion into `MessageRequest` via
`insert_state`, extraction via `FromMessageRequest`, and verification.

**New file: `tests/steps/request_context_steps.rs`** (~110 lines)

Step functions using `request-context` prefix to avoid collisions.

All step function parameters named `request_context_world` (rstest-bdd naming
requirement — parameter name must match the fixture function name).

Steps return `TestResult` (`Result<(), Box<dyn Error + Send + Sync>>`).

**New file: `tests/scenarios/request_context_scenarios.rs`** (~25 lines)

`#[scenario]` test functions binding each feature scenario to the fixture.

**Modify registration files:**

- `tests/fixtures/mod.rs` — add `pub mod request_context;`
- `tests/steps/mod.rs` — add `mod request_context_steps;`
- `tests/scenarios/mod.rs` — add `mod request_context_scenarios;`

Go/no-go: `cargo test --test bdd --all-features request_context` passes.

### Stage E: documentation updates

1. **Update `docs/users-guide.md`**: add a section on tenant primitives and
   request context, explaining `TenantId`, `TenantSlug`, `UserId`, `Tenant`,
   and `RequestContext` with usage examples.

2. **Update `docs/roadmap.md`**: add a new section 1.5 "Tenant context and
   identity isolation" (renumbering the existing 1.5 "Basic testing" to avoid
   collision, or inserting after section 1.4 and before the existing 1.5) with
   item 1.5.1 marked `[x]`.

3. **Update `src/prelude.rs`**: add `RequestContext` to the prelude exports so
   `use wireframe::prelude::*` includes it.

### Stage F: quality gates

Run all quality gates and fix any issues:

```shell
set -o pipefail
make fmt 2>&1 | tee /tmp/wireframe-1-5-1-fmt.log
make check-fmt 2>&1 | tee /tmp/wireframe-1-5-1-check-fmt.log
make lint 2>&1 | tee /tmp/wireframe-1-5-1-lint.log
make test 2>&1 | tee /tmp/wireframe-1-5-1-test.log
make markdownlint 2>&1 | tee /tmp/wireframe-1-5-1-markdownlint.log
```

Go/no-go: all five commands exit 0.

## Concrete steps

All commands run from the repository root `/home/user/project`.

Stage A:

1. Create `src/tenant/mod.rs` and `src/tenant/primitives.rs`.
2. Add `pub mod tenant;` to `src/lib.rs`.
3. Run `cargo check` to verify compilation.
4. Run `cargo test --lib tenant` to verify unit tests pass.

Stage B:

1. Create `src/context/mod.rs`, `src/context/identifiers.rs`, and
   `src/context/request_context.rs`.
2. Add `pub mod context;` to `src/lib.rs`.
3. Run `cargo check`.
4. Run `cargo test --lib context` to verify unit tests pass.

Stage C:

1. Create `src/extractor/context.rs`.
2. Add `mod context;` to `src/extractor/mod.rs`.
3. Run `cargo check`.
4. Run `cargo test --lib extractor` to verify unit tests pass.

Stage D:

1. Create `tests/features/request_context.feature`.
2. Create `tests/fixtures/request_context.rs`.
3. Create `tests/steps/request_context_steps.rs`.
4. Create `tests/scenarios/request_context_scenarios.rs`.
5. Add module registrations to `tests/fixtures/mod.rs`,
   `tests/steps/mod.rs`, `tests/scenarios/mod.rs`.
6. Run
   `set -o pipefail && cargo test --test bdd --all-features 2>&1 | tee /tmp/wireframe-1-5-1-bdd.log`.

Stage E:

1. Update `docs/users-guide.md`.
2. Update `docs/roadmap.md`.
3. Update `src/prelude.rs`.

Stage F:

1. Run all quality gates as listed above.
2. Fix any issues and re-run until all pass.

## Validation and acceptance

Success criteria from the task:

1. `TenantId`, `TenantSlug`, and `Tenant` domain primitives exist with full
   newtype semantics. Verified by: `cargo doc --no-deps` shows the types; unit
   tests in `src/tenant/primitives.rs` exercise construction, accessors,
   `Display`, equality, and hashing.

2. Initial tenancy is one owning user per tenant. Verified by: `Tenant` struct
   has `owner: UserId` field; construction test creates a tenant with an owner.

3. Separate user-versus-tenant identity model. Verified by: `UserId` and
   `TenantId` are distinct newtypes with no `From` conversion between them.

4. Cross-cutting `RequestContext` replaces ad-hoc audit context. Verified by:
   `RequestContext` carries all identifiers, is extractable via
   `FromMessageRequest`, and BDD tests demonstrate extraction.

5. Repository/service signatures can require tenant-aware request context.
   Verified by: `RequestContext::require_tenant_id()` returns
   `Result<TenantId, MissingTenantError>`, enabling function signatures like
   `fn create_widget(ctx: &RequestContext) -> Result<..>` where the caller
   handles the missing tenant case. BDD test exercises the failure path.

Quality criteria:

- Tests: `make test` passes (includes unit, integration, BDD, and doctests).
- Lint: `make lint` passes with no warnings.
- Format: `make check-fmt` passes.
- Markdown: `make markdownlint` passes.
- Docs: `cargo doc --no-deps` generates without warnings.

## Idempotence and recovery

All stages create new files or append to existing files. No existing code is
modified destructively. Each stage can be re-run by deleting the new files and
starting over. The BDD test registration lines are additive (appended to
`mod.rs` files) and can be removed individually.

If a quality gate fails partway through, fix the issue and re-run the gate. No
rollback is needed beyond fixing the immediate problem.

## Artifacts and notes

### New files (10)

| File                                           | Est. lines | Purpose                                              |
| ---------------------------------------------- | ---------- | ---------------------------------------------------- |
| `src/tenant/mod.rs`                            | ~15        | Module declaration                                   |
| `src/tenant/primitives.rs`                     | ~250       | `TenantId`, `TenantSlug`, `UserId`, `Tenant` + tests |
| `src/context/mod.rs`                           | ~15        | Module declaration                                   |
| `src/context/identifiers.rs`                   | ~180       | `CorrelationId`, `CausationId`, `SessionId` + tests  |
| `src/context/request_context.rs`               | ~280       | `RequestContext` + errors + tests                    |
| `src/extractor/context.rs`                     | ~70        | `FromMessageRequest` impl + tests                    |
| `tests/features/request_context.feature`       | ~35        | BDD feature file                                     |
| `tests/fixtures/request_context.rs`            | ~130       | BDD world and fixture                                |
| `tests/steps/request_context_steps.rs`         | ~110       | BDD step definitions                                 |
| `tests/scenarios/request_context_scenarios.rs` | ~25        | BDD scenario bindings                                |

### Modified files (6)

| File                     | Change                                       |
| ------------------------ | -------------------------------------------- |
| `src/lib.rs`             | Add `pub mod tenant;` and `pub mod context;` |
| `src/extractor/mod.rs`   | Add `mod context;`                           |
| `src/prelude.rs`         | Add `RequestContext` to prelude              |
| `tests/fixtures/mod.rs`  | Add `pub mod request_context;`               |
| `tests/steps/mod.rs`     | Add `mod request_context_steps;`             |
| `tests/scenarios/mod.rs` | Add `mod request_context_scenarios;`         |

### Documentation files modified (2)

| File                  | Change                                            |
| --------------------- | ------------------------------------------------- |
| `docs/users-guide.md` | Add tenant primitives and request context section |
| `docs/roadmap.md`     | Add section 1.5 and mark 1.5.1 done               |

Total: ~16 files, ~1100 estimated net lines (including tests).

## Interfaces and dependencies

### `src/tenant/primitives.rs`

```rust
/// Unique identifier for a tenant.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash,
         serde::Serialize, serde::Deserialize)]
pub struct TenantId(u64);

impl TenantId {
    pub fn new(id: u64) -> Self;
    pub fn as_u64(&self) -> u64;
}

/// Human-readable tenant slug for URL-friendly identification.
#[derive(Clone, Debug, PartialEq, Eq, Hash,
         serde::Serialize, serde::Deserialize)]
pub struct TenantSlug(String);

impl TenantSlug {
    pub fn new(slug: impl Into<String>) -> Self;
    pub fn as_str(&self) -> &str;
}

/// Unique identifier for a user.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash,
         serde::Serialize, serde::Deserialize)]
pub struct UserId(u64);

impl UserId {
    pub fn new(id: u64) -> Self;
    pub fn as_u64(&self) -> u64;
}

/// A tenant with its identifying metadata and owning user.
#[derive(Clone, Debug, PartialEq, Eq,
         serde::Serialize, serde::Deserialize)]
pub struct Tenant {
    id: TenantId,
    slug: TenantSlug,
    owner: UserId,
}

impl Tenant {
    pub fn new(id: TenantId, slug: TenantSlug, owner: UserId) -> Self;
    pub fn id(&self) -> TenantId;
    pub fn slug(&self) -> &TenantSlug;
    pub fn owner(&self) -> UserId;
}
```

### `src/context/identifiers.rs`

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash,
         serde::Serialize, serde::Deserialize)]
pub struct CorrelationId(u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash,
         serde::Serialize, serde::Deserialize)]
pub struct CausationId(u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash,
         serde::Serialize, serde::Deserialize)]
pub struct SessionId(u64);
```

Each has `new(u64)`, `as_u64()`, `From<u64>`, and `Display`.

### `src/context/request_context.rs`

```rust
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RequestContext { /* fields as described above */ }

impl RequestContext {
    pub fn new() -> Self;
    pub fn with_tenant_id(self, id: TenantId) -> Self;
    pub fn with_user_id(self, id: UserId) -> Self;
    pub fn with_session_id(self, id: SessionId) -> Self;
    pub fn with_correlation_id(self, id: CorrelationId) -> Self;
    pub fn with_causation_id(self, id: CausationId) -> Self;
    pub fn tenant_id(&self) -> Option<TenantId>;
    pub fn user_id(&self) -> Option<UserId>;
    pub fn session_id(&self) -> Option<SessionId>;
    pub fn correlation_id(&self) -> Option<CorrelationId>;
    pub fn causation_id(&self) -> Option<CausationId>;
    pub fn require_tenant_id(&self) -> Result<TenantId, MissingTenantError>;
    pub fn require_user_id(&self) -> Result<UserId, MissingUserError>;
    pub fn from_raw_correlation(id: Option<u64>) -> Self;
}

#[derive(Debug, thiserror::Error)]
#[error("tenant identifier is required but not present in request context")]
pub struct MissingTenantError;

#[derive(Debug, thiserror::Error)]
#[error("user identifier is required but not present in request context")]
pub struct MissingUserError;
```

### `src/extractor/context.rs`

```rust
impl FromMessageRequest for RequestContext {
    type Error = std::convert::Infallible;
    fn from_message_request(
        req: &MessageRequest,
        _payload: &mut Payload<'_>,
    ) -> Result<Self, Self::Error>;
}
```

### Dependencies

No new external crates. All types use:

- `serde` (already in deps, derive feature enabled)
- `thiserror` (already in deps)
- `std` types (`String`, `u64`, `Option`, `Result`)

### Reused existing code

- `AppDataStore` (`src/app_data_store.rs`) — for per-request context storage.
- `FromMessageRequest` trait (`src/extractor/trait_def.rs`) — for extraction.
- `MessageRequest::state::<T>()` and `insert_state::<T>()`
  (`src/extractor/request.rs`) — for read/write of context.
- `ConnectionId` pattern (`src/session.rs`) — as the template for all new
  newtypes.
