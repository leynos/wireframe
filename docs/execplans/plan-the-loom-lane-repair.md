# Make the Loom lane run the models, and make the models able to fail

This ExecPlan (execution plan) is a living document. The sections `Constraints`,
`Tolerances`, `Risks`, `Progress`, `Surprises & discoveries`, `Decision log`,
`Outcomes & retrospective`, `Conformance basis`, and `Verification plan` must
be kept up to date as work proceeds.

Status: DRAFT

Draft pending one ruling, recorded under `Decision log` as D-1: whether the
push queue's message channel is substituted under the model or declared out of
scope. Milestone one does not depend on that ruling and may proceed once the
plan is approved. Milestones two to four must not begin until D-1 is settled,
because D-1 decides what the models are for.

## Purpose / big picture

The scheduled **Advanced Tests** lane exists to model-check the push queue's
behaviour under concurrent producers. It has never done so. It fails while
compiling the library, before any model runs, and has failed every night since
at least 2026-09-09.

Repairing only that compile failure would turn the lane green while checking
almost nothing, because two further defects mean the models do not exercise the
state they assert about. A green lane that establishes nothing is worse than a
red one, because a red lane advertises its own uselessness and a green one does
not.

After this work a reader can see:

- the lane running the four push-queue models rather than failing to compile;
- a statement in the developers' guide naming exactly which synchronization
  primitives the models schedule and which they do not, so nobody has to
  re-derive it;
- every assertion in the model file proved able to fail, by a recorded
  mutation that the model rejects for the intended reason.

Until the last of those, the lane's honest description is "a compile check for
the `cfg(loom)` configuration", and the guide will say so in those words.

### Terms used here

**Loom** is a library that runs a piece of concurrent code many times, choosing
a different interleaving of threads each time, to find orderings that break an
assertion. It can only do that for threads it created and for synchronization
primitives it supplies, such as `loom::sync::Mutex`. A primitive it did not
supply is invisible: Loom cannot pause a thread inside it, so orderings
involving it are never explored.

**A model** is one closure passed to `loom::model`, which Loom runs repeatedly
under different schedules.

**`cfg(loom)`** is a compile-time configuration flag. The lane sets it with
`RUSTFLAGS="--cfg loom"`, which builds a different version of the crate in
which Loom's primitives replace the standard ones.

**A dead-letter queue**, abbreviated DLQ, is where this crate sends frames it
had to discard because a queue was full.

## Context and orientation

Assume no prior knowledge of this repository.

The lane is `.github/workflows/advanced-tests.yml`. It runs nightly at 17:00
UTC and its only command is:

```bash
RUSTFLAGS="--cfg loom" cargo test --features advanced-tests
```

The models live in one file, `tests/advanced/concurrency_loom.rs`, declared in
`Cargo.toml` as the `concurrency_loom` test target. It contains four models,
all exercising `PushHandle`:

- `concurrent_drops_reset_dlq_counter`, parameterized over two priorities,
- `concurrent_queue_full_errors_are_reported`,
- `dlq_probe_ignores_absent_channel`,
- `dlq_probe_reports_zero_when_dlq_idle`.

The subject is `src/push/queues/`, principally `handle.rs`. `PushHandle` is a
cloneable producer handle wrapping `PushHandleInner`, which holds two Tokio
channels for high- and low-priority frames, an optional Tokio channel for the
DLQ, an optional `leaky_bucket::RateLimiter`, a counter `dlq_drops`, and a mutex
`dlq_last_log` guarding the time of the last log line.

`handle.rs` already carries the conventional pattern for a Loom build: a private
`sync` module aliasing types differently under each configuration.

The test helpers are in `src/test_helpers.rs` and
`src/test_helpers/pool_client.rs`. The latter is a TCP client helper.

## Constraints

