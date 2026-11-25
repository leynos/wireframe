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
4. Run `make package-crates` and `make publish-check`.

## The two packaging checks

Both checks build each crate from its own package, so they catch a file left
out of a package, or a symbol that only a workspace build can see. They differ
in how `wireframe_testing` finds `wireframe`:

| Target                | `wireframe` comes from   | Needs an unpublished version |
| --------------------- | ------------------------ | ---------------------------- |
| `make package-crates` | The package just built   | No                           |
| `make publish-check`  | crates.io, as in release | Yes                          |

`make package-crates` is meaningful on any commit. `make publish-check` runs
`lading publish` as a dry run. While the workspace version is still the last
published one, Cargo resolves `wireframe` to that published release, and the
check fails as soon as `wireframe_testing` needs anything newer. After the
version bump, the missing version is expected, and lading reports it as a
warning because `wireframe` comes earlier in the release order.

`make publish-check` first runs lading's pre-flight: a workspace check and the
unit tests. Pass `LADING_SKIP_PREFLIGHT=1` when the suite has already run on
the same commit.
