"""Hold the Markdown doctest gate to compiling real fences and failing on bad ones.

``src/tested_documentation.rs`` includes every published guide with
``#[doc = include_str!(...)]``, so ``make test-doc`` compiles the Rust fences in
the shipped Markdown. Two contracts prove that gate.

The mechanism contract builds a scratch crate and shows that a marked fence
which compiles passes the gate while one with a type error fails it. The
shipped-tree contract copies the repository's published inputs to a temporary
root and runs the real gates there: ``make test-doc`` passes on the untouched
copy, fails with ``error[E0308]`` once a non-compiling fence is added to a
shipped guide, and the loader gate fails with the document path and
``missing its marker`` once a marker is removed. The marker comment is inert to
rustdoc, so marker validation is exercised through the loader test that
``tests/documentation_examples.rs`` already provides.

Run these contracts with ``make test-workflow-contracts``.
"""

from __future__ import annotations

import os
import shutil
import subprocess
from pathlib import Path

REPOSITORY_ROOT = Path(__file__).resolve().parents[2]

# Rebuilding the crate under a second path is slow, so the copied tree borrows
# the repository's target directory. Its dependency artefacts are already
# compiled with the same flags, so only this crate and its doctests are built.
SHARED_TARGET = REPOSITORY_ROOT / "target"

SHIPPED_GUIDE = "docs/users-guide.md"
SHIPPED_MARKER = "<!-- tested-example: guide-dependencies -->\n"

NON_COMPILING_FENCE = """\
<!-- tested-example: users-guide-9999 -->
```rust
fn main() {
    let total: u8 = "not a number";
    assert_eq!(total, 2);
}
```
"""

# Build output and caches are neither inputs to the gate nor cheap to copy. The
# `.uv-cache` directory in particular is regenerated, not read.
COPY_IGNORES = (
    "target",
    ".git",
    ".venv",
    ".uv-cache",
    "__pycache__",
    ".pytest_cache",
    ".hypothesis",
    ".ruff_cache",
    ".mypy_cache",
    "node_modules",
)

SCRATCH_MANIFEST = """\
[package]
name = "scratch-guide"
version = "0.0.0"
edition = "2024"

[lib]
path = "src/lib.rs"
"""

SCRATCH_LIB = """\
//! Scratch crate carrying a published guide through the same include the
//! repository uses.

#[doc = include_str!("guide.md")]
pub mod guide {}
"""

COMPILING_GUIDE = """\
# Scratch guide

<!-- tested-example: scratch-001 -->
```rust
fn main() {
    let total = 1 + 1;
    assert_eq!(total, 2);
}
```
"""

NON_COMPILING_GUIDE = """\
# Scratch guide

<!-- tested-example: scratch-001 -->
```rust
fn main() {
    let total: u8 = "not a number";
    assert_eq!(total, 2);
}
```
"""


def _run_doctests(root: Path, guide: str) -> subprocess.CompletedProcess[str]:
    """Run the Markdown doctests for ``guide`` in the scratch crate at ``root``."""
    (root / "src").mkdir(parents=True, exist_ok=True)
    (root / "Cargo.toml").write_text(SCRATCH_MANIFEST, encoding="utf-8")
    (root / "src" / "lib.rs").write_text(SCRATCH_LIB, encoding="utf-8")
    (root / "src" / "guide.md").write_text(guide, encoding="utf-8")
    cargo = shutil.which("cargo")
    assert cargo is not None, "the doctest contract needs cargo on PATH"
    environment = os.environ.copy()
    # Cargo colours "error:" on a terminal-attached runner, which would hide the
    # markers the assertions look for.
    environment["CARGO_TERM_COLOR"] = "never"
    return subprocess.run(
        [cargo, "test", "--offline", "--doc", "--target-dir", str(root / "target")],
        cwd=root,
        env=environment,
        capture_output=True,
        text=True,
        check=False,
    )