- **This plan must not modify `.github/workflows/advanced-tests.yml`.** That
  file is owned by separate in-flight work (wireframe #689) which moves the
  lane from a paid runner to a GitHub-hosted one. Milestone two needs the
  lane's command narrowed; see `Risks` R-3 for how that is sequenced without
  both changes touching the file.
- `PushHandle` and `PushQueues` are public API. Their signatures under
  `cfg(not(loom))` must not change. A change visible only under `cfg(loom)` is
  acceptable and expected.
- The pinned toolchain is `nightly-2026-03-26`. Do not bump it as part of
  this work; if a repair appears to need a newer toolchain, that is an
  escalation, not a decision.
- Loom stays at the pinned `0.7.2`. Its absence of a Tokio-compatible channel
  is a fact to plan around, not a version to chase.
- No production behaviour changes. Every edit is to configuration
  predicates, the `cfg(loom)` arms of the synchronization aliases, the
  `Makefile`, the `[dev-dependencies]` table of `Cargo.toml`, tests, or
  documentation. The `Makefile` is named explicitly because EP-M2 adds a target
  there, and `[dev-dependencies]` because EP-M2 adds `syn` there (see the
  dependency exception under `Tolerances`); an earlier draft of this list
  omitted both and so forbade the milestone it describes. No other part of
  `Cargo.toml` may change. If a repair appears to require changing what
  `try_push` or `route_to_dlq` does under `cfg(not(loom))`, stop: that is a
  behaviour change wearing a test repair's clothes.

## Tolerances (exception triggers)

- **Scope**: more than 8 files or 400 net lines changed in any one milestone,
  stop and escalate.
- **Interface**: any change to a public signature under `cfg(not(loom))`,
  stop and escalate.
- **Dependencies**: any new external dependency, stop and escalate. This
  includes a Loom-compatible channel crate, which would be a D-1 outcome
  requiring its own approval rather than an implementation detail.

  **One exception is declared here, and approving this plan approves it:**
  `syn` as a direct `[dev-dependencies]` entry, for V-2's checker in EP-M2
  (decision D-4). It is test-only, it is already resolved in `Cargo.lock`
  through the proc-macro crates so it adds no tree entry, and no consumer of
  the published crate sees it. The exception covers exactly that: `syn` at the
  version `Cargo.lock` already resolves, with only the features the checker
  needs, in `[dev-dependencies]`. Anything wider, such as a second crate, a new
  `Cargo.lock` entry, or `syn` in `[dependencies]`, is outside it and triggers
  this tolerance as usual. If the plan is approved with this exception struck,
  V-2 has no artefact, and EP-M2 must stop and escalate rather than substitute
  a weaker check.
- **Iterations**: if a model still fails after 3 attempts to repair it, stop
  and escalate with the counterexample Loom printed. A Loom counterexample is
  evidence about the subject, not noise to iterate past.
- **Runtime**: if the model file takes more than 10 minutes on a quiet host,
  stop and escalate rather than reducing exploration; a model whose state space
  has exploded is telling you the seam is drawn wrongly.
- **Ambiguity**: D-1 below is the known one. If a second question of the same
  kind appears, stop and present it rather than choosing.

## Risks

- **R-1**: Widening the `sync` alias to `loom::sync::Arc` surfaces further
  uninstrumented primitives, and the work grows past its tolerance. Severity:
  medium. Likelihood: medium. Mitigation: milestone two ends with the models
  *executing* and whatever they report recorded honestly. Discovering more gaps
  there is the milestone succeeding, not failing.

- **R-2**: The models, once they run, find a real defect in the push queue.
  Severity: medium. Likelihood: low. Mitigation: that is the lane doing its
  job. It is an escalation with the counterexample attached, and a separate
  piece of work; this plan does not budget for fixing the subject.

- **R-3**: The lane's command may need to narrow to the Loom target, and this
  plan may not edit the workflow. Severity: low. Likelihood: **unknown until
  EP-M1**, and conditional on it. The only failure there is evidence for is the
  library's; the `bdd_pool` target compiles to an empty crate under
  `--cfg loom` (see `Surprises & discoveries`), so narrowing is required only
  if EP-M1 reproduces a *separate* target-selection failure. Mitigation: if and
  only if EP-M1 does reproduce one, it is recorded under
  `Surprises & discoveries` with its command and errors, EP-M2 lands the
  narrowing as a Makefile target, and the workflow's one-line change to call it
  is taken as a separate one-line pull request (wireframe #689, which owned the
  file, merged on 2026-09-18). If EP-M1 reproduces no such failure, there is no
  narrowing, no workflow change, and no completion dependency on the workflow
  file; EP-M2 and EP-M4 below say the same.

- **R-4**: Milestone three's guide statement becomes stale when milestone
  four changes what is scheduled. Severity: low. Likelihood: medium.
  Mitigation: milestone four's acceptance includes re-reading that statement
  against the code and updating it in the same change.

- **R-5**: `cargo test` under `cfg(loom)` rebuilds the whole dependency tree
  in a second configuration, and the lane is slow. Severity: low. Likelihood:
  medium. Mitigation: observe it in milestone two and report; do not
  pre-optimize.

## Progress

- [x] (2026-09-17 11:56Z) Read-only diagnosis of the failure and of the
  models, recorded in `~/docs/wireframe-683-loom-diagnosis-2026-09-17.md` and
  summarized under `Surprises & discoveries`.
- [x] (2026-09-17 12:10Z) This plan drafted.
- [x] (2026-09-18) Review round: D-1 made to govern the V-4 and EP-M4
  acceptance set, V-2 given an executable artefact (D-4), and the DLQ counter
  mutations split so the increment is observed before the reset cancels it.
- [x] (2026-09-22) Review round: V-2's rule corrected from satisfiability to
  implication with a stated configuration universe; route (b)'s retained
  acceptance set named rather than left as a subtraction; two extant design
  documents found and the earlier claim that none existed withdrawn.
- [x] (2026-09-22) Review round: the `syn` dev-dependency declared as the one
  pre-approved exception to the dependency tolerance, and `Cargo.toml`'s
  `[dev-dependencies]` added to the permitted edits; target narrowing made
  conditional on EP-M1 reproducing a separate failure; V-2's predicates made
  inherited through enclosing modules, with a nested mutation and a narrowness
  case; V-2's configuration universe enumerated atom by atom.
- [ ] The design conflict referred alongside D-1:
      `multi-layered-testing-strategy.md`
  §4.2 targets the write loop, the lane models the push queues, and both
  documents assert queue-full coverage that finding two shows is absent.
- [ ] D-1 ruled on by the user. Blocks milestones two to four, and now also
  blocks filling in V-4's mutation table.
- [ ] EP-M1 reproduce and bound. Unblocked by plan approval alone.
- [ ] EP-M2 configuration boundary; models execute.
- [ ] EP-M3 the seam, and the guide statement.
- [ ] EP-M4 models that can fail.

## Surprises & discoveries

Three findings, from the read-only diagnosis. The first was already recorded in
issue #683; the second and third were not, and they change what the work is.

- **Observation**: The helper's configuration predicate admits a build in
  which the types it imports do not exist. Evidence: `src/test_helpers.rs` gates
  `pub mod pool_client` on `#[cfg(feature = "pool")]`. `src/lib.rs` gates
  `pub mod client` on `#[cfg(not(loom))]`, and `tokio::net` is
  `#![cfg(not(loom))]` inside Tokio. `Cargo.toml` line 60 declares a self
  dev-dependency
  `wireframe = { path = ".", features = ["test-support", "pool", "testkit"] }`,
  so feature unification enables `pool` in the library build even though the
  lane requests only `advanced-tests`. Runs 35143307834, 35017357350 and
  34895037905 each fail with the same eight errors and exit 101. Impact: the
  models never run.

  An earlier draft claimed a second layer, that the `bdd_pool` target would
  also fail to compile because feature unification selects it. **That claim is
  withdrawn.** `tests/bdd_pool/mod.rs` opens with
  `#![cfg(all(not(loom), feature = "pool"))]`, so under `--cfg loom` it is an
  empty crate that compiles cleanly: selected, and harmless. Whether the lane's
  command needs narrowing at all is therefore an open question that EP-M1's
  reproduction answers, not something this plan asserts. The library failure is
  the only one there is evidence for.

- **Observation**: Almost nothing the models touch is instrumented.
  Evidence: the `cfg(loom)` arm of the `sync` module in
  `src/push/queues/handle.rs` re-exports `std::sync::{Arc, Weak}`, aliasing only
  `Mutex` and `AtomicUsize` to Loom. The models clone the handle into every
  thread they spawn, so every reference-count operation is outside the schedule.
  `PushHandleInner` holds `tokio::sync::mpsc::Sender` for all three channels,
  and the models construct `tokio::sync::mpsc::channel` inside `loom::model`.
  `try_push` is a synchronous path over `tokio::sync::mpsc::try_send`. Impact:
  the `QueueFull` assertions in `concurrent_queue_full_errors_are_reported`
  exercise Tokio's channel under three threads Loom believes it controls. The
  Loom-visible surface of the whole subject is one `AtomicUsize` and one
  `Mutex`.

- **Observation**: Those two instrumented fields are on a path no model
  reaches, and one assertion is therefore unfalsifiable. Evidence: `dlq_drops`
  and `dlq_last_log` are written only inside `route_to_dlq`, in the branch
  taken when the DLQ channel's `try_send` returns `Full` or `Closed`.
  `concurrent_drops_reset_dlq_counter` builds the DLQ with capacity 4 and drops
  two frames, so both sends succeed and the branch is not entered. The reset
  the assertion names, `dlq_drops.store(0, ...)` in `log_dlq_drop`, is
  unreachable from every model in the file. Impact:
  `assert_eq!(probe.dlq_drop_count(), 0, "counter should reset after
  reaching the logging threshold")`
  passes because the counter was never incremented, not because it reset. No
  interleaving Loom can choose makes it fail. Repairing only the compile
  failure would turn the lane green on this.

## Decision log

- **D-1 (OPEN, blocks EP-M2 onward)**: what happens to the message channel
  under the model. `tokio::sync::mpsc` has no Loom equivalent, and Loom cannot
  schedule it. Two routes, which verify different things:
  - **(a) Substitute.** Put the channel behind a small trait and use a
    Loom-visible channel under `cfg(loom)`. The queue's own logic becomes
    model-checked, and the models stop exercising the channel production
    actually uses. It is the larger change and it is what the model file's
    own docstring currently claims to do.
  - **(b) Narrow.** Accept that the channel is out of scope, delete the
    assertions that depend on it, and state in the guide that these models
    check the DLQ counter and the log mutex and nothing else.
  Recommendation: **(b)**. It is smaller, it is honest, and it does not create
  a second implementation of the queue that exists only to be tested, which
  would leave the tested object and the shipped object different in the place
  that matters most. Route (a) should be reconsidered only if someone wants the
  queue's ordering properties model-checked, which is a larger goal than
  repairing this lane.

  **D-1 fixes the acceptance scope of V-4 and EP-M4, and they must not be read
  independently of it.** Route (b) puts channel behaviour out of scope, and
  `concurrent_queue_full_errors_are_reported` is three assertions on
  `PushError::QueueFull`, every one of them a channel assertion. Under route
  (b) that model cannot survive into the acceptance set: it is removed, or
  replaced by a model whose subject is a Loom-visible primitive. Under route
  (a) it stays, and the channel implementation joins the modelled surface.
  Wherever a later obligation says "every model" or "every assertion", it means
  the set route (b) leaves behind, or the set route (a) creates, and never the
  four models as they stand today. Deciding D-1 is therefore a precondition of
  writing V-4's mutation table, not merely of EP-M3.

  Status: referred to the user. Date/Author: 2026-09-17,
  jm-complete-chutoro-whitaker-2.

- **D-2**: this pull request carries the plan only, no source change.
  Rationale: D-1 decides what milestones two to four build, and writing them
  first would mean writing them twice. Date/Author: 2026-09-17, on the lead's
  instruction.

- **D-3**: the nightly cost of a known-red lane is not addressed here.
  Rationale: wireframe #689 moves this lane to a GitHub-hosted runner, where
  the minutes are free for a public repository, so the cost ends when that
  merges. Proposing to silence or suspend the lane would trade a visible
  problem for an invisible one. Date/Author: 2026-09-17.

- **D-4**: V-2 is discharged by a source-parsing contract using `syn`, not by a
  runtime test and not by a `trybuild` compile-fail harness. Rationale: the
  property is about `#[cfg(...)]` attributes, which the compiler consumes, so a
  test compiled into the crate cannot see the modules the configuration
  excluded; `trybuild` builds test files against the crate and cannot rebuild
  the crate under `--cfg loom`, which is where the defect lives. Reading the
  sources is the only method that observes both arms of every predicate. Cost:
  one new direct dev-dependency, recorded against EP-M2's conformance check and
  adding no `Cargo.lock` entry. Date/Author: 2026-09-18,
  jm-complete-chutoro-whitaker-2, on review.

## Outcomes & retrospective

To be completed at each milestone. Nothing to record yet beyond the diagnosis,
which is summarized above rather than referenced, so that this document stays
self-contained.

## Conformance basis

**Correction, 2026-09-22.** An earlier version of this section said there was
no design document for this lane and that the plan's only upstream artefacts
were the issue and the code. That was wrong, and it was wrong because the
search was for an architecture decision record and a Terms of Reference rather
than for the subject. Two existing documents govern what Loom is for here:

- `docs/multi-layered-testing-strategy.md` §4.2, "Concurrency Fuzzing with
  `loom`". It names the **target area** as the connection actor write loop, the
  `select!(biased; ...)` logic, sketches a model spawning concurrent
  high-priority and low-priority producers against a `MockConnection`, and sets
  the measurable objective: all permutations for two to three concurrent
  producers, with no data race and no deadlock.
- `docs/formal-verification-methods-in-wireframe.md`, the "Keep Loom" section
  and its opening survey. It assigns Loom "real synchronization interleavings
  over concrete concurrent code", assigns abstract protocol-state interleavings
  to Stateright, and describes the current models as exploring `PushQueues`
  interleavings and checking "dead-letter queue accounting and queue-full
  behaviour under concurrent producers".

Two things follow, and neither is this plan's to decide.

**The design's target is not what the lane models.** §4.2 names the write loop's
`select!` arm ordering. The models in `tests/advanced/concurrency_loom.rs`
model the push queues instead. The `MockConnection` its example uses does not
exist in the tree. So the lane is not a partial implementation of that design;
it is a different, smaller thing that grew beside it.

**The design asserts coverage that finding two shows is absent.** Both
documents state that the models check queue-full behaviour. They do not: the
`QueueFull` assertions run through a `tokio::sync::mpsc` channel that Loom
cannot schedule, so no interleaving Loom chooses affects them. D-1's route (b)
therefore removes an assertion two documents claim exists, which makes route
(b) a documentation change as well as a test change, and the guide statement in
EP-M3 has to say which document it corrects.

That conflict is referred alongside D-1 rather than resolved here. This plan
repairs a lane that does not run; deciding whether the lane should instead be
rebuilt against §4.2's write-loop target is a larger question with a design
document behind it, and answering it inside a repair would be deciding it by
implication.

- Issue #683, "Fix pool-client test-helper cfg leakage blocking daily Loom
  execution", opened 2026-09-08, which records finding one. Findings two and
  three are new here and should be added to that issue or to this plan's
  successor, not left only in a review comment.
- The models' own stated intent, in the docstring of
  `tests/advanced/concurrency_loom.rs`: "loom explores interleavings to ensure
  DLQ accounting and queue-full errors remain deterministic under concurrent
  producers." Finding two shows the shipped configuration does not meet that
  claim, so either the claim or the configuration must move. D-1 is exactly
  that choice.

Traceability, such as it is:

```plaintext
#683 -> EP-M2 -> lane compiles and models execute
finding two -> D-1 -> EP-M3 -> docs/developers-guide.md scheduling statement
finding three -> EP-M4 -> every assertion mutation-proved
```

## Verification plan

The subject of this plan is a verification mechanism, so the obligations are
mostly about the mechanism rather than about the crate's behaviour.

**Obligation V-1: the Loom configuration compiles.** Method: the build itself,
run as a repository target. Rationale: there is no smaller check; a compile
failure is the current symptom. Domain: the library and the `concurrency_loom`
target under `--cfg loom`. Artefact: a `make` target added in EP-M2. Evidence:
the target succeeds where it currently fails with eight errors. Non-vacuity:
restore the feature-only predicate on `pool_client` and the build must fail
again with the same error classes.

**Obligation V-2: no module reachable under `cfg(loom)` depends on a
`cfg(not(loom))` module.** Method: a source-level checker, not a runtime
assertion about the crate it is linked into. Rationale: the defect class is a
predicate admitting a configuration its dependencies exclude. A compile check
catches today's instance; a contract catches the next one, and this repository
has already had one.

An earlier draft said "a parameterized contract test reading the predicates",
and that artefact cannot exist. `#[cfg(...)]` is resolved by the compiler, so a
test compiled into the crate sees only the configuration it was itself built
under: the excluded modules are absent, their attributes are gone, and there is
nothing left to read. A fixed parameter list is worse still, because the second
stated mutation adds a *new* module and a fixed list cannot name it. The
artefact must read the sources as text and decide for itself what is there.

Artefact: `tests/loom_configuration_contract.rs`, an ordinary integration test
whose subject is the repository's own source files rather than the linked
crate. Discovery mechanism: parse `src/lib.rs` with `syn` into an item tree and
walk it **recursively**. For every `syn::Item::Mod` found, inline or declared,
take its `cfg` predicate from its attributes; for a declared module, resolve
the name to its file relative to the parent module's directory (both the
`name.rs` and `name/mod.rs` spellings, and a `#[path]` attribute where one is
present), parse that file, and continue the walk inside it. Collect every `use`
path rooted at `crate::` together with the predicate of the `use` item itself.

