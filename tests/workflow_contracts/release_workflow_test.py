"""Hold the release workflows to the boundaries that make publishing safe.

An upload to crates.io cannot be undone, and a release runs rarely enough that
drift would be found by the release it breaks. These contracts pin who can start
a publication, which job holds a credential and for how long, what must succeed
before each irreversible step, and the commands the steps reach.

Run these contracts with ``make test-workflow-contracts``.
"""

from __future__ import annotations

import os
import re
import stat
import subprocess
import tomllib
import typing as typ
from pathlib import Path

import pytest
import yaml

REPOSITORY_ROOT = Path(__file__).resolve().parents[2]
WORKFLOWS = REPOSITORY_ROOT / ".github" / "workflows"
RELEASE = "release.yml"
PACKAGE = "release-package.yml"
DRY_RUN = "release-dry-run.yml"
SCRIPTS = REPOSITORY_ROOT / "scripts" / "release"
PINNED_ACTION = re.compile(r"[\w.-]+/[\w./-]+@[0-9a-f]{40}")


def _workflow(name: str) -> dict[typ.Any, typ.Any]:
    """Parse one workflow."""
    return yaml.safe_load((WORKFLOWS / name).read_text(encoding="utf-8"))


def _triggers(name: str) -> dict[str, typ.Any]:
    """Return a workflow's triggers; PyYAML reads the bare key as a boolean."""
    workflow = _workflow(name)
    return workflow.get("on", workflow.get(True))


def _job(name: str, job: str) -> dict[str, typ.Any]:
    """Return one job."""
    return _workflow(name)["jobs"][job]


def _runs(job: dict[str, typ.Any]) -> list[str]:
    """Return a job's shell commands, in order."""
    return [step["run"] for step in job["steps"] if "run" in step]


def _position(job: dict[str, typ.Any], fragment: str) -> int:
    """Return the one step whose command or action contains ``fragment``."""
    matching = [
        index
        for index, step in enumerate(job["steps"])
        if fragment in step.get("run", "") or fragment in step.get("uses", "")
    ]
    assert len(matching) == 1, f"expected one step for {fragment!r}, saw {matching}"
    return matching[0]


def test_only_a_version_tag_starts_a_release() -> None:
    """No caller, schedule, or dispatch can reach the publishing job."""
    assert _triggers(RELEASE) == {"push": {"tags": ["v*.*.*"]}}


def test_a_release_is_never_cancelled() -> None:
    """A second run of a tag waits rather than abandoning a publication."""
    concurrency = _workflow(RELEASE)["concurrency"]
    assert concurrency["cancel-in-progress"] is False
    assert "github.ref" in concurrency["group"]


@pytest.mark.parametrize("name", [RELEASE, PACKAGE, DRY_RUN])
def test_every_job_states_its_permissions(name: str) -> None:
    """The default token grant is denied, and each job names what it needs."""
    workflow = _workflow(name)
    assert workflow["permissions"] == {}
    assert all("permissions" in job for job in workflow["jobs"].values())


def test_one_job_holds_each_write_authority() -> None:
    """Only publication may use OIDC, and only the last job may write."""
    granted = {
        (name, job): {scope for scope, access in body["permissions"].items() if access == "write"}
        for name in (RELEASE, PACKAGE, DRY_RUN)
        for job, body in _workflow(name)["jobs"].items()
    }
    writers = {key: scopes for key, scopes in granted.items() if scopes}
    assert writers == {
        (RELEASE, "publish-crates-io"): {"id-token"},
        (RELEASE, "github-release"): {"contents"},
    }


def test_the_rehearsal_reaches_only_the_packaging_workflow() -> None:
    """A pull request runs the read-only workflow, with no secrets or tag."""
    assert set(_triggers(DRY_RUN)) == {"pull_request"}
    (job,) = _workflow(DRY_RUN)["jobs"].values()
    assert job == {
        "permissions": {"contents": "read"},
        "uses": f"./.github/workflows/{PACKAGE}",
    }


def test_the_reusable_workflow_cannot_publish() -> None:
    """Nothing a caller can reach holds a credential or uploads a crate."""
    assert set(_triggers(PACKAGE)) == {"workflow_call"}
    text = (WORKFLOWS / PACKAGE).read_text(encoding="utf-8")
    for forbidden in ("secrets.", "crates-io-auth-action", "publish-crates", "id-token"):
        assert forbidden not in text, f"{PACKAGE} must not mention {forbidden}"


