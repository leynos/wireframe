"""Shared fixtures for the nextest discovery and override-mutation contracts.

The repository assertion and the constructed fixture crates exercise the same
reader, so both test modules share this loading and fixture-writing code. The
source constants are the acceptance and rejection shapes a target can take:
each one is either a layout Cargo discovers with a crate-root ``compile_tests``
test, or a layout in which the checked-in exact matcher selects nothing.
"""

from __future__ import annotations

import copy
import functools
import tomllib
import typing as typ
from pathlib import Path

import pytest

from cargo_targets import CargoTestTarget, read_test_targets
from nextest_policy import override_target_violations
from runner_placement_reader import REPO_ROOT

if typ.TYPE_CHECKING:
    from collections.abc import Callable, Sequence

NEXTEST_CONFIG: Path = REPO_ROOT / ".config" / "nextest.toml"
MANIFEST: Path = REPO_ROOT / "Cargo.toml"

TRYSBUILD_SOURCE = """\
#[test]
fn compile_tests() {
    let _ = trybuild::TestCases::new();
}
"""

RENAMED_TEST_SOURCE = """\
#[test]
fn not_compile_tests() {
    let _ = trybuild::TestCases::new();
}
"""

# Nextest names this test `contracts::compile_tests`, so the exact matcher
# `test(=compile_tests)` selects nothing even though the source text contains
# the name.
NESTED_MODULE_TEST_SOURCE = """\
mod contracts {
    #[test]
    fn compile_tests() {
        let _ = trybuild::TestCases::new();
    }
}
"""

# A commented-out definition is not evidence that nextest will run anything.
COMMENTED_OUT_TEST_SOURCE = """\
// #[test]
// fn compile_tests() {
//     let _ = trybuild::TestCases::new();
// }
"""


def read_source(path: Path) -> str:
    """Read one target's source text."""
    return path.read_text(encoding="utf-8")


@functools.cache
def _repository_config() -> dict[str, typ.Any]:
    """Load the repository nextest configuration once."""
    with NEXTEST_CONFIG.open("rb") as config_file:
        return tomllib.load(config_file)


def repository_config() -> dict[str, typ.Any]:
    """Return a private copy of the repository nextest configuration.

    Callers mutate the configuration to build rejected shapes, so handing out
    the cached mapping would let one test corrupt every later reading.

    Returns
    -------
    dict[str, typ.Any]
        A fresh copy of the parsed ``.config/nextest.toml``.
    """
    return copy.deepcopy(_repository_config())


@functools.cache
def repository_targets() -> tuple[CargoTestTarget, ...]:
    """Discover the workspace's test targets once."""
    return read_test_targets(MANIFEST)


def assert_no_override_violations(
    config: dict[str, typ.Any],
    targets: Sequence[CargoTestTarget],
    source_reader: Callable[[Path], str],
) -> None:
    """Require every override to resolve to an uncovered trybuild target."""
    violations = override_target_violations(config, targets, source_reader)
    assert violations == [], "\n".join(violations)


def write_crate(
    root: Path,
    *,
    package_extra: Sequence[str] = (),
    test_declarations: Sequence[str] = (),
    files: dict[str, str],
) -> Path:
    """Write a dependency-free fixture crate and return its manifest path."""
    lines = [
        "[package]",
        'name = "wireframe"',
        'version = "0.0.0"',
        'edition = "2021"',
        *package_extra,
        "",
        "[workspace]",
        *test_declarations,
    ]
    (root / "src").mkdir(parents=True, exist_ok=True)
    (root / "src" / "lib.rs").write_text("", encoding="utf-8")
    manifest = root / "Cargo.toml"
    manifest.write_text("\n".join(lines) + "\n", encoding="utf-8")
    for relative, source in files.items():
        path = root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(source, encoding="utf-8")
    return manifest


DECLARED_MOD_RS = (
    "",
    "[[test]]",
    'name = "compile_error"',
    'path = "tests/compile_error/mod.rs"',
)
DECLARED_AUTOTESTS_OFF = (
    "",
    "[[test]]",
    'name = "compile_error"',
    'path = "tests/compile_error.rs"',
)
RENAMED_TARGET = (
    "",
    "[[test]]",
    'name = "renamed"',
    'path = "tests/compile_error.rs"',
)

#: ``(case, package declarations, test declarations, files, is_clean)`` for
#: every discovery shape the reader accepts or rejects.
DISCOVERY_CASES = [
    pytest.param(
        "automatic",
        (),
        (),
        {"tests/compile_error.rs": TRYSBUILD_SOURCE},
        True,
        id="automatic",
    ),
    pytest.param(
        "directory-main",
        (),
        (),
        {"tests/compile_error/main.rs": TRYSBUILD_SOURCE},
        True,
        id="directory-main",
    ),
    pytest.param(
        "declared-mod-rs",
        (),
        DECLARED_MOD_RS,
        {"tests/compile_error/mod.rs": TRYSBUILD_SOURCE},
        True,
        id="declared-mod-rs",
    ),
    pytest.param(
        "autotests-declared",
        ("autotests = false",),
        DECLARED_AUTOTESTS_OFF,
        {"tests/compile_error.rs": TRYSBUILD_SOURCE},
        True,
        id="autotests-declared",
    ),
    pytest.param(
        "renamed-target",
        (),
        RENAMED_TARGET,
        {"tests/compile_error.rs": TRYSBUILD_SOURCE},
        False,
        id="renamed-target",
    ),
    pytest.param(
        "directory-move-undeclared",
        (),
        (),
        {"tests/compile_error/mod.rs": TRYSBUILD_SOURCE},
        False,
        id="directory-move-undeclared",
    ),
    pytest.param(
        "autotests-undeclared",
        ("autotests = false",),
        (),
        {"tests/compile_error.rs": TRYSBUILD_SOURCE},
        False,
        id="autotests-undeclared",
    ),
    pytest.param(
        "renamed-test-function",
        (),
        (),
        {"tests/compile_error.rs": RENAMED_TEST_SOURCE},
        False,
        id="renamed-test-function",
    ),
    pytest.param(
        "nested-module-test",
        (),
        (),
        {"tests/compile_error.rs": NESTED_MODULE_TEST_SOURCE},
        False,
        id="nested-module-test",
    ),
    pytest.param(
        "commented-out-test",
        (),
        (),
        {"tests/compile_error.rs": COMMENTED_OUT_TEST_SOURCE},
        False,
        id="commented-out-test",
    ),
]
