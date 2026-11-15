# Architectural decision record (ADR) 016: forbid unsafe code workspace-wide

## Status

Proposed.

First proposed on 2026-10-31 in issue
[#804](https://github.com/leynos/wireframe/issues/804).

## Date

2026-10-31.

## Context and problem statement

The workspace contains no unsafe code. Every library, binary, test, benchmark
and example is written in safe Rust, and the allocation-counting benchmarks
install `stats_alloc`'s `StatsAlloc<System>` wrapper rather than a handwritten
`GlobalAlloc`. The lint configuration had not caught up with that fact:
`unsafe_code` was denied crate by crate, so a new target root could be created
with no prohibition at all, and the question "where is unsafe code in this
repository?" could not be answered from the manifests alone.

Issue #804 originally proposed isolating the counting allocator into an audited
crate so that `forbid` could be applied everywhere else. That allocator no
longer exists: the change that removed the last unsafe code replaced it with a
safe wrapper. What remains is the enforcement question, and it is worth
answering independently of any single unsafe site, because the answer has to
hold for sites that do not exist yet.

## Traceability

Primary code surfaces:

- `Cargo.toml` — `[workspace.lints.rust]`, which every member inherits;
- the header of every Cargo target root, which repeats the prohibition;
- `tests/feature_gate/Cargo.toml` — the standalone package, which cannot
  inherit the workspace table and copies it;
- `tests/workspace_unsafe_boundary.rs` — the textual contract;
- `scripts/check-unsafe-boundary.sh` — the compiler probes;
- `docs/rust-boundary-verification.md` — what the checks do and do not
  establish.

Related decisions:

- [ADR 011: runtime ownership and task-lifetime boundaries](adr-011-runtime-ownership-and-task-lifetime-boundaries.md),
  which owns the benchmarks whose allocation counting once needed a handwritten
  allocator.

## Decision drivers

- A new target root must fail closed: forgetting the prohibition must not
  silently permit unsafe code.
- No `#[allow]` anywhere in the workspace; an expectation must carry a reason
  and be scoped to the code that needs it.
- The prohibition must be demonstrably effective, not merely present in text.
- The published `wireframe` crate's dependency graph must not change.
- The enforcement must not depend on a second toolchain, a second CI job's
  runtime budget, or a new crate.

## Options considered

### Option A: keep the per-crate denial

Each crate keeps whatever lint level it happens to declare. This is the status
quo: it works only while every author remembers, and it gives a root that
forgets a weaker protection than the workspace intends — usually none at all.

### Option B: the workspace lint table alone

Set `unsafe_code = "forbid"` in `[workspace.lints]` and stop there. Cargo
applies a workspace lint table only to packages that opt in with
`[lints] workspace = true`, and the standalone `tests/feature_gate` package
declares its own `[workspace]`, so it cannot inherit the table at all. A new
package, or a package that drops the opt-in, silently loses the prohibition.

### Option C: the workspace table, a per-root attribute, and enforcement checks (preferred)

Set `unsafe_code = "forbid"` in `[workspace.lints]`, repeat
`#![forbid(unsafe_code)]` in the header of every target root, hand-copy the
same level into the standalone package, and check all of it with a textual
contract test plus compiler probes.

| Topic                                 | Option A: per-crate | Option B: table only | Option C: table and roots |
| ------------------------------------- | ------------------- | -------------------- | ------------------------- |
| New root fails closed                 | No                  | No                   | Yes                       |
| Standalone package covered            | Only by hand        | No                   | Yes                       |
| Prohibition present in the source     | Varies              | No                   | Yes, per root             |
| Demonstrably enforced by the compiler | No                  | No                   | Yes, by probes            |
| New crates or dependencies            | No                  | No                   | No                        |

_Table 1: Trade-offs for enforcing the unsafe-code prohibition._

## Decision outcome

Adopt Option C.

### 1. The workspace lint table forbids unsafe code

`[workspace.lints.rust]` sets `unsafe_code = "forbid"`. Every member opts in
with `[lints] workspace = true`, so the prohibition arrives with the workspace
rather than with each package's memory.

### 2. Every target root repeats the prohibition

Each `lib`, `bin`, `test`, `bench` and `example` root carries
`#![forbid(unsafe_code)]` in its header, before the first item. The attribute
is redundant where the manifest already applies; it earns its place by making
the prohibition travel with the file. A root that is added, moved out of a
member, or copied into a different package still forbids unsafe code, and a
reviewer reading one file sees the rule that governs it.

`forbid` is absolute: a scoped `#[expect(unsafe_code, …)]` inside such a root
is itself a compile error (`E0453`), so the attribute cannot be worked around
by the mechanism the workspace uses for every other lint.

### 3. The standalone package copies the table

`tests/feature_gate` declares its own `[workspace]` so that a consumer outside
the workspace can be compiled against the published crate. That isolation also
prevents lint inheritance, so its manifest hand-copies `unsafe_code = "forbid"`
into its own `[lints.rust]` table and its roots carry the attribute like every
other root.

### 4. The prohibition is enforced, not just documented

Three independent checks keep it from eroding:

- `tests/workspace_unsafe_boundary.rs` reads `cargo metadata` for the workspace
  and for the standalone package, fails when any target root omits the
  attribute, fails when a member stops inheriting the workspace lint tables,
  fails when the standalone package stops copying the prohibition or silently
  joins the workspace, fails when unsafe syntax appears in any package source,
  and fails when workspace membership changes without an inventory decision.
- `make unsafe-boundary` compiles deliberate unsafe probes on the pinned
  toolchain and requires the `unsafe_code` lint itself to reject them. It also
  carries controls showing that safe code compiles under both the attribute and
  the manifest table, and a probe showing that `forbid` overrules a scoped
  expectation with `E0453`. The blocking CI lane runs it.
- The existing Clippy census with `-D warnings` and
  `allow_attributes = "deny"` keeps a reintroduced `#[allow]` a finding in its
  own right.

`docs/rust-boundary-verification.md` records the scope and the bounds of each
layer, including what a textual contract cannot see.

## Superseded requirements

Issue #804 also asked for a boundary crate to hold the allocator's unsafe
implementation, for both benchmark consumers to use that type, and for a pinned
Miri toolchain with focused tests of the allocator's four methods. The change
that removed the last unsafe code deleted the allocator those requirements
describe, so none of them has a subject left:

- there is no unsafe implementation to move into a boundary crate;
- there is no duplicated allocator for the benchmarks to share;
- there is no unsafe code for Miri to interpret, and the third-party
  `stats_alloc` crate that replaced the allocator is not workspace source.

The prohibition is what keeps that state true. If unsafe code is ever
reintroduced, it fails the build before a Miri run could be scheduled, and
admitting it would have to revisit this record.

## Consequences

### Positive

- The unsafe census has exactly one answer: nothing.
- A target root that forgets the prohibition is still protected by the
  workspace table, and a root that tries to weaken it does not compile.
- The rule is visible in the file it governs, not only in a manifest two
  directories above it.
- The checks run in the existing blocking lane on the existing toolchain pin.

### Negative

- Every new target root must carry one line, and the contract test fails loudly
  until it does.
- The standalone package keeps a hand-copied lint table, which is exactly the
  kind of duplication that drifts. The contract test compares it with the
  workspace level, so drift fails rather than accumulates.
- The textual half cannot see unsafe syntax produced by macro expansion from a
  dependency, and the root's `forbid` does not guarantee rejection of it: the
  pinned compiler accepts an unsafe block emitted by a dependency macro inside
  a root that forbids unsafe code. The prohibition therefore covers workspace
  source, not everything a build links.

## Rejected shortcuts

- Do not add a workspace `deny` floor with an audited crate for unsafe code:
  there is no unsafe code left to audit, and a floor lets a root reopen the
  prohibition with a scoped expectation.
- Do not rely on the workspace lint table alone; it does not reach the
  standalone package and does not travel with a root.
- Do not search the sources as plain text without parsing the header: a
  prohibition nested inside a multi-line attribute reads as present but forbids
  nothing.
- Do not treat the textual contract as proof that the lint works; compile the
  probes.
- Do not suppress a failing check to land a change; fix the root.

## Verification

- `cargo metadata --no-deps --format-version 1` lists the reviewed members, and
  every target root it reports carries `#![forbid(unsafe_code)]`.
- `make unsafe-boundary` rejects every unsafe construct under the crate-level
  attribute, rejects an unsafe block under the manifest table, accepts both
  safe controls, and reports `E0453` for the expectation probe.
- `RUSTFLAGS="-D warnings" cargo clippy --workspace --all-targets --all-features`
  reports zero findings with no `#[allow]` anywhere.
- `make test` passes, so the prohibition compiles in every configuration the
  workspace builds.

## References

- [Issue #804](https://github.com/leynos/wireframe/issues/804)
- [Rust reference: lint levels](https://doc.rust-lang.org/reference/attributes/diagnostics.html#lint-check-attributes)
- [The `unsafe_code` lint](https://doc.rust-lang.org/rustc/lints/listing/allowed-by-default.html#unsafe-code)
- [Cargo: lint configuration](https://doc.rust-lang.org/cargo/reference/manifest.html#the-lints-section)
- [`stats_alloc`](https://docs.rs/stats_alloc), the wrapper the allocation
  benchmarks install.
