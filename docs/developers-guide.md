# Wireframe developers' guide

This guide defines the architectural vocabulary used across Wireframe source,
rustdoc, and user-facing documentation. Treat it as the naming contract for new
APIs and refactors.

## Layer model and glossary

| Layer                 | Canonical term | Primary types                                    | Description                                                                                                                |
| --------------------- | -------------- | ------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------- |
| Transport framing     | Frame          | `FrameCodec::Frame`, `LengthDelimitedFrameCodec` | Physical wire unit read from or written to the socket.                                                                     |
| Routing envelope      | Envelope       | `Packet`, `Envelope`, `PacketParts`              | Routable wrapper that carries route id, optional correlation id, and payload bytes (`Packet` names the trait abstraction). |
| Domain payload        | Message        | `message::Message`, extractor `Message<T>`       | Typed application data encoded into envelope payload bytes.                                                                |
| Transport subdivision | Fragment       | `FragmentHeader`, `Fragmenter`, `Reassembler`    | Size-limited chunk used when a payload exceeds frame budget.                                                               |

## Naming invariants

- Frame terms belong to codec and socket boundaries only.
- Envelope terms belong to routing and middleware request/response wrappers;
  packet is an alias reserved for trait-level abstractions (`Packet`,
  `PacketParts`).
- Message terms belong to typed payload encode/decode concerns.
- Fragment terms belong to transport splitting and reassembly.
- Correlation identifiers are cross-layer metadata and may appear on frame,
  packet, and message-adjacent APIs when protocol metadata requires it.

## Allowed aliases and prohibited mixing

| Canonical term | Allowed aliases                     | Avoid in the same context                 |
| -------------- | ----------------------------------- | ----------------------------------------- |
| Frame          | wire frame, transport frame         | packet, envelope, message                 |
| Envelope       | packet (trait abstraction contexts) | frame (unless describing codec transport) |
| Message        | payload type, domain message        | frame, fragment                           |
| Fragment       | fragment chunk                      | packet, message                           |

## API and docs checklist

Use this checklist before merging API naming changes:

- Confirm the identifier name matches the owning layer.
- Ensure rustdoc examples use the same term as the symbol being documented.
- Verify `docs/users-guide.md` and migration notes describe the same meaning.
- Label cross-layer terms (for example, `correlation_id`) explicitly as shared
  metadata.

## Message sequence validation architecture

Message continuation ordering lives in `src/message_assembler/series.rs`. The
public-facing control point is `validate_and_advance_sequence()`, which keeps
the series state machine readable by delegating to two private helpers:

- `start_sequence_tracking()` handles the first numbered continuation after an
  untracked start. It switches the series into tracked mode and then delegates
  to `advance_sequence_or_overflow()` to derive and record the next expected
  sequence number.
- `advance_tracked_sequence()` handles all later numbered continuations once
  tracking is active. It rejects duplicates, gaps, and sequence overflow before
  calling `advance_sequence_or_overflow()` to advance the expected sequence.

`advance_sequence_or_overflow()` is the shared leaf helper: it calls
`checked_increment()` on the incoming sequence and raises
`MessageSeriesError::SequenceOverflow` only when the counter wraps while more
frames are still expected.

This three-helper split is intentional. `start_sequence_tracking()` isolates
the untracked-to-tracked transition; `advance_tracked_sequence()` owns the
duplicate/gap/out-of-order validation; and `advance_sequence_or_overflow()`
owns the overflow decision. Each piece is independently testable and the
decision tree stays flat and readable during future refactors.

### `fill_buf_with_prefix`

The private helper `fill_buf_with_prefix(buf, prefix, endianness)` in
`src/frame/conversion.rs` copies a validated length-prefix byte slice into the
correct position of an 8-byte staging buffer. For big-endian prefixes it places
the bytes at the high end of the buffer (`buf[8 - size..]`); for little-endian
prefixes it places them at the low end (`buf[..size]`). Callers must guarantee
that `prefix.len()` is one of `{1, 2, 4, 8}`; this invariant is enforced
upstream in `bytes_to_u64`, which is why the helper validates the range before
copying and reports an error when the prefix width falls outside the supported
set.

