"""Contract tests that the Loom lane has models to run.

``loom_lane_test`` holds the lane to ``cargo test -p wireframe-loom`` under
``--cfg loom``. That is necessary and not sufficient: ``cargo test`` passes
when it runs zero tests, so a model file whose gate no longer names
``loom``, a test compiled out by its own ``cfg``, an ``#[ignore]``, or a
manifest that stops discovering ``tests/`` each leave the lane green with
nothing scheduled. These tests close those routes.

The rules are pure functions of a file's text, so the synthetic cases below
drive the same code the sweep runs; a rule exercised only over files that
already satisfy it passes whether or not it discriminates.

Run via ``make test-workflow-contracts``.
"""

from __future__ import annotations

import re
import tomllib
import typing as typ
from pathlib import Path

import pytest

PACKAGE_DIR: typ.Final = (
    Path(__file__).resolve().parents[2] / "crates" / "wireframe-loom"
)
MODEL_DIR: typ.Final = PACKAGE_DIR / "tests"

#: The one conditional-compilation attribute a model file may carry.
LOOM_GATE: typ.Final = "#![cfg(loom)]"

#: Any ``cfg`` or ``cfg_attr`` attribute, inner or outer. A second one can
#: compile a test out, or ignore it, under ``--cfg loom``.
CFG_ATTRIBUTE: typ.Final = re.compile(r"#!?\[\s*cfg(?:_attr)?\s*\(")

#: A test function's attribute, as the model files write it.
TEST_ATTRIBUTE: typ.Final = re.compile(r"^\s*#\[(?:test|rstest)\]\s*$", re.MULTILINE)

#: An ``#[ignore]`` in any form; libtest skips the test and reports success.
IGNORE_ATTRIBUTE: typ.Final = re.compile(r"#\[\s*ignore\b")

#: The Loom entry point. A test that never calls it schedules nothing.
MODEL_CALL: typ.Final = re.compile(r"\bmodel\s*\(")


def model_violations(text: str) -> list[str]:
    """Return every way a model file could run no model under the lane.

    Examples
    --------
    >>> model_violations("#![cfg(loom)]\\n#[test]\\nfn a() { model(|| {}); }\\n")
    []
    """
    violations = []
    gates = [
        text[match.start() :].split("\n", 1)[0].strip()
        for match in CFG_ATTRIBUTE.finditer(text)
    ]
    if gates != [LOOM_GATE]:
        violations.append(
            f"must carry exactly one cfg attribute, {LOOM_GATE}; found {gates}"
        )
    if not TEST_ATTRIBUTE.search(text):
        violations.append("declares no #[test] or #[rstest] function")
    if IGNORE_ATTRIBUTE.search(text):
        violations.append("ignores a test, which libtest skips and reports as success")
    if not MODEL_CALL.search(text):
        violations.append("never calls loom::model, so no test schedules anything")
    return violations


def manifest_violations(manifest: dict[str, object]) -> list[str]:
    """Return every way the package manifest could stop discovering its models.

    Examples
    --------
    >>> manifest_violations({"package": {"name": "wireframe-loom"}})
    []
    """
    package = manifest.get("package")
    violations = []
    if isinstance(package, dict) and "autotests" in package:
        violations.append("sets autotests, which can stop tests/ being discovered")
    if "test" in manifest:
        violations.append(
            "declares [[test]] targets, which can drop or disable a model file"
        )
    return violations


def _model_files() -> list[Path]:
    """Return the package's integration-test files, which are its models."""
    return sorted(MODEL_DIR.glob("*.rs"))


def test_the_package_holds_model_files() -> None:
    """The swept set is non-empty.

    A rule over an empty directory reports every file as conforming, so a
    package whose models were all deleted would pass the sweep below.
    """
    assert _model_files(), f"{MODEL_DIR} holds no model files, so the lane runs nothing"


@pytest.mark.parametrize("path", _model_files(), ids=lambda path: path.name)
def test_each_model_file_runs_a_model_under_loom(path: Path) -> None:
    """Each model file compiles under ``--cfg loom`` and runs a model there."""
    violations = model_violations(path.read_text(encoding="utf-8"))
    assert not violations, f"{path.name} " + "; ".join(violations)


def test_the_manifest_discovers_every_model_file() -> None:
    """Cargo's default discovery builds every file in ``tests/``."""
    manifest = tomllib.loads((PACKAGE_DIR / "Cargo.toml").read_text(encoding="utf-8"))
    violations = manifest_violations(manifest)
    assert not violations, "wireframe-loom's manifest " + "; ".join(violations)


CONFORMING: typ.Final = (
    "//! Doc.\n#![cfg(loom)]\n\n#[test]\nfn a() {\n    model(|| {});\n}\n"
)


def test_the_conforming_shape_is_accepted() -> None:
    """The shape the model files use reports no violation.

    Without this the rejection cases below would pass against a rule that
    refused everything.
    """
    assert not model_violations(CONFORMING)


@pytest.mark.parametrize(
    ("text", "fragment"),
    [
        pytest.param(
            CONFORMING.replace("#![cfg(loom)]", "#![cfg(loom_x)]"),
            "exactly one cfg",
            id="gate-renamed",
        ),
        pytest.param(
            CONFORMING.replace("#![cfg(loom)]\n", ""),
            "exactly one cfg",
            id="gate-removed",
        ),
        pytest.param(
            CONFORMING.replace("#[test]", "#[test]\n#[cfg(any())]"),
            "exactly one cfg",
            id="test-compiled-out",
        ),
        pytest.param(
            CONFORMING.replace("#[test]", "#[test]\n#[cfg_attr(loom, ignore)]"),
            "exactly one cfg",
            id="ignored-under-loom",
        ),
        pytest.param(
            CONFORMING.replace("#[test]", "#[test]\n#[ignore]"),
            "ignores a test",
            id="ignored",
        ),
        pytest.param(CONFORMING.replace("#[test]\n", ""), "no #[test]", id="no-test"),
        pytest.param(
            CONFORMING.replace("model(|| {});", "let _ = 1;"),
            "never calls",
            id="no-model",
        ),
    ],
)
def test_a_model_file_that_runs_nothing_is_rejected(text: str, fragment: str) -> None:
    """Each way of leaving the lane green with no model scheduled is named."""
    violations = model_violations(text)
    assert any(fragment in violation for violation in violations), violations


@pytest.mark.parametrize(
    ("manifest", "fragment"),
    [
        pytest.param(
            {"package": {"autotests": False}}, "autotests", id="autotests-off"
        ),
        pytest.param(
            {"package": {}, "test": [{"name": "push_dlq", "test": False}]},
            "[[test]]",
            id="test-table",
        ),
    ],
)
def test_a_manifest_that_drops_models_is_rejected(
    manifest: dict[str, object], fragment: str
) -> None:
    """A manifest that stops Cargo discovering ``tests/`` is named."""
    violations = manifest_violations(manifest)
    assert any(fragment in violation for violation in violations), violations
