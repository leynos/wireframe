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
INSTALL_NIXIE_ACTION_RE = re.compile(
    r"^leynos/shared-actions/\.github/actions/install-nixie@[0-9a-f]{40}$"
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
    """The spelling gate installs the reviewed Merman and Nixie releases.

    Both arrive through the shared ``install-nixie`` action, which downloads
    Merman's release archive and verifies it against a pinned checksum.
    """
    tools = _find_step(_load_steps(), "Install Nixie and Merman CLI")
    assert INSTALL_NIXIE_ACTION_RE.match(str(tools.get("uses", ""))), (
        "CI must install Nixie and Merman through the pinned install-nixie action"
    )
    assert tools.get("with") == {
        "nixie-version": "1.1.0",
        "merman-version": "0.7.0",
        "python-version": "3.14",
    }, "Nixie, Merman and Nixie's Python runtime must stay on the reviewed releases"


def test_no_step_compiles_merman() -> None:
    """No CI step builds Merman from source.

    ``cargo install merman-cli`` compiled Merman from crates.io on every run,
    about 2.3 minutes of a paid runner, before the prebuilt archive replaced
    it. Any ``run`` command naming Merman is refused, whatever its flags or
    toolchain, because the only sanctioned route is the action asserted above.
    """
    workflow = yaml.safe_load(WORKFLOW_PATH.read_text(encoding="utf-8"))
    assert isinstance(workflow, dict), "the CI workflow must be a mapping"
    offenders = [
        f"{job_name}: {step.get('name', '<unnamed>')}"
        for job_name, job in workflow.get("jobs", {}).items()
        for step in job.get("steps", [])
        if isinstance(step, dict) and "merman" in str(step.get("run", "")).lower()
    ]
    assert not offenders, (
        f"these steps install or build Merman themselves: {offenders}; use the "
        "install-nixie action, which installs the verified release archive"
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
        "Install Nixie and Merman CLI",
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
        "publish-artefact": "false",
    }, "coverage generation must produce the ratcheted LCOV report"


def _checkout_steps(steps: list[dict[str, object]]) -> list[dict[str, object]]:
    """Return the steps that run ``actions/checkout``."""
    return [
        step
        for step in steps
        if str(step.get("uses", "")).startswith("actions/checkout@")
    ]


def test_build_test_checks_out_at_the_default_depth() -> None:
    """Scenario: a full-history fetch returns to the pull-request lane.

    Invariant: ``build-test`` checks out exactly once, and that checkout sets
    no ``fetch-depth``, so it fetches the action's default single commit. The
    full fetch served the CodeScene changed-line gate, which left this lane;
    nothing here reads history now, and every pull-request run would pay for
    it. A step that comes to need history changes this test with its reason.
    """
    checkouts = _checkout_steps(_load_steps())
    assert len(checkouts) == 1, (
        f"build-test should check out exactly once; found {len(checkouts)}"
    )
    options = checkouts[0].get("with") or {}
    assert isinstance(options, dict), "the checkout's with: must be a mapping"
    assert "fetch-depth" not in options, (
        "build-test must check out at the default depth; it sets fetch-depth "
        f"{options.get('fetch-depth')!r}, and nothing in the lane reads history"
    )
