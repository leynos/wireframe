"""Hold the release configuration to the crates the workspace publishes.

A publishable crate missing from ``lading.toml`` would be published in an order
nobody reviewed, or not at all, and one listed before its dependency fails part
way through a release that cannot be undone. The Makefile and the tool pin are
checked here too, because drift in either keeps working until release day.

Run these contracts with ``make test-workflow-contracts``.
"""

from __future__ import annotations

import json
import re
import subprocess
import tomllib
from pathlib import Path

import pytest

REPOSITORY_ROOT = Path(__file__).resolve().parents[2]
LADING_CONFIG = REPOSITORY_ROOT / "lading.toml"
TOOL_PROJECT = REPOSITORY_ROOT / "tools" / "lading"
EXACT_LADING = re.compile(r"lading==\d+\.\d+\.\d+(?:(?:a|b|rc)\d+)?")


def _load(path: Path) -> dict[str, object]:
    """Parse one TOML file."""
    return tomllib.loads(path.read_text(encoding="utf-8"))


def _workspace_packages() -> list[dict[str, object]]:
    """Return Cargo's metadata for every workspace member."""
    metadata = json.loads(
        subprocess.run(
            ["cargo", "metadata", "--no-deps", "--format-version", "1", "--offline"],
            cwd=REPOSITORY_ROOT,
            capture_output=True,
            text=True,
            check=True,
        ).stdout
    )
    return metadata["packages"]


def _is_publishable(package: dict[str, object]) -> bool:
    """Cargo reports ``publish = false`` as an empty registry list."""
    return package["publish"] is None or bool(package["publish"])


def _publishable_packages() -> dict[str, set[str]]:
    """Map each publishable workspace crate to its workspace dependencies.

    Development dependencies are ignored: Cargo drops the path-only ones this
    workspace uses from a published manifest, so they impose no order.
    """
    packages = {
        package["name"]: package
        for package in _workspace_packages()
        if _is_publishable(package)
    }
    return {
        name: {
            dependency["name"]
            for dependency in package["dependencies"]
            if dependency["kind"] != "dev" and dependency["name"] in packages
        }
        for name, package in packages.items()
    }


def _dry_run(target: str) -> str:
    """Return the commands ``target`` would run."""
    return subprocess.run(
        ["make", "--no-print-directory", "--dry-run", target],
        cwd=REPOSITORY_ROOT,
        capture_output=True,
        text=True,
        check=True,
    ).stdout


def test_every_publishable_crate_is_listed_after_its_dependencies() -> None:
    """The order names every publishable crate, dependencies first."""
    order = _load(LADING_CONFIG)["publish"]["order"]
    publishable = _publishable_packages()
    assert sorted(order) == sorted(publishable), "lading.toml lists the wrong crates"
    for position, crate in enumerate(order):
        unmet = publishable[crate] - set(order[:position])
        assert not unmet, f"{crate} is published before {sorted(unmet)}"


def test_the_published_crates_share_one_version() -> None:
    """One tag releases the workspace, so its crates carry one version."""
    root = _load(REPOSITORY_ROOT / "Cargo.toml")["package"]["version"]
    companion = _load(REPOSITORY_ROOT / "wireframe_testing" / "Cargo.toml")
    assert companion["package"]["version"] == root
    assert companion["dependencies"]["wireframe"]["version"] == root, (
        "wireframe_testing must depend on the release it ships with"
    )


def test_the_package_target_covers_the_release_order() -> None:
    """`make package-crates` packages what the release publishes."""
    order = _load(LADING_CONFIG)["publish"]["order"]
    command = _dry_run("package-crates")
    assert "package --locked" in command
    assert re.findall(r"-p (\S+)", command) == order


def test_the_rehearsal_uses_the_locked_tool_and_uploads_nothing() -> None:
    """`make publish-check` runs the locked lading, without ``--live``."""
    command = _dry_run("publish-check")
    assert "--project tools/lading --locked lading publish" in command
    assert "--live" not in command


def test_the_tool_pin_is_exact_and_hash_locked() -> None:
    """The project names one lading release and the lock hashes every package."""
    (requirement,) = _load(TOOL_PROJECT / "pyproject.toml")["project"]["dependencies"]
    assert EXACT_LADING.fullmatch(requirement), f"{requirement} is not an exact pin"
    locked = _load(TOOL_PROJECT / "uv.lock")["package"]
    versions = {package["name"]: package["version"] for package in locked}
    assert requirement == f"lading=={versions['lading']}"
    for package in locked:
        if "registry" not in package["source"]:
            assert package["name"] == "wireframe-release-tools"
            continue
        files = [*package.get("wheels", []), *([package["sdist"]] if "sdist" in package else [])]
        assert files and all(entry["hash"].startswith("sha256:") for entry in files), (
            f"{package['name']} is not hash-locked"
        )


def test_the_bump_excludes_exactly_the_unpublished_crates() -> None:
    """A version bump leaves alone only the crates that are never published."""
    unpublished = sorted(
        package["name"]
        for package in _workspace_packages()
        if not _is_publishable(package)
    )
    assert sorted(_load(LADING_CONFIG)["bump"]["exclude"]) == unpublished


def test_patches_are_stripped_per_crate() -> None:
    """Each crate is published without the workspace's local patches."""
    assert _load(LADING_CONFIG)["publish"]["strip_patches"] == "per-crate"


def test_the_preflight_runs_the_unit_tests_only() -> None:
    """A workstation rehearsal runs the unit tests, not the whole suite."""
    assert _load(LADING_CONFIG)["preflight"]["unit_tests_only"] is True


def test_no_preflight_skip_is_configured_for_everyone() -> None:
    """Skipping the pre-flight belongs to the release step, never to the file.

    Any key naming a skip is refused, whatever its value, so a renamed or
    narrower skip cannot slip past a check for one spelling.
    """
    preflight = _load(LADING_CONFIG).get("preflight", {})
    skips = [key for key in preflight if "skip" in key.lower()]
    assert not skips, f"lading.toml configures {skips} for every run"


@pytest.mark.parametrize("target", ["package-crates", "publish-check"])
def test_release_builds_displace_the_development_rustflags(target: str) -> None:
    """Release packaging overrides ``RUSTFLAGS`` with a blank, non-empty value.

    The blank value displaces the `rustflags` in `.cargo/config.toml`, which
    name a linker that a release runner does not install, from the build that
    verifies each archive.
    """
    flags = re.search(r'RUSTFLAGS="([^"]*)"', _dry_run(target))
    assert flags, f"{target} does not set RUSTFLAGS"
    assert flags.group(1) and not flags.group(1).strip(), (
        f"{target} must set a blank, non-empty RUSTFLAGS"
    )
