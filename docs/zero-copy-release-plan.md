# Zero-copy release plan

This plan records the release sequencing, communication, canaries, validation
gates, rollback criteria, and versioning determination for the zero-copy
programme in Epic [#284](https://github.com/leynos/wireframe/issues/284). It
supplies the release-facing content that roadmap item `5.1.1` of the
[zero-copy migration roadmap](zero-copy-frame-and-payload-migration-roadmap.md)
and roadmap item `14.1.1` of the [project roadmap](roadmap.md) ask for.

It is a plan, not a release. Nothing here asserts that the migration is
implemented. As of this document the serializer still returns `Vec<u8>`,
`PayloadBytes` does not exist, and roadmap items `11.1.1` through `13.2.3` are
open. The plan exists so that when those items land, the release has a
checklist rather than a scramble.

## Versioning determination

The historical issue title refers to a v2.0 release. That number is not
supported by the current release policy, and no planning document should repeat
it.

Evidence:

| Fact                                          | Source                                          |
| --------------------------------------------- | ----------------------------------------------- |
| Current crate version is `0.3.0`              | `Cargo.toml`                                    |
| Latest published release is `v0.3.0`          | `gh release list`; remote tags stop at `v0.3.0` |
| The project has not cut `1.0`                 | no `v1.*` tag exists on the remote              |
| A `0.4.0` migration guide is already in tree  | `docs/v0-3-0-to-v0-4-0-migration-guide.md`      |
| Helper removal is `0.N` to `0.N+2` before 1.0 | [ADR 009](adr-009-vec-u8-migration-rollout.md)  |

Under Cargo's semver rules a `0.x` crate may break compatibility in a minor
bump, so the next breaking release is a minor increment, not a major one. Two
readings remain, and the difference matters because it fixes every deprecation
string in the migration guide:

- if the zero-copy break joins the unreleased prepared-application work, the
  release is `0.4.0`;
- if `0.4.0` ships first, the zero-copy break is `0.5.0`.

The rule that does not depend on that choice: compatibility helpers introduced
in release `N` are removed no earlier than `N+2`. A `0.4.0` introduction
therefore implies removal at `0.6.0`; a `0.5.0` introduction implies `0.7.0`.

Roadmap item `14.1.1` owns the final decision, and its prerequisites (`13.1.2`,
`13.2.3`) are unmet, so the item stays open. This plan records the
determination and the rule, not a settled version number.

## Release sequencing

The sequence follows dependency order rather than calendar order. Each step is
a gate: the next step does not start until the previous one's evidence exists.

| Step | Content                                                            | Roadmap items                        | Evidence produced                       |
| ---- | ------------------------------------------------------------------ | ------------------------------------ | --------------------------------------- |
| 1    | Accept or reject the serializer amendment                          | `11.1.2.1`                           | ADR status change, migration text freed |
| 2    | Land internal byte foundations and the actor/codec boundary        | `11.1.1`, `11.1.3`, `11.2.1`         | Pointer and allocation regressions      |
| 3    | Deliver shared serializer output through server and no-hook client | `11.1.2`, `11.1.2.2`                 | End-to-end default and Serde matrix     |
| 4    | Flip the public packet, middleware, hook, and serializer surfaces  | `12.1.1`–`12.1.3`, `12.2.1`–`12.2.3` | Migration examples, updated tests       |
| 5    | Publish the migration guide and breaking-change summary            | `13.1.1`, `13.1.2`                   | `docs/` guide, changelog entry          |
| 6    | Run the benchmark comparison and downstream canaries               | `13.2.1`–`13.2.3`                    | Comparison report against thresholds    |
| 7    | Cut the breaking release                                           | `14.1.1`, `14.1.2`                   | Release notes, tag                      |
| 8    | Review helpers and deferred surfaces                               | `14.2.1`, `14.2.2`                   | Recorded keep-or-remove decisions       |

Step 4 is the first irreversible step for downstream users, so steps 1 to 3
must be complete before it starts. That ordering is what keeps a half-migrated
serializer from shipping inside a public API break.

## Communication

The repository's actual channels, in the order a downstream user meets them:

| Channel         | Artefact                                                                        | Owner step |
| --------------- | ------------------------------------------------------------------------------- | ---------- |
| `CHANGELOG.md`  | Entry under `## Unreleased`, marked breaking, linking the guide                 | 5          |
| Migration guide | New `docs/` page built from the [outline](zero-copy-migration-guide-outline.md) | 5          |
| Release notes   | Tag annotation and GitHub release body summarizing the break                    | 7          |
| User's guide    | Behaviour changes a user of the library will notice                             | 5          |
| Epic issue      | Progress comment linking each artefact as it lands                              | 5, 6, 7    |

The changelog already carries breaking entries under `## Unreleased`, for
example the prepared-application change that points at its migration guide, so
the convention exists and needs no invention.

Two communication rules follow from the ADR status split:

- the guide must not describe an unaccepted serializer amendment as policy;
- the release notes must name the finite helper set and each helper's removal
  version, because that is the single fact downstream users plan around.

## Downstream canaries

Canaries are in-repository examples that already exercise the affected
surfaces. They are cheap to compile and they fail loudly when a public API
moves.

| Canary              | Surface exercised                       | Path                                                  |
| ------------------- | --------------------------------------- | ----------------------------------------------------- |
| Packet enum routing | `Packet`, `PacketParts`, `Envelope`     | `examples/packet_enum.rs`                             |
| Metadata routing    | `Envelope` construction and dispatch    | `examples/metadata_routing.rs`                        |
| Hotline protocol    | Custom `FrameCodec` with a native frame | `examples/hotline_codec.rs`                           |
| MySQL protocol      | Custom `FrameCodec`, protocol-native    | `examples/mysql_codec.rs`                             |
| RESP protocol       | Custom `FrameCodec` with adapter split  | `examples/resp_codec.rs`, `examples/resp_codec_impl/` |
| Client login flow   | `BeforeSendHook`, client send pipeline  | `examples/client_echo_login.rs`                       |

`hotline_codec`, `mysql_codec`, and `resp_codec` cover the protocol-native
requirement from roadmap item `1.2.1` without a synthetic harness: they are
real codecs whose frames store payload bytes directly.

Roadmap item `13.2.3` runs these against the release candidate and requires
that breakage is either fixed or called out with a workaround in the guide.

## Validation gates

No release step is complete on a green unit test run alone. The gates, and what
each one is evidence for:

| Gate                       | Command                            | Evidence                                                                                                          |
| -------------------------- | ---------------------------------- | ----------------------------------------------------------------------------------------------------------------- |
| Formatting                 | `make check-fmt`                   | Style conformance                                                                                                 |
| Lint                       | `make lint`                        | Clippy and dylint, warnings denied                                                                                |
| Typecheck                  | `make typecheck`                   | Workspace type correctness                                                                                        |
| Unit and behavioural tests | `make test`                        | Correctness contracts                                                                                             |
| Feature matrix             | feature-gate scripts in `scripts/` | Optional-feature builds stay sound                                                                                |
| Documentation              | `make markdownlint`, `make nixie`  | Guide and diagrams valid                                                                                          |
| Benchmarks                 | `make bench`, `make bench-codec`   | Threshold comparison against baseline                                                                             |
| Formal tooling             | `make formal-pr`                   | Stateright tests now; `kani` and `verus` are stubs until roadmap `15.3.1` and `15.5.2`, so they prove nothing yet |

Warnings are remediation items, not noise. A gate that cannot run is recorded
as unavailable with its reason; it is never silently skipped and never counted
as a pass.

The benchmark gate is the one that decides whether the migration achieved its
purpose. It compares against the baselines recorded in
[zero-copy benchmark baselines](zero-copy-benchmark-baselines.md) and applies
the thresholds published there.

## Rollback criteria

Rollback here means reverting a release candidate or holding a release, not
rewriting published history. The triggers, in severity order:

| Trigger                                                                                            | Response                                     |
| -------------------------------------------------------------------------------------------------- | -------------------------------------------- |
| Any default-path throughput regression beyond the agreed limit                                     | Hold the release; investigate before cut     |
| The outbound default path still allocates a payload buffer                                         | Hold; the migration's headline claim failed  |
| A read-only stage gains allocations per operation                                                  | Hold; zero-copy claim is false for that path |
| Correctness coverage fails in mutation, wire output, correlation, fragmentation, or hook behaviour | Block; fix before any further validation     |
| A canary breaks without a documented workaround                                                    | Block; fix or publish the workaround         |
| The migration guide contradicts an accepted ADR                                                    | Block; reconcile documentation first         |

Because the change is a public API break, a post-release rollback is a `yank`
plus a follow-up patch release. That is a last resort: the cheaper protection
is holding the release until the benchmark and canary gates pass.

## Owners

No owner is recorded for this programme in the repository, and this plan cannot
invent one. The roles that must be assigned before step 4 are:

| Role                 | Responsibility                                           |
| -------------------- | -------------------------------------------------------- |
| Migration lead       | Owns steps 1 to 4; answers API questions during the flip |
| Performance reviewer | Owns the baseline capture and threshold sign-off         |
| Release owner        | Owns steps 5 to 7, changelog wording, and the tag        |
| Downstream liaison   | Owns canary results and the workaround list              |

Assignment is an open action on Epic #284. Until the roles are assigned, steps
1 to 3 can proceed because they are internal, and step 4 cannot.

## Tentative timeline

The plan is milestone-relative, not calendar-bound, because a date invented at
planning time is a promise nobody measured. The sequence is:

1. steps 1 to 3 in the release cycle that follows acceptance of the serializer
   decision;
2. step 4 in the same cycle, gated on the allocation regressions from step 2;
3. steps 5 and 6 in the cycle before the cut;
4. step 7 at the next breaking mini-release boundary;
5. step 8 in the two cycles after the release, which is the earliest point at
   which helper usage in the wild can be observed.

The only fixed interval in the whole plan is ADR 009's removal rule: helpers
introduced in `0.N` are reviewed at `0.N+2`. Everything else is dependency
gated.

## Blockers

| Blocker                                                     | Blocks                                            | Owner item     |
| ----------------------------------------------------------- | ------------------------------------------------- | -------------- |
| Serializer amendment is proposed, not accepted              | Step 1, and every serializer example in the guide | `11.1.2.1`     |
| No middleware, hook, or outbound-encode benchmark harness   | The headline retention claim cannot be evidenced  | Proposed issue |
| `PayloadBytes` is unimplemented                             | Steps 3 and 4                                     | `11.1.1`       |
| `Envelope`'s bincode derive has no decided wrapper strategy | Step 4                                            | `3.1.1`        |
| Programme roles are unassigned                              | Step 4 onwards                                    | Epic #284      |
| Target release version (`0.4.0` versus `0.5.0`) undecided   | Step 5 wording                                    | `14.1.1`       |

None of these blockers prevents publishing this plan. Each prevents a claim of
readiness, which is the distinction the epic needs to keep.

## References

- [ADR 009: `Vec<u8>` migration rollout](adr-009-vec-u8-migration-rollout.md)
- [ADR 010: transport frame boundary](adr-010-transport-frame-boundary-for-zero-copy.md)
- [Zero-copy migration guide outline](zero-copy-migration-guide-outline.md)
- [Zero-copy benchmark plan](zero-copy-benchmark-plan.md)
- [Zero-copy benchmark baselines](zero-copy-benchmark-baselines.md)
- [Zero-copy frame and payload migration roadmap](zero-copy-frame-and-payload-migration-roadmap.md)
- [Project roadmap](roadmap.md)
