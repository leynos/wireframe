# Unsafe-code boundary verification

This document records what the workspace's `unsafe_code` prohibition
establishes, how each check works, and what it does not establish. The decision
itself is [ADR 016](adr-016-unsafe-code-boundary.md); this page is the evidence
contract.

## The boundary in one paragraph

`[workspace.lints.rust]` sets `unsafe_code = "forbid"`, and every member opts
in with `[lints] workspace = true`. Every Cargo target root — `lib`, `bin`,
`test`, `bench` and `example`, in every member and in the standalone
`tests/feature_gate` package — carries `#![forbid(unsafe_code)]` in its header.
The standalone package cannot inherit the workspace table, so it hand-copies
the same level into its own `[lints.rust]`. No package contains unsafe code.

The duplication is deliberate rather than accidental. Cargo applies a workspace
lint table only to a package that opts in, and a target root is bound only by
the package it belongs to, so a root that is added or moved has to state the
rule itself. `forbid` is absolute: a scoped `#[expect(unsafe_code, …)]` inside
such a root is a compile error (`E0453`), which means the workspace's usual
escape hatch for a lint cannot reopen this one.

## Inventory

| Package                  | Role                               | Lint posture                       |
| ------------------------ | ---------------------------------- | ---------------------------------- |
| `wireframe`              | Published library and binaries     | `#![forbid(unsafe_code)]` per root |
| `wireframe_testing`      | Shared test fixtures and helpers   | `#![forbid(unsafe_code)]` per root |
| `wireframe-loom`         | Loom model targets                 | `#![forbid(unsafe_code)]` per root |
| `wireframe-verification` | Stateright verification support    | `#![forbid(unsafe_code)]` per root |
| `tests/feature_gate`     | Standalone feature-absence package | `#![forbid(unsafe_code)]` per root |

_Table 1: The packages and their lint posture._

Adding or removing a workspace member is an inventory decision. The contract
test fails until the change is recorded here and in the test's
`EXPECTED_MEMBERS` table.

## Layer 1: the textual contract

`tests/workspace_unsafe_boundary.rs` discovers target roots from
`cargo metadata --no-deps --format-version 1`, run once for the workspace and
once against `tests/feature_gate/Cargo.toml`, so it sees automatic and
explicitly configured targets alike, including the package the workspace
metadata does not describe. It asserts:

- `workspace_manifest_forbids_unsafe_code` — the workspace lint table declares
  `unsafe_code = "forbid"`.
- `workspace_members_inherit_the_lint_tables` — every member sets
  `[lints] workspace = true` and declares no package-local lint table, which
  Cargo rejects beside the opt-in.
- `the_feature_gate_package_declares_the_prohibition` — the standalone package's
  hand-copied `[lints.rust]` table still matches the workspace level.
- `the_feature_gate_package_stays_standalone` — the package has not silently
  joined the workspace, which would remove the standalone compile that
  `make test-feature-gate` depends on.
- `every_target_root_forbids_unsafe_code` — each root carries
  `#![forbid(unsafe_code)]` in its header, meaning before the first item. The
  header is parsed, not searched: a prohibition nested inside a multi-line
  attribute is part of that attribute and does not count, because a file in
  that state does not compile.
- `unsafe_syntax_is_absent_from_every_package` — no package source contains an
  unsafe construct, a scoped `#[expect(unsafe_code, …)]`, or an
  `#[allow(unsafe_code)]`. Every `.rs` file under each package's directory is
  read, not only the roots Cargo reports, so a child module behind a disabled
  feature cannot hide one. The standalone package is read by path for the same
  reason. Constructs are matched after whitespace is collapsed, so a spelling
  split across a line break is still found. Two test helpers whose purpose is
  to name the constructs are exempt.
- `workspace_membership_matches_the_reviewed_inventory` — the set of workspace
  packages is exactly the reviewed list.

This layer is deliberately textual. It proves that the prohibition is present
and that unsafe syntax is absent from the files it can read. It does not prove
that the attribute has any effect, and it cannot see unsafe syntax produced by
a macro invocation from a dependency.