**`cfg_attr` is expanded, not ignored.** `#[cfg_attr(pred, attrs...)]` generates
`attrs` only where `pred` holds, so
`#[cfg_attr(loom, cfg(not(loom)))] mod helper;` is removed under `loom` while
carrying no plain `cfg` a naive reader would see. For each assignment in the
universe below, the checker expands every `cfg_attr` on a reachable `mod` or
`use` item recursively (a generated attribute may itself be a `cfg_attr`), then
derives predicates and resolves `#[path]` from the attributes that result.
Where the expansion generates a `cfg`, the item's predicate for that assignment
is the conjunction of the generated ones; where it generates a `path`,
resolution uses it for that assignment. If an implementation does not support
expansion, it must instead refuse, naming the site, any reachable `mod` or
`use` item carrying a `cfg_attr` that could generate `cfg` or `path`; it must
never read such an item as unconditional. Today no module or `use` item in the
crate carries `cfg_attr`, so this is a rule for the checker, not a change to
the tree. Nothing is enumerated by hand at any point, so a module added
tomorrow, at any depth, is in the domain the moment it is written.

**Predicates are inherited.** A module's *effective* predicate is the
conjunction of its own `cfg` predicate with the effective predicate of the
module enclosing it, down from the crate root, whose effective predicate is
`true`. An item is compiled only when every enclosing gate holds, so a local
predicate alone is the wrong object on both sides of an edge:

