# Releasing the Wireframe crates

This guide describes how a release of the Wireframe workspace is prepared and
checked. One version covers the workspace, and two crates are published to
[crates.io](https://crates.io/), in this order:

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
