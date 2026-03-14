# Close phase 8.6 documentation for streaming requests and shared message assembly

This ExecPlan (execution plan) is a living document. The sections
`Constraints`, `Tolerances`, `Risks`, `Progress`, `Surprises & Discoveries`,
`Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work
proceeds.

Status: COMPLETE

## Purpose / big picture

Roadmap items `8.6.1` through `8.6.3` close the documentation phase for
streaming request bodies, the generic `MessageAssembler` abstraction, and the
standardized per-connection memory-budget model introduced by
`docs/adr-002-streaming-requests-and-shared-message-assembly.md`.

After this work, a protocol author or library consumer should be able to read
the design documents and understand:

- where transport fragmentation stops and protocol-level message assembly
  begins;
- how `RequestParts`, `RequestBodyStream`, and `StreamingBody` fit into the
  inbound request path;
- how `MessageAssembler` composes with transport fragmentation and handler
  dispatch;
- how per-connection budgets, soft pressure, and hard caps apply across the
  shared inbound assembly pipeline; and
- which public APIs and behavioural guarantees are stable enough to rely on.

Success is observable when:

1. `docs/generic-message-fragmentation-and-re-assembly-design.md` contains
   composition guidance that matches the implemented ordering and budget
   semantics.
2. `docs/multi-packet-and-streaming-responses-design.md` contains a streaming
   request-body section that explains the implemented inbound API rather than
   only outbound symmetry.
3. `docs/the-road-to-wireframe-1-0-feature-set-philosophy-and-capability-maturity.md`
   describes `MessageAssembler` and the three-tier budget model as current
   capability, not as a vague future idea.
4. `docs/users-guide.md` is updated anywhere the public interface or consumer
   guidance would otherwise be stale or incomplete.
5. The feature is backed by both `rstest` and `rstest-bdd` coverage, including
   closing the current behavioural-test gap for streaming request bodies.
6. `docs/roadmap.md` marks `8.6.1`, `8.6.2`, and `8.6.3` done only after all
   relevant gates pass.

## Constraints

- Scope is the documentation closure for roadmap phase `8.6`, plus any tests
  and public API examples needed to make the documented behaviour trustworthy.
- Do not introduce new product behaviour merely to make the documents read
  better. If documentation work exposes an implementation bug, fix only the
  smallest issue required to align the code with ADR 0002 and document the
  decision.
- Preserve existing public API signatures unless a contradiction in the current
  API surface makes the documentation impossible to write accurately. If that
  happens, stop and escalate.
- Prefer editing the existing relevant sections in place instead of appending
  duplicate sections that drift later.
- Keep all new or expanded source and test files at or below the repository's
  400-line guideline.
- Use `rstest` for unit or integration validation and `rstest-bdd` for
  behavioural validation.
- Use the existing design docs as the source of truth to reconcile wording:
  `docs/adr-002-streaming-requests-and-shared-message-assembly.md`,
  `docs/hardening-wireframe-a-guide-to-production-resilience.md`,
  `docs/generic-message-fragmentation-and-re-assembly-design.md`,
  `docs/multi-packet-and-streaming-responses-design.md`, and
  `docs/the-road-to-wireframe-1-0-feature-set-philosophy-and-capability-maturity.md`.
- Keep documentation and comments in en-GB-oxendict spelling.
- Use Makefile targets for final quality gates, and run them with
  `set -o pipefail` and `tee` so truncated terminal output does not hide
  failures.
- Do not mark roadmap item `8.6` complete until documentation, tests, and the
  user-facing guide are all in sync.

## Tolerances (exception triggers)

- Scope: if the work grows beyond 16 touched files or 1,000 net lines, stop
  and re-evaluate the split between documentation closure and follow-up
  implementation work.
- Interface: if any public API signature must change, stop and escalate.
- Behaviour: if accurate documentation would require a semantic redesign of the
  inbound assembly pipeline rather than a narrow fix, stop and escalate.
- Dependencies: if any new external crate is required, stop and escalate.
- Test gap: if adding behavioural coverage for streaming request bodies
  requires a second new BDD suite beyond one focused `streaming_request`
  feature flow, stop and re-scope.
- Baseline quality gates: if repo-wide Markdown or doctest gates fail because
  of unrelated pre-existing issues, document the exact failure, do not mark the
  roadmap item done, and escalate with options.
- Iterations: if the same targeted gate still fails after 3 focused attempts,
  stop and escalate with evidence.

## Risks

- Risk: the target documents already contain partial material for roadmap items
  `8.6.1` through `8.6.3`, so careless edits could duplicate or contradict
  existing sections instead of clarifying them. Severity: medium Likelihood:
  high Mitigation: start with a claim-by-claim audit against ADR 0002, then
  revise the existing sections in place.

- Risk: the user guide already documents `RequestParts`, `MessageAssembler`,
  and `MemoryBudgets`, so a narrow docs-only pass could miss smaller public API
  examples in Rustdoc comments. Severity: medium Likelihood: medium Mitigation:
  audit both `docs/users-guide.md` and the Rustdoc on `src/request/mod.rs`,
  `src/extractor/streaming.rs`, `src/app/builder/protocol.rs`, and
  `src/app/builder/config.rs`.

- Risk: streaming request bodies currently have `rstest` coverage but no
  matching `rstest-bdd` flow, which blocks the prompt's requirement for both
  unit and behavioural validation of the feature. Severity: high Likelihood:
  high Mitigation: add one focused BDD suite for streaming request bodies and
  keep it small by reusing existing fixture patterns.

- Risk: repo-wide Markdown validation may still fail on unrelated historical
  docs, preventing clean roadmap closure. Severity: medium Likelihood: medium
  Mitigation: run the full gates near the end, capture logs, and repair any
  failures caused by this change immediately. Escalate only if the remaining
  failure is unrelated baseline debt.

## Progress

- [x] (2026-03-14 00:00Z) Drafted this ExecPlan after auditing the roadmap,
  ADR 0002, current design docs, user guide, and the existing test surface.
- [x] Stage A: build a documentation-claim matrix and decide the minimum set of
  in-place edits needed across the three roadmap documents.
- [x] Stage B: revise the three target design documents and record any new
  design decisions in the relevant design document.
- [x] Stage C: update `docs/users-guide.md` and any public Rustdoc examples
  that need to reflect the documented public interface.
- [x] Stage D: close the missing behavioural-test coverage for streaming
  request bodies and add any smaller focused `rstest` coverage required by new
  claims.
- [x] Stage E: run all relevant quality gates and mark roadmap items `8.6.1`
  through `8.6.3` done.

## Surprises & discoveries

- Observation: all three target documents already contain partial material for
  the roadmap item. The work is not blank-sheet authoring; it is a
  harmonization pass. Evidence: section `9` in
  `docs/generic-message-fragmentation-and-re-assembly-design.md`, section `11`
  in `docs/multi-packet-and-streaming-responses-design.md`, and the
  `MessageAssembler` and memory-cap sections in
  `docs/the-road-to-wireframe-1-0-feature-set-philosophy-and-capability-maturity.md`.
   Impact: edit existing sections in place instead of adding new headings
  unless structure is genuinely missing.

- Observation: `docs/users-guide.md` already documents `RequestParts`,
  `MessageAssembler`, and `MemoryBudgets`. Evidence: the guide sections around
  lines `761`, `788`, `842`, and `1074`. Impact: user-guide work should be a
  targeted sync pass, not a full new chapter.

- Observation: streaming request bodies currently have strong `rstest`
  coverage in `tests/streaming_request.rs`, but there is no matching
  behavioural suite under `tests/features/`. Impact: the implementation cannot
  honestly claim both unit and behavioural validation for the inbound streaming
  half of ADR 0002 until that BDD gap is closed.

- Observation: `MessageAssembler` and memory budgets already have substantial
  behavioural coverage through existing feature files such as
  `tests/features/message_assembler.feature`,
  `tests/features/memory_budgets.feature`,
  `tests/features/budget_enforcement.feature`,
  `tests/features/budget_cleanup.feature`, and
  `tests/features/budget_transitions.feature`. Impact: reuse those suites as
  evidence first and add only narrowly scoped follow-up tests if new
  documentation claims are not already covered.

- Observation: the missing behavioural gap for inbound streaming requests could
  be closed without introducing another runtime-heavy integration harness.
  Evidence: a focused `rstest-bdd` fixture around `body_channel`,
  `StreamingBody`, and `RequestBodyReader` covered the public API guarantees
  that were previously unit-test-only. Impact: phase `8.6` stayed within scope
  and avoided duplicating the heavier inbound message-assembly harness.

## Decision log

- Decision: treat roadmap phase `8.6` as a documentation harmonization and
  validation-closure task, not as a new architecture design pass. Rationale:
  ADR 0002 and the implementation already define the feature; the missing work
  is accurate narrative, cross-linking, and proof that the documented behaviour
  is exercised. Date/Author: 2026-03-14 / Codex

- Decision: add one focused `rstest-bdd` suite for streaming request bodies
  unless the audit uncovers an existing hidden equivalent. Rationale: the
  prompt requires both unit and behavioural validation, and the current inbound
  streaming coverage is unit-only. Date/Author: 2026-03-14 / Codex

- Decision: update `docs/users-guide.md` and public Rustdoc examples only where
  public consumer guidance would otherwise be stale or incomplete. Rationale:
  this keeps scope aligned with the roadmap item while still meeting the
  prompt's requirement to reflect public interface changes. Date/Author:
  2026-03-14 / Codex

- Decision: mark roadmap items `8.6.1` through `8.6.3` done only after the
  docs, behavioural coverage, doctest gates, and repo quality gates all pass.
  Rationale: a documentation checkbox should mean the docs are trustworthy and
  demonstrably aligned with the code. Date/Author: 2026-03-14 / Codex

- Decision: cover the streaming-request behavioural gap at the public request
  API boundary instead of through a second full inbound-actor integration
  fixture. Rationale: the missing guarantees were handler-facing stream
  semantics (`AsyncRead` adaptation, back-pressure, and error propagation), and
  those behaviours are exercised more directly through the request API than
  through another transport harness. Date/Author: 2026-03-14 / Codex

## Outcomes & retrospective

Completed deliverables:

- revised composition and budget guidance in
  `docs/generic-message-fragmentation-and-re-assembly-design.md`;
- revised inbound streaming-request guidance in
  `docs/multi-packet-and-streaming-responses-design.md`;
- revised capability-maturity language in
  `docs/the-road-to-wireframe-1-0-feature-set-philosophy-and-capability-maturity.md`;
- synced public guidance in `docs/users-guide.md` and public Rustdoc on
  `WireframeApp::with_message_assembler` and `WireframeApp::memory_budgets`;
- a focused `rstest-bdd` suite for `tests/features/streaming_request.feature`;
- updated roadmap checkboxes for `8.6.1`, `8.6.2`, and `8.6.3`; and
- captured gate logs for `make fmt`, `make markdownlint`, `make check-fmt`,
  `make lint`, `make test`, `make test-doc`, `make doctest-benchmark`, and
  `make nixie`.

## Context and orientation

The relevant implementation and documentation surface already exists. The task
is to align and close it.

### Existing documentation surface

- `docs/adr-002-streaming-requests-and-shared-message-assembly.md` is the
  normative decision record. It defines:
  - `RequestParts` and `RequestBodyStream`;
  - the `MessageAssembler` hook and its composition with transport
    fragmentation; and
  - standardized per-connection memory budgets, soft pressure, and hard caps.
- `docs/generic-message-fragmentation-and-re-assembly-design.md` already has
  section `9`, "Composition with streaming requests and MessageAssembler". It
  needs to become operational guidance for protocol implementers, not just a
  short restatement of ADR 0002.
- `docs/multi-packet-and-streaming-responses-design.md` already has section
  `11`, "Streaming request bodies", and subsection `11.4`, "Composition with
  MessageAssembler". These sections currently lean toward outbound symmetry and
  need a clearer inbound request-body story.
- `docs/the-road-to-wireframe-1-0-feature-set-philosophy-and-capability-maturity.md`
  already references both `MessageAssembler` and memory budgets, but parts of
  the text still read as aspirational future design rather than implemented
  maturity.
- `docs/hardening-wireframe-a-guide-to-production-resilience.md` is the
  resilience baseline for the budget story. It is a consistency source even if
  it does not end up being edited.
- `docs/users-guide.md` already exposes the relevant public API concepts:
  `RequestParts`, `RequestBodyStream`, `StreamingBody`, `MessageAssembler`, and
  `MemoryBudgets`.

### Existing implementation surface

- `src/request/mod.rs` defines `RequestParts`, `RequestBodyStream`,
  `RequestBodyReader`, and `body_channel`.
- `src/extractor/streaming.rs` defines the `StreamingBody` extractor.
- `src/app/builder/protocol.rs` exposes
  `WireframeApp::with_message_assembler` and `WireframeApp::message_assembler`.
- `src/app/builder/config.rs` exposes `WireframeApp::memory_budgets`.
- `src/app/frame_handling/assembly.rs` and
  `src/app/frame_handling/backpressure.rs` apply message-assembly and budget
  enforcement in the inbound runtime path.

### Existing test surface

- `tests/streaming_request.rs` provides `rstest` coverage for inbound request
  streaming, body readers, error propagation, ordering, and back-pressure.
- `tests/features/message_assembler.feature` plus its fixture, steps, and
  scenarios cover the public `MessageAssembler` contract behaviourally.
- `tests/features/memory_budgets.feature`,
  `tests/features/budget_enforcement.feature`,
  `tests/features/budget_cleanup.feature`,
  `tests/features/budget_transitions.feature`,
  `tests/features/memory_budget_backpressure.feature`,
  `tests/features/memory_budget_hard_cap.feature`, and
  `tests/features/derived_memory_budgets.feature` already cover the budget
  model behaviourally.
- There is no `tests/features/streaming_request.feature` or equivalent BDD
  flow today. That is the clearest validation gap for this roadmap closure.

### Terms used in this plan

- Transport fragmentation: splitting one transport packet into several bounded
  fragments and reassembling them before routing.
- Protocol-level message assembly: reconstructing or streaming a request body
  whose bytes arrive across multiple protocol packets.
- Soft pressure: short read pauses once buffered bytes reach 80% of the active
  aggregate budget.
- Hard cap: immediate `InvalidData` termination once buffered bytes strictly
  exceed the active aggregate budget.

## Plan of work

### Stage A: audit claims and decide what must change

Build a simple matrix that maps ADR 0002 claims to the current docs and tests.
Do not change files yet. The matrix should answer:

1. Which claims already exist in each roadmap-target document.
2. Which claims are missing, vague, duplicated, or contradictory.
3. Which public consumer-facing examples in `docs/users-guide.md` or Rustdoc
   need refreshing.
4. Which claims are already backed by `rstest` and `rstest-bdd`, and which
   still lack one side of that proof.

The most likely output of this stage is a short edit list such as:

- strengthen composition guidance in section `9` of the fragmentation design;
- rewrite section `11` of the streaming-responses design so the inbound API is
  concrete and implementation-aligned;
- revise the capability-maturity document so it describes the current
  `MessageAssembler` and three-tier budget model without future-tense drift;
- sync `docs/users-guide.md` and any public Rustdoc examples that would
  otherwise contradict those documents; and
- add one behavioural flow for streaming request bodies.

Go/no-go: if this audit shows that the current implementation contradicts ADR
0002 in a way that requires more than narrow fixes, stop and escalate before
editing.

### Stage B: revise the three roadmap-target design documents

Edit the target docs in place.

In `docs/generic-message-fragmentation-and-re-assembly-design.md`, revise
section `9` so it explains composition as guidance for protocol implementers.
It should explicitly answer:

1. what transport fragmentation is responsible for;
2. what `MessageAssembler` is responsible for;
3. the exact ordering through the inbound pipeline;
4. how budgets are shared across both layers; and
5. what operators or protocol authors must configure so the two layers do not
   fight each other.

In `docs/multi-packet-and-streaming-responses-design.md`, revise section `11`
so the inbound request-body model is specific and actionable. It should cover:

1. when a handler receives `RequestParts` plus `RequestBodyStream`;
2. how `StreamingBody` and `RequestBodyReader` relate to direct stream
   consumption;
3. how back-pressure works on inbound streaming requests; and
4. how `MessageAssembler` turns multi-packet request bodies into either buffered
   or streaming delivery.

In
`docs/the-road-to-wireframe-1-0-feature-set-philosophy-and-capability-maturity.md`,
 revise the relevant sections so they describe current capability maturity:

1. `MessageAssembler` is part of the implemented feature set, not just a
   concept;
2. per-connection budgets are standardized and derived from frame budgets when
   left implicit; and
3. the soft-pressure and hard-cap behaviour is part of the production
   hardening story.

If any new normative decision is made while clarifying these documents, record
it in the most relevant design document. Use ADR 0002 only if the decision is
truly architectural rather than editorial.

Validation at the end of Stage B:

- the three target docs no longer disagree about pipeline order or budget
  semantics;
- each doc links back to ADR 0002 where normative detail lives; and
- no duplicate sections were created unnecessarily.

### Stage C: sync the public consumer-facing documentation

Update `docs/users-guide.md` anywhere a library consumer would otherwise be
misled or left without enough guidance. The likely areas are:

- the `RequestParts` and streaming request-body sections;
- the `MessageAssembler` registration and usage section; and
- the memory-budget section, especially the three-tier protection model and the
  derived default story.

Then audit the public Rustdoc examples in:

- `src/request/mod.rs`
- `src/extractor/streaming.rs`
- `src/app/builder/protocol.rs`
- `src/app/builder/config.rs`

Only edit those comments if the examples or wording would now be stale. The
goal is consistency between the manual and the API docs, not broad comment
rewriting.

Validation at the end of Stage C:

- a new user can follow the guide without needing to read the ADR first; and
- any Rustdoc examples touched by this stage are ready for `make test-doc`.

### Stage D: close validation gaps before claiming the docs are complete

Start by re-running the targeted existing tests that already back the feature.
If the doc changes introduce new claims that are not obviously covered, add the
smallest focused tests that prove them.

The currently known gap is behavioural coverage for streaming request bodies.
Add one focused BDD flow:

- `tests/features/streaming_request.feature`
- `tests/fixtures/streaming_request.rs`
- `tests/steps/streaming_request_steps.rs`
- `tests/scenarios/streaming_request_scenarios.rs`

Register any new fixture, step, or scenario modules in the existing test module
tree.

Keep the scenarios small and observable. Good candidates are:

1. a handler consumes a streaming body incrementally and receives the chunks in
   order;
2. back-pressure suspends the producer while the bounded body channel is full;
   and
3. an inbound streaming-body error surfaces to the handler or reader as
   `InvalidData` or the exact propagated I/O error, depending on the path under
   test.

Use the established `rstest-bdd` rules from this repository:

- the fixture parameter name in every step must match the fixture function
  name exactly;
- do not prefix unused parameters with `_`; bind them normally and discard them
  inside the function body if needed; and
- prefer a focused world fixture over sprawling shared mutable state.

If Stage B or Stage C added claims about pipeline composition that are not
already covered by existing tests, prefer adding focused `rstest` coverage in
existing files such as `tests/streaming_request.rs` or
`src/app/frame_handling/assembly_tests.rs` rather than building another large
BDD harness.

Validation at the end of Stage D:

- the streaming request body feature now has both `rstest` and `rstest-bdd`
  coverage;
- all documentation claims added in Stages B and C are backed by tests or
  existing implementation evidence; and
- no new test file exceeds the 400-line limit.

### Stage E: run the full gates and close the roadmap item

Run the relevant repository gates with log capture. Because this task may touch
manual docs, user-guide docs, and public Rustdoc comments, the final gate set
should include:

- formatting and Markdown validation;
- Mermaid validation;
- Rust formatting, linting, and tests;
- doctests and the doctest-benchmark gate; and
- the roadmap checkbox update.

Only after all gates pass should `docs/roadmap.md` change:

- `8.6.1` to `[x]`
- `8.6.2` to `[x]`
- `8.6.3` to `[x]`

Do not update those checkboxes earlier, or the roadmap will stop reflecting
reality.

## Concrete steps

Run all commands from `/home/user/project`.

1. Audit the current docs and tests.

   ```bash
   rg -n "MessageAssembler|RequestBodyStream|memory budget|soft-limit|hard-cap" \
     docs/generic-message-fragmentation-and-re-assembly-design.md \
     docs/multi-packet-and-streaming-responses-design.md \
     docs/the-road-to-wireframe-1-0-feature-set-philosophy-and-capability-maturity.md \
     docs/users-guide.md \
     docs/adr-002-streaming-requests-and-shared-message-assembly.md \
     tests/streaming_request.rs \
     tests/features/message_assembler.feature \
     tests/features/memory_budgets.feature
   ```

   Expected evidence: matching hits in all four documentation files, `rstest`
   coverage in `tests/streaming_request.rs`, and no existing
   `tests/features/streaming_request.feature`.

2. Run the targeted existing validation first.

   ```bash
   set -o pipefail && cargo test --test streaming_request 2>&1 | tee /tmp/8-6-streaming-request.log
   set -o pipefail && cargo test --test bdd --all-features -- message_assembler 2>&1 | tee /tmp/8-6-message-assembler-bdd.log
   set -o pipefail && cargo test --test bdd --all-features -- memory_budgets 2>&1 | tee /tmp/8-6-memory-budgets-bdd.log
   set -o pipefail && cargo test --test bdd --all-features -- budget_ 2>&1 | tee /tmp/8-6-budget-bdd.log
   ```

   Expected transcript fragments:

   ```plaintext
   test result: ok.
   ```

3. After editing docs and tests, run the final gates with logs.

   ```bash
   set -o pipefail && timeout 300 make fmt 2>&1 | tee /tmp/8-6-fmt.log
   set -o pipefail && timeout 300 make markdownlint MDLINT=/root/.bun/bin/markdownlint-cli2 2>&1 | tee /tmp/8-6-markdownlint.log
   set -o pipefail && timeout 300 make nixie 2>&1 | tee /tmp/8-6-nixie.log
   set -o pipefail && timeout 300 make check-fmt 2>&1 | tee /tmp/8-6-check-fmt.log
   set -o pipefail && timeout 300 make lint 2>&1 | tee /tmp/8-6-lint.log
   set -o pipefail && timeout 300 make test 2>&1 | tee /tmp/8-6-test.log
   set -o pipefail && timeout 300 make test-doc 2>&1 | tee /tmp/8-6-test-doc.log
   set -o pipefail && timeout 300 make doctest-benchmark 2>&1 | tee /tmp/8-6-doctest-benchmark.log
   ```

   Expected transcript fragments:

   ```plaintext
   test result: ok.
   Finished
   ```

4. Update the roadmap checkboxes only after Step 3 succeeds.

## Validation and acceptance

Acceptance means all of the following are true:

- Documentation:
  - the three roadmap-target design documents describe the same composition
    order and budget semantics;
  - `docs/users-guide.md` does not leave library consumers with stale guidance
    about streaming requests, `MessageAssembler`, or memory budgets; and
  - any new design decisions are written in the relevant design document.

- Tests:
  - `tests/streaming_request.rs` passes;
  - the `MessageAssembler` BDD suite passes;
  - the memory-budget BDD suites pass; and
  - the new streaming-request BDD suite passes.

- Quality gates:
  - `make fmt` passes;
  - `make markdownlint MDLINT=/root/.bun/bin/markdownlint-cli2` passes;
  - `make nixie` passes;
  - `make check-fmt` passes;
  - `make lint` passes;
  - `make test` passes;
  - `make test-doc` passes; and
  - `make doctest-benchmark` passes.

- Roadmap closure:
  - `docs/roadmap.md` marks `8.6.1`, `8.6.2`, and `8.6.3` as done.

## Idempotence and recovery

All audit and test commands in this plan are safe to rerun.

Documentation edits are also safe to redo, but `make fmt` may rewrite Markdown
outside the immediate paragraphs that were touched. After each formatting run:

1. inspect `git diff --stat`;
2. confirm the changes are intentional;
3. if `make fmt` rewrote an unrelated document, either keep the clean
   normalization if harmless or restore only that unrelated file before
   continuing.

If a gate fails halfway through:

1. read the saved log under `/tmp/8-6-*.log`;
2. fix the smallest relevant issue;
3. rerun only the failed targeted gate first; and
4. rerun the full final gate set before touching the roadmap checkboxes.

If repo-wide Markdown or doctest gates fail only because of unrelated baseline
debt, stop before marking the roadmap item done and record the exact blocking
files in `Decision Log`.

## Artifacts and notes

Key evidence gathered during planning:

```plaintext
- Existing docs already contain partial 8.6 content.
- docs/users-guide.md already documents RequestParts, MessageAssembler, and MemoryBudgets.
- tests/streaming_request.rs exists and uses rstest.
- No tests/features/streaming_request.feature exists today.
```

Target new artefacts if the audit confirms the current gap:

```plaintext
tests/features/streaming_request.feature
tests/fixtures/streaming_request.rs
tests/steps/streaming_request_steps.rs
tests/scenarios/streaming_request_scenarios.rs
```

## Interfaces and dependencies

No new external dependencies are expected.

The plan depends on these existing public interfaces remaining stable:

- `wireframe::request::RequestParts`
- `wireframe::request::RequestBodyStream`
- `wireframe::request::RequestBodyReader`
- `wireframe::extractor::StreamingBody`
- `wireframe::message_assembler::MessageAssembler`
- `wireframe::app::MemoryBudgets`
- `WireframeApp::with_message_assembler(...)`
- `WireframeApp::message_assembler()`
- `WireframeApp::memory_budgets(...)`

The plan depends on these existing validation frameworks:

- `rstest` for focused Rust tests;
- `rstest-bdd` for behavioural tests;
- repo Makefile targets for formatting, linting, testing, doctests, and
  documentation validation.

Revision note (2026-03-14): Initial draft created after auditing the roadmap,
ADR 0002, current docs, user-guide coverage, and the existing tests. The
remaining work now explicitly includes a focused `rstest-bdd` streaming-request
suite because the current inbound streaming coverage is unit-only.
