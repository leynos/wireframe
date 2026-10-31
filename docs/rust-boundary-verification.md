# Unsafe-code boundary verification

This document records what the workspace's `unsafe_code` boundary check
establishes, how each half of it works, and what it does not establish. The
decision itself is [ADR 015](adr-015-unsafe-code-boundary.md); this page is the
evidence contract.

## The boundary in one paragraph

`[workspace.lints.rust]` sets `unsafe_code = "deny"` as a floor. Every safe
Cargo target root — `lib`, `bin`, `test`, `bench`, and `example`, including the
standalone `tests/feature_gate` package — raises that to
`#![forbid(unsafe_code)]` in its own source. Exactly one crate,
`crates/wireframe-bench-alloc`, carries unsafe code: the `GlobalAlloc`
implementation of the counting allocator that the allocation baselines install.
Its safe mechanism lives in `crates/wireframe-bench-counting`.

The floor matters. Cargo refuses to let a member override an inherited
`[workspace.lints]` entry, so the audited crate cannot lower the workspace
`deny` to `allow`; it can only name a scoped
`#[expect(unsafe_code, reason = …)]`, which is a documented exception rather
than a blanket permission. A safe root cannot override its own `forbid` at all:
`forbid` is absolute, and an `#[expect]` under it is a compile error (`E0453`).

## Inventory

| Package                    | Role                               | Lint posture                       |
| -------------------------- | ---------------------------------- | ---------------------------------- |
| `wireframe`                | Published library and binaries     | `#![forbid(unsafe_code)]` per root |
| `wireframe_testing`        | Shared test fixtures and helpers   | `#![forbid(unsafe_code)]` per root |
| `wireframe-loom`           | Loom model targets                 | `#![forbid(unsafe_code)]` per root |
| `wireframe-verification`   | Stateright verification support    | `#![forbid(unsafe_code)]` per root |
| `wireframe-bench-counting` | Safe counting mechanism            | `#![forbid(unsafe_code)]`          |
| `wireframe-bench-alloc`    | Counting `GlobalAlloc`             | `deny` + two scoped expectations   |
| `tests/feature_gate`       | Standalone feature-absence package | `#![forbid(unsafe_code)]`          |

_Table 1: The audited surface and the lint posture of every package._

Adding a package, or adding an audited crate, is an inventory decision. The
contract test fails until the change is recorded here and in the test's
`EXPECTED_MEMBERS` and `AUDITED_PACKAGES` tables.

## Layer 1: the textual contract

`tests/workspace_unsafe_boundary.rs` discovers target roots from
`cargo metadata --no-deps --format-version 1`, so it sees automatic and
explicitly configured targets alike, including the `tests/feature_gate` package
that the workspace metadata does not describe. It asserts:

- `workspace_manifest_declares_the_unsafe_code_floor` — the workspace lint
  table declares `unsafe_code = "deny"`.
- `every_safe_target_root_forbids_unsafe_code` — each root outside the audited
  package carries `#![forbid(unsafe_code)]` in its header, meaning before the
  first item.
- `unsafe_syntax_is_confined_to_the_audited_package` — no root outside the
  audited package contains an unsafe construct, and no root uses
  `#[allow(unsafe_code)]`.
- `the_audited_package_defines_the_allocator_once` — exactly one target root
  contains a `GlobalAlloc` implementation.
- `workspace_membership_matches_the_reviewed_inventory` — the set of workspace
  packages is exactly the reviewed list.

This half is deliberately textual. It proves that the prohibition is present
and that unsafe syntax is absent from the files it can read. It does not prove
that the attribute has any effect, and it cannot see unsafe syntax produced by
a macro invocation from a dependency.

## Layer 2: negative compiler probes

`make unsafe-boundary` runs `scripts/check-unsafe-boundary.sh`, which builds a
temporary package on the toolchain pinned in `rust-toolchain.toml` and compiles
one probe per case. Diagnostics are read from Cargo's JSON message stream, so
each probe must be rejected by the `unsafe_code` lint itself rather than by any
unrelated failure.

| Probe                       | Required outcome             |
| --------------------------- | ---------------------------- |
| Safe code under `forbid`    | Compiles (control)           |
| Unsafe block                | Rejected, code `unsafe_code` |
| Unsafe function             | Rejected, code `unsafe_code` |
| Unsafe trait declaration    | Rejected, code `unsafe_code` |
| Unsafe trait implementation | Rejected, code `unsafe_code` |
| Unsafe extern block         | Rejected, code `unsafe_code` |
| `#[expect]` under `forbid`  | Rejected, code `E0453`       |
| `#[expect]` under `deny`    | Compiles (control)           |

_Table 2: The probes and the outcome each must produce._