### `ServerShutdownHandle`

`ServerShutdownHandle` in `wireframe_testing::client_pair` is a type alias for
the tuple `(oneshot::Sender<()>, JoinHandle<Result<(), ServerError>>)` that
`PendingServer` stores between server start-up and explicit shutdown. Naming
the alias keeps `PendingServer`'s field types readable and makes
`PendingServer::take` return a self-documenting `Option<ServerShutdownHandle>`
instead of an opaque inline tuple. Tests that call `WireframePair::shutdown()`
do not interact with this type directly; it is an internal implementation
detail of the pair harness.

## Vocabulary normalization outcome (2026-02-20)

The 2026-02-20 normalization pass aligned docs and rustdoc terminology to this
model and did not rename public symbols. Existing API names (`FrameCodec`,
`Packet`, `Envelope`, `Message`, `Fragment*`) already map cleanly to separate
layers, so the implementation focused on clarifying boundaries rather than
introducing additional breaking changes.

## Quality gates

Use the Makefile targets as the contributor entrypoint for routine validation:

- `make check-fmt` verifies workspace formatting.
- `make lint` runs rustdoc with warnings denied, `cargo clippy`, and
  `whitaker --all -- --all-targets --all-features`.
- `make test` runs the main automated test suite with warnings treated as
  errors.

