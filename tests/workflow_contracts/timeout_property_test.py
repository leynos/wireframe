"""Drive the timeout-tier invariants across generated chains.

The named fixture tests pin the repository's reviewed numbers. These property
cases draw a chain that satisfies every ordering rule, then break one link at a
time, so a policy that only accepts the current files fails here.

Chains are drawn as strictly ascending integer seconds and turned into the
duration strings nextest parses, so a generated failure names the tier whose
bound was moved.
"""

from __future__ import annotations

import dataclasses
import typing as typ

from hypothesis import HealthCheck, given, settings
from hypothesis import strategies as st
from timeout_policy import (
    COVERAGE_ACTION_NAME,
    COVERAGE_WORKFLOWS,
    timeout_tier_violations,
)

PROPERTY_SETTINGS = settings(
    max_examples=25, deadline=None, suppress_health_check=[HealthCheck.too_slow]
)


@dataclasses.dataclass(frozen=True)
class _Tiers:
    """A strictly ordered timeout chain, in seconds except the job ceiling."""

    default_deadline: int
    override_deadline: int
    global_timeout: int
    watchdog: int
    ceiling_minutes: int


@st.composite
def _ordered_tiers(draw: st.DrawFn) -> _Tiers:
    """Draw ``default < override < global < watchdog < ceiling * 60``."""
    values = sorted(
        draw(st.lists(st.integers(1, 10_000), min_size=4, max_size=4, unique=True))
    )
    default_deadline, override_deadline, global_timeout, watchdog = values
    ceiling_minutes = watchdog // 60 + draw(st.integers(1, 30))
    return _Tiers(
        default_deadline=default_deadline,
        override_deadline=override_deadline,
        global_timeout=global_timeout,
        watchdog=watchdog,
        ceiling_minutes=ceiling_minutes,
    )


def _slow_timeout(deadline: int) -> dict[str, typ.Any]:
    """Return a slow-timeout whose deadline is ``deadline`` seconds."""
    return {"period": f"{deadline}s", "terminate-after": 1, "grace-period": "1s"}


def _config(tiers: _Tiers) -> dict[str, typ.Any]:
    """Return a nextest configuration whose tiers satisfy the ordering."""
    return {
        "profile": {
            "default": {
                "slow-timeout": _slow_timeout(tiers.default_deadline),
                "global-timeout": f"{tiers.global_timeout}s",
                "overrides": [
                    {"slow-timeout": _slow_timeout(tiers.override_deadline)}
                ],
            }
        }
    }


def _workflow(watchdog: int, ceiling_minutes: int) -> dict[str, typ.Any]:
    """Return one coverage workflow stating the watchdog and job ceiling."""
    return {
        "jobs": {
            "coverage-upload": {
                "timeout-minutes": ceiling_minutes,
                "steps": [
                    {
                        "uses": f"{COVERAGE_ACTION_NAME}@0123456789abcdef",
                        "with": {"cargo-wait-timeout": str(watchdog)},
                    }
                ],
            }
        }
    }


def _workflows(
    watchdog: int, ceiling_minutes: int
) -> dict[str, dict[str, typ.Any]]:
    """Return both coverage workflows, each stating the same watchdog."""
    return {
        name: _workflow(watchdog, ceiling_minutes) for name in COVERAGE_WORKFLOWS
    }


def _first_step(workflow: dict[str, typ.Any]) -> dict[str, typ.Any]:
    """Return the only step of the workflow's only job."""
    (job,) = workflow["jobs"].values()
    (step,) = job["steps"]
    return step


@PROPERTY_SETTINGS
@given(tiers=_ordered_tiers())
def test_a_strictly_ordered_chain_reports_no_violation(tiers: _Tiers) -> None:
    """Every tier stated and ordered outwards must satisfy the policy."""
    violations = timeout_tier_violations(
        _config(tiers), _workflows(tiers.watchdog, tiers.ceiling_minutes)
    )
    assert violations == [], f"{tiers!r} must be accepted, got {violations!r}"


#: Single breaks in a valid chain, each with the diagnostic it must produce.
DEFECTS = st.sampled_from([
    "default_deadline",
    "override_ceiling",
    "global_watchdog",
    "watchdog_ceiling",
    "mismatched_watchdogs",
    "missing_global",
    "missing_period",
    "invalid_duration",
    "missing_workflow",
    "missing_watchdog",
])


def _defective_chain(
    tiers: _Tiers, defect: str
) -> tuple[dict[str, typ.Any], dict[str, dict[str, typ.Any]], str]:
    """Return the valid chain with one link broken, and its diagnostic."""
    config = _config(tiers)
    workflows = _workflows(tiers.watchdog, tiers.ceiling_minutes)
    default = config["profile"]["default"]
    override = default["overrides"][0]
    last_workflow = workflows[COVERAGE_WORKFLOWS[-1]]
    match defect:
        case "default_deadline":
            override["slow-timeout"] = _slow_timeout(tiers.default_deadline)
            expected = "does not exceed the default deadline"
        case "override_ceiling":
            override["slow-timeout"] = _slow_timeout(tiers.global_timeout)
            expected = "does not precede the global-timeout"
        case "global_watchdog":
            default["global-timeout"] = f"{tiers.watchdog}s"
            expected = "does not exceed the global-timeout"
        case "watchdog_ceiling":
            ceiling = tiers.ceiling_minutes * 60
            for workflow in workflows.values():
                _first_step(workflow)["with"]["cargo-wait-timeout"] = str(ceiling)
            expected = "does not precede the job ceiling"
        case "mismatched_watchdogs":
            _first_step(last_workflow)["with"]["cargo-wait-timeout"] = str(
                tiers.watchdog + 1
            )
            expected = "disagree on cargo-wait-timeout"
        case "missing_global":
            del default["global-timeout"]
            expected = "has no global-timeout"
        case "missing_period":
            del override["slow-timeout"]["period"]
            expected = "slow-timeout omits period"
        case "invalid_duration":
            override["slow-timeout"]["period"] = "0s"
            expected = "is not positive"
        case "missing_workflow":
            del workflows[COVERAGE_WORKFLOWS[-1]]
            expected = (
                f"{COVERAGE_WORKFLOWS[-1]} has no {COVERAGE_ACTION_NAME} invocation"
            )
        case _:
            del _first_step(last_workflow)["with"]["cargo-wait-timeout"]
            expected = "is not a number"
    return config, workflows, expected


@PROPERTY_SETTINGS
@given(tiers=_ordered_tiers(), defect=DEFECTS)
def test_one_broken_link_in_the_chain_is_reported(
    tiers: _Tiers, defect: str
) -> None:
    """A single unordered or unstated tier must produce its own diagnostic."""
    config, workflows, expected = _defective_chain(tiers, defect)
    violations = timeout_tier_violations(config, workflows)
    assert any(expected in violation for violation in violations), (
        f"{defect!r} must report {expected!r}, got {violations!r}"
    )
