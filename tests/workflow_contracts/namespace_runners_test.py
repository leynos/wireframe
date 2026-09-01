"""Guard Wireframe's repository-owned Namespace runner assignments."""

from pathlib import Path

import pytest
import yaml

_REPO_ROOT = Path(__file__).resolve().parents[2]
_EXPECTED_RUNNERS = (
    ("advanced-tests.yml", "advanced"),
    ("ci.yml", "build-test"),
    ("coverage-main.yml", "coverage-upload"),
    ("delayed-pr-comment.yml", "delay_and_comment"),
    ("get-codescene-sha.yml", "refresh-sha"),
)
_NAMESPACE_RUNNER = "namespace-profile-default"


@pytest.mark.parametrize(("workflow_name", "job_name"), _EXPECTED_RUNNERS)
def test_repository_owned_linux_job_uses_shared_namespace_profile(
    workflow_name: str, job_name: str
) -> None:
    """Require each direct Linux job to retain its reviewed runner profile."""
    workflow_path = _REPO_ROOT / ".github" / "workflows" / workflow_name
    workflow = yaml.safe_load(workflow_path.read_text(encoding="utf-8"))
    assert workflow["jobs"][job_name]["runs-on"] == _NAMESPACE_RUNNER
