# Repository layout

The repository is a Rust workspace with a root library, a verification crate,
and shared test support. Its main areas are:

- `src/` contains the published `wireframe` library.
- `crates/wireframe-verification/` contains verification support.
- `wireframe_testing/` contains shared test fixtures and helpers.
- `tests/` contains integration and behaviour-driven tests.
- `tools/dev-fast/config.toml` is the explicitly selected development Cargo
  configuration. The
  [development builds](developers-guide.md#development-builds) section
  documents Make targets and toolchain requirements.
- `docs/` contains design documents, decisions, guides, and reference material.
- `.github/` contains CI workflows and repository automation.