- **the importer** of a `use` item is the conjunction of the enclosing module's
  effective predicate with the `use` item's own predicate. An import with no
  local attribute inside a module gated `#[cfg(loom)]` is reachable only under
  `cfg(loom)`, and must be judged so;
- **the imported** side is the effective predicate of the module the path
  resolves to, conjoined along every segment of that path, so `crate::a::b` is
  gated by `a`'s predicate as well as `b`'s. Moving a `not(loom)` gate from a
  module up to its parent must not make the module look ungated.

**The check is implication, not two separate satisfiability questions.** An
earlier draft asked whether the importing module's predicate is satisfiable
under `--cfg loom` and the imported module's is not. That is too weak, and the
gap is not exotic: two predicates can each be satisfiable under `--cfg loom`
while a configuration exists that enables the importer and not the imported.
`#[cfg(feature = "a")]` importing `#[cfg(feature = "b")]` is the shape, with
independent features; the rule passes and `--cfg loom --features a` does not
compile.

The rule is therefore: for every import edge, **every** configuration in the
universe that satisfies the importing module's predicate must also satisfy the
imported module's. The contract fails naming both predicates and a witness
configuration that satisfies the first and not the second, because a
counter-example a reader can pass to Cargo is worth more than a statement that
one exists.

The universe is finite and is stated here rather than assumed, so that two
implementations of the checker evaluate the same configurations and report the
same witnesses.