@pytest.mark.parametrize("name", [RELEASE, PACKAGE, DRY_RUN])
def test_every_action_is_pinned_to_a_commit(name: str) -> None:
    """A moved tag cannot change what a release runs."""
    uses = [
        step["uses"]
        for job in _workflow(name)["jobs"].values()
        for step in job.get("steps", [])
        if "uses" in step
    ]
    assert all(PINNED_ACTION.fullmatch(action) for action in uses), uses


def test_no_long_lived_registry_secret_is_used() -> None:
    """The only registry credential is the exchanged one."""
    for name in (RELEASE, PACKAGE, DRY_RUN):
        assert "secrets." not in (WORKFLOWS / name).read_text(encoding="utf-8")


def test_the_publishing_job_is_gated_and_declared_here() -> None:
    """crates.io trusts this file and this environment, so both are fixed."""
    job = _job(RELEASE, "publish-crates-io")
    assert "uses" not in job, "a reusable workflow would present its own file name"
    assert job["environment"] == "crates-io"
    assert job["needs"] == ["package", "require-main"]
    assert job["if"] == "needs.package.outputs.action == 'publish'"


def test_every_irreversible_job_requires_a_commit_on_main() -> None:
    """The ancestry check runs unconditionally, before publication or verification.

    It sits in its own job because publication is skipped when every crate is
    already on crates.io, and verification would then announce an off-main
    commit that was published by hand.
    """
    job = _job(RELEASE, "require-main")
    assert "if" not in job and "needs" not in job
    assert job["permissions"] == {"contents": "read"}
    checkout = job["steps"][0]
    assert checkout["with"]["fetch-depth"] == 0
    command = job["steps"][_position(job, "git merge-base --is-ancestor")]["run"]
    assert command == 'git merge-base --is-ancestor "$GITHUB_SHA" origin/main'
    for dependent in ("publish-crates-io", "verify-crates-io"):
        assert "require-main" in _job(RELEASE, dependent)["needs"]


def test_the_credential_exists_only_around_the_upload() -> None:
    """Every check and download precedes the exchange; only the upload follows."""
    job = _job(RELEASE, "publish-crates-io")
    ordered = [
        _position(job, fragment)
        for fragment in (
            "release_workspace_cli.py check",
            "make install-lading",
            "crates-io-auth-action",
            "make publish-crates",
        )
    ]
    assert ordered == sorted(ordered)
    assert ordered[-1] == len(job["steps"]) - 1, "nothing may run after the upload"
    assert ordered[-1] == ordered[-2] + 1, "nothing may run between the two"
    holders = [step for step in job["steps"] if "CARGO_REGISTRY_TOKEN" in step.get("env", {})]
    assert holders == [job["steps"][-1]], "only the upload may see the token"
    assert "CARGO_REGISTRY_TOKEN" not in job.get("env", {})


def test_the_publishing_job_reproduces_the_candidate_it_was_given() -> None:
    """The check repackages here and compares with this run's commit and tag."""
    job = _job(RELEASE, "publish-crates-io")
    command = job["steps"][_position(job, "release_workspace_cli.py check")]["run"]
    assert command.index("make package-crates") < command.index("release_workspace_cli")
    for argument in ('--commit "$GITHUB_SHA"', '--tag "$GITHUB_REF_NAME"'):
        assert argument in command
    assert command.startswith("set -euo pipefail")


def test_the_candidate_stays_outside_the_checkout() -> None:
    """An untracked file in the checkout would become part of the package."""
    steps = [
        step
        for name in (RELEASE, PACKAGE)
        for job in _workflow(name)["jobs"].values()
        for step in job.get("steps", [])
        if step.get("with", {}).get("name") == "release-candidate"
    ]
    assert len(steps) == 4, "one upload and three downloads"
    for step in steps:
        assert step["with"]["path"] == "${{ runner.temp }}/release-candidate"


def test_verification_follows_publication_or_a_completed_one() -> None:
    """A failed or unexpected skip of the publisher stops the release."""
    job = _job(RELEASE, "verify-crates-io")
    assert job["needs"] == ["package", "require-main", "publish-crates-io"]
    assert " ".join(job["if"].split()) == (
        "!cancelled() && needs.package.result == 'success' && "
        "needs.require-main.result == 'success' && "
        "(needs.publish-crates-io.result == 'success' || "
        "(needs.publish-crates-io.result == 'skipped' && "
        "needs.package.outputs.action == 'skip'))"
    )
    _position(job, "release_workspace_cli.py verify")
    _position(job, "scripts/release/consumer-smoke.sh")


def test_the_announcement_comes_last() -> None:
    """The GitHub release is created only once crates.io is verified."""
    job = _job(RELEASE, "github-release")
    assert job["needs"] == ["package", "verify-crates-io"]
    assert "needs.verify-crates-io.result == 'success'" in job["if"]


