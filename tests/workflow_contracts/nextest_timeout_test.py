"""Pin the checked-in nextest timeout policy and prove its ordering.

`compile_tests` gets an eight-minute override while the global ceiling stays at
ten, and both coverage workflows state a 900 s Cargo watchdog. That policy
applies to noncoverage nextest runs. The coverage invocation compiles the
compile-time harnesses out with `cfg(coverage)`, so the watchdog bounds the
runtime suite there rather than the compile test; the execution paths
themselves are pinned in `compile_gate_contract_test`.
"""

from __future__ import annotations

import tomllib
import typing as typ
from copy import deepcopy
from pathlib import Path

import pytest

from runner_placement_reader import REPO_ROOT, workflows
from timeout_policy import (
    COVERAGE_ACTION_NAME,
    coverage_action_name,
    timeout_tier_violations,
)

NEXTEST_CONFIG: Path = REPO_ROOT / ".config" / "nextest.toml"
COMPILE_TEST_FILTER: str = (
    "package(=wireframe) & binary(=compile_error) & test(=compile_tests)"
)


def nextest_config() -> dict[str, typ.Any]:
    """Load the repository nextest configuration."""
    with NEXTEST_CONFIG.open("rb") as config_file:
        return tomllib.load(config_file)


def coverage_action_step() -> tuple[dict[str, typ.Any], dict[str, typ.Any]]:
    """Return the coverage job and its pinned generate-coverage step."""
    workflow = typ.cast("dict[str, typ.Any]", workflows()["ci.yml"])
    job = typ.cast("dict[str, typ.Any]", workflow["jobs"]["build-test"])
    steps = typ.cast("list[dict[str, typ.Any]]", job["steps"])
    matches = [
        step
        for step in steps
        if "generate-coverage" in str(step.get("uses", ""))
    ]
    assert len(matches) == 1, "CI must have one generate-coverage step"
    return job, matches[0]


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
    assert 60 * override["slow-timeout"]["terminate-after"] < 10 * 60, (
        "the override deadline must stay inside the whole-run ceiling"
    )


def assert_no_timeout_violations(
    config: dict[str, typ.Any], workflow_documents: dict[str, dict[str, typ.Any]]
) -> None:
    """Require every timeout tier to be stated, ordered and reachable."""
    violations = timeout_tier_violations(config, workflow_documents)
    assert violations == [], "\n".join(violations)


def test_repository_timeout_tiers_are_proved_and_ordered() -> None:
    """The repository states every tier and orders them widest outwards."""
    config = nextest_config()
    workflow_documents = workflows()
    assert_no_timeout_violations(config, workflow_documents)
    for workflow_name, job_id in (
        ("ci.yml", "build-test"),
        ("coverage-main.yml", "coverage-upload"),
    ):
        watchdog = _coverage_step(workflow_name, job_id, workflow_documents)["with"][
            "cargo-wait-timeout"
        ]
        assert watchdog == "900", (
            f"{workflow_name} watchdog {watchdog!r} must stay at the reviewed 900s"
        )


def _coverage_step(
    workflow_name: str, job_id: str, workflow_documents: dict[str, dict[str, typ.Any]]
) -> dict[str, typ.Any]:
    """Return the one generate-coverage step in a workflow job."""
    steps = workflow_documents[workflow_name]["jobs"][job_id]["steps"]
    return next(
        step for step in steps if "generate-coverage" in str(step.get("uses", ""))
    )


def _drop_default_terminate_after(
    config: dict[str, typ.Any], _workflows: dict[str, dict[str, typ.Any]]
) -> None:
    """Remove the default per-test deadline."""
    config["profile"]["default"]["slow-timeout"].pop("terminate-after")


def _lengthen_override_deadline(
    config: dict[str, typ.Any], _workflows: dict[str, dict[str, typ.Any]]
) -> None:
    """Push the override deadline up to the whole-run ceiling."""
    config["profile"]["default"]["overrides"][0]["slow-timeout"][
        "terminate-after"
    ] = 10


def _drop_global_timeout(
    config: dict[str, typ.Any], _workflows: dict[str, dict[str, typ.Any]]
) -> None:
    """Remove the whole-run ceiling."""
    config["profile"]["default"].pop("global-timeout")


