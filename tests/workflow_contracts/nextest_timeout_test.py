"""Contracts for the coverage lane's cargo-nextest timeout policy."""

from __future__ import annotations

import tomllib
import typing as typ
from pathlib import Path

from runner_placement_reader import REPO_ROOT, workflows

NEXTEST_CONFIG: Path = REPO_ROOT / ".config" / "nextest.toml"
COVERAGE_ACTION: str = (
    "leynos/shared-actions/.github/actions/generate-coverage@"
    "e693d81d176d2305e2b736a0be0f9038085dec7f"
)
COMPILE_TEST_FILTER: str = (
    "package(=wireframe) & binary(=compile_error) & test(=compile_tests)"
)


def nextest_config() -> dict[str, typ.Any]:
    """Load the repository nextest configuration."""
    with NEXTEST_CONFIG.open("rb") as config_file:
        return tomllib.load(config_file)


def coverage_action_step() -> tuple[dict[str, typ.Any], dict[str, typ.Any]]:
    """Return the coverage job and its pinned generate-coverage step."""
    workflow = typ.cast(dict[str, typ.Any], workflows()["ci.yml"])
    job = typ.cast(dict[str, typ.Any], workflow["jobs"]["build-test"])
    steps = typ.cast(list[dict[str, typ.Any]], job["steps"])
    matches = [
        step
        for step in steps
        if "generate-coverage" in str(step.get("uses", ""))
    ]
    assert len(matches) == 1, "CI must have one generate-coverage step"
    return job, matches[0]


def test_coverage_action_can_discover_the_checked_in_config() -> None:
    """The pinned coverage action uses the repository config without override."""
    job, step = coverage_action_step()

    assert NEXTEST_CONFIG.is_file()
    assert step["uses"] == COVERAGE_ACTION
    assert "NEXTEST_CONFIG" not in (job.get("env") or {})
    assert "NEXTEST_CONFIG" not in (step.get("env") or {})


def test_action_timeout_and_coverage_ceiling_are_preserved() -> None:
    """Ordinary tests retain the action policy and whole-run ceiling."""
    default = nextest_config()["profile"]["default"]

    assert default["slow-timeout"] == {
        "period": "180s",
        "terminate-after": 1,
        "grace-period": "5s",
    }
    assert default["global-timeout"] == "10m"


def test_only_the_aggregate_compile_test_gets_the_extended_budget() -> None:
    """The finite eight-minute override matches only the nested UI suite."""
    default = nextest_config()["profile"]["default"]
    overrides = default["overrides"]

    assert len(overrides) == 1
    override = overrides[0]
    assert override["filter"] == COMPILE_TEST_FILTER
    assert override["slow-timeout"] == {
        "period": "60s",
        "terminate-after": 8,
        "grace-period": "5s",
    }
    assert 60 * override["slow-timeout"]["terminate-after"] < 10 * 60
