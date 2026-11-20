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
      `dcce92e5`, 2026-11-20, toolchain `nightly-2026-11-17`; recorded in
      `docs/runtime-ownership-baselines.md`.
- [x] Docs: developers-guide ownership inventory, ADR 012 traceability,
      regression baselines, changelog, migration guide, users guide.
- [x] CodeRabbit review round 1: four findings — the configured benchmark
      dispatch, an unbounded wait in the root-survival test, and two
      documentation corrections. All fixed and re-gated.
- [ ] Draft PR.

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
- The benchmark capture ran at a load average of 24–30 on 24 threads, so the
  timing rows are only meaningful within the capture. The allocation counts are
  the trustworthy part, and they show that #648 itself cannot move them: every
  removed layer was an `Arc` clone, and the allocation harness's own control
  records that a shared-`Arc` clone allocates nothing. The pool rows stayed at
  2052/2053 across captures while the actor and startup rows moved by hundreds,
  which attributes that drift to #644/#879 and the toolchain advance rather
  than to this change.

## Validation strategy

- `make check-fmt`, `make lint`, `make typecheck`, `make test`,
  `make markdownlint`, `make nixie`, `make spelling` (per AGENTS.md).
- `cargo bench --no-run` for both benchmark binaries.
- `make bench-runtime-ownership` for the before/after figures.
