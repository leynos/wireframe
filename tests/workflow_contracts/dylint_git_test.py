"""Verify the CI Git wrapper only changes Dylint's rust-clippy clone."""

import json
import os
import subprocess
from pathlib import Path

import pytest

WRAPPER = Path(__file__).resolve().parents[2] / "scripts/dylint-git/git"
CLIPPY = "https://github.com/rust-lang/rust-clippy"


@pytest.mark.parametrize(
    ("arguments", "expected"),
    [
        (
            ["clone", CLIPPY, "/tmp/destination", "--quiet"],
            ["clone", "--branch", "master", CLIPPY, "/tmp/destination", "--quiet"],
        ),
        (
            ["clone", "https://github.com/example/other", "/tmp/destination"],
            ["clone", "https://github.com/example/other", "/tmp/destination"],
        ),
        (["--version"], ["--version"]),
    ],
)
def test_git_wrapper_forwards_commands(
    tmp_path: Path, arguments: list[str], expected: list[str]
) -> None:
    """Select the legacy branch only for Dylint's exact clone command."""
    fake_git = tmp_path / "git"
    fake_git.write_text(
        "#!/usr/bin/env python3\nimport json, sys\nprint(json.dumps(sys.argv[1:]))\n",
        encoding="utf-8",
    )
    fake_git.chmod(0o755)
    env = os.environ | {"DYLINT_REAL_GIT": str(fake_git)}
    result = subprocess.run(
        [str(WRAPPER), *arguments], env=env, check=True, capture_output=True, text=True
    )
    assert json.loads(result.stdout) == expected


def test_clippy_clone_checks_out_master_when_default_is_main(tmp_path: Path) -> None:
    """A real clone exposes the local branch expected by Dylint."""
    source = tmp_path / "source"
    subprocess.run(["git", "init", "-q", "-b", "main", str(source)], check=True)
    subprocess.run(
        [
            "git",
            "-C",
            str(source),
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "--allow-empty",
            "-qm",
            "initial",
        ],
        check=True,
    )
    subprocess.run(["git", "-C", str(source), "branch", "master"], check=True)
    fake_git = tmp_path / "git"
    fake_git.write_text(
        "#!/usr/bin/env python3\n"
        "import os, sys\n"
        f"args = [({str(source)!r} if arg == {CLIPPY!r} else arg) for arg in sys.argv[1:]]\n"
        "os.execvp('git', ['git', *args])\n",
        encoding="utf-8",
    )
    fake_git.chmod(0o755)
    target = tmp_path / "clone"
    env = os.environ | {"DYLINT_REAL_GIT": str(fake_git)}
    subprocess.run(
        [str(WRAPPER), "clone", CLIPPY, str(target), "--quiet"], env=env, check=True
    )
    branch = subprocess.check_output(
        ["git", "-C", str(target), "branch", "--show-current"], text=True
    )
    assert branch.strip() == "master"
