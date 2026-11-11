"""Hold the Markdown doctest gate to compiling real fences and failing on bad ones.

``src/tested_documentation.rs`` includes every published guide with
``#[doc = include_str!(...)]``, so ``make test-doc`` compiles the Rust fences in
the shipped Markdown. This contract exercises that mechanism in a scratch crate:
a marked fence that compiles passes the gate, and a marked fence with a type
error fails it. The marker comment is inert to rustdoc, so marker validation
stays with the loader tests in ``tests/documentation_examples.rs``.

Run these contracts with ``make test-workflow-contracts``.
"""

from __future__ import annotations

import os
import shutil
import subprocess
from pathlib import Path

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