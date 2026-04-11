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

During roadmap item 10.1.1 the workspace is intentionally staged with only the
root package as a member and default member:

- `members = ["."]`
- `default-members = ["."]`

That means ordinary root-level commands such as `cargo build`, `cargo check`,
`cargo test`, and `cargo clippy` retain their existing ergonomics and continue
to target the main `wireframe` package by default.

Use plain root-level Cargo commands for day-to-day work on the main crate.
Reach for `--workspace` when a task is explicitly meant to cover every current
workspace member, for example repository-wide validation in CI or when later
roadmap items add internal verification crates.

One Cargo nuance is worth knowing: `cargo metadata` for this repository still
reports the in-tree helper crate `wireframe_testing` in `workspace_members`
because it lives under the repository root as a path dependency. That does not
change day-to-day command ergonomics because `default-members = ["."]` keeps
plain root-level Cargo commands focused on the main `wireframe` package.
