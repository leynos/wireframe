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

import subprocess
import typing as typ
from pathlib import Path

import pytest

import cargo_targets
from cargo_targets import read_test_targets
from nextest_contract_support import (
    DISCOVERY_CASES,
    NESTED_MODULE_TEST_SOURCE,
    assert_no_override_violations,
    read_source,
    repository_config,
    repository_targets,
    write_crate,
)
from nextest_policy import override_target_violations
from rust_source_scanning import drives_trybuild, is_loom_gated
from runner_placement_reader import REPO_ROOT


def test_repository_override_resolves_to_the_compile_error_target() -> None:
    """The checked-in override names a target Cargo actually discovers."""
    assert_no_override_violations(
        repository_config(), repository_targets(), read_source
    )
    target = next(
        target for target in repository_targets() if target.name == "compile_error"
    )
    assert target.package == "wireframe", "the target must belong to the named package"
    assert target.src_path == REPO_ROOT / "tests" / "compile_error.rs", (
        "the target must be the checked-in harness, not a directory main"
    )


def test_repository_loom_target_is_excluded_by_its_crate_gate() -> None:
    """The Loom harness drives trybuild but is not part of the ordinary run."""
    loom = next(
        target
        for target in repository_targets()
        if target.name == "loom_api_surface"
    )
    source = read_source(loom.src_path)
    assert drives_trybuild(source), "the Loom harness must drive trybuild"
    assert is_loom_gated(source), "the Loom harness must be gated out of the run"


@pytest.mark.parametrize(
    ("case", "package_extra", "test_declarations", "files", "is_clean"),
    DISCOVERY_CASES,
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


def test_missing_cargo_is_reported(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
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
