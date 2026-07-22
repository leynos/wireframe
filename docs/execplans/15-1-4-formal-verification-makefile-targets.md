# Add formal-verification Makefile targets (15.1.4)

This ExecPlan (execution plan) is a living document. The sections
`Constraints`, `Tolerances`, `Risks`, `Progress`, `Surprises & Discoveries`,
`Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work
proceeds.

Status: DRAFT (awaiting user approval)

## Purpose / big picture

Wireframe is adopting formal verification (roadmap section 15). The tool
installation plumbing already exists: `make install-kani`,
`make check-kani-version`, `make install-verus`, and `make run-verus` delegate to
the pinned `rust-prover-tools` entry point. What is missing is the *execution
surface* a contributor or CI job uses to run verification work.

Roadmap item 15.1.4 adds six Makefile targets that form that surface:

1. `make test-verification` runs the Stateright model tests in
   `crates/wireframe-verification`.
2. `make kani` runs the fast Kani smoke harnesses.
3. `make kani-full` runs every Kani harness.
4. `make verus` runs the Verus proof entry point.
5. `make formal-pr` is the fast gate intended for every pull request.
6. `make formal-nightly` is the deeper gate intended for scheduled runs.

After this change a contributor can type `make formal-pr` on a freshly checked
out, clean tree and watch it complete with exit code `0`, today, even though no
Kani harnesses and no Verus proofs exist yet. `make test-verification` runs for
real immediately, because the verification crate already contains a Stateright
model and roughly twenty test items. The three tool-driven targets (`kani`,
`kani-full`, `verus`) are **explicit stubs** until the later roadmap items that
own them land their harnesses and proofs.

You can observe success three ways:

1. `mbake validate Makefile` reports `Valid syntax` and exits `0`.
2. Each of the six targets exits `0` on a clean tree. The three stub targets
   print a single structured `FORMAL-SKIP:` notice to standard error explaining
   which roadmap item will replace the stub.
3. `make test` passes, including new regression tests in
   `tests/formal_tooling.rs` and a new behavioural scenario in
   `tests/features/formal_tooling.feature` that assert the targets exist,
   delegate correctly, and behave as specified (skip-by-default, fail under
   `FORMAL_STRICT=1`).

## Design rationale: explicit stubs, not self-activating guards

This is the single most important decision in this plan, so it is stated up
front. It was revised after a Logisphere design-review panel (see "Design review
summary" below); the rejected first approach is recorded in the Decision Log.

The roadmap success criterion for 15.1.4 is:

> each target is accepted by `mbake validate Makefile` and returns exit `0` on a
> clean tree.

Three of the six targets have nothing to run yet:

- `make kani` (smoke) would name specific harnesses that arrive in roadmap
  15.3.1 (`src/frame/*`).
- `make kani-full` would run `cargo kani -p wireframe`, but there are zero
  `#[kani::proof]` harnesses in the tree today.
- `make verus` would run `verus/wireframe_proofs.rs`, which does not exist; the
  `verus/` directory is absent. The developers' guide already documents that
  `make run-verus` is *expected to fail* with a missing-proof diagnostic until
  roadmap 15.5.2.

A target cannot both "run the real Verus proof" and "return exit `0` on a clean
tree" while the proof file is absent. The plan resolves this with **explicit
stubs**: `kani`, `kani-full`, and `verus` invoke a single tiny shared helper,
`scripts/formal-stub.sh`, which prints a structured `FORMAL-SKIP:` line naming
the roadmap item that will replace the stub, and exits `0`. There is no
filesystem `grep`, no `[ -f ]` test, no empty-variable loop, and no recursive
sub-make. When roadmap 15.3.x and 15.5.x land their harnesses and proofs, the
PR that adds them *replaces that target's one-line stub recipe* with the real
tool invocation — a loud, reviewable edit made in the same change that is
already touching that part of the tree.

Why explicit stubs rather than guards that self-activate when artefacts appear:

1. The guarded "self-activating" design is an outlier — none of the sibling
   repos (`netsuke`, `chutoro`, `mxd`) ship readiness oracles in their
   Makefiles. A Makefile is read far more often than it is run; optimise for the
   reader.
2. Self-activation fails *silently*: a harness gated behind a feature the grep
   does not see, or a smoke list a human forgets to populate, leaves a target
   printing "skipping" forever while everyone believes verification runs. An
   explicit stub fails *loudly*: the activating PR must edit the recipe, and the
   reviewer sees it.
3. The most-run gate (`formal-pr` → `kani`) was, in the guarded design, the most
   fragile (hand-maintained smoke list), while the least-run gate
   (`formal-nightly` → `kani-full`) self-healed. That is backwards. Explicit
   stubs make both tiers equally honest.

To keep the future activation safe, the plan adds `FORMAL_STRICT=1`: an opt-in
environment switch that turns every stub's skip into a non-zero exit. Roadmap
15.1.5 (CI jobs) can then run a strict job that fails if any `FORMAL-SKIP:`
marker appears once a target is supposed to be real, giving the team a tripwire
against a forgotten activation.

This design deliberately diverges from the literal Makefile snippet in
`docs/formal-verification-methods-in-wireframe.md` §"Recommended Makefile
changes", which assumes harnesses and proofs already exist. The six target names
and the aggregate composition match the doc; only the interim recipe bodies
differ, and only until 15.3.x/15.5.x fill them in.

## Constraints

Hard invariants that must hold throughout implementation. Violation requires
escalation, not a workaround.

1. Do not modify production crate source (`src/**`), the public API, or
   `Cargo.toml` dependency or feature tables. This item is build-and-test
   plumbing only. Adding `cfg(kani)` to `check-cfg` belongs to roadmap 15.3.1,
   alongside the first harness.
2. Keep the existing `run-verus`, `install-kani`, `check-kani-version`, and
   `install-verus` targets byte-for-byte unchanged. The regression suite in
   `tests/formal_tooling.rs` pins their exact recipes, and the developers' guide
   documents `run-verus` as the raw, fail-loud proof runner.
3. The Makefile must continue to pass `mbake validate Makefile` with exit `0`.
4. Recipes must use real tab indentation (Make requires tabs). Editors that
   convert tabs to spaces will break the build and `mbake`.
5. New formal targets must not be pulled into `make test`, `make lint`, or the
   default `all` target. They are a separate surface, consistent with the CI
   guidance that formal jobs stay independent of `build-test`.
6. The targets must exit `0` on a clean tree *without* Kani or Verus installed.
   Plumbing validation must not depend on a multi-minute toolchain install. By
   corollary, `formal-pr` must not depend on `check-kani-version` (which needs
   Kani installed); it composes only tool-free targets.
7. No test in the default `make test` suite may invoke `cargo kani`, `verus`, or
   `run-verus`, before or after 15.3.x/15.5.x. The stub targets are safe to
   execute in tests because they only run `scripts/formal-stub.sh`. When a
   future item replaces a stub recipe with a real tool invocation, that same PR
   must relocate or guard the corresponding execution test so the default suite
   stays tool-free. This invariant is documented in the developers' guide.
8. Recipes and `scripts/formal-stub.sh` must be POSIX-sh clean (no bashisms such
   as `[[ ]]` or `local`); the script must pass `shellcheck`.
9. `.PHONY` handling: the test helper is hardened to follow backslash
   continuations, so `.PHONY` may wrap; recipe blocks are separated by exactly
   one blank line.
10. All prose in changed Markdown wraps at 80 columns; fenced code blocks wrap at
    120 columns; use `-` for list bullets; follow
    `docs/documentation-style-guide.md`.

## Tolerances (exception triggers)

Stop and escalate (do not improvise) when any threshold is crossed:

1. Scope: this change legitimately spans the Makefile, a small shell helper, the
   formal-tooling test harness (integration test, shared helper, BDD fixture,
   feature file, step definitions), and two docs. If it grows beyond roughly 12
   files or 400 net lines, stop and escalate.
2. Source/API: if any change to `src/**`, the public API, or `Cargo.toml`
   `[dependencies]`/`[features]` appears necessary, stop and escalate.
3. Existing targets: if any change to the four existing prover-tools targets
   appears necessary, stop and escalate.
4. Tooling: if `mbake validate Makefile` cannot pass without restructuring
   unrelated parts of the Makefile, stop and escalate.
5. Test design: if making `make test` cover the new targets would require Kani
   or Verus to be installed, stop and escalate (Constraints 6 and 7).
6. Iterations: if the gates (`mbake`, `check-fmt`, `lint`, `test`) still fail
   after 3 fix attempts on the same milestone, stop and escalate.
7. Ambiguity: if a reviewer rejects the explicit-stub design, stop and present
   the trade-offs rather than choosing unilaterally.

## Risks

1. Risk: an automated "exit `0`" test that *really executes* `make formal-pr`
   would recurse into `cargo test -p wireframe-verification` from inside
   `cargo test`, inflating `make test` runtime.
   Severity: medium. Likelihood: high if done naively.
   Mitigation: real-execute only the stub targets (`kani`, `kani-full`, `verus`),
   which merely run `scripts/formal-stub.sh`; assert `test-verification`,
   `formal-pr`, and `formal-nightly` via `make --dry-run`, which does not execute
   recipes. Validate the full real run once, manually, recorded as evidence.
2. Risk: the existing test helpers cannot assert what the plan needs.
   `MakefileContent::target_recipe` discards the rule line, so it returns
   `Some("")` for the prerequisite-only aggregates (`formal-pr`,
   `formal-nightly`) and cannot see their prerequisites; `has_phony_target`
   breaks if `.PHONY` is wrapped onto a continuation line.
   Severity: high (would give false confidence). Likelihood: high.
   Mitigation: add a `target_prerequisites` helper that parses the rule line, and
   harden `has_phony_target` to join `\`-continued `.PHONY` lines. Assert the
   aggregates via both the new helper and `make --dry-run` content. Do not rely
   on `target_recipe` being `Some` for the aggregates (that proves nothing).
3. Risk: tab/space corruption in new recipes breaks Make and `mbake`.
   Severity: medium. Likelihood: medium.
   Mitigation: after editing, run `mbake validate Makefile` and `make --dry-run`
   for each new target before the wider gates.
4. Risk: a forgotten activation — a future item adds a harness or proof but does
   not replace the stub recipe — leaves a target skipping while looking green.
   Severity: medium. Likelihood: medium.
   Mitigation: explicit stubs make activation a visible recipe edit in the
   owning PR; `FORMAL_STRICT=1` lets 15.1.5 add a CI tripwire that fails on any
   `FORMAL-SKIP:` marker; the developers' guide records the activation contract
   and the tool-free-test invariant (Constraint 7) so 15.3.x/15.5.x authors know
   what to change.
5. Risk: when 15.3.x replaces the `kani` stub with a real recipe, it must invoke
   the *pinned* Kani (resolved through `rust-prover-tools` / the `KANI_ENV`
   `LD_LIBRARY_PATH` pattern used by `chutoro`), not an arbitrary `cargo kani` on
   `PATH`.
   Severity: low (future item). Likelihood: low.
   Mitigation: note this requirement in the developers' guide activation
   contract so the owning item gets it right; out of scope for 15.1.4.

## Progress

- [x] (2026-06-22) Research: studied roadmap 15.1.4, the formal-verification
      methods doc, prior-art Makefiles (`netsuke`, `chutoro`, `mxd`), the
      existing formal-tooling test harness, and current repo state.
- [x] (2026-06-22) Drafted the initial plan (self-activating skip guards).
- [x] (2026-06-22) Logisphere design-review panel; pivoted to explicit stubs and
      revised the plan accordingly.
- [ ] (pending) User approval of this plan (required before implementation).
- [ ] (pending) Stage A: confirm orientation and baseline.
- [ ] (pending) Stage B: red regression and behavioural tests.
- [ ] (pending) Stage C: implement the helper script and six Makefile targets.
- [ ] (pending) Stage D: documentation and roadmap tick.
- [ ] (pending) Stage E: full gate run and CodeRabbit review.

## Surprises & discoveries

- Observation: a complete formal-tooling test harness already exists.
  Evidence: `tests/formal_tooling.rs`, `tests/common/formal_tooling_support.rs`,
  `tests/fixtures/formal_tooling.rs`, and
  `tests/features/formal_tooling.feature` already assert the four existing
  prover-tools targets via recipe inspection and `make --dry-run`.
  Impact: 15.1.4 extends this harness rather than inventing a test approach.
- Observation: the shared helpers cannot assert prerequisite-only rules and
  break on a wrapped `.PHONY`.
  Evidence: `tests/common/formal_tooling_support.rs:16-39` —
  `target_recipe` discards the rule line; `has_phony_target` splits a single
  line only.
  Impact: the test plan must add a `target_prerequisites` helper and harden
  `has_phony_target` (Risk 2).
- Observation: the developers' guide already states `run-verus` is expected to
  fail until `verus/wireframe_proofs.rs` exists.
  Evidence: `docs/developers-guide.md` §"Formal verification tooling".
  Impact: confirms `run-verus` stays fail-loud; the new `verus` target is a
  separate stub, not a wrapper around `run-verus`.

## Decision log

- Decision: use explicit stubs (`scripts/formal-stub.sh`) for `kani`,
  `kani-full`, and `verus`, replaced by the owning roadmap items; add
  `FORMAL_STRICT=1` to turn skips into failures.
  Rationale: satisfies the exit-`0` criterion while keeping harnesses (15.3.x)
  and proofs (15.5.x) in their own items, keeps plumbing tool-free, and makes
  activation a loud, reviewable edit. Endorsed by the Logisphere panel.
  Date/Author: 2026-06-22, planning agent + design-review panel.
- Decision (superseded): the first draft used self-activating guards
  (`grep -rq 'kani::proof' src`, an empty `KANI_SMOKE_HARNESSES` loop, and
  `$(MAKE) run-verus` gated on `[ -f ]`).
  Rationale for reversal: three different readiness idioms in ~14 lines is too
  clever for plumbing; it is an outlier versus sibling repos; and its failure
  mode is a *silent* forever-skip (false green), worst on the most-run PR gate.
  Date/Author: 2026-06-22, design-review panel (Wafflecat, Dinolump, Doggylump).
- Decision: `make test-verification` uses `cargo test -p wireframe-verification`
  (not `cargo nextest`).
  Rationale: the formal-verification methods document recommends `cargo test` to
  minimise the delta from current Wireframe practice; switch to nextest only if
  the whole repo migrates.
  Date/Author: 2026-06-22, planning agent.
- Decision: do not introduce `KANI`/`KANI_*` Makefile variables in 15.1.4.
  Rationale: with stub recipes they would be dead config; 15.3.x introduces them
  with the real recipes (and must use the pinned Kani — Risk 5).
  Date/Author: 2026-06-22, design-review panel (Dinolump).
- Decision: keep the `formal` alias (`formal: formal-pr`); drop the `stateright`
  alias.
  Rationale: `formal` is a discoverable default with no semantic trap;
  `stateright` is a misleading synonym for `test-verification` (the CI job is
  named `stateright-models` but runs `cargo test`), inviting the belief that it
  runs the Stateright tool directly.
  Date/Author: 2026-06-22, design-review panel (Dinolump).
- Decision: do not add `cfg(kani)` to `check-cfg` or otherwise touch
  `Cargo.toml` in 15.1.4.
  Rationale: there is no `cfg(kani)` code yet; that wiring belongs to 15.3.1.
  Date/Author: 2026-06-22, planning agent.

## Outcomes & retrospective

To be completed at the end of implementation. Compare the delivered targets
against the six required names and the exit-`0` criterion; confirm the
`FORMAL_STRICT` tripwire works and the activation contract is documented for
15.3.x/15.5.x.

## Design review summary

A Logisphere pre-implementation panel reviewed the first draft. Key outcomes,
all folded into this revision:

- Pandalump/Telefono (structure and contracts): the *test plan* was the weak
  point — the existing helpers cannot assert the prerequisite-only aggregates
  and break on a wrapped `.PHONY`. Fixed by Risk 2's mitigations.
- Wafflecat (alternatives) and Dinolump (viability): the self-activating guards
  were too clever and an outlier versus sibling repos; recommended explicit
  stubs that the owning PRs replace, plus dropping the `stateright` alias and the
  premature `KANI_*` variables. Adopted.
- Doggylump (pre-mortem): the worst case is a silent "false green" (a target
  that skips forever) and a "time bomb" test that detonates inside `make test`
  once guards flip. Addressed by explicit stubs (loud activation), Constraint 7
  (tool-free default suite), and the `FORMAL_STRICT=1` tripwire with a greppable
  `FORMAL-SKIP:` marker.

## Context and orientation

The reader is assumed to know nothing about this repository.

This is the `wireframe` Rust library. The repository root holds the primary
crate; `crates/wireframe-verification` is an internal (unpublished) workspace
member holding Stateright models; `wireframe_testing` is a test-support crate.
The workspace:

```toml
[workspace]
members = [".", "crates/wireframe-verification", "wireframe_testing"]
default-members = ["."]
resolver = "3"
```

Key files for this task, by full path:

1. `Makefile` — the file to edit. Today it ends with the four prover-tools
   targets (`install-kani`, `check-kani-version`, `install-verus`, `run-verus`)
   and a `help` target. Relevant existing variables: `CRATE ?= wireframe`,
   `CARGO ?= cargo`, `BUILD_JOBS ?=`, `PROVER_TOOLS ?= ...`, and
   `VERUS_PROOF_FILE ?= verus/wireframe_proofs.rs`. The first line is the single
   `.PHONY` declaration.
2. `scripts/formal-stub.sh` — new. A small POSIX-sh helper that prints a
   `FORMAL-SKIP:` notice and exits `0`, or exits non-zero under
   `FORMAL_STRICT`. The repo already keeps helpers under `scripts/` (for example
   `scripts/doctest-benchmark.sh`).
3. `tests/formal_tooling.rs` — integration regression tests. Uses `rstest`,
   `proptest`, and helpers from `tests/common/formal_tooling_support.rs`. It
   already verifies targets are `.PHONY`, that recipes contain expected
   substrings, and that `make --dry-run <target>` emits the expected command.
   New red tests go here.
4. `tests/common/formal_tooling_support.rs` — shared helpers:
   `MakefileContent { has_phony_target, target_recipe }`, `makefile()`,
   `MAKEFILE_PATH`, version readers, and `run_make_dry_run`. This file gains a
   `target_prerequisites` helper, a hardened `has_phony_target` (follows `\`
   continuations), and a `run_make` helper (real execution, capturing status and
   stderr) beside `run_make_dry_run`.
5. `tests/fixtures/formal_tooling.rs` — the BDD world `FormalToolingWorld`. Gains
   `verify_formal_execution_targets(&self) -> TestResult`.
6. `tests/features/formal_tooling.feature` — the Gherkin feature. Gains a
   scenario for the execution targets. The matching step definitions live in the
   BDD module wired through `tests/bdd.rs`; locate the existing formal-tooling
   steps (search `tests/` for the step text "the Makefile exposes the formal
   verification tool entry points", or use
   `leta grep "formal" -k function`) and add steps beside them.
7. `docs/developers-guide.md` — §"Formal verification tooling" (around line 259)
   documents the four install targets. The six execution targets, the
   `FORMAL_STRICT` switch, the activation contract, and the tool-free-test
   invariant (Constraint 7) are documented here.
8. `docs/roadmap.md` — item 15.1.4 (line 603) is ticked on completion.

Reference material (read for rationale; do not edit):
`docs/formal-verification-methods-in-wireframe.md` §"Recommended Makefile
changes" and §"Recommended CI changes"; prior-art Makefiles in `leynos/netsuke`
(cheap `formal-pr`), `leynos/chutoro` (explicit Kani harness names plus a
`KANI_ENV` `LD_LIBRARY_PATH` wrapper), and `leynos/mxd`
(`test-verification` runs the verification crate).

Relevant skills to load while implementing: `execplans` (this document), `kani`
and `verus` (tool semantics; needed when the stubs are later replaced),
`rust-verification` (selection context), `rust-unit-testing` and `proptest`
(the existing tests in `formal_tooling.rs`), and `leta` for navigation;
`rust-router` routes deeper Rust questions.

## Plan of work

Proceed in stages with a validation gate at the end of each. Do not advance past
a failing gate. Do not begin Stage A until the user has approved this plan.

### Stage A: understand and propose (no code changes)

1. Run `git branch --show-current`; confirm
   `15-1-4-formal-verification-makefile-targets`.
2. Run `mbake validate Makefile`; expect `Valid syntax`, exit `0`.
3. Re-read the tail of the `Makefile` and the cases in `tests/formal_tooling.rs`
   that drive `makefile_declares_prover_tools_targets` and
   `make_targets_dry_run_to_expected_prover_tools_command`. Confirm the helper
   API in `tests/common/formal_tooling_support.rs`, in particular how
   `target_recipe` and `has_phony_target` parse lines.

Gate A: orientation notes recorded; no diffs.

### Stage B: red regression and behavioural tests

Add the smallest tests that specify the missing behaviour and watch them fail
for the right reason before touching the Makefile or the helper script.

1. Extend `tests/common/formal_tooling_support.rs`:
   - harden `has_phony_target` to join backslash-continued `.PHONY` lines;
   - add `target_prerequisites(&self, target) -> Option<Vec<String>>` that finds
     the rule line and returns the tokens after the colon;
   - add `run_make(target) -> FormalToolingResult<(ExitStatus, String, String)>`
     (real execution, returning status, stdout, stderr) and an
     `env`-aware variant or argument for `FORMAL_STRICT`.
2. In `tests/formal_tooling.rs`, add:
   - `formal_execution_targets_are_declared` (`rstest` cases over all six
     targets): each is `has_phony_target` true and present.
   - `direct_recipe_targets_have_expected_content` (`rstest` over
     `test-verification`, `kani`, `kani-full`, `verus`): `test-verification`
     recipe contains `test -p wireframe-verification`; each stub recipe contains
     `formal-stub.sh` and its own target name.
   - `aggregate_targets_declare_expected_prerequisites` (`rstest`): `formal-pr`
     prerequisites are exactly `test-verification kani verus`; `formal-nightly`
     are `test-verification kani-full verus` (via the new
     `target_prerequisites`).
   - `stub_targets_skip_and_exit_zero` (`rstest` over `kani`, `kani-full`,
     `verus`): `run_make` exits `0` and stderr contains `FORMAL-SKIP:` and the
     target name.
   - `stub_targets_fail_under_formal_strict` (`rstest` over the three stubs):
     with `FORMAL_STRICT=1`, `run_make` exits non-zero and stderr contains
     `FORMAL-SKIP:`.
   - `aggregate_targets_dry_run_zero` (`rstest` over `test-verification`,
     `formal-pr`, `formal-nightly`): `make --dry-run` exits `0`;
     `formal-pr`/`formal-nightly` dry-run output mentions `wireframe-verification`
     and `formal-stub.sh`.
3. In `tests/features/formal_tooling.feature`, add:

   ```gherkin
   Scenario: The repository exposes formal-verification execution targets
     Given the formal verification tooling metadata is loaded
     Then the Makefile exposes the formal verification execution targets
     And the formal execution stubs skip cleanly on a clean tree
   ```

   Add `verify_formal_execution_targets` to `FormalToolingWorld` and matching
   step definitions beside the existing formal-tooling steps.

Gate B (Red): run
`cargo test --test formal_tooling 2>&1 | tee /tmp/red-wireframe-$(git branch --show-current).out`
and `make test-bdd 2>&1 | tee -a /tmp/red-wireframe-$(git branch --show-current).out`.
Expect the new cases to fail because the targets and the stub script do not exist
yet. Record the failure text as evidence.

### Stage C: implement the helper and the six targets

1. Create `scripts/formal-stub.sh` (POSIX sh, executable, `shellcheck`-clean).
   Behaviour: argument 1 is the target name, the rest is a human message; print
   `FORMAL-SKIP: <target> not yet implemented — <message>` to stderr; if
   `FORMAL_STRICT` is non-empty, exit `1`, else exit `0`. Do not interpolate
   untrusted values into a `printf` format string (pass them as arguments).
2. Edit `Makefile`:
   - extend `.PHONY` with
     `test-verification kani kani-full verus formal-pr formal-nightly formal`;
   - add `VERIFICATION_CRATE ?= wireframe-verification`,
     `FORMAL_STUB ?= ./scripts/formal-stub.sh`, and `FORMAL_STRICT ?=` near the
     other tool variables (exporting `FORMAL_STRICT` so the script sees it, or
     passing it explicitly — see "Interfaces");
   - add the six targets and the `formal` alias (recipes in "Interfaces").
   Keep one blank line between target blocks; use real tabs.

Gate C (Green):

1. `mbake validate Makefile` → `Valid syntax`, exit `0`.
2. `shellcheck scripts/formal-stub.sh` → clean.
3. `make --dry-run` for each of the six targets exits `0`.
4. Re-run the Stage B tests; they pass. Tee to
   `/tmp/green-wireframe-$(git branch --show-current).out`.

### Stage D: documentation and roadmap

1. Expand `docs/developers-guide.md` §"Formal verification tooling" with a
   subsection covering the six targets, the `FORMAL_STRICT` switch and
   `FORMAL-SKIP:` marker, the activation contract (which roadmap item replaces
   each stub, and that the replacement must use the pinned Kani and keep the
   default test suite tool-free per Constraint 7). Wrap prose at 80 columns.
2. Run `make fmt`, then `make markdownlint`, and `make nixie` if the changed
   Markdown contains Mermaid.
3. Tick roadmap item 15.1.4 in `docs/roadmap.md` only after every gate passes.

Gate D: `make check-fmt` and `make markdownlint` pass.

### Stage E: full gates and review

1. Run sequentially (never in parallel; caching is shared), teeing each to
   `/tmp`: `make check-fmt`, `make lint`, `make test`.
2. Run `mbake validate Makefile` once more.
3. Manually run all six targets for real on a clean tree; capture a transcript
   proving each exits `0`, and run the three stubs with `FORMAL_STRICT=1` to show
   they exit non-zero.
4. Commit, then run `coderabbit review --agent`; resolve every concern before
   declaring the milestone complete.

Gate E: all gates green; CodeRabbit concerns cleared.

## Concrete steps

Run everything from the repository root.

Baseline:

```bash
git branch --show-current
mbake validate Makefile
```

Expected:

```plaintext
15-1-4-formal-verification-makefile-targets
✓ Makefile: Valid syntax
```

Red stage:

```bash
cargo test --test formal_tooling \
  2>&1 | tee /tmp/red-wireframe-$(git branch --show-current).out
```

Expect failures naming the missing targets and the missing stub script.

Green stage (after Stage C):

```bash
mbake validate Makefile
shellcheck scripts/formal-stub.sh
for t in test-verification kani kani-full verus formal-pr formal-nightly; do
  echo "== dry-run $t =="; make --dry-run "$t"; echo "exit=$?";
done
cargo test --test formal_tooling \
  2>&1 | tee /tmp/green-wireframe-$(git branch --show-current).out
```

Stub behaviour proof on a clean tree:

```bash
for t in kani kani-full verus; do
  make "$t"; echo "$t exit=$?";
  FORMAL_STRICT=1 make "$t"; echo "$t strict exit=$?";
done
```

Expected (abbreviated):

```plaintext
FORMAL-SKIP: kani not yet implemented — roadmap 15.3.1 adds src/frame Kani smoke harnesses
kani exit=0
FORMAL-SKIP: kani not yet implemented — roadmap 15.3.1 adds src/frame Kani smoke harnesses
kani strict exit=1
...
```

Wider gates (sequential):

```bash
make check-fmt 2>&1 | tee /tmp/check-fmt-wireframe-$(git branch --show-current).out
make lint      2>&1 | tee /tmp/lint-wireframe-$(git branch --show-current).out
make test      2>&1 | tee /tmp/test-wireframe-$(git branch --show-current).out
```

## Validation and acceptance

Acceptance is behavioural:

1. Plumbing valid: `mbake validate Makefile` prints `✓ Makefile: Valid syntax`
   and exits `0`; `shellcheck scripts/formal-stub.sh` is clean.
2. Each of the six targets exits `0` on a clean tree. `kani`, `kani-full`, and
   `verus` print a `FORMAL-SKIP:` notice; `test-verification`, `formal-pr`, and
   `formal-nightly` run the verification crate tests and pass.
3. `FORMAL_STRICT=1 make kani` (and `kani-full`, `verus`) exits non-zero — the
   tripwire works.
4. Red-Green evidence: the new cases in `tests/formal_tooling.rs` fail before the
   implementation (target/script missing) and pass after.
5. `make test` passes overall, including the new behavioural scenario.
6. Activation contract documented: the developers' guide states which roadmap
   item replaces each stub, that the replacement must use the pinned Kani, and
   that the default test suite must stay tool-free (Constraint 7).

Quality criteria ("done" means):

- Tests: `make test` passes; new `formal_tooling` cases and BDD scenario pass.
- Lint/format: `make check-fmt`, `make lint`, `make markdownlint` exit `0`;
  `shellcheck` clean.
- Makefile: `mbake validate Makefile` exits `0`.
- Review: `coderabbit review --agent` reports no outstanding concerns.

Quality method: run the gates sequentially from the repository root, teeing each
to `/tmp`, and review the logs. Run CodeRabbit only after the deterministic gates
are green.

## Idempotence and recovery

All steps are re-runnable. The Makefile edits are additive (new targets, a few
variables, an extended `.PHONY` line); re-applying them is a no-op once present.
The stub script and the stub recipes have no side effects beyond a stderr line.
If a recipe breaks `mbake`, revert the Makefile hunk with
`git checkout -- Makefile` and re-apply carefully, watching for tab/space
corruption. The test additions are independent and can be reverted in isolation.
No destructive operations are involved.

## Artifacts and notes

Record, on completion: the baseline and red/green transcripts; the clean-tree
stub transcript including the `FORMAL_STRICT=1` non-zero exits; and the final
`make test` summary line (counts passed).

## Interfaces and dependencies

Dependencies (roadmap): 15.1.2 (the `wireframe-verification` crate, present) and
15.1.3 (pinned tool metadata and the four prover-tools targets, present). No new
external crates. No `Cargo.toml` changes.

`scripts/formal-stub.sh` (new; leading spaces shown for readability):

```sh
#!/bin/sh
# Placeholder runner for formal-verification Make targets whose real harnesses
# or proofs arrive in later roadmap items. Prints a structured skip marker and
# exits 0, or exits non-zero when FORMAL_STRICT is set so CI can assert that a
# target which should now be real is not still skipping.
set -eu

target="${1:?target name required}"
shift
message="$*"

printf 'FORMAL-SKIP: %s not yet implemented — %s\n' "$target" "$message" >&2

if [ -n "${FORMAL_STRICT:-}" ]; then
    exit 1
fi
exit 0
```

Proposed Makefile additions (authoritative for this milestone; the six target
names, the aggregate composition, and the `FORMAL_STRICT` semantics are the
contract). Recipes shown with leading spaces — they MUST be real tabs:

```make
VERIFICATION_CRATE ?= wireframe-verification
FORMAL_STUB ?= ./scripts/formal-stub.sh
FORMAL_STRICT ?=
export FORMAL_STRICT

test-verification: ## Run the Stateright verification crate tests
	RUSTFLAGS="-D warnings" $(CARGO) test -p $(VERIFICATION_CRATE) $(BUILD_JOBS)

kani: ## Run Kani smoke harnesses (stub until roadmap 15.3.1)
	@$(FORMAL_STUB) kani "roadmap 15.3.1 adds src/frame Kani smoke harnesses"

kani-full: ## Run every Kani harness (stub until roadmap 15.3.x)
	@$(FORMAL_STUB) kani-full "roadmap 15.3.x adds the full Kani harness set"

verus: ## Run Verus proofs (stub until roadmap 15.5.2)
	@$(FORMAL_STUB) verus "roadmap 15.5.2 adds verus/wireframe_proofs.rs"

formal-pr: test-verification kani verus ## Fast pull-request formal gate

formal-nightly: test-verification kani-full verus ## Deeper scheduled formal gate

formal: formal-pr ## Default formal suite (alias for the PR gate)
```

Notes:

- `formal-pr` deliberately does not depend on `check-kani-version`; that target
  needs Kani installed and would violate the tool-free clean-tree criterion
  (Constraint 6).
- The aggregates assume serial execution; `make -j formal-pr` would race
  `test-verification` against the stubs harmlessly, but the documented contract
  is serial.

Test interfaces that must exist at the end of the milestone:

- `tests/common/formal_tooling_support.rs`: hardened `has_phony_target`; new
  `target_prerequisites`; new `run_make` (real execution capturing status and
  stderr, honouring `FORMAL_STRICT`).
- `tests/formal_tooling.rs`: the six new `rstest` case sets described in Stage B.
- `tests/fixtures/formal_tooling.rs`:
  `FormalToolingWorld::verify_formal_execution_targets(&self) -> TestResult`.
- `tests/features/formal_tooling.feature`: the execution-targets scenario with
  matching step definitions.

## Revision note

- Change: pivoted from self-activating skip guards (grep/file-existence/sub-make)
  to explicit stubs via `scripts/formal-stub.sh`, added `FORMAL_STRICT=1` and the
  `FORMAL-SKIP:` marker, dropped the premature `KANI_*` variables and the
  `stateright` alias, kept the `formal` alias, and rewrote the test strategy to
  add a `target_prerequisites` helper and harden `has_phony_target`.
- Why: a Logisphere design-review panel found the guarded design too clever, an
  outlier versus sibling repos, and silently failure-prone (false green), and
  found the existing test helpers unable to assert the prerequisite-only
  aggregates or a wrapped `.PHONY`.
- Effect on remaining work: implementation is simpler and tool-free; activation
  becomes an explicit recipe edit owned by 15.3.x/15.5.x, guarded by the
  `FORMAL_STRICT` tripwire and Constraint 7. Awaiting user approval before
  Stage A.
