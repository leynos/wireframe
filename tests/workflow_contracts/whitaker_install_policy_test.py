"""Hold the Whitaker install judgement to constructed workflows.

A check over this repository's own correct workflow passes whether or not it
discriminates anything, so the judgement is driven over constructed workflows
in both directions: one baseline that conforms, then one case per way the
install can break the contract.

Run these tests with ``make test-workflow-contracts``.
"""

from __future__ import annotations

import pytest
from whitaker_install_policy import (
    INSTALL_WHITAKER_ACTION_RE,
    LocatedStep,
    Step,
    install_problems,
)

INSTALL_USES = "leynos/shared-actions/.github/actions/install-whitaker@" + "0" * 40
LINT_STEP: Step = {"name": "Lint", "run": "make lint"}
INSTALL_STEP: Step = {"name": "Install Whitaker", "uses": INSTALL_USES}


def _workflow(install: Step = INSTALL_STEP, *, job: str = "build-test") -> list[LocatedStep]:
    """Return constructed workflow steps holding one install in `job`."""
    return [("ci.yml", job, install)]


def _with(options: object) -> Step:
    """Return the install step passing `options` as its inputs."""
    return {**INSTALL_STEP, "with": options}


def test_a_conforming_install_has_no_problems() -> None:
    """The constructed baseline passes, so each case below breaks one thing."""
    assert install_problems(_workflow(), [INSTALL_STEP, LINT_STEP]) == []


@pytest.mark.parametrize(
    ("steps", "build_test_steps", "expected"),
    [
        pytest.param([], [LINT_STEP], "found 0", id="no-install"),
        pytest.param(
            [*_workflow(), *_workflow()],
            [INSTALL_STEP, LINT_STEP],
            "found 2",
            id="two-installs",
        ),
        pytest.param(
            _workflow(job="other"),
            [INSTALL_STEP, LINT_STEP],
            "build-test job",
            id="outside-build-test",
        ),
        pytest.param(
            [("other.yml", "build-test", INSTALL_STEP)],
            [INSTALL_STEP, LINT_STEP],
            "build-test job",
            id="outside-ci-workflow",
        ),
        pytest.param(
            _workflow({"uses": INSTALL_USES.replace("0" * 40, "main")}),
            [LINT_STEP],
            "full commit SHA",
            id="mutable-reference",
        ),
        pytest.param(
            _workflow(_with({"suite-version": "abc"})),
            [_with({"suite-version": "abc"}), LINT_STEP],
            "owns the pins",
            id="suite-pin",
        ),
        pytest.param(
            _workflow(_with({"installer-version": "0.2.12"})),
            [_with({"installer-version": "0.2.12"}), LINT_STEP],
            "owns the pins",
            id="installer-pin",
        ),
        pytest.param(
            _workflow(_with("pin")),
            [_with("pin"), LINT_STEP],
            "must be a mapping",
            id="inputs-not-a-mapping",
        ),
        pytest.param(
            _workflow(), [LINT_STEP, INSTALL_STEP], "before the Lint step", id="after-lint"
        ),
        pytest.param(
            _workflow(), [LINT_STEP], "must be a build-test step", id="not-in-build-test"
        ),
        pytest.param(_workflow(), [INSTALL_STEP], "must run `make lint`", id="no-lint"),
    ],
)
def test_the_contract_reports_each_way_the_install_breaks_it(
    steps: list[LocatedStep], build_test_steps: list[Step], expected: str
) -> None:
    """Report a missing, duplicated, misplaced, unpinned or late install."""
    problems = install_problems(steps, build_test_steps)
    assert any(expected in problem for problem in problems), (
        f"expected a problem naming {expected!r}, saw {problems}"
    )


@pytest.mark.parametrize(
    "uses",
    [
        "leynos/shared-actions/.github/actions/install-whitaker@main",
        "leynos/shared-actions/.github/actions/install-whitaker",
        "leynos/shared-actions/.github/actions/install-whitaker@" + "0" * 39,
    ],
)
def test_the_install_action_must_be_pinned_to_a_full_commit(uses: str) -> None:
    """A mutable or short reference does not satisfy the install pattern."""
    assert INSTALL_WHITAKER_ACTION_RE.fullmatch(uses) is None
