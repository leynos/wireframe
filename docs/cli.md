# Command line interface

Wireframe includes a small command line interface for demonstration. The CLI
uses `clap` to parse arguments. An optional `--name` flag allows customising
the greeting printed by the `wireframe` binary.

Manual pages are generated during the build via `clap_mangen`. The `build.rs`
script writes `wireframe.1` to `target/generated-man`, and the `release` GitHub
workflow uploads this file.
