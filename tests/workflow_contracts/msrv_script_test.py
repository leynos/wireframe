"""Check what ``scripts/check-msrv.sh`` asks the toolchain to do.

The script is the MSRV lane's whole implementation, so its orchestration is
worth proving separately from Cargo: which version it reads, which checks it
runs, what it hands a new consumer, and whether a failure reaches the job.

Each test copies the script into a scratch repository and puts recording
stand-ins for ``rustup`` and ``cargo`` first on ``PATH``. The stand-ins answer
for the toolchain instead of compiling anything, so a passing case fails only
on the script's own decisions and a failed case fails at the command boundary.
Deleting a check, hard-coding a version, writing the wrong consumer manifest,
or swallowing an exit status therefore fails a contract here.

Run these contracts with ``make test-workflow-contracts``.
"""

from __future__ import annotations

import os
import shutil
import stat
import subprocess
import tomllib
import typing as typ
from pathlib import Path

REPOSITORY_ROOT: typ.Final = Path(__file__).resolve().parents[2]
SCRIPT_PATH: typ.Final = REPOSITORY_ROOT / "scripts" / "check-msrv.sh"

#: A deliberately older floor than the repository declares. Selecting it proves
#: the script reads the manifest rather than knowing the current version.
SCRATCH_FLOOR: typ.Final = "1.94"

MANIFEST_WITH_FLOOR: typ.Final = """\
[package]
name = "wireframe"
version = "0.3.0"
edition = "2024"
rust-version = "{floor}"
"""

MANIFEST_WITHOUT_FLOOR: typ.Final = """\
[package]
name = "wireframe"
version = "0.3.0"
edition = "2024"
"""

# The stand-ins append one blank-line-terminated record per invocation, so the
# tests can read back argv, working directory and inherited flags.
FAKE_RUSTUP: typ.Final = """\
#!/bin/sh
set -eu
{
    printf 'tool=rustup\\n'
    for argument in "$@"; do
        printf 'arg=%s\\n' "$argument"
    done
    printf '\\n'
} >> "$MSRV_TEST_LOG"
status="${MSRV_TEST_RUSTUP_STATUS:-0}"
if [ "$status" -ne 0 ]; then
    exit "$status"
fi
"""

FAKE_CARGO: typ.Final = """\
#!/bin/sh
set -eu
kind=workspace
if [ "$#" -eq 2 ]; then
    kind=consumer
fi
{
    printf 'tool=cargo\\n'
    printf 'cwd=%s\\n' "$PWD"
    printf 'kind=%s\\n' "$kind"
    if [ "${RUSTFLAGS+set}" = set ]; then
        printf 'rustflags=%s\\n' "$RUSTFLAGS"
    else
        printf 'rustflags=<unset>\\n'
    fi
    for argument in "$@"; do
        printf 'arg=%s\\n' "$argument"
    done
    printf '\\n'
} >> "$MSRV_TEST_LOG"
if [ "$kind" = consumer ] && [ -n "${MSRV_TEST_SNAPSHOT:-}" ]; then
    cp Cargo.toml "$MSRV_TEST_SNAPSHOT/consumer-Cargo.toml"
    cp src/lib.rs "$MSRV_TEST_SNAPSHOT/consumer-lib.rs"
fi
if [ "${MSRV_TEST_CARGO_FAIL:-}" = "$kind" ]; then
    exit "${MSRV_TEST_CARGO_STATUS:-101}"
fi
"""


def _records(log: Path) -> list[dict[str, typ.Any]]:
    """Parse the stand-ins' invocation log into one dictionary per call."""
    if not log.exists():
        return []
    records: list[dict[str, typ.Any]] = []
    for block in log.read_text(encoding="utf-8").split("\n\n"):
        if not block.strip():
            continue
        record: dict[str, typ.Any] = {"args": []}
        for line in block.splitlines():
            key, _, value = line.partition("=")
            if key == "arg":
                record["args"].append(value)
            else:
                record[key] = value
        records.append(record)
    return records


