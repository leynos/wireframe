"""Prove the timeout tiers are stated and ordered, not inherited.

A test that overruns should report how long it ran before it is killed. That
only happens when every bound is explicit and each one is wider than the one
inside it: the per-test deadline, the override deadline, the whole-run
ceiling, the Cargo watchdog and the job ceiling. An inherited default is a
number nobody chose, and it presents as a cancelled run rather than a slow
test.

``timeout_tier_violations`` reads the parsed nextest configuration and the
parsed workflows and returns every way that chain is broken. It holds no
opinion about the values themselves; the reviewed numbers live in
``.config/nextest.toml`` and the workflow files, and the tests beside this
module pin them by value as well as by ordering.
"""

from __future__ import annotations

import math
import re
import typing as typ

#: The coverage action this repository invokes, without its version pin.
COVERAGE_ACTION_NAME: typ.Final = (
    "leynos/shared-actions/.github/actions/generate-coverage"
)

#: The workflows that must each state the Cargo watchdog explicitly.
COVERAGE_WORKFLOWS: typ.Final = ("ci.yml", "coverage-main.yml")

#: A duration nextest accepts, restricted to the units this policy reviews.
_DURATION: typ.Final = re.compile(r"^(?P<value>\d+(?:\.\d+)?)(?P<unit>[smh])$")
_UNIT_SECONDS: typ.Final = {"s": 1.0, "m": 60.0, "h": 3600.0}


class DurationSyntaxError(ValueError):
    """A duration this policy will not read."""


def coverage_action_name(step: dict[str, typ.Any]) -> str:
    """Return the action a step invokes, without its version pin.

    Parameters
    ----------
    step
        One parsed workflow step.

    Returns
    -------
    str
        The ``uses`` value with its ``@`` pin removed.

    Raises
    ------
    AssertionError
        When the invocation carries no action name or no version pin.

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


def parse_duration(text: str) -> float:
    """Return a nextest duration in seconds.

    Parameters
    ----------
    text
        A duration such as ``"180s"``, ``"10m"`` or ``"1h"``.

    Returns
    -------
    float
        The duration in seconds.

    Raises
    ------
    DurationSyntaxError
        When the value is not a positive number followed by ``s``, ``m`` or
        ``h``. Unknown units are refused rather than guessed at.

    Examples
    --------
    >>> parse_duration("10m")
    600.0
    """
    match = _DURATION.match(text.strip()) if isinstance(text, str) else None
    if match is None:
        message = f"duration {text!r} is not a number followed by s, m or h"
        raise DurationSyntaxError(message)
    seconds = float(match.group("value")) * _UNIT_SECONDS[match.group("unit")]
    if seconds <= 0:
        message = f"duration {text!r} is not positive"
        raise DurationSyntaxError(message)
    return seconds


def _is_positive_int(value: object) -> bool:
    """Return whether ``value`` is an integer above zero rather than a bool."""
    return not isinstance(value, bool) and isinstance(value, int) and value > 0


def _slow_timeout_deadline(
    slow_timeout: object, label: str, violations: list[str]
) -> float | None:
    """Return ``period * terminate-after``, recording why it cannot be read."""
    if not isinstance(slow_timeout, dict):
        violations.append(f"{label} has no slow-timeout mapping")
        return None
    missing = [
        key
        for key in ("period", "terminate-after", "grace-period")
        if key not in slow_timeout
    ]
    if missing:
        violations.append(f"{label} slow-timeout omits {', '.join(missing)}")
        return None
    try:
        period = parse_duration(slow_timeout["period"])
        parse_duration(slow_timeout["grace-period"])
    except DurationSyntaxError as exc:
        violations.append(f"{label} slow-timeout: {exc}")
        return None
    terminate_after = slow_timeout["terminate-after"]
    if not _is_positive_int(terminate_after):
        violations.append(
            f"{label} slow-timeout terminate-after {terminate_after!r} is not "
            "a positive integer"
        )
        return None
    return period * terminate_after


def _is_numeric(value: object) -> bool:
    """Return whether ``value`` may be read as a number rather than a bool."""
    return not isinstance(value, bool) and isinstance(value, (int, float, str))


def _is_positive_finite(seconds: float) -> bool:
    """Return whether ``seconds`` is a finite value above zero."""
    return math.isfinite(seconds) and seconds > 0


def _watchdog_seconds(raw: object, label: str, violations: list[str]) -> float | None:
    """Return a Cargo watchdog budget in seconds, or record why it is unusable."""
    if not _is_numeric(raw):
        violations.append(f"{label} cargo-wait-timeout {raw!r} is not a number")
        return None
    try:
        seconds = float(raw)
    except ValueError:
        violations.append(f"{label} cargo-wait-timeout {raw!r} is not a number")
        return None
    if not _is_positive_finite(seconds):
        violations.append(
            f"{label} cargo-wait-timeout {raw!r} is not a positive number of seconds"
        )
        return None
    return seconds


def _invokes_coverage(step: dict[str, typ.Any]) -> bool:
    """Return whether a step invokes the coverage action at its pinned name."""
    return "uses" in step and coverage_action_name(step) == COVERAGE_ACTION_NAME


def _workflow_invocations(
    workflow_name: str, workflow: dict[str, typ.Any]
) -> typ.Iterator[tuple[str, str, dict[str, typ.Any], dict[str, typ.Any]]]:
    """Yield the coverage invocations in one workflow."""
    for job_id, job in (workflow.get("jobs") or {}).items():
        for step in job.get("steps") or []:
            if _invokes_coverage(step):
                yield workflow_name, job_id, job, step


def _coverage_invocations(
    workflows: dict[str, dict[str, typ.Any]],
) -> list[tuple[str, str, dict[str, typ.Any], dict[str, typ.Any]]]:
    """Return ``(workflow, job id, job, step)`` for every coverage invocation."""
    return [
        invocation
        for workflow_name, workflow in workflows.items()
        for invocation in _workflow_invocations(workflow_name, workflow)
    ]


def _default_tier_violations(
    default: dict[str, typ.Any], violations: list[str]
) -> tuple[float | None, float | None]:
    """Check the default deadline and whole-run ceiling; return both in seconds."""
    default_deadline = _slow_timeout_deadline(
        default.get("slow-timeout"), "default", violations
    )
    if "global-timeout" not in default:
        violations.append("default profile has no global-timeout")
        return default_deadline, None
    try:
        return default_deadline, parse_duration(default["global-timeout"])
    except DurationSyntaxError as exc:
        violations.append(f"default global-timeout: {exc}")
        return default_deadline, None


def _override_tier_violations(
    overrides: object,
    default_deadline: float | None,
    global_timeout: float | None,
    violations: list[str],
) -> None:
    """Check every override sits between the default deadline and the ceiling."""
    if not isinstance(overrides, list):
        violations.append("profile.default.overrides is not a list")
        return
    for index, override in enumerate(overrides):
        deadline = _slow_timeout_deadline(
            override.get("slow-timeout"), f"override {index}", violations
        )
        if deadline is None:
            continue
        if default_deadline is not None and not default_deadline < deadline:
            violations.append(
                f"override {index} deadline {deadline}s does not exceed the "
                f"default deadline {default_deadline}s"
            )
        if global_timeout is not None and not deadline < global_timeout:
            violations.append(
                f"override {index} deadline {deadline}s does not precede the "
                f"global-timeout {global_timeout}s"
            )


def _missing_workflow_violations(
    invocations: typ.Sequence[
        tuple[str, str, dict[str, typ.Any], dict[str, typ.Any]]
    ],
    violations: list[str],
) -> None:
    """Require each coverage workflow to invoke the coverage action."""
    seen_workflows = {workflow for workflow, _, _, _ in invocations}
    for workflow_name in COVERAGE_WORKFLOWS:
        if workflow_name not in seen_workflows:
            violations.append(
                f"{workflow_name} has no {COVERAGE_ACTION_NAME} invocation"
            )


def _ceiling_violations(
    label: str, seconds: float, job: dict[str, typ.Any], violations: list[str]
) -> None:
    """Require the watchdog to sit inside the job's stated ceiling."""
    ceiling = job.get("timeout-minutes")
    if not _is_positive_int(ceiling):
        violations.append(f"{label} has no positive timeout-minutes")
    elif not seconds < ceiling * 60:
        violations.append(
            f"{label} cargo-wait-timeout {seconds}s does not precede the "
            f"job ceiling {ceiling * 60}s"
        )