## Layer 2: negative compiler probes

`make unsafe-boundary` runs `scripts/check-unsafe-boundary.sh`, which builds
two temporary packages outside the repository on the toolchain pinned in
`rust-toolchain.toml` and compiles one probe per case. Diagnostics are read
from Cargo's JSON message stream, so each probe must be rejected by the
`unsafe_code` lint itself rather than by an unrelated failure. The blocking CI
lane runs `make unsafe-boundary`, so the probes execute on every pull request;
they need only the existing toolchain pin.

The first package states the prohibition in its source; the second states it in
its manifest's `[lints.rust]` table, which is the form the workspace table
takes for a package that never writes the attribute.

| Probe                                     | Required outcome             |
| ----------------------------------------- | ---------------------------- |
| Safe code under a crate-level `forbid`    | Compiles (control)           |
| Unsafe block                              | Rejected, code `unsafe_code` |
| Unsafe function                           | Rejected, code `unsafe_code` |
| Unsafe trait declaration                  | Rejected, code `unsafe_code` |
| Unsafe trait implementation               | Rejected, code `unsafe_code` |
| Unsafe extern block                       | Rejected, code `unsafe_code` |
| `#[expect]` under `forbid`                | Rejected, code `E0453`       |
| Safe code under a manifest-level table    | Compiles (control)           |
| Unsafe block under a manifest-level table | Rejected, code `unsafe_code` |

_Table 2: The probes and the outcome each must produce._

The two safe controls are what make the probes meaningful: they prove that each
probe package is otherwise compilable, so a rejection is attributable to the
unsafe construct rather than to a typo or a missing dependency. The `E0453`
probe records the property the whole design leans on — that a scoped
expectation cannot reopen a `forbid`.

## Layer 3: the lint census

The boundary is only useful if nothing reintroduces unsafe code under a lint
allowance. The census is:

```sh
RUSTFLAGS="-D warnings" cargo clippy --workspace --all-targets --all-features
```

A clean run reports zero findings. `allow_attributes = "deny"` in
`[workspace.lints.clippy]` (`Cargo.toml`) means any `#[allow]` is itself a
finding, so a workspace with no `#[allow]` is enforced rather than asserted.
This census does not cover `tests/feature_gate`; its manifest copies the same
Clippy tables and its contract test covers the lint level, but no target runs
Clippy over it.

## What these checks cannot establish

- **Macro expansion.** The textual scan reads source, not expanded code. A
  dependency macro that expands to unsafe syntax inside a root is invisible to
  it. `forbid(unsafe_code)` is the backstop: the lint sees the expanded
  attribute-less tokens attributed to the root. The probes show the lint
  rejects the constructs they write, not every expansion rule.
- **Toolchain behaviour.** A toolchain that stopped applying crate-level
  `forbid` would leave the textual contract satisfied. The probes are the check
  for that, and a probe whose diagnostic code changes must be investigated
  before the expectation is adjusted.
- **A new package.** Cargo's `exclude` can remove a directory from the
  workspace without changing the member list. The contract test reads every
  package the metadata reports and the standalone directory by path, so an
  excluded package elsewhere in the tree would not be scanned. Keeping the tree
  free of such packages is a review obligation.
- **Unsafe code that does not use the listed spellings.** The scan matches six
  literal construct spellings. It is a second line behind the compiler, not a
  parser, and a construct it does not recognize is still rejected by `forbid`.

## Re-validating after a toolchain bump

1. Run `make unsafe-boundary`. If a probe's diagnostic code changes, decide
   whether the prohibition still holds before adjusting the expectation.
2. Run `make test`, so the prohibition compiles in every target the workspace
   builds.
3. Re-run the lint census.

## References

- [ADR 016: forbid unsafe code workspace-wide](adr-016-unsafe-code-boundary.md)
- [The `unsafe_code` lint](https://doc.rust-lang.org/rustc/lints/listing/allowed-by-default.html#unsafe-code)
- [Cargo: lint configuration](https://doc.rust-lang.org/cargo/reference/manifest.html#the-lints-section)