def _scratch_repository(tmp_path: Path, manifest: str) -> Path:
    """Copy the script into a repository-shaped directory with a given manifest."""
    repository = tmp_path / "repo"
    (repository / "scripts").mkdir(parents=True)
    shutil.copy2(SCRIPT_PATH, repository / "scripts" / "check-msrv.sh")
    (repository / "Cargo.toml").write_text(manifest, encoding="utf-8")
    return repository


def _stand_in_tools(tmp_path: Path) -> Path:
    """Create executable ``rustup`` and ``cargo`` stand-ins."""
    binaries = tmp_path / "bin"
    binaries.mkdir()
    for name, body in (("rustup", FAKE_RUSTUP), ("cargo", FAKE_CARGO)):
        tool = binaries / name
        tool.write_text(body, encoding="utf-8")
        tool.chmod(tool.stat().st_mode | stat.S_IXUSR)
    return binaries


def _run_script(
    tmp_path: Path, repository: Path, binaries: Path, **settings: str
) -> tuple[subprocess.CompletedProcess[str], Path]:
    """Run the scratch script with the stand-ins first on ``PATH``."""
    log = tmp_path / "invocations.log"
    environment = os.environ.copy()
    environment.pop("RUSTFLAGS", None)
    environment["PATH"] = os.pathsep.join(
        (str(binaries), environment.get("PATH", ""))
    )
    environment["MSRV_TEST_LOG"] = str(log)
    environment["MSRV_TEST_SNAPSHOT"] = str(tmp_path)
    environment["CARGO_TARGET_DIR"] = str(tmp_path / "target")
    environment.update(settings)
    completed = subprocess.run(
        ["bash", str(repository / "scripts" / "check-msrv.sh")],
        env=environment,
        capture_output=True,
        text=True,
        check=False,
        timeout=60,
    )
    return completed, log


def _cargo_records(records: list[dict[str, typ.Any]]) -> list[dict[str, typ.Any]]:
    """Return only the ``cargo`` invocations."""
    return [record for record in records if record.get("tool") == "cargo"]


def _consumer_manifest(tmp_path: Path) -> dict[str, typ.Any]:
    """Parse the consumer manifest the script handed to ``cargo``."""
    return tomllib.loads(
        (tmp_path / "consumer-Cargo.toml").read_text(encoding="utf-8")
    )


def test_the_script_selects_the_version_the_manifest_declares(tmp_path: Path) -> None:
    """Scenario: the floor moves to any version; the script must follow it."""
    repository = _scratch_repository(
        tmp_path, MANIFEST_WITH_FLOOR.format(floor=SCRATCH_FLOOR)
    )
    completed, log = _run_script(
        tmp_path, repository, _stand_in_tools(tmp_path)
    )

    assert completed.returncode == 0, completed.stderr
    calls = _records(log)
    assert calls[0]["tool"] == "rustup"
    assert calls[0]["args"] == [
        "toolchain",
        "install",
        SCRATCH_FLOOR,
        "--profile",
        "minimal",
        "--no-self-update",
    ]
    for cargo in _cargo_records(calls):
        assert cargo["args"][0] == f"+{SCRATCH_FLOOR}", cargo
    assert f"Rust {SCRATCH_FLOOR} compiles Wireframe" in completed.stdout


def test_the_script_checks_the_workspace_from_the_lock_file(tmp_path: Path) -> None:
    """The first check is the whole workspace, locked, with all targets."""
    repository = _scratch_repository(
        tmp_path, MANIFEST_WITH_FLOOR.format(floor="1.95")
    )
    completed, log = _run_script(
        tmp_path, repository, _stand_in_tools(tmp_path)
    )

    assert completed.returncode == 0, completed.stderr
    workspace, consumer = _cargo_records(_records(log))
    assert workspace["kind"] == "workspace"
    assert Path(str(workspace["cwd"])) == repository.resolve()
    assert workspace["args"] == [
        "+1.95",
        "check",
        "--locked",
        "--workspace",
        "--all-targets",
        "--all-features",
    ]
    # A blank value displaces the nightly-only `-Zthreads` in the repository's
    # Cargo configuration; leaving it unset would let that flag through.
    assert workspace["rustflags"] != "<unset>"
    assert workspace["rustflags"].strip() == ""
    assert consumer["rustflags"] == "<unset>"