- **Target.** One target, the lane's host target and no other. The lane runs
  `cargo test` with no `--target` on `ubuntu-latest`, which is
  `x86_64-unknown-linux-gnu`. Every target atom is therefore a constant, not a
  variable: `target_os = "linux"`, `target_family = "unix"`, `unix` true,
  `windows` false, `target_arch = "x86_64"`, `target_pointer_width = "64"`,
  `target_env = "gnu"`, `target_vendor = "unknown"`,
  `target_endian = "little"`. If the lane ever gains a target matrix, the
  universe gains the matrix's rows, and this list changes in the same pull
  request.
- **`loom`.** A free boolean, the `--cfg` flag the lane sets. It is free rather
  than fixed to true because the edge rule is about the configurations the
  predicates admit, and today's witness is stated in terms of it.
- **Features.** One free boolean per key of `Cargo.toml`'s `[features]` table,
  read from the file, never listed by hand: today `default`, `metrics`, `pool`,
  `serializer-bincode`, `serializer-serde`, `advanced-tests`, `examples`,
  `test-support` and `testkit`. An assignment is admitted only if it is closed
  under the table's feature-to-feature implications (`default` enabling
  `metrics` and `serializer-bincode`); `dep:` entries name optional
  dependencies, not features, and add no variable. Without the closure, a
  witness could name a configuration Cargo cannot produce.
