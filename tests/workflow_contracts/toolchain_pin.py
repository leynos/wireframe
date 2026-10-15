"""Read the repository toolchain pin for build contract tests."""

from __future__ import annotations

from functools import lru_cache
from pathlib import Path
import tomllib
from typing import Any

ROOT = Path(__file__).resolve().parents[2]


@lru_cache(maxsize=1)
def pinned_toolchain() -> dict[str, Any]:
    """Return the parsed repository toolchain manifest once per test process."""
    manifest = ROOT / "rust-toolchain.toml"
    return tomllib.loads(manifest.read_text(encoding="utf-8"))


def pinned_channel() -> str:
    """Return the pinned Rust toolchain channel from the manifest."""
    toolchain = pinned_toolchain().get("toolchain")
    if not isinstance(toolchain, dict):
        raise TypeError("expected [toolchain] mapping")
    channel = toolchain.get("channel")
    if not isinstance(channel, str):
        raise TypeError("expected string toolchain.channel")
    return channel
