"""Prove the nextest override reaches the Cargo target it names.

The override in ``.config/nextest.toml`` is only useful while Cargo still
discovers a test target called ``compile_error`` in package ``wireframe`` and
that target still defines ``compile_tests``. A rename, a move into a directory
form, or an ``autotests = false`` declaration that no longer matches would
leave the filter matching nothing, and the test would fall back to the short
allowance with nothing reporting it.

The repository assertion runs the real reader over the workspace. The fixture
crates below run the same reader over constructed packages, so each accepted
and rejected discovery shape is proved rather than assumed.
"""

from __future__ import annotations

import functools
import subprocess
import tomllib
import typing as typ
from copy import deepcopy
from pathlib import Path

import pytest

import cargo_targets
from cargo_targets import CargoTestTarget, read_test_targets
from nextest_policy import (
    drives_trybuild,
    is_loom_gated,
    override_target_violations,
)
from runner_placement_reader import REPO_ROOT

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
def repository_config() -> dict[str, typ.Any]:
    """Load the repository nextest configuration once."""
    with NEXTEST_CONFIG.open("rb") as config_file:
        return tomllib.load(config_file)


@functools.cache
def repository_targets() -> tuple[CargoTestTarget, ...]:
    """Discover the workspace's test targets once."""
    return read_test_targets(MANIFEST)


def assert_no_override_violations(
    config: dict[str, typ.Any],
    targets: typ.Sequence[CargoTestTarget],
    source_reader: typ.Callable[[Path], str],
) -> None:
    """Require every override to resolve to an uncovered trybuild target."""
    violations = override_target_violations(config, targets, source_reader)
    assert violations == [], "\n".join(violations)


def test_repository_override_resolves_to_the_compile_error_target() -> None:
    """The checked-in override names a target Cargo actually discovers."""
    assert_no_override_violations(
        repository_config(), repository_targets(), read_source
    )
    target = next(
        target for target in repository_targets() if target.name == "compile_error"
    )
    assert target.package == "wireframe"
    assert target.src_path == REPO_ROOT / "tests" / "compile_error.rs"


def test_repository_loom_target_is_excluded_by_its_crate_gate() -> None:
    """The Loom harness drives trybuild but is not part of the ordinary run."""
    loom = next(
        target
        for target in repository_targets()
        if target.name == "loom_api_surface"
    )
    source = read_source(loom.src_path)
    assert drives_trybuild(source)
    assert is_loom_gated(source)