def _watchdog_violations(
    invocations: typ.Sequence[
        tuple[str, str, dict[str, typ.Any], dict[str, typ.Any]]
    ],
    global_timeout: float | None,
    violations: list[str],
) -> list[float]:
    """Check each invocation states a watchdog ordered inside its job ceiling."""
    watchdogs: list[float] = []
    for workflow_name, job_id, job, step in invocations:
        label = f"{workflow_name}/{job_id}"
        raw = (step.get("with") or {}).get("cargo-wait-timeout")
        seconds = _watchdog_seconds(raw, label, violations)
        if seconds is None:
            continue
        watchdogs.append(seconds)
        if global_timeout is not None and not global_timeout < seconds:
            violations.append(
                f"{label} cargo-wait-timeout {seconds}s does not exceed the "
                f"global-timeout {global_timeout}s"
            )
        _ceiling_violations(label, seconds, job, violations)
    return watchdogs


def _agreement_violations(
    watchdogs: typ.Sequence[float], violations: list[str]
) -> None:
    """Require every coverage invocation to state the same watchdog."""
    distinct = set(watchdogs)
    if len(distinct) > 1:
        violations.append(
            "coverage invocations disagree on cargo-wait-timeout: "
            + ", ".join(f"{seconds:g}s" for seconds in sorted(distinct))
        )


def timeout_tier_violations(
    nextest_config: dict[str, typ.Any],
    workflows: dict[str, dict[str, typ.Any]],
) -> list[str]:
    """Return every way the timeout tiers are unordered or unstated.

    The required chain is: default per-test deadline < override deadline <
    ``global-timeout`` < ``cargo-wait-timeout`` < the enclosing coverage
    job's ``timeout-minutes``. Both coverage workflows must state the same
    watchdog, so neither inherits the action's default by omission.

    Parameters
    ----------
    nextest_config
        The parsed ``.config/nextest.toml``.
    workflows
        Parsed workflow documents keyed by file name.

    Returns
    -------
    list[str]
        Violations; empty means every tier is stated and ordered.
    """
    violations: list[str] = []
    default = nextest_config.get("profile", {}).get("default", {})
    default_deadline, global_timeout = _default_tier_violations(
        default, violations
    )
    _override_tier_violations(
        default.get("overrides", []), default_deadline, global_timeout, violations
    )
    invocations = _coverage_invocations(workflows)
    _missing_workflow_violations(invocations, violations)
    watchdogs = _watchdog_violations(invocations, global_timeout, violations)
    _agreement_violations(watchdogs, violations)
    return violations