# Collapse nested application-owned sharing beneath `PreparedApp` (#648)

This ExecPlan is a living document. Keep the Progress, Surprises, and Decision
Log sections current as work proceeds so another agent can continue after a
context compaction.

## Purpose

`Arc<PreparedApp>` is the deliberate shared root for every connection task. A
child value that cannot outlive that root does not need its own reference
count. This change removes the avoidable inner `Arc` layers for application
owned values while keeping the layers whose independent lifetime is real:

- connection setup/teardown callbacks become `Box<ConnectionSetup<C>>` /
  `Box<ConnectionTeardown<C>>`, borrowed per connection;
- the serializer becomes `Box<S>`, borrowed as `&S`;
- the message assembler becomes `Option<Box<dyn MessageAssembler>>`, borrowed
  as `Option<&dyn MessageAssembler>`;
- handler `Arc`s move out of the consumed builder into prepared services
  without a clone;
- the protocol keeps `Arc` because #644 gives each connection exactly one
  strong reference through `ProtocolHooks`.

## Base and branch

- Branch:
  `issue-648-collapse-nested-application-owned-sharing-beneath-preparedapp`
- Base:
  `issue-879-run-the-remaining-wireframeprotocol-hooks-on-server-connections-including-on-eof`
  (itself based on #644).
- Tracking:
  `origin/issue-648-collapse-nested-application-owned-sharing-beneath-preparedapp`.

## Progress

- [x] Branch renamed, reset onto the #879 base, pushed, upstream fixed.
- [x] Lody session titled.
- [x] Characterization tests (`tests/prepared_app/ownership_support.rs`,
      `ownership.rs`) — four tests cover live-connection survival, the final
      root drop, an outliving protocol handle, a handler allocation shared by
      two routes, and two connections interleaving one assembler/serializer.
- [x] Boxed callbacks + boxed serializer + borrowed contexts.
- [x] Owned handler move into prepared services.
- [x] Boxed assembler + borrowed accessor (#598 disposition).
- [x] Trybuild fixtures: non-`Clone`/`!Unpin` serializer passes the relaxed
      connection bounds; assembler accessors pin the borrowed return type.
- [x] Benchmark configured-startup workload and preparation bracket. Capture
      `dcce92e5` (restacked as `62c28fc1`), 2026-11-20, toolchain
      `nightly-2026-11-17`; recorded in `docs/runtime-ownership-baselines.md`.
- [x] Docs: developers-guide ownership inventory, ADR 012 traceability,
      regression baselines, changelog, migration guide, users guide.
- [x] CodeRabbit review round 1: four findings — the configured benchmark
      dispatch, an unbounded wait in the root-survival test, and two
      documentation corrections. All fixed and re-gated.
- [x] Restacked the seven #648 commits onto the rewritten #879 parent
      (`fb3c9fe1`), resolving conflicts against the new inbound-handler split
      and the moved prepared-application modules. All gates green again.
- [x] Draft PR [#936](https://github.com/leynos/wireframe/pull/936), stacked
      on #879 (#925) in GitHub stack #926 beneath #644 (#923).
- [x] Follow-up review round: protocol setup now dispatches through the hooks
      value that owns the connection's single reference, measured while setup
      is paused (`tests/prepared_protocol_setup_ownership.rs`); two overlapping
      connections prove distinct lifecycle state and one teardown each; the
      four CodeScene duplications and the large method are refactored away.
- [x] Controlled before-and-after allocation capture on the parent tree
      (`3376f734` with this branch's harness) and on `cfc7d2a7`. The configured
      scopes lose exactly four allocations per preparation, the plain prepared
      root one per iteration, and the actor and pool controls do not move.
      Recorded in `docs/runtime-ownership-baselines.md`.
- [x] Restacked onto the rewritten #879 parent (`3376f734`), whose new
      context-sharing commit changed the setup boundary this branch reworks.
      Setup still dispatches through the one hooks owner, and the setup-mutated
      context the parent now returns is the one installed in the pipeline.
      The controlled pair was re-run on the new parent and head: the parent
      still records 116/500/696/424 and the branch 100/483/680/420.
- [x] Restacked again onto the parent's next rewrite (`70df9586`), which
      replaced the prepared actor's copied context with a named constructor.
      Setup still dispatches through the one hooks owner, and the new
      constructor receives only the connection identity it logs with. The
      controlled pair's revisions are unchanged because the parent's new
      commits do not touch preparation, the builder, or the harness.
- [ ] Push the follow-up round and renew hosted checks and review.

## Decision log

- **Protocol stays `Arc`.** #644 already establishes one strong reference per
  connection via `ProtocolDispatch`. This ticket does not redesign it.
- **Assembler accessor becomes `Option<&dyn MessageAssembler>`** in the pending
  0.4.0 breaking release (the #598 disposition). No supported use case clones
  the assembler out of the app; the only legitimate out-of-app ownership is the
  caller's own value before `with_message_assembler` consumes it.
- **Handler `Arc` is retained as a type.** The public
  `Handler`/`ResponseHandler` aliases are `Arc<dyn Fn>`, and a caller may
  legitimately retain a clone. Preparation *moves* the existing `Arc` out of
  the consumed builder rather than cloning it.
- **`AppDataStore` is gone.** ADR 017 removed it before 0.4.0, so the
  store-semantics criterion needs no code change; record that in the guide.
- **Setup dispatches through `ProtocolHooks`, not a second owned clone.** The
  connection's hooks value already owns its one strong reference and already
  runs setup at most once, so the processing context carries no protocol field.
  Setup borrows the hooks, then the same value moves into the frame pipeline.

## Surprises

- Installing a message assembler changes what the envelope payload means: every
  inbound payload is parsed as a fragment header, so a test that wants its
  handler to run must send `first_frame_payload(.., is_last = true, None)`
  rather than a bare body. A bare body is rejected before dispatch and the
  connection ends without the handler ever running.
- `trybuild` fixtures inherit the full `--all-features` dependency build the
  first time they run. Under a congested compile-admission pool that is a 30–40
  minute gate; reruns are cheap. Blessing stderr with `TRYBUILD=overwrite` on
  the `compile_tests` filter is enough; the whole suite then passes.
- The borrowed-serializer compile-fail fixture's stderr needed re-blessing
  because its module doc grew by two lines, shifting every reported line
  number. The diagnostic itself was unchanged.
- The configured startup benchmark's serving phase ignored the base path, so
  its `per_connection_factory` rows measured one shared preparation under a
  label claiming one factory call per connection. The dispatch now follows the
  path, and the affected rows were re-captured; the earlier per-connection
  timings are recorded as incomparable rather than reused.
- The `module-max-lines` rule counts raw file lines, including test modules.
  `tests/prepared_app/ownership.rs` had grown to 496 lines once the probes,
  handler, protocol, and interleaving proofs were all in one file, so it was
  split into `ownership.rs` (root survival and single-owner proofs) and
  `ownership_interleaved.rs` (two-connection interleaving). The probe
  identities moved to `ownership_support.rs` so both modules name the same
  values.
- The benchmark capture ran at a load average of 24–30 on 24 threads, so its
  timing rows are only meaningful within that capture; a reader comparing
  across captures should treat them as inconclusive.
- The first allocation conclusion was wrong, and only a controlled pair shows
  why. Cloning a shared `Arc` allocates nothing, so the per-connection clones
  this change removed were free — but constructing each `Arc` *did* allocate,
  and the configured workload installs four zero-sized values whose `Arc::new`
  allocated a control block that `Box::new` does not. Running the same harness
  on the parent tree and on this branch, sequentially on one host, shows the
  configured scopes losing exactly four allocations per preparation while the
  actor, push, and pool controls stay identical. Those control rows are the
  evidence that the movement belongs to preparation; the earlier account of
  cross-capture drift was an inference with no control behind it, so it has
  been replaced by the controlled result in
  `docs/runtime-ownership-baselines.md`.
- The #879 parent branch was rebased after this branch was cut from it. The
  old branch point (`e7f7c6c6`) is no longer an ancestor of the parent tip
  (`fb3c9fe1`), and the merge base had fallen back to `a4064d27`, so a review
  of the branch appeared to delete 212 files of newer parent work. The fix was
  `git rebase --onto` from the old branch point, not a merge, because only the
  seven #648 commits belong to this branch. Most of the replay conflicts came
  from the parent's own refactors: `inbound_handler/decode.rs` was absorbed into
  `dispatch.rs`, `prepared_app.rs` became `prepared_app/mod.rs` with the
  accessors in their own module, and `ConnectionLoop` took a new constructor
  and push/session fields. Re-applying the ownership changes to the new
  structures was more reliable than reconciling the three-way hunks.

- The shared test-support modules under `tests/common/` are compiled into
  every test binary that includes them, and `-D warnings` makes each binary see
  any helper it does not call as dead code. Adding a probe helper to the
  recording protocol fixture therefore broke unrelated test targets. The
  setup-boundary probe is a standalone test target instead, so its helpers are
  used by exactly one binary.
- CodeScene's duplication rule compares functions within one module, so the
  borrowed and owned route-folding loops and the two route constructors were
  duplicates even though they expressed two real paths. Sharing the fold and
  cloning the route handle at the borrowed boundary removed the duplication
  without changing which path clones.

## Validation strategy

- `make check-fmt`, `make lint`, `make typecheck`, `make test`,
  `make markdownlint`, `make nixie`, `make spelling` (per AGENTS.md).
- `cargo bench --no-run` for both benchmark binaries.
- `make bench-runtime-ownership` for the before/after figures.
