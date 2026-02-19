# Unify the public error surface

This ExecPlan is a living document. The sections `Constraints`, `Tolerances`,
`Risks`, `Progress`, `Surprises & Discoveries`, `Decision Log`, and
`Outcomes & Retrospective` must be kept up to date as work proceeds.

Status: COMPLETE (2026-02-18)

`PLANS.md` is not present in this repository at the time this plan was drafted.

## Purpose / big picture

Wireframe currently exposes multiple top-level error families, including two
public `WireframeError` definitions and a root `Result` alias coupled to app
setup. This plan consolidates user-facing errors into one canonical
`wireframe::WireframeError` type and one canonical `wireframe::Result<T>`
alias, so downstream users have a single error contract throughout the crate.

## Constraints

- End state must have exactly one public `WireframeError` type at crate level.
- End state must have exactly one public `Result<T>` alias at crate level,
  backed by the canonical `WireframeError`.
- Subsystem-specific errors may remain, but they must convert into the single
  `WireframeError` via `From` implementations.
- Public error docs and examples in `docs/users-guide.md` must reflect the new
  unified contract.
- Keep `thiserror`-derived semantic variants; avoid introducing opaque
  catch-all boxed error variants as the primary public surface.

## Tolerances (exception triggers)

- Scope: if unification requires edits in more than 50 files, stop and
  escalate.
- Interface: if consumer migration cannot be expressed clearly in one migration
  guide section, stop and escalate.
- Dependencies: if new error-handling crates are needed, stop and escalate.
- Iterations: if `make test` fails after 3 full integration loops, stop and
  escalate with categorized failures.
- Ambiguity: if subsystem semantics force incompatible variant designs, stop
  and escalate with explicit alternatives.

## Risks

- Risk: collapsing multiple error types may blur domain intent if variant
  boundaries are vague. Severity: high Likelihood: medium Mitigation: define
  subsystem-specific nested enums and preserve variant intent through clear
  names and docs.

- Risk: client/server/request APIs currently parameterized over protocol errors
  may become difficult to model. Severity: high Likelihood: medium Mitigation:
  settle on one canonical protocol variant strategy early and add regression
  tests for typed protocol failure paths.

- Risk: migration burden for existing users may be high due to type renames.
  Severity: medium Likelihood: high Mitigation: provide explicit before/after
  mapping in migration guide with compile-focused examples.

## Progress

- [x] (2026-02-18) Drafted ExecPlan for error surface unification.
- [x] (2026-02-18) Catalogued public error types and `Result` aliases.
- [x] (2026-02-18) Defined canonical `WireframeError` and conversion policy.
- [x] (2026-02-18) Refactored modules to use canonical crate-level error type.
- [x] (2026-02-18) Updated user guide and migration guide error contract docs.
- [x] (2026-02-18) Ran full quality gates.

## Surprises & Discoveries

- Observation: `wireframe::WireframeError<()>` did not satisfy
  `std::error::Error`, which broke `thiserror(transparent)` wrappers in
  `wireframe_testing`. Evidence: `make lint` failed with `as_dyn_error`
  trait-bound diagnostics in `wireframe_testing/src/integration_helpers.rs`.
  Impact: `std::error::Error` was implemented for all `E: Debug + 'static`,
  with protocol sources reported as `None` for generic payloads.

## Decision Log

- Decision: Canonicalize on a dedicated error module (for example,
  `src/error.rs`) and re-export only one `WireframeError` and one `Result<T>`
  from `src/lib.rs`. Rationale: This enforces a single front-door contract and
  removes naming collisions. Date/Author: 2026-02-18 / Codex.

- Decision: Keep subsystem enums where they add semantic value, but make them
  implementation detail or nested variants of canonical error. Rationale:
  Preserves meaning without fragmenting top-level API. Date/Author: 2026-02-18
  / Codex.

- Decision: Keep the canonical error generic as `WireframeError<E = ()>` and
  add the builder-specific `DuplicateRoute` variant to it, instead of forcing a
  non-generic top-level error. Rationale: preserves existing typed protocol
  error paths (`Protocol(E)`) while removing duplicate top-level
  `WireframeError` definitions. Date/Author: 2026-02-18 / Codex.

## Outcomes & Retrospective

