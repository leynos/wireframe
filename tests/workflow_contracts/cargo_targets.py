"""Read Cargo's view of the test targets a nextest filter must name.

A nextest override names a package and a binary. Nothing in nextest's own
configuration proves that pair exists: a target renamed, or moved from
``tests/name.rs`` to ``tests/name/mod.rs``, silently stops matching and the
test falls back to the short allowance. ``cargo metadata`` is the authority on
what Cargo will actually build, so the discovery contract asks it directly
rather than trusting a list written beside the filter.

This module only reads. The decision about which targets must carry an
override lives in ``nextest_policy``, and the assertions that hold the two
together live in the tests that call it.
"""

from __future__ import annotations

import dataclasses
import json
import subprocess
from pathlib import Path


class CargoUnavailableError(RuntimeError):
    """``cargo`` could not be executed, so discovery cannot be answered."""


class CargoMetadataError(RuntimeError):
    """``cargo metadata`` ran but did not describe the workspace."""


@dataclasses.dataclass(frozen=True)
class CargoTestTarget:
    """One test target Cargo would build.

    Attributes
    ----------
    package
        The owning package's name.
    name
        The target name, which is the ``binary`` a nextest filter matches.
    kinds
        The target's ``kind`` list exactly as Cargo reported it.
    src_path
        The source file Cargo would compile.
    """

    package: str
    name: str
    kinds: tuple[str, ...]
    src_path: Path


def read_test_targets(manifest_path: Path) -> tuple[CargoTestTarget, ...]:
    """Return every test target ``cargo metadata`` discovers.

    The manifest's directory is passed explicitly as the working directory so
    a fixture crate anywhere on disk is read as itself rather than as part of
    whichever workspace happens to contain the caller.

    Parameters
    ----------
    manifest_path
        The ``Cargo.toml`` to inspect.

    Returns
    -------
    tuple[CargoTestTarget, ...]
        Test targets in Cargo's order.

    Raises
    ------
    CargoUnavailableError
        When ``cargo`` cannot be executed. Discovery fails loudly rather than
        returning an empty tuple a caller might read as "no targets".
    CargoMetadataError
        When ``cargo metadata`` exits non-zero or emits no JSON document.

    Examples
    --------
    >>> targets = read_test_targets(Path("Cargo.toml"))  # doctest: +SKIP
    >>> any(target.name == "compile_error" for target in targets)
    True
    """
    argv = [
        "cargo",
        "metadata",
        "--no-deps",
        "--format-version",
        "1",
        "--offline",
        "--manifest-path",
        str(manifest_path),
    ]
    try:
        completed = subprocess.run(
            argv,
            cwd=manifest_path.parent,
            capture_output=True,
            text=True,
            check=True,
        )
    except FileNotFoundError as exc:
        message = (
            "cargo is not available on PATH, so the test targets a nextest "
            "override names cannot be discovered"
        )
        raise CargoUnavailableError(message) from exc
    except subprocess.CalledProcessError as exc:
        message = (
            f"cargo metadata failed for {manifest_path}: "
            f"{exc.stderr.strip() or exc.stdout.strip()}"
        )
        raise CargoMetadataError(message) from exc
    return tuple(_test_targets_from_metadata(completed.stdout, manifest_path))


def _test_targets_from_metadata(raw: str, manifest_path: Path) -> list[CargoTestTarget]:
    """Extract the test targets from one ``cargo metadata`` document."""
    try:
        document = json.loads(raw)
    except json.JSONDecodeError as exc:
        message = f"cargo metadata for {manifest_path} did not emit JSON: {exc}"
        raise CargoMetadataError(message) from exc
    return [
        CargoTestTarget(
            package=str(package["name"]),
            name=str(target["name"]),
            kinds=tuple(str(kind) for kind in target.get("kind", [])),
            src_path=Path(str(target["src_path"])),
        )
        for package in document.get("packages", [])
        for target in package.get("targets", [])
        if "test" in target.get("kind", [])
    ]