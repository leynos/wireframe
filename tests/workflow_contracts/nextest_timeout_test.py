"""Contracts for the coverage lane's cargo-nextest timeout policy."""

from __future__ import annotations

import tomllib
import typing as typ
from pathlib import Path

import pytest

from runner_placement_reader import REPO_ROOT, workflows

NEXTEST_CONFIG: Path = REPO_ROOT / ".config" / "nextest.toml"
#: The action this test is about, without its version pin.
COVERAGE_ACTION_NAME: str = "leynos/shared-actions/.github/actions/generate-coverage"
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


def coverage_action_name(step: dict[str, typ.Any]) -> str:
    """Return the action a step invokes, without its version pin.

    The pin itself is not this contract's business: ``shared_actions_test``
    already requires every shared-actions invocation to carry a full commit
    SHA and to agree on one revision across the workflow tree. Repeating the
    literal SHA here made a dependabot bump of that shared revision fail a
    timeout test, because the two had to be updated together.

    Parameters
    ----------
    step
        One parsed workflow step.

    Returns
    -------
    str
        The ``uses`` value with its ``@`` pin removed.

    Examples
    --------
    >>> coverage_action_name(
    ...     {"uses": "leynos/shared-actions/.github/actions/generate-coverage@abc"}
    ... )
    'leynos/shared-actions/.github/actions/generate-coverage'
    """
    uses = str(step.get("uses", ""))
    name, separator, version = uses.partition("@")
    assert separator and name, f"action invocation {uses!r} carries no action name"
    assert version, f"action invocation {uses!r} carries an empty version pin"
    return name


@pytest.mark.parametrize(
    ("uses", "expected"),
    [
        (
            "leynos/shared-actions/.github/actions/generate-coverage@abc123",
            "leynos/shared-actions/.github/actions/generate-coverage",
        ),
        ("actions/checkout@v4", "actions/checkout"),
    ],
)
def test_coverage_action_name_strips_the_version_pin(uses: str, expected: str) -> None:
    """A pinned invocation yields the action path alone."""
    assert coverage_action_name({"uses": uses}) == expected


@pytest.mark.parametrize(
    "uses",
    [
        "actions/checkout",
        "actions/checkout@",
        "@v4",
        "",
    ],
)
def test_coverage_action_name_rejects_an_invocation_without_a_pin(uses: str) -> None:
    """A ``uses`` value with no pin, an empty pin, or no name is refused."""
    with pytest.raises(AssertionError):
        coverage_action_name({"uses": uses})


def test_coverage_action_can_discover_the_checked_in_config() -> None:
    """The coverage action uses the repository config without override."""
    job, step = coverage_action_step()

    assert NEXTEST_CONFIG.is_file()
    assert coverage_action_name(step) == COVERAGE_ACTION_NAME
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