def write_crate(
    root: Path,
    *,
    package_extra: typ.Sequence[str] = (),
    test_declarations: typ.Sequence[str] = (),
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


@pytest.mark.parametrize(
    ("case", "package_extra", "test_declarations", "files", "is_clean"),
    [
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
    ],
)
def test_fixture_crate_discovery(
    tmp_path: Path,
    case: str,
    package_extra: typ.Sequence[str],
    test_declarations: typ.Sequence[str],
    files: dict[str, str],
    is_clean: bool,
) -> None:
    """Each discovery shape is accepted or rejected by the real reader."""
    manifest = write_crate(
        tmp_path,
        package_extra=package_extra,
        test_declarations=test_declarations,
        files=files,
    )
    targets = read_test_targets(manifest)
    violations = override_target_violations(
        repository_config(), targets, read_source
    )
    if is_clean:
        assert violations == [], f"{case}: {violations}"
    else:
        assert violations, f"{case} should be rejected"


def test_nested_module_violation_names_the_qualified_path(tmp_path: Path) -> None:
    """The rejection reports the module path the exact matcher cannot select."""
    manifest = write_crate(
        tmp_path,
        files={"tests/compile_error.rs": NESTED_MODULE_TEST_SOURCE},
    )
    violations = override_target_violations(
        repository_config(), read_test_targets(manifest), read_source
    )
    assert any(
        "contracts::compile_tests" in violation for violation in violations
    ), violations


def _non_trybuild_target(targets: typ.Sequence[CargoTestTarget]) -> CargoTestTarget:
    """Return a discovered target that does not drive trybuild."""
    return next(
        target
        for target in targets
        if not drives_trybuild(read_source(target.src_path))
        and not is_loom_gated(read_source(target.src_path))
    )


def _set_filter(
    config: dict[str, typ.Any], _targets: typ.Sequence[CargoTestTarget], text: str
) -> None:
    """Replace the one override's filter with ``text``."""
    config["profile"]["default"]["overrides"][0]["filter"] = text


def _without_binary(
    config: dict[str, typ.Any], targets: typ.Sequence[CargoTestTarget]
) -> None:
    """Drop the binary matcher from the filter."""
    _set_filter(config, targets, "package(=wireframe) & test(=compile_tests)")


def _without_package(
    config: dict[str, typ.Any], targets: typ.Sequence[CargoTestTarget]
) -> None:
    """Drop the package matcher from the filter."""
    _set_filter(config, targets, "binary(=compile_error) & test(=compile_tests)")


def _without_test(
    config: dict[str, typ.Any], targets: typ.Sequence[CargoTestTarget]
) -> None:
    """Drop the test matcher from the filter."""
    _set_filter(config, targets, "package(=wireframe) & binary(=compile_error)")


def _wrong_package(
    config: dict[str, typ.Any], targets: typ.Sequence[CargoTestTarget]
) -> None:
    """Name a package that discovers no such target."""
    _set_filter(
        config,
        targets,
        "package(=other) & binary(=compile_error) & test(=compile_tests)",
    )


def _non_exact_binary(
    config: dict[str, typ.Any], targets: typ.Sequence[CargoTestTarget]
) -> None:
    """State the binary matcher as a substring rather than an exact match."""
    _set_filter(
        config,
        targets,
        "package(=wireframe) & binary(compile_error) & test(=compile_tests)",
    )


def _extra_override(
    config: dict[str, typ.Any], targets: typ.Sequence[CargoTestTarget]
) -> None:
    """Append an override for a target that does not drive trybuild."""
    target = _non_trybuild_target(targets)
    config["profile"]["default"]["overrides"].append({
        "filter": (
            f"package(={target.package}) & binary(={target.name})"
            " & test(=some_test)"
        ),
        "slow-timeout": {
            "period": "60s",
            "terminate-after": 8,
            "grace-period": "5s",
        },
    })


_OVERRIDE_MUTATIONS: typ.Final = {
    "no_binary": _without_binary,
    "no_package": _without_package,
    "no_test": _without_test,
    "wrong_package": _wrong_package,
    "non_exact": _non_exact_binary,
    "extra_override": _extra_override,
}


def _mutate_override_config(
    mutation: str,
    config: dict[str, typ.Any],
    targets: typ.Sequence[CargoTestTarget],
) -> None:
    """Apply one override-config mutation in place."""
    try:
        mutate = _OVERRIDE_MUTATIONS[mutation]
    except KeyError:
        message = f"unknown override mutation {mutation!r}"
        raise AssertionError(message) from None
    mutate(config, targets)


def _source_reader_with(
    extra: dict[Path, str],
) -> typ.Callable[[Path], str]:
    """Return a reader that serves ``extra`` before the filesystem."""

    def read(path: Path) -> str:
        return extra[path] if path in extra else read_source(path)

    return read


@pytest.mark.parametrize(
    "mutation",
    [
        "no_binary",
        "no_package",
        "no_test",
        "wrong_package",
        "non_exact",
        "extra_override",
        "uncovered_trybuild_target",
    ],
)
def test_override_mutations_are_rejected(mutation: str) -> None:
    """Each mutation alone leaves an override unproved or a target uncovered."""
    config = deepcopy(repository_config())
    targets: tuple[CargoTestTarget, ...] = repository_targets()
    extra_sources: dict[Path, str] = {}
    if mutation == "uncovered_trybuild_target":
        synthetic = CargoTestTarget(
            package="wireframe",
            name="trybuild_extra",
            kinds=("test",),
            src_path=Path("/synthetic/trybuild_extra.rs"),
        )
        targets = (*targets, synthetic)
        extra_sources[synthetic.src_path] = TRYSBUILD_SOURCE
    else:
        _mutate_override_config(mutation, config, targets)
    with pytest.raises(AssertionError):
        assert_no_override_violations(
            config, targets, _source_reader_with(extra_sources)
        )


def test_missing_cargo_is_reported(monkeypatch: pytest.MonkeyPatch, tmp_path: Path) -> None:
    """Discovery fails loudly when ``cargo`` cannot be executed."""
    def raise_missing(*args: object, **kwargs: object) -> None:
        raise FileNotFoundError("cargo")

    monkeypatch.setattr(cargo_targets.subprocess, "run", raise_missing)
    with pytest.raises(cargo_targets.CargoUnavailableError, match="cargo"):
        read_test_targets(tmp_path / "Cargo.toml")


def test_metadata_failure_is_reported(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    """A non-zero ``cargo metadata`` exit is reported with its stderr."""
    def fail(*args: object, **kwargs: object) -> None:
        raise subprocess.CalledProcessError(1, "cargo", stderr="boom")

    monkeypatch.setattr(cargo_targets.subprocess, "run", fail)
    with pytest.raises(cargo_targets.CargoMetadataError, match="boom"):
        read_test_targets(tmp_path / "Cargo.toml")