Install Whitaker through the standalone installer described in the
[Whitaker user's guide](whitaker-users-guide.md) so local linting matches
continuous integration (CI).

## Cargo workspace semantics

Wireframe now uses a hybrid root manifest: the repository root `Cargo.toml`
contains both `[package]` and `[workspace]`.

After roadmap items 15.1.1 and 15.1.2 the workspace explicitly lists the root
package and the internal verification crate, while keeping only the root
package as a default member:

- `members = [".", "crates/wireframe-verification"]`
- `default-members = ["."]`

That means ordinary root-level commands such as `cargo build`, `cargo check`,
`cargo test`, and `cargo clippy` retain their existing ergonomics and continue
to target the main `wireframe` package by default.

Use plain root-level Cargo commands for day-to-day work on the main crate.
Reach for `--workspace` when a task is explicitly meant to cover every current
workspace member, for example repository-wide validation in CI or when a change
also touches `crates/wireframe-verification`.

Use `cargo test -p wireframe-verification` to exercise the Stateright crate in
isolation. The existing `Makefile` targets still focus on the root `wireframe`
crate because the dedicated formal-verification targets belong to later roadmap
items.

One Cargo nuance is worth knowing: `cargo metadata` for this repository still
reports the in-tree helper crate `wireframe_testing` in `workspace_members`
because it lives under the repository root as a path dependency. That sits
alongside the explicit `wireframe-verification` member and does not change
day-to-day command ergonomics because `default-members = ["."]` keeps plain
root-level Cargo commands focused on the main `wireframe` package.

### Workspace manifest test support

The shared module `tests/common/workspace_manifest_support.rs` keeps the
workspace-contract helpers beside the integration tests that use them. It is a
module, not a library crate, because the code is test-only scaffolding and
should not widen the published crate surface or add another Cargo target.

The support layer uses `cap-std` with the `fs_utf8` feature for
capability-oriented directory access, `camino` for UTF-8-typed paths, and
`serde_json` for structured assertions over `cargo metadata` output.

- `repo_root()` locates the repository root as a `Utf8PathBuf`.
- `repo_dir()` opens that root as a `cap_std::fs_utf8::Dir`.
- `root_manifest()` reads the root `Cargo.toml` into a `String`.
- `run_cargo(args)` runs `cargo` in the repository root and returns UTF-8
  stdout, or an error that includes stderr.
- `cargo_metadata()` wraps `cargo metadata --no-deps --format-version 1`.
- `root_package_id()` wraps `cargo pkgid -- wireframe` and trims trailing
  whitespace.
- `has_manifest_line(manifest, line)` checks for a complete trimmed line rather
  than a substring match.

`WorkspaceManifestWorld` in `tests/fixtures/workspace_manifest.rs` is the
behaviour-driven development (BDD) fixture for these assertions. Extend it by
loading more workspace-state inputs in `load()` and adding focused verification
methods that the step definitions and scenario can reuse.

## Formal verification tooling

Formal-verification tools are pinned in repository metadata and installed
through concise Makefile entry points. Contributors should use these targets
from the repository root instead of running long `uv tool run` commands by
hand:

- `make install-kani` installs the Kani version named in
  `tools/kani/VERSION`.
- `make check-kani-version` verifies that the installed Kani binary matches
  `tools/kani/VERSION`.
- `make install-verus` installs the Verus release named in
  `tools/verus/VERSION` after checking `tools/verus/SHA256SUMS`.
- `make run-verus` runs the proof file selected by `VERUS_PROOF_FILE`, or
  `verus/wireframe_proofs.rs` by default.

The targets delegate to `prover-tools`, supplied by the pinned
`rust-prover-tools` source in `tools/rust-prover-tools/REF`. The Makefile
should stay thin: it constructs the pinned `uv tool run --python 3.14`
invocation and lets `prover-tools` own Kani installation, Kani version checks,
Verus download and checksum verification, Verus binary resolution, and Verus
toolchain handling.

Keep Verus proof files outside the normal Cargo build under `verus/`. The
`run-verus` target is expected to fail with a clear missing-proof-file
diagnostic until later formal-verification roadmap work adds
`verus/wireframe_proofs.rs`.

### Formal tooling test support

The shared module `tests/common/formal_tooling_support.rs` keeps
repository-contract helpers beside the integration tests that use them. It
reads the tool metadata files, extracts Makefile target recipes, and verifies
that those recipes delegate to `prover-tools` rather than embedding installer
commands such as `cargo install`, `curl`, or `rustup toolchain install`.

`FormalToolingWorld` in `tests/fixtures/formal_tooling.rs` is the BDD fixture
for the contributor workflow. Extend it by loading additional repository
metadata in `load()` and adding focused verification methods that scenario
functions and step definitions can reuse.

## Test infrastructure and framework

### rstest and rstest-bdd

The test suite uses [`rstest`](https://crates.io/crates/rstest) for
fixture-based parametric tests and `rstest-bdd` (via `rstest_bdd_macros`) for
behaviour-driven development (BDD) scenarios expressed in Gherkin.

Fixtures are plain Rust functions annotated with `#[fixture]`. Inject them into
tests by listing them as parameters; `rstest` constructs each fixture before
running the test body.

BDD scenarios live in `.feature` files under `tests/features/`. Each file
describes one or more scenarios using the standard Given/When/Then syntax.
Scenario functions are annotated with `#[scenario(path = "…", name = "…")]` and
receive fixture parameters by name.

### Feature files and step definitions

Each `.feature` file under `tests/features/` has a corresponding
step-definition module under `tests/steps/`. Step functions are annotated with
`#[given]`, `#[when]`, or `#[then]` and accept a mutable reference to the BDD
world fixture as their first argument.

Add new scenarios by:

1. Writing a new Gherkin scenario in the relevant `.feature` file.
2. Implementing the missing step functions in the corresponding
   `tests/steps/` module.
3. Adding a scenario function in `tests/scenarios/` that names the new
   scenario, injects the fixture, and delegates to a helper that invokes the
   step logic in sequence.

### `LoggerHandle::Default` and `ObservabilityHandle::Default`

Both `LoggerHandle` (in `wireframe_testing::logging`) and `ObservabilityHandle`
(in `wireframe_testing::observability`) implement `Default`. The `default()`
method delegates to `new()` in each case, providing a convenient way to acquire
a fresh handle without explicitly calling the constructor.

`LoggerHandle::new()` tolerates a poisoned mutex: if a prior test panicked
while holding the logger lock, `new()` recovers the guard via `into_inner()`
and drains any buffered log records, so the next test starts from a clean state.