- Result: Added `src/error.rs` as the canonical error module and re-exported
  `wireframe::WireframeError` plus `wireframe::Result<T>` from `src/lib.rs`.
- Result: Removed the duplicate `WireframeError` definition from
  `src/app/error.rs`; app builder APIs now resolve through canonical
  `wireframe::WireframeError`.
- Result: `src/response.rs` now uses the canonical error type, preserving the
  `response::WireframeError` path as a re-export of the same type for
  compatibility.
- Result: Updated migration documentation and user guide examples to point to
  the canonical contract (`wireframe::WireframeError`, `wireframe::Result<T>`).
- Validation: Passed `make check-fmt`, `make lint`, `make test`,
  `make markdownlint`, and `make nixie`.

## Context and orientation

Current public error-related surface includes:

- `src/app/error.rs` with `WireframeError`, `SendError`, and `Result<T>`.
- `src/response.rs` with generic `WireframeError<E>`.
- `src/lib.rs` re-exporting `app::error::Result` while also re-exporting
  response `WireframeError`.
- Additional public subsystem errors in client, codec, server, fragment,
  message assembler, and extractor modules.

The objective is not to remove domain information. The objective is to ensure
all public failure paths resolve through one top-level error type that users
can reliably match and log.

Likely touched files:

- `src/lib.rs`
- `src/app/error.rs`
- `src/response.rs`
- `src/client/error.rs`
- `src/server/error.rs`
- `src/codec/error.rs`
- `src/fragment/error.rs`
- `docs/users-guide.md`
- `docs/v0-1-0-to-v0-2-0-migration-guide.md`

## Plan of work

Stage A performs an error inventory. Build a source-of-truth list of all public
error enums and aliases, plus where each appears in public signatures.

Stage B defines the canonical model. Introduce one top-level `WireframeError`
enum with semantically grouped variants (for example: app configuration,
transport I/O, codec, protocol, client, server, fragmentation, extractor, and
send path failures). Include strict conversion rules.

Stage C migrates module signatures. Replace local top-level `WireframeError`
exports with either internal domain errors or aliases that resolve to canonical
`WireframeError`. Eliminate duplicate top-level names.

Stage D refreshes docs and migration guidance. Update user guide examples and
publish explicit migration mappings for renamed or removed variants/types.

Stage E validates behaviour through compile and test gates.

## Concrete steps

Run all commands from repository root (`/home/user/project`).

1. Inventory error types and aliases.

   `rg -n "pub enum .*Error|pub type Result<|WireframeError" src`

2. Introduce canonical error module and integrate conversions.

   `make check-fmt`

3. Compile and run tests.

   `make lint` `make test`

4. Validate docs after migration updates.

   `make fmt` `make markdownlint` `make nixie`

Expected success indicators:

- One public `WireframeError` remains.
- One public `Result<T>` alias remains.
- All previously public error paths convert into canonical error.

## Validation and acceptance

Acceptance criteria:

- `wireframe::WireframeError` is the only public top-level error type named
  `WireframeError`.
- `wireframe::Result<T>` is the only public crate-level result alias.
- Public signatures that previously returned alternative top-level error types
  now return `WireframeError` (directly or via alias).
- Migration guide contains before/after mapping snippets.
- `make check-fmt`, `make lint`, and `make test` pass.
- `make fmt`, `make markdownlint`, and `make nixie` pass for doc changes.

## Idempotence and recovery

The migration is refactor-oriented and re-runnable in slices. If a migration
step causes broad breakage, restore the last coherent module boundary and
re-run inventory before continuing.

## Artifacts and notes

Implementation should capture:

- Error type inventory (before and after).
- Variant mapping table used for migration docs.
- Representative downstream callsite updates.

## Interfaces and dependencies

No new dependencies are planned.

Target interface sketch at completion:

    pub enum WireframeError {
        App(AppError),
        Transport(std::io::Error),
        Codec(CodecError),
        Protocol(ProtocolError),
        Client(ClientError),
        Server(ServerError),
        Fragment(FragmentError),
        // Additional semantic variants as required.
    }

    pub type Result<T> = std::result::Result<T, WireframeError>;

Revision note: Initial draft created on 2026-02-18 to plan consolidation of all
public error paths into one canonical type.