The two controls are what make the probes meaningful. The first proves the
probe package is otherwise compilable, so a rejection is attributable to the
unsafe construct. The last proves the audited crate's construction — a scoped
expectation under a `deny` floor — is accepted, so the `E0453` result is a
property of `forbid` rather than of the syntax in the probe.

## Layer 3: Miri

`make boundary-miri` runs:

```sh
cargo +nightly-2026-08-07 miri test -p wireframe-bench-alloc -- --test-threads=1
```

The toolchain is pinned separately from `rust-toolchain.toml` as
`MIRI_TOOLCHAIN`, because Miri lags the compiler pin. A missing `miri`
component fails the target loudly rather than skipping it.

### Scope

Miri interprets:

- the four delegating methods of `CountingAllocator` — `alloc`, `dealloc`,
  `alloc_zeroed`, and `realloc` — through the crate's unit tests, which call
  each method directly and check the round trip, the zero fill, and prefix
  preservation across a reallocation;
- the integration test that installs the allocator as the process's
  `#[global_allocator]` and drives a positive counting window, a zero window,
  and a window that closes after the measured operation fails;
- the atomic enable gate and counter in `wireframe-bench-counting`, which those
  tests execute as part of every delegation.

### Exclusions

- The benchmark binaries are not interpreted. They run a multi-threaded Tokio
  runtime, read a real monotonic clock, and open loopback sockets for the pool
  scope. Miri cannot run that at benchmark scale, and its results would not
  describe the measurements anyway.
- `std::alloc::System` is not the host allocator under Miri. Miri models
  allocation, so the run establishes that the delegating bodies pass layouts
  and pointers consistently with the obligation they inherit, not that the host
  system allocator behaves correctly. The host allocator's correctness is
  Rust's contract, not this workspace's.
- Miri explores a finite set of executions under its weak-memory model rather
  than every interleaving. The ordering argument for the gate is separate: an
  acquire load pairs with the release fences around the window, and the counter
  uses `SeqCst` operations.
- Leak checking covers the memory the interpreted tests allocate. It does not
  say anything about allocations made by code the tests do not run.

### Bounds

- The audited surface is one unit struct and four method bodies. The unit tests
  drive all four; adding a fifth method or a second unsafe type would leave
  part of the surface uninterpreted until a test covered it. The textual
  contract does not catch that, so it is a review obligation recorded here.
- The tests are serialized (`--test-threads=1`, plus `serial_test` in the
  crate) because the counter and the gate are process-global. Parallel
  interpretation would let one test's open window be observed by another.

## Layer 4: the lint census

The boundary is only useful if nothing else reintroduces unsafe code under a
lint allowance. The census is:

```sh
RUSTFLAGS="-D warnings" cargo clippy --workspace --all-targets --all-features
```

A clean run reports zero findings. `allow_attributes = "deny"` in `clippy.toml`
means any `#[allow]` is itself a finding, so a workspace with no `#[allow]` is
enforced rather than asserted. The audited crate's expectations carry a
`reason` and cover only the trait implementation and its method tests.

## Benchmark smoke check

The boundary exists to serve the benchmarks, so the change is verified from
their side too:

```sh
make bench-codec
make bench-runtime-ownership
```

Both must run and report non-zero allocation counts. A counter that observed
nothing would still pass every check above; the benchmark self-checks in
`benches/runtime_ownership_baselines_alloc/probes.rs` and `pool_bracket.rs`
fail loudly when the gate never opens.

## Re-validating after a toolchain bump

1. Install Miri for the new nightly and update `MIRI_TOOLCHAIN` in the
   `Makefile`.
2. Run `make unsafe-boundary`. If a probe's diagnostic code changes, decide
   whether the prohibition still holds before adjusting the expectation.
3. Run `make boundary-miri`.
4. Re-run the lint census and both benchmark targets.

## What would invalidate these claims

- A dependency that expands an unsafe macro into a safe root. The textual
  contract cannot see expansion; `forbid(unsafe_code)` would still reject
  unsafe _syntax_ attributed to the root, which is the backstop.
- A toolchain that stops applying crate-level `forbid` to macro output. The
  probes catch a change in how the lint reports unsafe constructs, but not
  every macro-expansion rule; Miri and the benchmarks are the behavioural
  backstop.
- A new audited crate added without an entry here and in the contract test's
  inventory. The membership test fails, which is why the inventory is part of
  the test rather than a comment.

## References

- [ADR 015: a single audited unsafe boundary for the counting allocator](adr-015-unsafe-code-boundary.md)
- [Runtime-ownership baselines](runtime-ownership-baselines.md)
- [The `unsafe_code` lint](https://doc.rust-lang.org/rustc/lints/listing/allowed-by-default.html#unsafe-code)
- [Miri](https://github.com/rust-lang/miri)