def test_a_compiling_published_example_passes_the_doctest_gate(tmp_path: Path) -> None:
    """A marked Rust fence that compiles is executed by the doctest gate."""
    completed = _run_doctests(tmp_path, COMPILING_GUIDE)
    output = completed.stdout + completed.stderr
    assert completed.returncode == 0, f"expected a passing doctest, saw:\n{output}"
    assert "1 passed" in output, f"the guide should contribute one doctest, saw:\n{output}"


def test_a_non_compiling_published_example_fails_the_doctest_gate(
    tmp_path: Path,
) -> None:
    """A marked Rust fence with a type error fails the gate with a compiler error."""
    completed = _run_doctests(tmp_path, NON_COMPILING_GUIDE)
    output = completed.stdout + completed.stderr
    assert completed.returncode != 0, f"the gate must fail, saw:\n{output}"
    assert "error[E0308]" in output, f"expected a type error, saw:\n{output}"


def _copy_published_inputs(destination: Path) -> Path:
    """Copy the repository, without build output, into ``destination``."""
    root = destination / "repository"
    shutil.copytree(
        REPOSITORY_ROOT,
        root,
        ignore=shutil.ignore_patterns(*COPY_IGNORES),
    )
    return root


def _gate_environment() -> dict[str, str]:
    """Return the environment that points a copied build at the shared target."""
    environment = os.environ.copy()
    environment["CARGO_TERM_COLOR"] = "never"
    environment["CARGO_TARGET_DIR"] = str(SHARED_TARGET)
    return environment


def _run_in_copy(root: Path, command: list[str]) -> subprocess.CompletedProcess[str]:
    """Run ``command`` in the copied repository and capture its output."""
    return subprocess.run(
        command,
        cwd=root,
        env=_gate_environment(),
        capture_output=True,
        text=True,
        check=False,
        timeout=3600,
    )


def test_the_rustdoc_gate_runs_shipped_documents(tmp_path: Path) -> None:
    """The real gates pass on a copy of the shipped inputs and fail when broken."""
    root = _copy_published_inputs(tmp_path)
    make = shutil.which("make")
    cargo = shutil.which("cargo")
    assert make is not None, "the shipped-document contract needs make on PATH"
    assert cargo is not None, "the shipped-document contract needs cargo on PATH"

    unchanged = _run_in_copy(root, [make, "test-doc"])
    unchanged_output = unchanged.stdout + unchanged.stderr
    assert unchanged.returncode == 0, (
        "the shipped guides must pass the doctest gate:\n"
        f"{unchanged_output[-4000:]}"
    )

    guide = root / SHIPPED_GUIDE
    original = guide.read_text(encoding="utf-8")
    # A blank line keeps the added block a separate Markdown paragraph, so the
    # new fence is a doctest in its own right rather than a continuation.
    guide.write_text(
        f"{original.rstrip()}\n\n{NON_COMPILING_FENCE}", encoding="utf-8"
    )

    broken = _run_in_copy(root, [make, "test-doc"])
    broken_output = broken.stdout + broken.stderr
    assert broken.returncode != 0, (
        "a shipped guide with a non-compiling fence must fail the gate:\n"
        f"{broken_output[-4000:]}"
    )
    assert "error[E0308]" in broken_output, (
        f"expected the type error from the added fence, saw:\n{broken_output[-4000:]}"
    )

    guide.write_text(original.replace(SHIPPED_MARKER, "", 1), encoding="utf-8")
    loader = _run_in_copy(
        root,
        [
            cargo,
            "test",
            "--offline",
            "--test",
            "documentation_examples",
            "--all-features",
        ],
    )
    loader_output = loader.stdout + loader.stderr
    assert loader.returncode != 0, (
        "a shipped guide without one marker must fail the loader gate:\n"
        f"{loader_output[-4000:]}"
    )
    assert "missing its marker" in loader_output, (
        f"expected the missing-marker diagnostic, saw:\n{loader_output[-4000:]}"
    )
    assert SHIPPED_GUIDE in loader_output, (
        f"expected the failure to name {SHIPPED_GUIDE}, saw:\n{loader_output[-4000:]}"
    )