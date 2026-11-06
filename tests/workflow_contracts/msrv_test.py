"""Hold the weekly lane to the Rust version the manifests declare.

The lane proves a promise made to users, so it fails quietly if it stops reading
the promise: a hard-coded toolchain would keep passing after ``rust-version``
moved, and a published crate with no floor of its own would be left out of what
the promise covers.

Run these contracts with ``make test-workflow-contracts``.
"""

from __future__ import annotations

import subprocess
import tomllib
from pathlib import Path

import yaml

REPOSITORY_ROOT = Path(__file__).resolve().parents[2]
WORKFLOW_PATH = REPOSITORY_ROOT / ".github" / "workflows" / "msrv.yml"
SCRIPT_PATH = REPOSITORY_ROOT / "scripts" / "check-msrv.sh"
PUBLISHED_MANIFESTS = (
    REPOSITORY_ROOT / "Cargo.toml",
    REPOSITORY_ROOT / "wireframe_testing" / "Cargo.toml",
)


def _workflow() -> dict[object, object]:
    """Parse the MSRV workflow."""
    return yaml.safe_load(WORKFLOW_PATH.read_text(encoding="utf-8"))


def test_the_lane_runs_weekly_on_a_github_hosted_runner() -> None:
    """A weekly schedule on the hosted pool, not the paid pull-request pool."""
    workflow = _workflow()
    # PyYAML reads the bare key `on` as the boolean true.
    triggers = workflow.get("on", workflow.get(True))
    schedules = triggers["schedule"]
    assert len(schedules) == 1, "the lane must have exactly one schedule"
    minute, hour, day_of_month, month, day_of_week = schedules[0]["cron"].split()
    assert (day_of_month, month) == ("*", "*"), "the schedule must recur every week"
    assert day_of_week.isdigit(), "the schedule must name one day of the week"
    assert minute.isdigit() and hour.isdigit(), "the schedule must run once that day"
    assert workflow["jobs"]["msrv"]["runs-on"] == "ubuntu-latest"


def test_the_lane_runs_the_make_target_unconditionally() -> None:
    """One step runs the target, and nothing lets it be skipped or ignored."""
    steps = _workflow()["jobs"]["msrv"]["steps"]
    matching = [step for step in steps if step.get("run") == "make test-msrv"]
    assert len(matching) == 1, "the job must run `make test-msrv` exactly once"
    assert "if" not in matching[0], "the build must run for every event"
    assert not matching[0].get("continue-on-error"), "a failure must fail the job"


def test_the_make_target_runs_the_script() -> None:
    """The workflow and a contributor reach the same script."""
    dry_run = subprocess.run(
        ["make", "--no-print-directory", "-n", "test-msrv"],
        cwd=REPOSITORY_ROOT,
        capture_output=True,
        text=True,
        check=True,
    ).stdout
    assert dry_run.strip() == "./scripts/check-msrv.sh"


def test_the_script_reads_the_version_from_the_manifest() -> None:
    """No toolchain version is written anywhere but the manifest."""
    script = SCRIPT_PATH.read_text(encoding="utf-8")
    assert 'rust-version *= *"' in script, "the script must read rust-version"
    assert 'cargo "+$msrv" build --locked --workspace' in script
    assert 'cargo "+$msrv" build\n' in script, "the consumer must resolve afresh"


def test_published_crates_declare_the_same_minimum() -> None:
    """Both published crates carry the floor the documentation states."""
    declared = {
        path.parent.name: tomllib.loads(path.read_text(encoding="utf-8"))["package"][
            "rust-version"
        ]
        for path in PUBLISHED_MANIFESTS
    }
    assert len(set(declared.values())) == 1, f"minimums disagree: {declared}"