- **Profile atoms.** `debug_assertions` is true and `test` is a free boolean:
  the lane builds the dev profile, and the library is compiled both with
  `cfg(test)` for its unit tests and without it for the integration targets.
  `doc`, `docsrs` and `miri` are false.
- **Anything else is refused.** A `cfg` atom not in this list, such as
  `feature = "x"` for an `x` the table does not declare, `panic = "abort"`, or a
  `--cfg` name other than `loom`, fails the contract naming the atom and its
  site. The universe cannot then widen silently because someone wrote a
  predicate the checker does not understand.

Enumerating the assignments is exponential in the feature count, so the check
evaluates the two predicates as boolean formulae over the free variables above
and asks whether `importer && !imported` is satisfiable, which is the same
question without the enumeration. With nine features, `loom` and `test`, the
universe has at most 2^11 assignments before the closure, so a brute-force
evaluation over the powerset is also acceptable; EP-M2 picks one and records
why.

Today's instance is exactly one such edge, and it is an implication failure
rather than a satisfiability one: `test_helpers::pool_client` is gated
`#[cfg(feature = "pool")]` and imports `crate::client`, gated
`#[cfg(not(loom))]`. The witness is `--cfg loom --features pool`, which
satisfies the importer and not the imported.

`syn` is not currently a direct dev-dependency, though version 2.0.117 is
already resolved in `Cargo.lock` through the proc-macro crates, so this adds a
direct edge without adding a tree entry. EP-M2's conformance check is amended
below to record that addition rather than to forbid it. A `trybuild`
compile-fail harness was considered and rejected: `trybuild` builds test files
against the crate and gives no way to rebuild the crate itself under
`--cfg loom`, which is the configuration the defect lives in.

Domain: every module reachable from `src/lib.rs`, at any depth, with its
effective predicate, paired with the `crate::`-rooted imports it contains.
Evidence: passes on the repaired tree, and the run reports the number of module
declarations and import edges it examined, so a checker that silently found
nothing is distinguishable from one that found nothing wrong. Non-vacuity: five
mutations must be rejected, and one narrowness case must pass. Remove
`not(loom)` from the repaired `pool_client` predicate, and the contract fails.
Add a new `cfg(not(loom))` module and import it from an ungated one, and the
contract fails, which is the mutation that proves discovery is dynamic. Point
the resolver at a fixture directory holding a constructed bad pair, and the
contract fails there too: a contract run only over correct files discriminates
nothing, and the real tree will be correct once EP-M2 lands. **Nested:** in a
fixture, gate a parent module `#[cfg(not(loom))]` and leave its child ungated,
then import the child by its full path from an ungated module; the contract
must fail, naming the parent's predicate as the imported side. A checker
reading local predicates only sees an ungated child and passes, which is the
defect inheritance exists to prevent. The narrowness case is the mirror image:
an import with no local attribute, placed inside a module gated
`#[cfg(not(loom))]`, of a module also gated `#[cfg(not(loom))]`, must pass,
because the importer's inherited predicate implies the imported one. A checker
without inheritance flags it. **`cfg_attr`:** in a fixture, gate a module
`#[cfg_attr(loom, cfg(not(loom)))]` and import it from a module gated
`#[cfg(loom)]`; the contract must fail. A checker that ignores `cfg_attr` reads
the module as ungated and passes.

**Obligation V-3: each model is scheduled over the primitives the guide says it
is scheduled over.** Method: a documented statement plus a review checkpoint,
not an automated test. Rationale: this is a claim about what Loom instruments,
which is a property of the alias module and of Loom's API rather than of any
value the program computes. An automated assertion here would be a restatement,
which the skill's standard rejects. The honest artefact is a written statement
that a reader can check against the alias module in one screen. Domain: the
`cfg(loom)` arm of `sync` in `src/push/queues/handle.rs`, and the types held by
`PushHandleInner`. Artefact: a section of `docs/developers-guide.md`, added in
EP-M3. Evidence: the section names every field of `PushHandleInner` and says,
for each, whether Loom schedules it. Non-vacuity: not applicable; recorded as a
deliberate exception under the skill's provision for an impractical negative
control, with the review checkpoint as the compensating control.

**Obligation V-4: every assertion in the model file can fail.** Method:
mutation, one per assertion, each recorded. Rationale: finding three is
precisely an assertion that cannot fail, so a count of passing models is not
evidence. This is the obligation the whole plan exists to discharge.

Domain: **the acceptance set D-1 leaves behind**, plus any model added in
EP-M4, and not "all four models" as a standing phrase. The table below cannot
be filled in until D-1 is answered, and the milestone that fills it says so.

