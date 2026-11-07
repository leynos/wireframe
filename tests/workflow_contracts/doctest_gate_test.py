"""Hold CI to running doctests, and to failing when they fail.

``make test`` selects ``--all-targets``, which excludes doctests, and
cargo-nextest cannot run them, so one workflow step is all that stands between
a documented example and silent rot (#578). A workflow can keep that step and
still lose the gate, through a condition, ``continue-on-error``, or a recipe
that swallows the exit status, so each of those is checked here.

The end-to-end check runs the workflow step's own command, extracted from
``ci.yml``, against a scratch crate whose doctest always fails. The scratch
``cargo`` delegates to the real toolchain, so the run fails because a doctest
failed, not because a stand-in was told to fail. Deleting the step, pointing it
elsewhere, or swallowing its exit status therefore fails the contract.

Run these contracts with ``make test-workflow-contracts``.
"""

from __future__ import annotations

import os
import shlex
import shutil
import stat
import subprocess
from pathlib import Path

import yaml

REPOSITORY_ROOT = Path(__file__).resolve().parents[2]
WORKFLOW_PATH = REPOSITORY_ROOT / ".github" / "workflows" / "ci.yml"
DOCTEST_RUN = "make test-doc"
COMPANION_TEST_RUN = "make test-wireframe-testing"
FAILING_EXIT_STATUS = 101
SCRATCH_MANIFEST = """\
[package]
name = "failing-doctest"
version = "0.0.0"
edition = "2021"
"""
SCRATCH_SOURCE = """\
//! Scratch crate carrying a doctest that always fails.

/// A documented function whose example cannot pass.
///
/// ```
/// assert_eq!(1 + 1, 3);
/// ```
pub fn documented_example() {}
"""


def _build_test_steps() -> list[dict[str, object]]:
    """Return the ``build-test`` job's steps."""
    workflow = yaml.safe_load(WORKFLOW_PATH.read_text(encoding="utf-8"))
    steps = workflow["jobs"]["build-test"]["steps"]
    return list(steps)


def _steps_running(command: str) -> list[dict[str, object]]:
    """Return every ``build-test`` step that runs ``command``."""
    return [step for step in _build_test_steps() if step.get("run") == command]


def _stand_in_cargo(tmp_path: Path) -> tuple[Path, Path]:
    """Create a stand-in ``cargo`` that records its arguments and fails."""
    record = tmp_path / "cargo-arguments"
    cargo = tmp_path / "cargo"
    cargo.write_text(
        f'#!/bin/sh\nprintf \'%s\\n\' "$@" > "{record}"\nexit {FAILING_EXIT_STATUS}\n',
        encoding="utf-8",
    )
    cargo.chmod(cargo.stat().st_mode | stat.S_IXUSR)
    return cargo, record


def _run_recipe_with_failing_cargo(tmp_path: Path) -> tuple[int, list[str]]:
    """Run ``make test-doc`` on a failing ``cargo``; return status and arguments."""
    cargo, record = _stand_in_cargo(tmp_path)
    completed = subprocess.run(
        ["make", "--no-print-directory", "test-doc", f"CARGO={cargo}"],
        cwd=REPOSITORY_ROOT,
        env=os.environ.copy(),
        capture_output=True,
        text=True,
        check=False,
    )
    return completed.returncode, record.read_text(encoding="utf-8").splitlines()


def _failing_doctest_cargo(tmp_path: Path) -> Path:
    """Create a ``cargo`` that runs a scratch crate's genuinely failing doctest."""
    crate = tmp_path / "failing-doctest"
    (crate / "src").mkdir(parents=True)
    (crate / "Cargo.toml").write_text(SCRATCH_MANIFEST, encoding="utf-8")
    (crate / "src" / "lib.rs").write_text(SCRATCH_SOURCE, encoding="utf-8")

    real_cargo = shutil.which("cargo")
    assert real_cargo is not None, "the doctest contract needs cargo on PATH"
    manifest = crate / "Cargo.toml"
    target = tmp_path / "target"
    cargo = tmp_path / "cargo"
    cargo.write_text(
        "#!/bin/sh\n"
        f"exec {shlex.quote(real_cargo)} test --manifest-path {shlex.quote(str(manifest))} "
        f"--doc --target-dir {shlex.quote(str(target))}\n",
        encoding="utf-8",
    )
    cargo.chmod(cargo.stat().st_mode | stat.S_IXUSR)
    return cargo


def _run_workflow_step_with_failing_doctest(tmp_path: Path) -> tuple[int, str]:
    """Run the workflow's doctest step over a scratch crate that must fail."""
    steps = _steps_running(DOCTEST_RUN)
    assert len(steps) == 1, f"build-test must run `{DOCTEST_RUN}` exactly once"
    command = steps[0]["run"]
    assert isinstance(command, str) and command, "the doctest step must run a command"

    environment = os.environ.copy()
    environment["CARGO"] = str(_failing_doctest_cargo(tmp_path))
    completed = subprocess.run(
        ["bash", "-c", command],
        cwd=REPOSITORY_ROOT,
        env=environment,
        capture_output=True,
        text=True,
        check=False,
    )
    return completed.returncode, completed.stdout + completed.stderr


def test_build_test_runs_doctests_unconditionally() -> None:
    """Exactly one step runs doctests, and nothing lets it be skipped or ignored."""
    steps = _steps_running(DOCTEST_RUN)
    assert len(steps) == 1, f"build-test must run `{DOCTEST_RUN}` exactly once"
    step = steps[0]
    assert "if" not in step, "the doctest step must run for every event"
    assert not step.get("continue-on-error"), "a doctest failure must fail the job"


def test_build_test_runs_the_companion_crate_tests() -> None:
    """The explicit companion-crate gate runs in the same job (#578)."""
    steps = _steps_running(COMPANION_TEST_RUN)
    assert len(steps) == 1, f"build-test must run `{COMPANION_TEST_RUN}` exactly once"
    step = steps[0]
    assert "if" not in step, "the companion test step must run for every event"
    assert not step.get("continue-on-error"), "a companion failure must fail the job"


def test_a_failing_doctest_run_fails_the_make_target(tmp_path: Path) -> None:
    """The recipe propagates cargo's failure instead of swallowing it."""
    status, _ = _run_recipe_with_failing_cargo(tmp_path)
    assert status != 0, "`make test-doc` must fail when cargo fails"


def test_a_real_failing_doctest_fails_the_workflow_step(tmp_path: Path) -> None:
    """A failing doctest, run through the workflow's own command, fails the step."""
    status, output = _run_workflow_step_with_failing_doctest(tmp_path)
    assert status != 0, "the workflow's doctest step must fail when a doctest fails"
    assert "error: doctest failed" in output, f"expected a doctest failure, saw:\n{output}"


def test_the_make_target_covers_every_workspace_doctest(tmp_path: Path) -> None:
    """The recipe selects doctests across the workspace with every feature on."""
    _, arguments = _run_recipe_with_failing_cargo(tmp_path)
    for required in ("test", "--workspace", "--doc", "--all-features"):
        assert required in arguments, f"`make test-doc` must pass {required}"
    assert "--exclude" not in arguments, "no crate may be left out of the gate"