"""Judge where and how CI installs Whitaker.

CI installs Whitaker through exactly one shared ``install-whitaker`` action
step, at a full commit SHA, in ``ci.yml``'s ``build-test`` job before the
``make lint`` step that runs it, passing no installer or suite pin because the
action owns them. The judgement is a pure function over parsed steps so
``whitaker_install_policy_test`` can drive it over constructed workflows in
both directions, and ``whitaker_provisioning_test`` can hold the real files to
it.
"""

from __future__ import annotations

import re
import typing as typ
from collections.abc import Callable

type Step = dict[str, object]
type LocatedStep = tuple[str, str, Step]

INSTALL_WHITAKER_ACTION_RE: typ.Final = re.compile(
    r"^leynos/shared-actions/\.github/actions/install-whitaker@[0-9a-f]{40}$"
)
#: Inputs the action owns, which a caller must not pass.
ACTION_OWNED_INPUTS: typ.Final = frozenset({"suite-version", "installer-version"})


def step_labels(
    steps: list[LocatedStep], predicate: Callable[[Step], bool]
) -> list[str]:
    """Return ``file:job: step`` for each step the predicate selects."""
    return [
        f"{file}:{job}: {step.get('name')}"
        for file, job, step in steps
        if predicate(step)
    ]


def install_steps(steps: list[LocatedStep]) -> list[LocatedStep]:
    """Return the steps that use the shared ``install-whitaker`` action."""
    return [
        located
        for located in steps
        if "install-whitaker@" in str(located[2].get("uses", ""))
    ]


def ordering_problems(install: Step, build_test_steps: list[Step]) -> list[str]:
    """Return why the install is not a `build-test` step before `make lint`.

    `make lint` runs `whitaker`, so provisioning it in another job, or after
    the `Lint` step, leaves the lint command running without its tool while a
    count-only check still passes.
    """
    install_index = next(
        (i for i, step in enumerate(build_test_steps) if step == install), None
    )
    lint_index = next(
        (
            i
            for i, step in enumerate(build_test_steps)
            if str(step.get("run", "")).strip() == "make lint"
        ),
        None,
    )
    if install_index is None:
        return ["install-whitaker must be a build-test step"]
    if lint_index is None:
        return ["build-test must run `make lint`"]
    if install_index > lint_index:
        return ["install-whitaker must run before the Lint step that invokes whitaker"]
    return []


def pin_problems(install: Step) -> list[str]:
    """Return the pin inputs the action owns but the step passes."""
    options = install.get("with") or {}
    if not isinstance(options, dict):
        return ["the install step's with: must be a mapping"]
    pinned = sorted(key for key in options if key in ACTION_OWNED_INPUTS)
    return [f"the action owns the pins; the step passes {pinned}"] if pinned else []


def install_problems(
    steps: list[LocatedStep], build_test_steps: list[Step]
) -> list[str]:
    """Return every way the Whitaker install breaks the provisioning contract.

    Parameters
    ----------
    steps
        ``(workflow file, job, step)`` for every step of every workflow.
    build_test_steps
        The steps of `ci.yml`'s `build-test` job, in order.

    Returns
    -------
    list[str]
        Empty when exactly one install action step, at a full commit SHA, sits
        in `ci.yml`'s `build-test` job before `make lint` and passes no pin.
    """
    installs = install_steps(steps)
    if len(installs) != 1:
        return [f"expected one install-whitaker action step, found {len(installs)}"]
    (file, job, install) = installs[0]
    problems: list[str] = []
    if (file, job) != ("ci.yml", "build-test"):
        problems.append("the install must run in ci.yml's build-test job")
    if not INSTALL_WHITAKER_ACTION_RE.fullmatch(str(install["uses"])):
        problems.append("the install action must be pinned to a full commit SHA")
    return [
        *problems,
        *pin_problems(install),
        *ordering_problems(install, build_test_steps),
    ]