Under route (a) every model is in scope and the `QueueFull` assertions are
proved like any other.

Under route (b) the retained set is **named here rather than left as a
subtraction**, because "what is left after removing the channel assertions" is
not a list anyone can check a milestone against:

- `concurrent_drops_reset_dlq_counter`, retained in full. Its subject is the
  DLQ counter and the log mutex, which are the two fields Loom actually
  instruments.
- DLQ counter coverage for **both** `PushPriority::High` and
  `PushPriority::Low`. Priority decides which queue a frame is taken from and
  the accounting must hold for either, so one priority is half a rule.
- The zero-count assertions for an **absent** DLQ and for an **idle** one. Both
  are DLQ accounting rather than channel behaviour: they say the counter stays
  at zero when there is nothing to drop, which is the narrowness half of the
  increment rule and is exactly what a broken increment would also satisfy if
  only the positive case were asserted.
- `concurrent_queue_full_errors_are_reported` is **excluded**, because its
  three assertions are on `PushError::QueueFull` reaching a producer through a
  Tokio channel that Loom does not schedule.

Route (b) does **not** owe a new deterministic queue-full test.
`tests/push.rs::try_push_respects_policy` already asserts
`PushError::QueueFull` under the deterministic runtime, which is the right
place for it: the property is about the policy, not about an interleaving.
EP-M3's guide statement cites that test as the replacement coverage, so the
guide records where the property moved rather than implying it was dropped.

Every retained assertion above appears as a row in V-4's mutation table with
its own mutation, including the two zero-count cases, which are the ones a
table written from the positive assertions alone would omit.

Artefact: `tests/advanced/concurrency_loom.rs` and a table in this plan.
Evidence: for each assertion, a named mutation of the subject, and the model
that must fail because of it.

The DLQ counter needs two mutations, not one, and the obvious pair does not
work as an earlier draft claimed. `route_to_dlq` calls
`dlq_drops.fetch_add(1, ...)` and passes the incremented value to
`log_dlq_drop`, which stores `0` back into the counter once the value reaches
`dlq_log_every_n`. Replacing `fetch_add` with a no-op makes the local `dropped`
stay at `1` on every drop, so the threshold is never reached, the reset never
runs, and the counter finishes at `0`, which is exactly what
`assert_eq!(probe.dlq_drop_count(), 0, ...)` already expects. The mutation is
invisible to the assertion it was supposed to discriminate, because the
suppressed increment and the suppressed reset cancel. Reset and increment are
therefore separate obligations and need separately observed evidence:

- **Increment.** Assert the counter *before* the reset can fire: drive
  `route_to_dlq` into its error branch exactly once with `dlq_log_every_n`
  greater than one, and assert `dlq_drop_count() == 1`. Mutation: `fetch_add`
  to a no-op. That assertion then reads `0` and the model fails.
- **Reset.** With the increment intact, drive the counter to the threshold and
  assert it returns to `0`. Mutation: remove the `dlq_drops.store(0, ...)` in
  `log_dlq_drop`, or change the threshold comparison so the reset branch is not
  taken. The counter then reads the accumulated value and the model fails.

Every other assertion carried into the acceptance set gets the same treatment:
a named mutation, the model that must fail, and the observed value that changes.

Non-vacuity: the mutations must fail for the stated reason and not merely fail.
A mutation that makes the crate stop compiling proves nothing, so each mutation
must leave a compiling program that behaves differently. And a mutation whose
effect is cancelled downstream before any assertion observes it proves nothing
either, which is the trap the DLQ pair above exists to avoid; each recorded
mutation must name the value the model reads and how that value moves.

**Axioms.** That Loom 0.7.2 schedules exactly the primitives it supplies and
the threads it spawns; that Tokio's channels and `leaky_bucket` use primitives
Loom does not supply; that `--cfg loom` reaches the library and the selected
test targets. The first two are third-party internals and are assumed rather
than verified, per the standard. The third is observable in the build output
and is checked by V-1.

## Plan of work

**Stage A, understand and propose.** Complete: the diagnosis and this plan. No
code changes.

**Stage B, red first.** In EP-M1 the "red test" is the recorded reproduction of
the build failure, which is the existing behaviour and needs no new code. In
EP-M2 the contract of V-2 is written before the predicates are repaired, and
must fail on the current tree for the right reason.

**Stage C, implementation with verification.** EP-M2 repairs the predicates
alongside the contract, and the target selection only if EP-M1 found it broken
(R-3). EP-M3 widens the aliases across all three sites that use them and writes
the guide statement together, so the statement is derived from the code rather
than from intent. EP-M4 rewrites the models and proves each assertion by
mutation in the same change.

**Stage D, refactor and wider validation.** After EP-M4, re-read the guide
statement against the alias module, run the repository's full gates, and update
the issue with what the models now check.

Each stage ends with its validation. Do not proceed past a failing stage.

## Milestones and plateaus

