# Architectural decision record (ADR) 019: publish the workspace through Trusted Publishing

## Status

Accepted. No live release has exercised it yet.

A version tag publishes `wireframe` and `wireframe_testing` to crates.io with
lading, using a short-lived credential exchanged through Trusted Publishing.
The published bytes are then compared with a candidate packaged beforehand.

## Date

2026-11-25.

## Context and problem statement

Earlier versions were uploaded with an API token rather than through Trusted
Publishing. Nothing recorded which commit the uploaded bytes came from, or that
the two crates belonged to the same release.

`leynos/netsuke` publishes through Trusted Publishing and verifies what the
registry serves, but it ships one crate and several binaries.
`leynos/rstest-bdd` publishes a workspace in a declared order with lading, but
by hand. Wireframe is a library workspace of two dependent crates, so it needs
the first project's safeguards and the second project's ordering.

## Decision drivers

- An upload to crates.io cannot be undone, so every check that can precede it
  must.
- No long-lived registry credential may be stored.
- `wireframe_testing` cannot be resolved from crates.io until `wireframe` is
  there at the same version.
- A pull request must exercise the packaging steps that a release runs, without
  being able to reach a job that publishes.
- An interrupted release must be completed by running it again.

## Options considered

Table 1 compares the ways to upload the workspace.

| Concern                           | Netsuke's own uploader | lading, live                  | `cargo publish --workspace` |
| --------------------------------- | ---------------------- | ----------------------------- | --------------------------- |
| Uploads the retained bytes        | Yes                    | Repackages from the commit    | Repackages from the commit  |
| Publishes dependents in order     | Would need to be built | Yes, from `lading.toml`       | Yes                         |
| Resumes after one crate published | Per crate              | Yes                           | No                          |
| Rehearses without uploading       | Partly                 | Yes                           | Yes                         |
| Builds run beside the credential  | No                     | Yes, Cargo verifies a package | Yes                         |

_Table 1: Comparison of publication routes._

## Decision outcome

Publish with lading, and close the gap that repackaging leaves by proving the
bytes on both sides of the upload.

1. `release-package.yml` packages both crates, records each archive's digest
   and commit, surveys crates.io, and rehearses the publication. It is reusable
   and holds only `contents: read`.
2. `release.yml` starts only for a `v*.*.*` tag push. It has no `workflow_call`
   and no `workflow_dispatch`, so Netsuke's write-authorization policy, which
   guards a reusable release workflow, is not needed.
3. `require-main` requires a commit on `main`. Every later job needs it, so a
   run in which every crate is already published still cannot verify or
   announce a commit that is not on `main`.
4. `publish-crates-io` packages the crates again, and requires the recorded
   digests. Only then does it exchange its identity for a token, and only the
   step that runs lading receives it. The job is declared in `release.yml`
   itself and names the `crates-io` environment, because crates.io identifies a
   publisher by both.
5. `verify-crates-io` downloads what crates.io serves, requires the recorded
   digests, and builds a consumer on the minimum Rust version. It holds no
   credential.
6. `github-release` announces the release last, with the archives attached.

An archive that Cargo packages twice from one commit with one toolchain is
byte-identical, and `wireframe_testing` embeds the digest of the `wireframe`
archive it resolved. The candidate therefore fixes both crates before either is
uploaded.

Table 2 records what was taken from each project.

| Source       | Taken                                                                                                             | Left out                                                                   |
| ------------ | ----------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------- |
| `netsuke`    | Candidate commit, tag and version gate, environment and OIDC exchange, registry classification, byte verification | Binary builds, admission evidence, its uploader, sidecars, asset inventory |
| `rstest-bdd` | `lading.toml`, a pinned lading, `make publish-check`, pre-flight skipped only on the CI step                      | Compiler-cache statistics, published-fixture staging                       |

_Table 2: What the release process adapts from each project._

## Known risks and limitations

- lading runs `cargo package` and `cargo publish`, which compile dependencies
  to verify each package while the token is in the environment. The
  dependencies are fixed by `Cargo.lock` and `tools/lading/uv.lock`, the token
  is short-lived, and crates.io limits it to the crates that trust this
  workflow. Netsuke's uploader avoids this exposure.
- lading skips a version that crates.io reports as already published, without
  comparing it. The survey compares it first, and verification compares it
  afterwards.
- The environment and the trusted publishers are account settings that this
  repository cannot hold. GitHub creates a missing environment without any
  protection, so it must be configured before the first tag.
- While the workspace carries a published version, the lading rehearsal is
  skipped, because Cargo would resolve `wireframe` to that older release.
  Packaging is still checked.

## Verification

- `tests/workflow_contracts/release_workflow_test.py` holds the triggers,
  permissions, job order, and commands.
- `tests/workflow_contracts/lading_config_test.py` holds the release order and
  the tool pin.
- `make test-release-scripts` covers the candidate record and the registry
  decisions.

## References

- [Releasing the Wireframe crates](../releasing-crates.md)
- [Release scripts](../../scripts/release/README.md)
- [ADR 020: import Netsuke's release-verification modules](adr-0020-import-netsukes-release-verification-modules.md)
- [crates.io Trusted Publishing](https://crates.io/docs/trusted-publishing)
