"""Protect pull-request coverage enforcement in CI.

Coverage generation, the spelling toolchain and the Markdown linter's pin are
asserted here. Where CodeScene may and may not appear is a separate question
with a separate reason, and lives in ``ci_codescene_placement_test``.

Run these workflow contract tests with ``make test-workflow-contracts``.
"""

from __future__ import annotations

import re
from pathlib import Path
from typing import cast

import yaml

WORKFLOW_PATH = Path(__file__).resolve().parents[2] / ".github" / "workflows" / "ci.yml"
MAKEFILE_PATH = Path(__file__).resolve().parents[2] / "Makefile"
MARKDOWNLINT_ACTION_RE = re.compile(
    r"^DavidAnson/markdownlint-cli2-action@[0-9a-f]{40}$"
)


def _load_steps() -> list[dict[str, object]]:
    """Parse and return the CI build-test steps."""
    workflow = yaml.safe_load(WORKFLOW_PATH.read_text(encoding="utf-8"))
    assert isinstance(workflow, dict), "the CI workflow must be a mapping"
    jobs = workflow.get("jobs")
    assert isinstance(jobs, dict), "the CI workflow must declare jobs"
    build_test = jobs.get("build-test")
    assert isinstance(build_test, dict), "the CI workflow must declare build-test"
    steps = build_test.get("steps")
    assert isinstance(steps, list), "the build-test job must declare steps"
    assert all(isinstance(step, dict) for step in steps), (
        "every build-test step must be a mapping"
    )
    return cast("list[dict[str, object]]", steps)


def _find_step(steps: list[dict[str, object]], name: str) -> dict[str, object]:
    """Return the uniquely named workflow step."""
    matches = [step for step in steps if step.get("name") == name]
    assert len(matches) == 1, f"expected one {name!r} step, found {len(matches)}"
    return matches[0]


def test_spelling_tool_installations_are_pinned() -> None:
    """The spelling gate installs the reviewed Merman and Nixie releases."""
    steps = _load_steps()
    merman = _find_step(steps, "Install Merman CLI")
    nixie = _find_step(steps, "Install Nixie")
    assert merman.get("run") == (
        'cargo +1.95.0 install merman-cli --version "=0.7.0" --locked'
    ), "Merman CLI must remain pinned to the reviewed release"
    assert nixie.get("run") == 'uv tool install --python 3.14 "nixie-cli==1.1.0"', (
        "Nixie must remain pinned to the reviewed release and Python runtime"
    )


def test_markdownlint_runner_is_pinned() -> None:
    """The Markdown gate resolves the CLI locally and pins the CI action.

    Under the estate ``markdown-formatting-baseline`` rule the Makefile probes
    ``PATH`` for ``markdownlint-cli2`` and CI lints through the upstream
    action, whose release carries the reviewed CLI; the pin therefore lives on
    the action reference rather than on an ``npx`` version.
    """
    makefile = MAKEFILE_PATH.read_text(encoding="utf-8")
    assert (
        "MDLINT ?= $(shell command -v markdownlint-cli2 2>/dev/null || "
        "printf '%s' \"$$HOME/.bun/bin/markdownlint-cli2\")"
    ) in makefile, (
        "the Markdown gate must resolve markdownlint-cli2 with the estate probe"
    )
    lint = _find_step(_load_steps(), "Lint Markdown")
    assert MARKDOWNLINT_ACTION_RE.match(str(lint.get("uses", ""))), (
        "CI must lint Markdown through the pinned markdownlint-cli2 action"
    )
    assert lint.get("with") == {"globs": "**/*.md"}, (
        "the Markdown lint action must cover every Markdown file"
    )


def test_spelling_toolchain_steps_are_consecutive() -> None:
    """Installation, spelling and Mermaid validation retain their CI order."""
    steps = _load_steps()
    # The estate ``markdown-formatting-baseline`` rule lints through the
    # pinned action, so spelling (once a prerequisite of ``make markdownlint``)
    # runs as its own step immediately after it.
    step_names = (
        "Install Rust for Merman",
        "Install Merman CLI",
        "Install Nixie",
        "Lint Markdown",
        "Enforce en-GB-oxendict spelling",
        "Validate Mermaid diagrams",
        "Workflow contract tests",
    )
    indices = [steps.index(_find_step(steps, name)) for name in step_names]
    assert indices == list(range(indices[0], indices[0] + len(indices))), (
        "the spelling toolchain steps must remain consecutive and ordered"
    )
    spelling = _find_step(steps, "Enforce en-GB-oxendict spelling")
    validation = _find_step(steps, "Validate Mermaid diagrams")
    assert spelling.get("run") == "make spelling", "CI must run the spelling gate"
    assert validation.get("run") == "make nixie", "CI must validate Mermaid diagrams"


def test_coverage_generation_stays_pull_request_only_and_ratcheted() -> None:
    """The ratchet gate runs on pull requests and compares against a baseline.

    This is what is left of the two CodeScene assertions that stood here. The
    generation half is still ours and still enforced: it runs only on pull
    requests, because main-branch coverage is produced by
    ``coverage-main.yml``, and it carries ``with-ratchet`` so the lane
    actually compares against the stored baseline rather than merely
    producing a report.

    The CodeScene half moved and inverted. It used to require a
    ``cs-coverage check`` step immediately after this one; under CV-005 that
    step must not exist in a pull-request lane at all, and
    ``ci_codescene_placement_test`` asserts its absence along with the
    absence of the token and of any direct CLI invocation.
    """
    generation = _find_step(_load_steps(), "Test and Measure Coverage")
    assert generation.get("if") == "github.event_name == 'pull_request'", (
        "coverage generation must remain pull-request-only"
    )
    assert generation.get("with") == {
        "output-path": "lcov.info",
        "format": "lcov",
        "with-ratchet": "true",
    }, "coverage generation must produce the ratcheted LCOV report"
