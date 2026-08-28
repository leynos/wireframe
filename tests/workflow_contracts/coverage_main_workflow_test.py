"""Protect CodeScene's default-branch coverage baseline workflow.

Run these workflow contract tests with ``make test-workflow-contracts``.
"""

from __future__ import annotations

import re
from pathlib import Path
from typing import cast

import yaml

WORKFLOW_PATH: Path = (
    Path(__file__).resolve().parents[2] / ".github" / "workflows" / "coverage-main.yml"
)
CODESCENE_USES_RE: re.Pattern[str] = re.compile(
    r"^leynos/shared-actions/\.github/actions/upload-codescene-coverage@"
    r"[0-9a-f]{40}$"
)


def _load_steps() -> list[dict[str, object]]:
    """Parse and return the default-branch coverage-upload steps."""
    workflow = yaml.safe_load(WORKFLOW_PATH.read_text(encoding="utf-8"))
    assert isinstance(workflow, dict), "the coverage workflow must be a mapping"
    jobs = workflow.get("jobs")
    assert isinstance(jobs, dict), "the coverage workflow must declare jobs"
    coverage_upload = jobs.get("coverage-upload")
    assert isinstance(coverage_upload, dict), (
        "the coverage workflow must declare coverage-upload"
    )
    steps = coverage_upload.get("steps")
    assert isinstance(steps, list), "the coverage-upload job must declare steps"
    assert all(isinstance(step, dict) for step in steps), (
        "every coverage-upload step must be a mapping"
    )
    return cast("list[dict[str, object]]", steps)


def _find_step(steps: list[dict[str, object]], name: str) -> dict[str, object]:
    """Return the uniquely named default-branch coverage workflow step."""
    matches = [step for step in steps if step.get("name") == name]
    assert len(matches) == 1, f"expected one {name!r} step, found {len(matches)}"
    return matches[0]


def test_codescene_upload_follows_successful_coverage_generation() -> None:
    """Upload the newly generated LCOV report before PR gates can use its baseline."""
    steps = _load_steps()
    generation = _find_step(steps, "Test and Measure Coverage")
    upload = _find_step(steps, "Upload coverage data to CodeScene")
    assert steps.index(upload) == steps.index(generation) + 1, (
        "the CodeScene upload must immediately follow coverage generation"
    )
    assert generation.get("with") == {
        "output-path": "lcov.info",
        "format": "lcov",
        "with-ratchet": "true",
    }, "main must generate the ratcheted LCOV report before uploading it"


def test_codescene_upload_uses_wireframe_project_and_repository() -> None:
    """Upload main coverage to the project and repository used by PR checks."""
    steps = _load_steps()
    checkout = steps[0]
    assert checkout.get("uses") == (
        "actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1"
    ), (
        "the coverage workflow must start from Wireframe's checkout"
    )
    assert checkout.get("with") == {
        "repository": "leynos/wireframe",
        "persist-credentials": False,
    }, (
        "the checkout origin must identify github.com/leynos/wireframe without "
        "persisting credentials"
    )

    upload = _find_step(steps, "Upload coverage data to CodeScene")
    assert upload.get("env") == {"CS_ACCESS_TOKEN": "${{ secrets.CS_ACCESS_TOKEN }}"}, (
        "the CodeScene token must remain scoped to the upload step"
    )
    assert upload.get("if") == "env.CS_ACCESS_TOKEN != ''", (
        "the upload must remain safe for contexts without the CodeScene secret"
    )
    uses = upload.get("uses")
    assert isinstance(uses, str) and CODESCENE_USES_RE.fullmatch(uses), (
        "the upload must invoke upload-codescene-coverage at a full commit SHA"
    )
    assert upload.get("with") == {
        "format": "lcov",
        "mode": "upload",
        "project-url": "https://api.codescene.io/v2/projects/68308",
        "access-token": "${{ env.CS_ACCESS_TOKEN }}",
        "installer-checksum": "${{ vars.CODESCENE_CLI_SHA256 }}",
    }, "the upload must target the project used by the pull-request gate"
