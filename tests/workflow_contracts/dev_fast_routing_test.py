"""Protect the explicit development-backend routing contract.

Run with ``make test-workflow-contracts``. The dry runs use a probe Cargo
command, so they inspect Make's evaluated recipes without compiling the Rust
workspace.
"""

from __future__ import annotations

import os
import platform
import subprocess
import tomllib
from pathlib import Path
from typing import cast

import pytest
import yaml

ROOT = Path(__file__).resolve().parents[2]
MAKEFILE_PATH = ROOT / "Makefile"
CONFIG_RELATIVE_PATH = Path("tools/dev-fast/config.toml")
CONFIG_PATH = ROOT / CONFIG_RELATIVE_PATH
TOOLCHAIN_PATH = ROOT / "rust-toolchain.toml"
CI_WORKFLOW_PATH = ROOT / ".github" / "workflows" / "ci.yml"
DEV_FAST_ARGUMENT = f"--config {CONFIG_RELATIVE_PATH.as_posix()}"
LINKER_ARGUMENT = "-Clink-arg=-fuse-ld=mold"


def _make_dry_run(
    target: str,
    makefile: Path = MAKEFILE_PATH,
    *,
    force: bool = True,
    working_directory: Path = ROOT,
    build_target: str | None = None,
) -> list[str]:
    """Return the evaluated recipe lines for one target without running them."""
    environment = os.environ.copy()
    # Make's debug recipes preserve this caller-provided value while adding the
    # Linux linker flag. A fixed input makes that contract observable here.
    environment["RUSTFLAGS"] = "-D warnings"
    environment.pop("CARGO_BUILD_TARGET", None)
    if build_target is not None:
        environment["CARGO_BUILD_TARGET"] = build_target
    make_args = ["make", "--dry-run"]
    if force:
        make_args.append("-B")
    result = subprocess.run(
        [
            *make_args,
            "-f",
            str(makefile),
            target,
            "CARGO=probe-cargo",
            "WHITAKER=probe-whitaker",
        ],
        check=True,
        cwd=working_directory,
        env=environment,
        capture_output=True,
        text=True,
    )
    return result.stdout.splitlines()


def _cargo_lines(lines: list[str]) -> list[str]:
    """Return every evaluated Cargo invocation in a Make dry run."""
    return [line for line in lines if "probe-cargo" in line]


def _assert_lint_cargo_commands(cargo_lines: list[str]) -> None:
    """Require lint to invoke rustdoc and Clippy separately."""
    doc_lines = [line for line in cargo_lines if "doc" in line.split()]
    clippy_lines = [line for line in cargo_lines if "clippy" in line.split()]
    assert len(doc_lines) == 1 and len(clippy_lines) == 1, (
        "lint must retain distinct rustdoc and Clippy Cargo invocations"
    )


def _assert_dev_fast_fragment(target: str, cargo_lines: list[str]) -> None:
    """Require every Cargo line to select the development config fragment."""
    missing_fragment = [
        line for line in cargo_lines if DEV_FAST_ARGUMENT not in line
    ]
    assert not missing_fragment, (
        f"every Cargo invocation in {target} must select {DEV_FAST_ARGUMENT}:\n"
        + "\n".join(missing_fragment)
    )


def _assert_linux_linker(target: str, cargo_lines: list[str]) -> None:
    """Require the Linux debug recipes to preserve the `mold` linker flag."""
    if platform.system() == "Linux":
        missing_linker = [line for line in cargo_lines if LINKER_ARGUMENT not in line]
        assert not missing_linker, (
            f"every Linux debug Cargo invocation in {target} must preserve the "
            f"`mold` linker flag:\n" + "\n".join(missing_linker)
        )


def _assert_debug_routing(target: str, lines: list[str]) -> None:
    """Require every Cargo invocation in a debug target to select dev-fast."""
    cargo_lines = _cargo_lines(lines)
    expected_count = 2 if target == "lint" else 1
    assert len(cargo_lines) == expected_count, (
        f"{target} should produce exactly {expected_count} probe-cargo "
        f"invocations, found {len(cargo_lines)}"
    )
    assert all("probe-cargo" in line for line in cargo_lines), (
        f"{target} must honour the injected CARGO command"
    )
    if target == "lint":
        _assert_lint_cargo_commands(cargo_lines)
    _assert_dev_fast_fragment(target, cargo_lines)
    _assert_linux_linker(target, cargo_lines)


def _assert_non_debug_routing(target: str, lines: list[str]) -> None:
    """Require non-development Cargo recipes to avoid dev-fast and `mold`."""
    cargo_lines = _cargo_lines(lines)
    assert cargo_lines, f"{target} should produce a probe-cargo invocation"
    assert all("probe-cargo" in line for line in cargo_lines), (
        f"{target} must honour the injected CARGO command"
    )
    contaminated = [
        line
        for line in cargo_lines
        if DEV_FAST_ARGUMENT in line or LINKER_ARGUMENT in line
    ]
    assert not contaminated, (
        f"{target} must not select the dev-fast fragment or Make's `mold` flag:\n"
        + "\n".join(contaminated)
    )


