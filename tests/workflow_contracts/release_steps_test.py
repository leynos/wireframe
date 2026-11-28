"""Run the release workflow's steps against stand-ins, not just read them.

The workflow contracts pin which step runs where. These tests execute the
steps themselves, so a step that always succeeds, or checks the wrong thing,
fails here rather than on release day: the ancestry check against a real
repository, the consumer smoke build against stand-in ``rustup`` and ``cargo``,
and the tool installation through ``make --dry-run``.

Run these tests with ``make test-workflow-contracts``.
"""

from __future__ import annotations

import os
import shutil
import stat
import subprocess
import typing as typ
from pathlib import Path

import pytest
import yaml

REPOSITORY_ROOT = Path(__file__).resolve().parents[2]
RELEASE = REPOSITORY_ROOT / ".github" / "workflows" / "release.yml"
SMOKE = REPOSITORY_ROOT / "scripts" / "release" / "consumer-smoke.sh"
GIT_IDENTITY = {
    "GIT_AUTHOR_NAME": "Release Test",
    "GIT_AUTHOR_EMAIL": "release-test@example.invalid",
    "GIT_COMMITTER_NAME": "Release Test",
    "GIT_COMMITTER_EMAIL": "release-test@example.invalid",
}


def _ancestry_command() -> str:
    """Return the command the ``require-main`` job runs."""
    job = yaml.safe_load(RELEASE.read_text(encoding="utf-8"))["jobs"]["require-main"]
    (command,) = [step["run"] for step in job["steps"] if "run" in step]
    return command


def _git(cwd: Path, *arguments: str) -> str:
    """Run one Git command with a fixed identity and return its output."""
    environment = {"PATH": os.environ["PATH"], "HOME": str(cwd), **GIT_IDENTITY}
    return subprocess.run(
        ["git", *arguments],
        cwd=cwd,
        env=environment,
        capture_output=True,
        text=True,
        check=True,
    ).stdout.strip()


class _Clone(typ.NamedTuple):
    """A clone whose ``origin/main`` is known, with one commit off it."""

    path: Path
    on_main: str
    off_main: str


@pytest.fixture
def clone(tmp_path: Path) -> _Clone:
    """Build an origin with two commits on ``main`` and a clone of it.

    The off-main commit branches from the first commit, as a release tagged on
    an unmerged branch would.
    """
    origin = tmp_path / "origin"
    origin.mkdir()
    _git(origin, "init", "--quiet", "--initial-branch=main")
    _git(origin, "commit", "--quiet", "--allow-empty", "-m", "first")
    first = _git(origin, "rev-parse", "HEAD")
    _git(origin, "commit", "--quiet", "--allow-empty", "-m", "second")
    on_main = _git(origin, "rev-parse", "HEAD")
    _git(origin, "switch", "--quiet", "--create", "feature", first)
    _git(origin, "commit", "--quiet", "--allow-empty", "-m", "unmerged")
    off_main = _git(origin, "rev-parse", "HEAD")
    _git(origin, "switch", "--quiet", "main")
    copy = tmp_path / "clone"
    _git(tmp_path, "clone", "--quiet", "--no-local", str(origin), str(copy))
    _git(copy, "fetch", "--quiet", "origin", "feature")
    return _Clone(copy, on_main, off_main)


@pytest.mark.parametrize(("commit", "expected"), [("on_main", 0), ("off_main", 1)])
def test_the_ancestry_step_accepts_only_a_commit_on_main(
    clone: _Clone, commit: str, expected: int
) -> None:
    """The step succeeds for a commit on ``main`` and fails for any other."""
    completed = subprocess.run(
        ["bash", "-c", _ancestry_command()],
        cwd=clone.path,
        env={"PATH": os.environ["PATH"], "GITHUB_SHA": getattr(clone, commit)},
        capture_output=True,
        text=True,
        check=False,
    )
    assert completed.returncode == expected, completed.stderr


class _Smoke(typ.NamedTuple):
    """The outcome of one smoke run and the stand-in calls it made."""

    status: int
    output: str
    calls: list[str]
    manifest: str


def _stand_in(directory: Path, name: str, log: Path, body: str = "") -> None:
    """Write an executable that records its arguments, then runs ``body``."""
    script = directory / name
    script.write_text(
        "#!/usr/bin/env bash\n"
        f'printf "%s %s\\n" "{name}" "$*" >>"{log}"\n'
        f"{body}\n",
        encoding="utf-8",
    )
    script.chmod(script.stat().st_mode | stat.S_IXUSR)


