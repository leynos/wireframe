"""Protect pull-request coverage enforcement in CI.

Run these workflow contract tests with ``make test-workflow-contracts``.
"""

from __future__ import annotations

from pathlib import Path
from typing import cast

import yaml

WORKFLOW_PATH = Path(__file__).resolve().parents[2] / ".github" / "workflows" / "ci.yml"


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

def test_spelling_toolchain_steps_are_consecutive() -> None:
    """Installation, spelling and Mermaid validation retain their CI order."""
    steps = _load_steps()
    step_names = (
        "Install Rust for Merman",
        "Install Merman CLI",
        "Install Nixie",
        "Lint Markdown and enforce en-GB-oxendict spelling",
        "Validate Mermaid diagrams",
        "Workflow contract tests",
    )
    indices = [steps.index(_find_step(steps, name)) for name in step_names]
    assert indices == list(range(indices[0], indices[0] + len(indices))), (
        "the spelling toolchain steps must remain consecutive and ordered"
    )
    markdown = _find_step(steps, "Lint Markdown and enforce en-GB-oxendict spelling")
    validation = _find_step(steps, "Validate Mermaid diagrams")
    assert markdown.get("run") == "make markdownlint", (
        "CI must lint Markdown and run the spelling gate"
    )
    assert validation.get("run") == "make nixie", "CI must validate Mermaid diagrams"
def test_codescene_check_immediately_follows_coverage_generation() -> None:
    """The changed-line gate consumes the LCOV report produced just before it."""
    steps = _load_steps()
    generation = _find_step(steps, "Test and Measure Coverage")
    check = _find_step(steps, "Check coverage against CodeScene gates")
    assert steps.index(check) == steps.index(generation) + 1, (
        "the CodeScene check must immediately follow coverage generation"
    )
    assert generation.get("if") == "github.event_name == 'pull_request'", (
        "coverage generation must remain pull-request-only"
    )
    assert generation.get("with") == {
        "output-path": "lcov.info",
        "format": "lcov",
        "with-ratchet": "true",
    }, "coverage generation must produce the ratcheted LCOV report"


def test_codescene_check_uses_the_guarded_project_contract() -> None:
    """The CodeScene check is fork-safe and targets Wireframe's project."""
    check = _find_step(_load_steps(), "Check coverage against CodeScene gates")
    assert check.get("env") == {"CS_ACCESS_TOKEN": "${{ secrets.CS_ACCESS_TOKEN }}"}, (
        "the CodeScene token must remain scoped to the check step"
    )
    assert check.get("if") == (
        "env.CS_ACCESS_TOKEN != '' && github.event_name == 'pull_request'"
    ), "the CodeScene check must skip pull requests without the secret"
    assert check.get("uses") == (
        "leynos/shared-actions/.github/actions/upload-codescene-coverage@"
        "4977418856133491c6aa7407d40668744df21818"
    ), "the CodeScene check must use the reviewed shared-action pin"
    assert check.get("with") == {
        "format": "lcov",
        "mode": "check",
        "project-url": "https://api.codescene.io/v2/projects/68308",
        "access-token": "${{ env.CS_ACCESS_TOKEN }}",
        "installer-checksum": "${{ vars.CODESCENE_CLI_SHA256 }}",
    }, "the CodeScene check must pass the canonical project and check-mode inputs"