def _load_build_test_steps() -> list[dict[str, object]]:
    """Parse the CI build-test steps for backend-provisioning assertions."""
    workflow = yaml.safe_load(CI_WORKFLOW_PATH.read_text(encoding="utf-8"))
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


def _step_named(steps: list[dict[str, object]], name: str) -> dict[str, object]:
    """Return the uniquely named CI step."""
    matches = [step for step in steps if step.get("name") == name]
    assert len(matches) == 1, f"expected one {name!r} step, found {len(matches)}"
    return matches[0]


@pytest.mark.parametrize(
    "target",
    [
        pytest.param("build", id="make_build"),
        pytest.param("test", id="make_test"),
        pytest.param("test-bdd", id="make_test_bdd"),
        pytest.param("test-doc", id="make_test_doc"),
        pytest.param("lint", id="make_lint"),
        pytest.param("typecheck", id="make_typecheck"),
        pytest.param("dev-build", id="make_dev_build"),
        pytest.param("dev-test", id="make_dev_test"),
    ],
)
def test_debug_make_targets_select_dev_fast_for_every_cargo_line(target: str) -> None:
    """Every debug Cargo line selects the explicit fragment and injected tool."""
    _assert_debug_routing(target, _make_dry_run(target))


def test_dev_build_rebuilds_with_a_preexisting_library(tmp_path: Path) -> None:
    """A cached library must not skip the explicit development build."""
    library = tmp_path / "target/debug/libwireframe.rlib"
    library.parent.mkdir(parents=True)
    library.write_bytes(b"preexisting")
    assert not _cargo_lines(
        _make_dry_run("build", force=False, working_directory=tmp_path)
    ), "the standard build should respect an existing library"
    lines = _make_dry_run("dev-build", force=False, working_directory=tmp_path)
    _assert_debug_routing("dev-build", lines)


@pytest.mark.skipif(platform.system() != "Linux", reason="Linux host linker only")
@pytest.mark.parametrize(
    "target",
    (
        "build",
        "test",
        "test-bdd",
        "test-doc",
        "lint",
        "typecheck",
        "dev-build",
        "dev-test",
    ),
)
def test_cross_target_does_not_inherit_host_linker(target: str) -> None:
    """Cross-target debug recipes retain warnings without the host linker."""
    cargo_lines = _cargo_lines(_make_dry_run(target, build_target="wasm32-wasip1"))
    assert cargo_lines, f"{target} should produce a probe-cargo invocation"
    _assert_dev_fast_fragment(target, cargo_lines)
    if target in {"build", "dev-build"}:
        assert all("RUSTFLAGS=" not in line for line in cargo_lines), (
            f"{target} must inherit caller warning flags for cross-target builds"
        )
    else:
        assert all("-D warnings" in line for line in cargo_lines), (
            f"{target} must preserve caller warning flags for cross-target builds"
        )
    assert all(LINKER_ARGUMENT not in line for line in cargo_lines), (
        f"{target} must not pass the Linux host linker to a cross target"
    )


@pytest.mark.parametrize(
    "target",
    [
        pytest.param("release", id="make_release"),
        pytest.param("test-verification", id="make_test_verification"),
        pytest.param("bench-codec", id="make_bench_codec"),
        pytest.param("fmt", id="make_fmt"),
        pytest.param("check-fmt", id="make_check_fmt"),
    ],
)
def test_non_debug_make_targets_do_not_select_dev_fast_or_linker_flag(
    target: str,
) -> None:
    """Release, verification, benchmark, and formatter recipes stay separate."""
    _assert_non_debug_routing(target, _make_dry_run(target))


def test_lint_keeps_dev_fast_off_the_whitaker_invocation() -> None:
    """Clippy and rustdoc use dev-fast while Whitaker stays on its own toolchain."""
    lines = _make_dry_run("lint")
    whitaker_lines = [line for line in lines if "probe-whitaker" in line]
    assert len(whitaker_lines) == 1, (
        f"lint should produce one injected Whitaker line, found {len(whitaker_lines)}"
    )
    assert DEV_FAST_ARGUMENT not in whitaker_lines[0], (
        "Whitaker must not receive the development Cargo configuration"
    )
    assert LINKER_ARGUMENT not in whitaker_lines[0], (
        "Whitaker must not receive Make's development linker flags"
    )


