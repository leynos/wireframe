# Repository layout

The repository is a Rust workspace with a root library, a verification crate,
and shared test support. Its main areas are:

- `src/` contains the published `wireframe` library.
- `crates/wireframe-verification/` contains verification support.
- `wireframe_testing/` contains shared test fixtures and helpers.
- `tests/` contains integration and behaviour-driven tests. An
  integration-test root laid out as `tests/<name>/mod.rs` needs an explicit
  `[[test]]` entry in `Cargo.toml`, because Cargo auto-discovers `tests/*.rs`
  and `tests/<name>/main.rs` but not `tests/<name>/mod.rs`. Without the entry
  the root is silently not built or run.
- `tests/feature_gate/` is a standalone Cargo package, not a workspace member,
  that depends on `wireframe` alone. It is the only place a feature-gated API
  can be observed as absent: trybuild forwards the harness features to its
  fixture and the workspace's `wireframe_testing` dev-dependency enables
  `test-support` for every fixture. `make test-feature-gate` drives it.
- `tools/dev-fast/config.toml` is the explicitly selected development Cargo
  configuration. The
  [development builds](developers-guide.md#development-builds) section
  documents Make targets and toolchain requirements.
- `docs/` contains design documents, decisions, guides, and reference material.
- `.github/` contains CI workflows and repository automation.