def test_the_script_checks_a_fresh_consumer(tmp_path: Path) -> None:
    """The second check resolves dependencies afresh, outside the repository."""
    repository = _scratch_repository(
        tmp_path, MANIFEST_WITH_FLOOR.format(floor="1.95")
    )
    completed, log = _run_script(
        tmp_path, repository, _stand_in_tools(tmp_path)
    )

    assert completed.returncode == 0, completed.stderr
    _, consumer = _cargo_records(_records(log))
    assert consumer["kind"] == "consumer"
    consumer_directory = Path(str(consumer["cwd"]))
    assert not consumer_directory.is_relative_to(repository.resolve()), (
        "a consumer check must not inherit the repository's Cargo configuration"
    )
    assert consumer["args"] == ["+1.95", "check"], "the lock file must not bind it"

    manifest = _consumer_manifest(tmp_path)
    package = manifest["package"]
    assert package["name"] == "msrv-consumer"
    assert package["rust-version"] == "1.95"
    assert package["publish"] is False
    assert "workspace" in manifest, "the consumer needs its own workspace root"
    dependencies = manifest["dependencies"]
    assert dependencies["wireframe"]["path"] == str(repository.resolve())
    assert dependencies["wireframe"]["features"] == ["pool", "serializer-serde"]
    assert dependencies["wireframe_testing"]["path"] == str(
        repository.resolve() / "wireframe_testing"
    )
    source = (tmp_path / "consumer-lib.rs").read_text(encoding="utf-8")
    assert "pub use wireframe;" in source
    assert "pub use wireframe_testing;" in source


def test_the_script_refuses_a_manifest_without_a_floor(tmp_path: Path) -> None:
    """Scenario: the floor is deleted; the lane must fail, not check a guess."""
    repository = _scratch_repository(tmp_path, MANIFEST_WITHOUT_FLOOR)
    completed, log = _run_script(
        tmp_path, repository, _stand_in_tools(tmp_path)
    )

    assert completed.returncode != 0
    assert "Cargo.toml declares no rust-version" in completed.stderr
    assert _records(log) == [], "no toolchain work belongs to a missing floor"


def test_the_script_propagates_a_failed_workspace_check(tmp_path: Path) -> None:
    """Scenario: the workspace does not compile on the floor."""
    repository = _scratch_repository(
        tmp_path, MANIFEST_WITH_FLOOR.format(floor="1.95")
    )
    completed, log = _run_script(
        tmp_path,
        repository,
        _stand_in_tools(tmp_path),
        MSRV_TEST_CARGO_FAIL="workspace",
        MSRV_TEST_CARGO_STATUS="101",
    )

    assert completed.returncode == 101
    assert [call["kind"] for call in _cargo_records(_records(log))] == ["workspace"]


def test_the_script_propagates_a_failed_consumer_check(tmp_path: Path) -> None:
    """Scenario: the workspace passes but freshly resolved dependencies do not."""
    repository = _scratch_repository(
        tmp_path, MANIFEST_WITH_FLOOR.format(floor="1.95")
    )
    completed, log = _run_script(
        tmp_path,
        repository,
        _stand_in_tools(tmp_path),
        MSRV_TEST_CARGO_FAIL="consumer",
        MSRV_TEST_CARGO_STATUS="100",
    )

    assert completed.returncode == 100
    assert [call["kind"] for call in _cargo_records(_records(log))] == [
        "workspace",
        "consumer",
    ]


def test_the_script_propagates_a_failed_toolchain_install(tmp_path: Path) -> None:
    """Scenario: the minimum toolchain cannot be installed."""
    repository = _scratch_repository(
        tmp_path, MANIFEST_WITH_FLOOR.format(floor="1.95")
    )
    completed, log = _run_script(
        tmp_path,
        repository,
        _stand_in_tools(tmp_path),
        MSRV_TEST_RUSTUP_STATUS="70",
    )

    assert completed.returncode == 70
    assert _cargo_records(_records(log)) == [], "no check runs without its toolchain"