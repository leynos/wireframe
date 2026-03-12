# 8.5.3 Add deterministic assertion helpers for reassembly outcomes

This ExecPlan (execution plan) is a living document. The sections
`Constraints`, `Tolerances`, `Risks`, `Progress`, `Surprises & Discoveries`,
`Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work
proceeds.

Status: DONE

## Purpose / big picture

Roadmap item `8.5.3` closes the last missing testkit utility from
[ADR 0002](../adr-002-streaming-requests-and-shared-message-assembly.md):
downstream tests need a first-class way to assert fragment reassembly and
message assembly outcomes without re-implementing bespoke pattern matches in
every fixture, step file, and integration test.

After this work, library consumers will be able to use a public
`wireframe_testing` assertion surface to verify outcomes such as:

- an assembly remains incomplete and buffered;
- a logical message completed with a specific key, metadata, or body;
- a specific deterministic error occurred, such as `MessageTooLarge`,
  `SequenceMismatch`, or fragment index mismatch; and
- cleanup side effects occurred, such as buffered-byte reclamation or expiry
  eviction.

Success is observable in three places:

1. new `rstest` integration tests fail before the helper module exists and
   pass after it is implemented;
2. existing or newly-added `rstest-bdd` scenarios use the helper-backed
   assertions instead of ad hoc fixture logic; and
3. `docs/users-guide.md`, `docs/wireframe-testing-crate.md`, and the ADR note
   document the public helper API and the design choice behind it.

The roadmap item `8.5.3` is marked done only after the full validation suite
passes and the public documentation is updated.

## Constraints

- Keep the change additive. Existing runtime behaviour and existing public
  signatures in `wireframe` must not change.
- Put the public helper API in `wireframe_testing`, not in
  `wireframe::message_assembler` or `wireframe::fragment`. ADR 0002 treats this
  as a testkit concern.
- `wireframe_testing` is not a workspace member, so helper tests must live in
  the repository root `tests/` tree rather than under
  `wireframe_testing/src/**`.
- No single source file may exceed 400 lines. This matters immediately because
  [`tests/fixtures/message_assembly.rs`](../../tests/fixtures/message_assembly.rs)
   is already 340 lines and
  [`tests/steps/message_assembly_steps.rs`](../../tests/steps/message_assembly_steps.rs)
   is already 368 lines.
- Use `rstest` for the focused helper tests and `rstest-bdd` v0.5.0 for the
  behavioural coverage. Fixture names in step functions must match exactly, and
  step parameters must not be underscore-prefixed.
- Public helpers must not rely on `assert!` or `panic!` for the happy path.
  They should return a result type with deterministic diagnostics so they work
  in both `rstest` functions that return `Result` and BDD step functions.
- Document public helper functions with Rustdoc comments and examples.
- Update the relevant design documentation with any API-shape decision taken
  during implementation.
- Update [`docs/users-guide.md`](../users-guide.md) for any public API exposed
  to library consumers.

## Tolerances (exception triggers)

- Scope: if implementation requires more than 16 touched files or 1,000 net
  lines, stop and escalate. This item should be a focused testkit addition, not
  a broad refactor.
- API: if satisfying the requirement would force a breaking change to existing
  `wireframe_testing` helper names or signatures, stop and escalate.
- Dependencies: if a new crate dependency is required, stop and escalate.
- Abstraction: if a single public helper surface cannot cover both fragment
  reassembly and message assembly without becoming vague or misleading, stop
  and present the split-module alternatives.
- Validation: if repo-wide markdown gates or Rust gates fail for unrelated
  baseline reasons, record the evidence and stop before marking `8.5.3` done.
- Iterations: if the new targeted tests still fail after 5 fix attempts, stop
  and escalate with the last failing output.

## Risks

- Risk: over-generalising the API could make the helper names obscure and the
  diagnostics worse than the current bespoke assertions. Severity: medium.
  Likelihood: medium. Mitigation: group shared data into typed snapshots, but
  keep fragment and message-assembly expectations distinct rather than forcing
  one universal enum.

- Risk: adding more assertion methods directly to existing BDD worlds will push
  them over the 400-line cap. Severity: high. Likelihood: high. Mitigation:
  move reusable assertion logic into `wireframe_testing` and keep fixture/world
  methods as thin wrappers.

- Risk: public helper examples in `wireframe_testing` may not be covered by the
  workspace-level doctest gate because `wireframe_testing` is not a workspace
  member. Severity: medium. Likelihood: medium. Mitigation: keep root-level
  `rstest` coverage as the primary enforcement mechanism, and add targeted
  doc-validation commands if needed during implementation.

- Risk: fragment reassembly and message assembly use different error types and
  state models. Severity: medium. Likelihood: high. Mitigation: share only the
  result-shaping pattern, not the domain enums themselves.

## Progress

- [x] (2026-03-11 00:00Z) Read the roadmap item, ADR 0002, adjacent ExecPlans,
  and the existing message-assembly/fragment assertion code.
- [x] (2026-03-11 00:00Z) Identified the main duplication points in
  `tests/fixtures/message_assembly.rs`,
  `tests/steps/message_assembly_steps.rs`, and
  `tests/fixtures/fragment/reassembly.rs`.
- [x] (2026-03-11 00:00Z) Drafted this ExecPlan.
- [x] (2026-03-12 00:00Z) Finalized the public assertion API shape in
  `wireframe_testing` as a dedicated `reassembly` module with separate
  message-assembly and fragment-reassembly helpers.
- [x] (2026-03-12 00:00Z) Implemented helper modules and exports in
  `wireframe_testing/src/reassembly/` and re-exported them from
  `wireframe_testing/src/lib.rs`.
- [x] (2026-03-12 00:00Z) Refactored the existing BDD worlds and steps to
  consume the helper API via thin wrapper methods instead of bespoke pattern
  matches.
- [x] (2026-03-12 00:00Z) Added `rstest` integration tests for the public
  helper surface in `tests/reassembly_assertion_helpers.rs`.
- [x] (2026-03-12 00:00Z) Extended the `rstest-bdd` coverage by switching the
  message-assembly and fragment-reassembly fixtures/steps to the helper-backed
  assertions.
- [x] (2026-03-12 00:00Z) Updated design and user documentation, including the
  ADR implementation note and the users guide entry for the new public API.
- [x] (2026-03-12 00:00Z) Ran the relevant quality gates for code, tests,
  doctests, formatting, markdown, and Mermaid validation.
- [x] (2026-03-12 00:00Z) Marked roadmap item `8.5.3` done in
  `docs/roadmap.md`.

## Surprises & Discoveries

- Observation: the repository already has three separate styles of reassembly
  assertion code: `tests/fixtures/message_assembly.rs`,
  `tests/fixtures/fragment/reassembly.rs`, and
  `tests/fixtures/budget_enforcement.rs`. They solve the same problem with
  different helper shapes and different diagnostic quality.

- Observation: the current message-assembly BDD fixture and step file are both
  close to the 400-line cap, so this roadmap item is as much about extracting
  duplication as it is about adding a public helper.

- Observation: `wireframe_testing` helper tests cannot live under
  `wireframe_testing/src/**` because that crate is not a workspace member and
  would not be exercised by `make test`.

- Observation: the cleanest public API was not one giant assertion enum. The
  implementation stayed easier to explain and easier to lint by exposing two
  separate expectation families: `MessageAssemblyErrorExpectation` and
  `FragmentReassemblyErrorExpectation`.

- Observation: the strict Clippy configuration shaped the helper tests and
  fixtures. A reusable test constructor with five parameters triggered
  `clippy::too_many_arguments`, and a wildcard arm over the fragment error enum
  triggered `clippy::match_wildcard_for_single_variants`, so the final tests
  build snapshots inline and the fixture matches variants explicitly.

- Observation: repo-wide markdown validation was previously blocked by older
  execplans with ordered-list numbering problems. Those baseline MD029 issues
  had to be corrected in this session before `make markdownlint` could pass for
  the full repository.

## Decision Log

- Decision: implement the new public surface in `wireframe_testing` as a
  dedicated reassembly assertion module, not as more methods on individual BDD
  worlds. Rationale: the feature is a testkit utility, and the current worlds
  are already near the file-size cap. Date/Author: 2026-03-11 / Codex

- Decision: prefer typed expectation values and snapshot structs over assertion
  macros. Rationale: macros would encourage panic-style tests, are harder to
  document with Rustdoc examples, and provide weaker reuse in `rstest-bdd` step
  functions. Date/Author: 2026-03-11 / Codex

- Decision: keep fragment reassembly and message assembly as separate public
  expectation families that share internal formatting helpers, rather than one
  universal error enum. Rationale: the domains overlap conceptually but expose
  different state and different error types. Date/Author: 2026-03-11 / Codex

- Decision: make the public helpers return
  `wireframe_testing::integration_helpers::TestResult<()>` rather than panic.
  Rationale: the same helpers must compose cleanly in `rstest` functions that
  return `Result` and in `rstest-bdd` step functions, and deterministic error
  messages are more useful than assertion macro panics. Date/Author: 2026-03-12
  / Codex

- Decision: keep the existing BDD scenarios and fixture worlds, but reduce
  them to snapshot builders and thin wrapper methods around the public helper
  functions. Rationale: this preserves scenario readability while moving the
  core assertion semantics into the reusable testkit surface required by the
  roadmap item. Date/Author: 2026-03-12 / Codex

## Outcomes & Retrospective

Implementation completed on 2026-03-12.

Delivered artefacts:

- new public `wireframe_testing::reassembly` helper modules for
  message-assembly and fragment-reassembly assertions;
- root-level `rstest` coverage in
  `tests/reassembly_assertion_helpers.rs` because `wireframe_testing` is not a
  workspace member;
- helper-backed message-assembly and fragment-reassembly BDD worlds/steps; and
- updated user-facing and design-facing documentation, plus the roadmap item
  marked done.

Validation completed with:

- targeted helper and BDD coverage during implementation:
  `cargo test --test reassembly_assertion_helpers --all-features`,
  `cargo test --test bdd --all-features -- message_assembly`, and
  `cargo test --test bdd --all-features -- fragment`;
- repository quality gates:
  `make fmt`, `make markdownlint MDLINT=/root/.bun/bin/markdownlint-cli2`,
  `make check-fmt`, `make lint`, `make test`, `make test-doc`,
  `make doctest-benchmark`, and `make nixie`.

Retrospective:

- Extracting the assertions into `wireframe_testing` reduced duplication and
  improved diagnostic consistency without changing runtime behaviour.
- The public snapshot-plus-expectation pattern scales better than fixture-local
  boolean helpers because it keeps scenario steps declarative while allowing
  integration tests to assert exact failure semantics.

## Context and orientation

The relevant code is split between the production crate, the
`wireframe_testing` companion crate, and the main repository test suite.

Production-side context:

- [`src/message_assembler/state.rs`](../../src/message_assembler/state.rs)
  owns multi-frame request assembly state, returns
  `Result<Option<AssembledMessage>, MessageAssemblyError>`, and exposes
  counters such as `buffered_count()` and `total_buffered_bytes()`.
- [`src/fragment/reassembler.rs`](../../src/fragment/reassembler.rs) owns
  transport-level fragment reassembly and returns either a completed
  reassembled message or `ReassemblyError`.

Current test-only duplication:

- [`tests/fixtures/message_assembly.rs`](../../tests/fixtures/message_assembly.rs)
  stores `last_result`, completed messages, and eviction state, then exposes
  bespoke helpers such as `last_result_is_incomplete()`,
  `completed_body_for_key()`, `is_sequence_mismatch()`, and
  `is_message_too_large()`.
- [`tests/steps/message_assembly_steps.rs`](../../tests/steps/message_assembly_steps.rs)
  contains generic assertion helpers plus many domain-specific `Then` step
  matchers that reproduce knowledge already present in the error enums.
- [`tests/fixtures/fragment/reassembly.rs`](../../tests/fixtures/fragment/reassembly.rs)
  has a second ad hoc assertion style for `ReassemblyError`,
  `last_reassembled`, buffered fragment counts, and eviction results.
- [`tests/fixtures/budget_enforcement.rs`](../../tests/fixtures/budget_enforcement.rs)
  carries a third set of outcome assertions around acceptance, completion, and
  buffered-byte accounting.

Testkit/public API touch points:

- [`wireframe_testing/src/lib.rs`](../../wireframe_testing/src/lib.rs) is the
  public re-export surface.
- [`wireframe_testing/src/helpers.rs`](../../wireframe_testing/src/helpers.rs)
  currently re-exports the in-memory driver helpers added in `8.5.1` and
  `8.5.2`.
- [`docs/wireframe-testing-crate.md`](../wireframe-testing-crate.md) is the
  design-facing description of the companion crate and should record the final
  assertion-helper design decision.
- [`docs/users-guide.md`](../users-guide.md) already documents fragment,
  message-assembly, and slow-I/O testing surfaces, so it is the right place to
  document the public API for these new helpers.
- [`docs/adr-002-streaming-requests-and-shared-message-assembly.md`](../adr-002-streaming-requests-and-shared-message-assembly.md)
  already names deterministic reassembly assertions as an explicit testkit
  requirement and should gain an implementation note for `8.5.3`.

## Plan of work

### Stage A: Finalize the public helper contract

Create one new public module under `wireframe_testing`, preferably
`wireframe_testing/src/reassembly/`, so the code can be split into focused
files if it approaches the 400-line limit.

The public API should expose two families of types:

1. message-assembly snapshots and expectations for
   `MessageAssemblyState`-driven outcomes; and
2. fragment-reassembly snapshots and expectations for
   `Reassembler`-driven outcomes.

The exact names may vary, but the contract must support these assertions
without caller-side closures:

- incomplete assembly with expected buffered counts;
- completed assembly with expected key, metadata, or body;
- expected message-assembly errors such as duplicate first frame, missing first
  frame, sequence mismatch, duplicate frame, message-too-large, and budget
  violations;
- fragment reassembly success with expected payload length or bytes;
- fragment reassembly failure with expected error kind; and
- expiry/eviction expectations when relevant to the current world state.

The helpers should return `wireframe_testing::TestResult<()>` or another
publicly reusable non-panicking result type with stable, human-readable
messages.

Go/no-go rule: do not begin refactoring fixtures until the helper API is small
enough to explain in one short `docs/users-guide.md` example.

### Stage B: Implement the helper module and wire exports

Add the new module files in `wireframe_testing` and export them through
`wireframe_testing/src/lib.rs`.

Implementation guidance:

1. keep shared formatting/comparison code private;
2. keep the public surface typed and explicit;
3. place Rustdoc comments on every public function or type alias; and
4. include examples that show both a completion assertion and an error
   assertion.

Do not add helper macros unless Stage A proves a function-based API cannot meet
the ergonomics requirement.

### Stage C: Refactor existing fixture and step code to consume the helper API

Refactor the existing test worlds so they build domain snapshots and delegate
to the new public helpers instead of open-coding the checks.

This stage should touch, at minimum:

1. [`tests/fixtures/message_assembly.rs`](../../tests/fixtures/message_assembly.rs)
2. [`tests/steps/message_assembly_steps.rs`](../../tests/steps/message_assembly_steps.rs)
3. [`tests/fixtures/fragment/reassembly.rs`](../../tests/fixtures/fragment/reassembly.rs)

If the new helper surface also cleanly covers the budget-enforcement outcome
assertions, refactor
[`tests/fixtures/budget_enforcement.rs`](../../tests/fixtures/budget_enforcement.rs)
 as well. That refactor is in scope only if it remains additive and does not
expand the touched-file count beyond tolerance.

Target outcome: fixture methods become thin wrappers that gather state and call
the public helper, while step files keep only Gherkin binding logic.

### Stage D: Add focused `rstest` integration coverage for the public API

Create a new root-level test file, for example
`tests/reassembly_assertion_helpers.rs`, because `wireframe_testing` is not a
workspace member.

Use `rstest` parameterization to cover the public contract directly. The suite
must include both passing and failing-path coverage so the helper diagnostics
are stable:

1. message assembly incomplete outcome;
2. message assembly completion for single-frame and multi-frame cases;
3. message assembly error expectations for at least sequence mismatch,
   duplicate first frame, and message-too-large;
4. fragment reassembly success and failure;
5. buffered-count or buffered-byte cleanup after completion or rejection; and
6. at least one negative assertion test that verifies the returned error string
   is specific enough to debug mismatches quickly.

The tests should fail before Stage B/C and pass afterward.

### Stage E: Add behavioural coverage with `rstest-bdd`

Use `rstest-bdd` v0.5.0 to prove that the helper-backed assertions work in the
same behavioural flows that downstream protocol crates will copy.

Prefer extending the existing BDD coverage rather than inventing a parallel
test universe:

1. add one or more scenarios to
   [`tests/features/message_assembly.feature`](../../tests/features/message_assembly.feature)
    that specifically exercise helper-backed completion and error assertions;
2. add one or more fragment-reassembly scenarios if the current fragment BDD
   coverage does not already touch the new helper path; and
3. keep step functions thin, with fixture-parameter names matching their
   fixture function names exactly.

Behavioural acceptance is satisfied only when the observable assertions flow
through the new helper module, not when the scenarios merely continue to pass
through old bespoke methods.

### Stage F: Update design and user documentation

Update these documents during implementation, not after the fact:

1. [`docs/wireframe-testing-crate.md`](../wireframe-testing-crate.md)
   Document the new public reassembly assertion surface, explain why it uses
   typed expectations instead of macros, and show one short example.
2. [`docs/users-guide.md`](../users-guide.md)
   Add a consumer-facing section showing how to assert message assembly and
   fragment reassembly outcomes with `wireframe_testing`.
3. [`docs/adr-002-streaming-requests-and-shared-message-assembly.md`](../adr-002-streaming-requests-and-shared-message-assembly.md)
   Add an implementation note for roadmap item `8.5.3`, mirroring the style
   already used for `8.5.2`.

If the implementation reveals a more durable design rationale than expected,
record it in `docs/wireframe-testing-crate.md`; that is the primary
design-facing home for this feature.

### Stage G: Validation, evidence, and roadmap closure

Run the full relevant quality gates with `tee` and `set -o pipefail` so the
logs survive truncation:

```sh
set -o pipefail && make fmt 2>&1 | tee /tmp/8-5-3-fmt.log
set -o pipefail && make check-fmt 2>&1 | tee /tmp/8-5-3-check-fmt.log
set -o pipefail && make lint 2>&1 | tee /tmp/8-5-3-lint.log
set -o pipefail && make test 2>&1 | tee /tmp/8-5-3-test.log
set -o pipefail && make test-doc 2>&1 | tee /tmp/8-5-3-test-doc.log
set -o pipefail && make doctest-benchmark 2>&1 | tee /tmp/8-5-3-doctest-benchmark.log
set -o pipefail && make markdownlint MDLINT=/root/.bun/bin/markdownlint-cli2 2>&1 | tee /tmp/8-5-3-markdownlint.log
set -o pipefail && make nixie 2>&1 | tee /tmp/8-5-3-nixie.log
```

Acceptance criteria:

1. all new helper tests pass;
2. helper-backed BDD scenarios pass;
3. public docs and design docs are updated;
4. no file exceeds 400 lines; and
5. [`docs/roadmap.md`](../roadmap.md) marks `8.5.3` as done.

Only after all of the above pass should the implementation flip:

```md
- [ ] 8.5.3. Add deterministic assertion helpers for reassembly outcomes.
```

to:

```md
- [x] 8.5.3. Add deterministic assertion helpers for reassembly outcomes.
```