def test_dev_fast_fragment_exists_outside_cargo_auto_discovery() -> None:
    """The explicit config keeps dev builds fast and tests on LLVM."""
    assert CONFIG_PATH.is_file(), f"missing {CONFIG_RELATIVE_PATH}"
    assert ".cargo" not in CONFIG_RELATIVE_PATH.parts, (
        "dev-fast must stay outside Cargo's automatically discovered config path"
    )
    configuration = tomllib.loads(CONFIG_PATH.read_text(encoding="utf-8"))
    assert configuration.get("unstable", {}).get("codegen-backend") is True, (
        "the dev-fast fragment must enable the pinned experimental backend"
    )
    assert configuration.get("profile", {}).get("dev", {}).get(
        "codegen-backend"
    ) == "cranelift", "debug builds must select Cranelift"
    assert configuration.get("profile", {}).get("test", {}).get(
        "codegen-backend"
    ) == "llvm", (
        "test binaries stay on LLVM to protect against the pinned-nightly regression"
    )
    linux_target = configuration.get("target", {}).get(
        "cfg(target_os = \"linux\")", {}
    )
    assert LINKER_ARGUMENT in linux_target.get("rustflags", []), (
        "the Linux dev-fast fragment must retain the `mold` linker setting"
    )


def test_pinned_toolchain_declares_cranelift_component() -> None:
    """The repository toolchain provisions the component selected by dev-fast."""
    toolchain = tomllib.loads(TOOLCHAIN_PATH.read_text(encoding="utf-8"))
    components = toolchain.get("toolchain", {}).get("components", [])
    assert "rustc-codegen-cranelift-preview" in components, (
        "rust-toolchain.toml must install the Cranelift backend component"
    )


def test_ci_provisions_backend_and_linker_before_lint() -> None:
    """CI installs Cranelift and `mold` after Rust setup and before lint."""
    steps = _load_build_test_steps()
    setup = _step_named(steps, "Setup Rust")
    provision = _step_named(steps, "Install development backend and linker")
    lint = _step_named(steps, "Lint")
    assert steps.index(setup) < steps.index(provision) < steps.index(lint), (
        "development backend and linker provisioning must follow Rust setup "
        "and precede lint"
    )
    commands = provision.get("run")
    assert isinstance(commands, str), "backend and linker provisioning needs run commands"
    required_commands = (
        "rustup component add rustc-codegen-cranelift-preview "
        "--toolchain nightly-2026-03-26",
        "sudo apt-get update",
        "sudo apt-get install -y mold",
        "mold --version",
    )
    for command in required_commands:
        assert command in commands, (
            f"CI backend/linker provisioning is missing {command!r}"
        )
    command_positions = [commands.index(command) for command in required_commands]
    assert command_positions == sorted(command_positions), (
        "CI must install Cranelift, then `mold`, and confirm the linker version"
    )


def test_debug_contract_detects_a_removed_fragment(tmp_path: Path) -> None:
    """A private Makefile mutation proves the positive contract is binding."""
    source = MAKEFILE_PATH.read_text(encoding="utf-8")
    original = (
        '\tRUSTFLAGS="$(DEV_WARNING_FLAGS)" $(CARGO) $(DEV_FAST) test '
        "--workspace --all-targets --all-features $(BUILD_JOBS)"
    )
    mutated = (
        '\tRUSTFLAGS="$(DEV_WARNING_FLAGS)" $(CARGO) test '
        "--workspace --all-targets --all-features $(BUILD_JOBS)"
    )
    assert source.count(original) == 1, "the test recipe mutation must be unambiguous"
    mutated_makefile = tmp_path / "Makefile"
    mutated_makefile.write_text(source.replace(original, mutated), encoding="utf-8")
    lines = _make_dry_run("test", mutated_makefile)
    with pytest.raises(AssertionError, match="must select --config"):
        _assert_debug_routing("test", lines)


def test_lint_contract_detects_a_removed_cargo_invocation(tmp_path: Path) -> None:
    """A private Makefile mutation proves both lint Cargo lines are required."""
    source = MAKEFILE_PATH.read_text(encoding="utf-8")
    removed = (
        '\tRUSTFLAGS="$(DEV_WARNING_FLAGS)" RUSTDOCFLAGS="$(RUSTDOC_FLAGS)" '
        "$(CARGO) $(DEV_FAST) doc --workspace --no-deps\n"
    )
    assert source.count(removed) == 1, "the lint recipe mutation must be unambiguous"
    mutated_makefile = tmp_path / "Makefile"
    mutated_makefile.write_text(source.replace(removed, "", 1), encoding="utf-8")
    lines = _make_dry_run("lint", mutated_makefile)
    with pytest.raises(AssertionError, match="exactly 2 probe-cargo invocations"):
        _assert_debug_routing("lint", lines)


def test_non_debug_contract_detects_dev_fast_added_to_release(tmp_path: Path) -> None:
    """A private Makefile mutation proves the negative contract is binding."""
    source = MAKEFILE_PATH.read_text(encoding="utf-8")
    original = "$(if $(findstring release,$(@)),,$(DEV_FAST))"
    mutated = "$(DEV_FAST)"
    assert source.count(original) == 1, "the release recipe mutation must be unambiguous"
    mutated_makefile = tmp_path / "Makefile"
    mutated_makefile.write_text(source.replace(original, mutated), encoding="utf-8")
    lines = _make_dry_run("release", mutated_makefile)
    with pytest.raises(AssertionError, match="must not select the dev-fast"):
        _assert_non_debug_routing("release", lines)
