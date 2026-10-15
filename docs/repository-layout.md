# Repository layout

The repository is a Rust workspace with a root library, a verification crate,
and shared test support. Its main areas are:

- `src/` contains the published `wireframe` library.
- `crates/wireframe-loom/` contains Loom concurrency models.
- `crates/wireframe-verification/` contains verification support.
- `wireframe_testing/` contains shared test fixtures and helpers.
- `tests/` contains integration and behaviour-driven tests. An
  integration-test root laid out as `tests/<name>/mod.rs` needs an explicit
  `[[test]]` entry in `Cargo.toml`, because Cargo auto-discovers `tests/*.rs`
  and `tests/<name>/main.rs` but not `tests/<name>/mod.rs`. Without the entry
  the root is silently not built or run.
- `tools/dev-fast/config.toml` is the explicitly selected development Cargo
  configuration. The
  [development builds](developers-guide.md#development-builds) section
  documents Make targets and toolchain requirements.
- `docs/` contains design documents, decisions, guides, and reference material.
- `.github/` contains CI workflows and repository automation.