def _run_smoke(
    tmp_path: Path, manifest: str, *arguments: str, cargo_exit: int = 0
) -> _Smoke:
    """Run a copy of the smoke script beside ``manifest``, with stand-ins.

    The script finds ``Cargo.toml`` two directories above itself, so it is
    copied into a throwaway repository layout. The ``cargo`` stand-in saves the
    consumer manifest it is asked to check, then exits with ``cargo_exit``.
    """
    root = tmp_path / "repository"
    (root / "scripts" / "release").mkdir(parents=True)
    shutil.copy2(SMOKE, root / "scripts" / "release" / SMOKE.name)
    (root / "Cargo.toml").write_text(manifest, encoding="utf-8")
    bin_directory = tmp_path / "bin"
    bin_directory.mkdir()
    log = tmp_path / "calls"
    log.touch()
    saved = tmp_path / "consumer.toml"
    _stand_in(bin_directory, "rustup", log)
    _stand_in(bin_directory, "cargo", log, f'cp Cargo.toml "{saved}"; exit {cargo_exit}')
    completed = subprocess.run(
        [str(root / "scripts" / "release" / SMOKE.name), *arguments],
        env={"PATH": f"{bin_directory}{os.pathsep}{os.environ['PATH']}"},
        capture_output=True,
        text=True,
        check=False,
    )
    return _Smoke(
        completed.returncode,
        completed.stdout + completed.stderr,
        log.read_text(encoding="utf-8").splitlines(),
        saved.read_text(encoding="utf-8") if saved.exists() else "",
    )


WITH_MSRV = '[package]\nname = "wireframe"\nrust-version = "1.85"\n'


def test_the_smoke_needs_exactly_one_version(tmp_path: Path) -> None:
    """A missing version is a usage error, before any toolchain is touched."""
    result = _run_smoke(tmp_path, WITH_MSRV)
    assert result.status == 2
    assert "usage" in result.output
    assert result.calls == []


def test_the_smoke_refuses_a_workspace_without_a_minimum_rust_version(
    tmp_path: Path,
) -> None:
    """Without a declared MSRV the script fails before installing anything."""
    result = _run_smoke(tmp_path, '[package]\nname = "wireframe"\n', "1.0.0")
    assert result.status == 1
    assert "rust-version" in result.output
    assert result.calls == []


def test_the_smoke_checks_the_exact_release_on_the_minimum_rust_version(
    tmp_path: Path,
) -> None:
    """The consumer pins both crates exactly and builds on the MSRV."""
    result = _run_smoke(tmp_path, WITH_MSRV, "1.0.0-alpha1")
    assert result.status == 0, result.output
    assert result.calls == [
        "rustup toolchain install 1.85 --profile minimal --no-self-update",
        "cargo +1.85 check",
    ]
    for dependency in (
        'wireframe = "=1.0.0-alpha1"',
        'wireframe_testing = "=1.0.0-alpha1"',
        'rust-version = "1.85"',
    ):
        assert dependency in result.manifest


def test_the_smoke_fails_when_the_consumer_does_not_build(tmp_path: Path) -> None:
    """A failing ``cargo check`` fails the smoke instead of reporting success."""
    result = _run_smoke(tmp_path, WITH_MSRV, "1.0.0", cargo_exit=101)
    assert result.status == 101
    assert "consumer smoke: wireframe" not in result.output


def _dry_run(target: str) -> str:
    """Return the commands ``target`` would run."""
    return subprocess.run(
        ["make", "--no-print-directory", "--dry-run", target],
        cwd=REPOSITORY_ROOT,
        capture_output=True,
        text=True,
        check=True,
    ).stdout


def test_the_tool_is_installed_from_the_lock_without_running_it() -> None:
    """``install-lading`` syncs the locked project and runs nothing else."""
    (command,) = _dry_run("install-lading").splitlines()
    assert "sync --python 3.14 --project tools/lading --locked" in command
    assert " run " not in command


def test_the_upload_runs_only_what_was_installed() -> None:
    """``publish-crates`` cannot sync, while ``publish-check`` stays locked."""
    upload = _dry_run("publish-crates")
    assert "--project tools/lading --no-sync lading publish" in upload
    assert "--locked lading" not in upload
    rehearsal = _dry_run("publish-check")
    assert "--project tools/lading --locked lading publish" in rehearsal