def test_the_packaging_workflow_surveys_before_it_rehearses() -> None:
    """lading rehearses exactly when crates.io lacks the version."""
    job = _job(PACKAGE, "package")
    ordered = [
        _position(job, fragment)
        for fragment in (
            "make package-crates",
            "release_workspace_cli.py record",
            "release_workspace_cli.py survey",
            "make publish-check",
            "actions/upload-artifact",
        )
    ]
    assert ordered == sorted(ordered)
    rehearsal = job["steps"][ordered[3]]
    assert rehearsal["if"] == "steps.survey.outputs.action == 'publish'"
    assert rehearsal["env"] == {"LADING_SKIP_PREFLIGHT": "true"}
    assert not any(step.get("continue-on-error") for step in job["steps"])


def test_a_tag_is_compared_with_every_published_manifest() -> None:
    """Both crates must carry the tag's version."""
    job = _job(PACKAGE, "package")
    step = job["steps"][_position(job, "ensure-cargo-version")]
    assert step["if"] == "inputs.release-tag != ''"
    order = tomllib.loads((REPOSITORY_ROOT / "lading.toml").read_text())["publish"]["order"]
    names = {
        tomllib.loads((REPOSITORY_ROOT / path).read_text())["package"]["name"]
        for path in step["with"]["manifests"].split()
    }
    assert names == set(order)


def test_the_upload_runs_the_installed_tool_live() -> None:
    """The target publishes from a clean tree, and installs nothing."""
    command = subprocess.run(
        ["make", "--no-print-directory", "--dry-run", "publish-crates"],
        cwd=REPOSITORY_ROOT,
        capture_output=True,
        text=True,
        check=True,
    ).stdout
    for argument in ("--no-sync", "--live", "--forbid-dirty", "--skip-preflight"):
        assert argument in command


class _Gh(typ.NamedTuple):
    """Describe one scripted ``gh`` and what it recorded."""

    status: int
    calls: list[str]


def _run_github_release(tmp_path: Path, tag: str, view: str) -> _Gh:
    """Run the release script against a stand-in ``gh``.

    ``view`` is the stand-in's answer to ``gh release view``.
    """
    log = tmp_path / "calls"
    gh = tmp_path / "bin" / "gh"
    gh.parent.mkdir()
    gh.write_text(
        "#!/usr/bin/env bash\n"
        f'printf "%s\\n" "$*" >>"{log}"\n'
        f'if [[ "$1 $2" == "release view" ]]; then {view}; fi\n',
        encoding="utf-8",
    )
    gh.chmod(gh.stat().st_mode | stat.S_IXUSR)
    candidate = tmp_path / "candidate"
    candidate.mkdir()
    (candidate / "wireframe-1.0.0.crate").write_bytes(b"crate")
    completed = subprocess.run(
        [str(SCRIPTS / "github-release.sh"), tag, str(candidate)],
        env={"PATH": f"{gh.parent}{os.pathsep}{os.environ['PATH']}"},
        capture_output=True,
        text=True,
        check=False,
    )
    return _Gh(completed.returncode, log.read_text(encoding="utf-8").splitlines())


@pytest.mark.parametrize(
    ("tag", "prerelease"), [("v1.0.0", False), ("v1.0.0-alpha1", True)]
)
def test_a_new_release_stays_a_draft_until_its_assets_are_attached(
    tmp_path: Path, tag: str, prerelease: bool
) -> None:
    """Create, upload, then publish, marking a pre-release as one."""
    result = _run_github_release(tmp_path, tag, "exit 1")
    assert result.status == 0
    verbs = [" ".join(call.split()[:2]) for call in result.calls]
    assert verbs == ["release view", "release create", "release upload", "release edit"]
    create, upload = result.calls[1], result.calls[2]
    assert "--draft" in create and "--verify-tag" in create
    assert ("--prerelease" in create) is prerelease
    assert "wireframe-1.0.0.crate" in upload and "SHA256SUMS" in upload
    assert result.calls[3] == f"release edit {tag} --draft=false"


def test_an_interrupted_draft_is_completed(tmp_path: Path) -> None:
    """A draft left by a failed run is filled and published, not recreated."""
    result = _run_github_release(tmp_path, "v1.0.0", "echo true")
    assert result.status == 0
    assert [call.split()[1] for call in result.calls] == ["view", "upload", "edit"]


def test_a_public_release_is_left_alone(tmp_path: Path) -> None:
    """Published assets are never replaced."""
    result = _run_github_release(tmp_path, "v1.0.0", "echo false")
    assert result.status == 0
    assert [call.split()[1] for call in result.calls] == ["view"]
