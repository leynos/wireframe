# Releasing the Wireframe crates

This guide describes how a release of the Wireframe workspace is prepared,
checked, and published. One version covers the workspace, and two crates are
published to [crates.io](https://crates.io/), in this order:

1. `wireframe`
2. `wireframe_testing`, which depends on `wireframe`

`wireframe-loom`, `wireframe-verification`, and the feature-gate package are
internal and are never published. The order lives in `lading.toml`, and
`tests/workflow_contracts/lading_config_test.py` holds it to the workspace's
publishable crates and their dependencies.

## Tooling

[lading](https://github.com/leynos/lading) packages and publishes the
workspace. It is pinned to v0.4.0-beta1 by `tools/lading/pyproject.toml`, and
`tools/lading/uv.lock` hashes it together with everything it imports, so every
run uses the same bytes. To move the pin, edit `pyproject.toml`, run
`uv lock --project tools/lading`, and commit both files. lading needs
[uv](https://docs.astral.sh/uv/), which supplies Python 3.14.

## Preparing a release

1. Set the version, which must be the same for both crates:

   ```bash
   uv run --python 3.14 --project tools/lading --locked \
     lading bump 0.4.0-alpha1 --workspace-root .
   ```

2. Update `CHANGELOG.md`, the migration guide, and any documentation that names
   the release.
3. Run `make check-fmt`, `make lint`, `make test`, `make test-doc`, and
   `make test-msrv`.
4. Review and commit these changes. Cargo refuses to package a tree with
   uncommitted changes, and the packages record the commit they came from.
5. Run `make package-crates` and `make publish-check`.
6. Merge the change to `main`.

## Publishing

Push an annotated tag for the merged commit. Its name is `v` followed by the
version, and the commit must be on `main`. Fetch `main` first, so that
`origin/main` names the merged commit rather than an earlier one:

```bash
git fetch origin main
git tag -a v0.4.0-alpha1 -m "Wireframe v0.4.0-alpha1" origin/main
git push origin v0.4.0-alpha1
```

The `Release` workflow then runs these jobs:

1. `package` packages both crates, records their digests, checks that crates.io
   permits the version, and rehearses the publication.
2. `require-main` checks that the tagged commit is on `main`. Every later job
   needs it.
3. `publish-crates-io` waits for the `crates-io` environment's approval,
   reproduces the recorded archives, and publishes them with lading.
4. `verify-crates-io` compares what crates.io serves with the record, and
   builds a consumer from crates.io.
5. `github-release` creates the GitHub release, and attaches both archives and
   their checksums. A version with a hyphen is marked as a pre-release.

Nothing can undo an upload. A version published in error can only be yanked.

### When a run fails

Run the failed jobs again. Publication skips a crate whose published bytes
already match the record, so a release interrupted between the two crates
resumes with the second. A version that crates.io holds with *different* bytes,
or that has been yanked, stops the run, and needs a new version.

### Setting up a repository

Publication needs settings that only an owner can make. Create them before the
first tag, because GitHub creates a missing environment without any protection:

1. In the repository settings, create an environment named `crates-io`. Require
   a reviewer, and allow deployments only from tags that match `v*`.
2. On crates.io, open **Settings**, then **Trusted Publishing**, for `wireframe`
   and again for `wireframe_testing`. Add a GitHub publisher with the owner
   `leynos`, the repository `wireframe`, the workflow `release.yml`, and the
   environment `crates-io`.
3. Revoke any long-lived crates.io token that earlier releases used.

[ADR 019](adrs/adr-0019-publish-the-workspace-through-trusted-publishing.md)
records why the process has this shape.

## The two packaging checks

`make package-crates` builds and verifies both crates from their own packages,
so it catches a file left out of a package, or a symbol that only a workspace
build can see. `make publish-check` runs the release itself as a dry run, but
cannot build `wireframe_testing` until `wireframe` is on crates.io. The two
differ in how `wireframe_testing` finds `wireframe`:

| Target                | `wireframe` comes from   | Needs an unpublished version |
| --------------------- | ------------------------ | ---------------------------- |
| `make package-crates` | The package just built   | No                           |
| `make publish-check`  | crates.io, as in release | Yes                          |

`make package-crates` is meaningful on any commit. `make publish-check` runs
`lading publish` as a dry run. While the workspace version is still the last
published one, Cargo resolves `wireframe` to that published release, and the
check fails as soon as `wireframe_testing` needs anything newer. After the
version bump, the missing version is expected, and lading reports it as a
warning because `wireframe` comes earlier in the release order. It then
packages and checks `wireframe` alone, so only `make package-crates` shows that
`wireframe_testing` builds.

`make publish-check` first runs lading's pre-flight: a workspace check and the
unit tests. Pass `LADING_SKIP_PREFLIGHT=1` when the suite has already run on
the same commit.

No continuous integration job runs either target yet. That coverage is deferred
to the release workflows that pull request #981 adds: `release-dry-run.yml`
calls `release-package.yml` for every pull request. It runs
`make package-crates` on a fresh checkout and fails unless both archives are
recorded for that commit. It runs `make publish-check` only when the workspace
version is unpublished, so the rehearsal is skipped until the version bump.
Until then, run both targets by hand before a release.