**EP-M1: reproduce and bound.** Outcome: the build failure reproduced locally
on the pinned toolchain, with the command and output kept as regression
evidence, and confirmation that the dependency graph has not introduced a
different failure since 2026-09-08. It also settles R-3: with the library
failure repaired locally on a scratch copy, run the lane's unnarrowed command
and record whether any test target still fails to build under `--cfg loom`.
Requirements and gaps: establishes the baseline for #683. Acceptance evidence:
the recorded command, its eight errors, and their error codes matching the
three scheduled runs; and R-3's answer, either the recorded target-selection
failure or the recorded clean build that shows there is none. Conformance
check: no source change, so no interface, dependency, trust boundary or format
moves. Recovery: nothing to revert. Remaining gaps: everything else.
Compatibility decision: none required. Note: this is the only milestone that
does not depend on D-1.

**EP-M2: the configuration boundary, and models that execute.** Outcome:
`pool_client` and its re-exports gated on the configuration they need as well
as the feature; the V-2 contract in place and mutation-proved; a `make` target
that runs the lane's command, which is V-1's artefact. **Conditional on EP-M1
(R-3):** only if EP-M1 recorded a separate target-selection failure does the
`make` target also narrow the build to the Loom target, so the workflow's
one-line change becomes a call rather than a script. Without that recorded
failure, the target runs the lane's command unchanged and no workflow change is
owed. The models execute, and whatever they report is recorded honestly.
Requirements and gaps: discharges #683's stated defect. Acceptance evidence:
V-1 and V-2 above. Conformance check: public API under `cfg(not(loom))`
unchanged; the workflow file untouched, per `Constraints` and R-3; one new
direct dev-dependency, `syn`, for V-2's checker, which adds no entry to
`Cargo.lock` because the proc-macro crates already resolve it, and which is
dev-only so no consumer of the published crate sees it. Recovery: the change is
confined to predicates, a contract and a Makefile target; reverting the commit
restores the previous state exactly. Remaining gaps: findings two and three
untouched. **The lane may legitimately still be red at the end of this
milestone**, if the models fail once they run. That is a valid plateau and must
not be papered over. Compatibility decision: none required; the affected
surface is test-only.

**EP-M3: the seam, and the statement.** Outcome: D-1 implemented; the
synchronization aliases widened to cover `Arc` and `Weak` **at every site that
constructs or consumes them**, not only in `handle.rs`; a section of
`docs/developers-guide.md` naming every field of `PushHandleInner` and whether
Loom schedules it.

The seam is wider than one file, and an earlier draft of this plan said
otherwise. `src/push/queues/mod.rs` imports `std::sync::Arc` and calls
`Arc::new(inner)` to build the value `PushHandle::from_arc` receives, and
`src/session.rs` imports `std::sync::{Arc, Weak}` and stores
`Weak<PushHandleInner<F>>` in its `DashMap`, upgrading each entry back to an
`Arc`. Swapping the alias in `handle.rs` alone therefore does not compile under
`cfg(loom)`: the constructor and the registry would still hand it the standard
types. Those three sites move together or not at all.

Requirements and gaps: discharges finding two, and makes finding two impossible
to lose. Acceptance evidence: V-3, plus the models still executing, plus a
clean build under `--cfg loom` covering all three sites. Conformance check: if
D-1 resolved to route (a), a new dependency or a new trait in a public module
may be required; either is a tolerance breach and must have been approved as
part of D-1 rather than decided here. `SessionRegistry` is public, so confirm
its signature is unchanged under `cfg(not(loom))`. Recovery: three files and an
additive guide section; revert the commit. Remaining gaps: the models still do
not reach the instrumented path. Compatibility decision: none under
`cfg(not(loom))`.

**EP-M4: models that can fail.** Outcome: the DLQ models rewritten to fill or
close the dead-letter channel so `route_to_dlq` takes its error branch, and
split so the increment is observed before the reset can hide it; every
assertion **in the acceptance set D-1 fixed** mutation-proved, with V-4's table
filled in; the guide statement re-read against the code and updated.

Under route (b) that set excludes `concurrent_queue_full_errors_are_reported`,
whose three `QueueFull` assertions are channel assertions: the model is removed
or replaced here, and the guide sentence added in EP-M3 says which models
remain and what they check. Under route (a) it is retained and proved with the
rest. The milestone cannot start before D-1 is answered, because the answer
decides what it is proving. Requirements and gaps: discharges finding three.
Acceptance evidence: V-4, with the mutation table filled in. Conformance check:
tests and documentation only. Recovery: test-only; revert the commit. Remaining
gaps: **the lane's command, which this plan may not edit.** Only if EP-M1
recorded a separate target-selection failure (R-3), the one-line change to
`.github/workflows/advanced-tests.yml` is a dependency of completion, not a
hand-off: EP-M4 is not complete until that line is landed, by whoever owns the
file. Tracking it elsewhere does not make it happen, and a plan that declares
itself finished while the scheduled lane still runs the wrong command has not
repaired the lane. If a model finds a defect in the push queue, that is R-2 and
becomes separate work. Compatibility decision: none required.

Until EP-M4 is complete, `docs/developers-guide.md` describes this lane as a
compile check for the `cfg(loom)` configuration, and not as verification of the
push queue. That sentence lands in EP-M3 and is removed in EP-M4.