def _drop_watchdog(
    _config: dict[str, typ.Any], workflows: dict[str, dict[str, typ.Any]]
) -> None:
    """Remove the watchdog from one coverage workflow."""
    _coverage_step("coverage-main.yml", "coverage-upload", workflows)["with"].pop(
        "cargo-wait-timeout"
    )


def _disagree_on_watchdog(
    _config: dict[str, typ.Any], workflows: dict[str, dict[str, typ.Any]]
) -> None:
    """State a different watchdog in one coverage workflow."""
    _coverage_step("coverage-main.yml", "coverage-upload", workflows)["with"][
        "cargo-wait-timeout"
    ] = "901"


def _widen_watchdog(
    _config: dict[str, typ.Any], workflows: dict[str, dict[str, typ.Any]]
) -> None:
    """State a watchdog beyond the enclosing job ceiling."""
    for workflow_name, job_id in (
        ("ci.yml", "build-test"),
        ("coverage-main.yml", "coverage-upload"),
    ):
        _coverage_step(workflow_name, job_id, workflows)["with"][
            "cargo-wait-timeout"
        ] = "1800"


def _drop_job_ceiling(
    _config: dict[str, typ.Any], workflows: dict[str, dict[str, typ.Any]]
) -> None:
    """Remove the coverage job's stated ceiling."""
    workflows["ci.yml"]["jobs"]["build-test"].pop("timeout-minutes")


def _drop_coverage_invocation(
    _config: dict[str, typ.Any], workflows: dict[str, dict[str, typ.Any]]
) -> None:
    """Remove the coverage step from one workflow, keeping the workflow."""
    job = workflows["ci.yml"]["jobs"]["build-test"]
    job["steps"] = [
        step
        for step in job["steps"]
        if "generate-coverage" not in str(step.get("uses", ""))
    ]


def _rename_coverage_action(
    _config: dict[str, typ.Any], workflows: dict[str, dict[str, typ.Any]]
) -> None:
    """Point one workflow at a differently named action, keeping the steps.

    Every job of the workflow is renamed, the warm-up's build-only step
    included: the mutation must leave the workflow with no coverage invocation.
    """
    for job_id in ("coverage-upload", "coverage-warmup"):
        step = _coverage_step("coverage-main.yml", job_id, workflows)
        step["uses"] = (
            "leynos/shared-actions/.github/actions/generate-coverage-v2@abc123"
        )


_TIMEOUT_MUTATIONS: typ.Final = {
    "no_default_terminate_after": _drop_default_terminate_after,
    "override_terminate_after_10": _lengthen_override_deadline,
    "no_global_timeout": _drop_global_timeout,
    "missing_watchdog": _drop_watchdog,
    "different_watchdogs": _disagree_on_watchdog,
    "watchdog_1800": _widen_watchdog,
    "no_job_ceiling": _drop_job_ceiling,
    "missing_coverage_invocation": _drop_coverage_invocation,
    "renamed_coverage_action": _rename_coverage_action,
}


def _mutate_timeout_tiers(
    mutation: str,
    config: dict[str, typ.Any],
    workflow_documents: dict[str, dict[str, typ.Any]],
) -> None:
    """Apply one timeout-tier mutation in place."""
    try:
        mutate = _TIMEOUT_MUTATIONS[mutation]
    except KeyError:
        message = f"unknown timeout mutation {mutation!r}"
        raise AssertionError(message) from None
    mutate(config, workflow_documents)


@pytest.mark.parametrize(
    "mutation",
    [
        "no_default_terminate_after",
        "override_terminate_after_10",
        "no_global_timeout",
        "missing_watchdog",
        "different_watchdogs",
        "watchdog_1800",
        "no_job_ceiling",
        "missing_coverage_invocation",
        "renamed_coverage_action",
    ],
)
def test_timeout_tier_mutations_break_the_chain(mutation: str) -> None:
    """Each mutation alone breaks an ordering or leaves a tier unstated."""
    config = deepcopy(nextest_config())
    workflow_documents = deepcopy(workflows())
    _mutate_timeout_tiers(mutation, config, workflow_documents)
    with pytest.raises(AssertionError):
        assert_no_timeout_violations(config, workflow_documents)
