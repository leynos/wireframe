"""Protect the development-only scope of test and doctest dependencies."""

from __future__ import annotations

import tomllib
import typing as typ
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]


@pytest.fixture
def manifest() -> dict[str, typ.Any]:
    """Read Cargo's dependency tables as structured data."""
    with (ROOT / "Cargo.toml").open("rb") as cargo_file:
        return tomllib.load(cargo_file)


@pytest.mark.parametrize("name", ["tracing-subscriber", "metrics-exporter-prometheus", "itoa"])
def test_test_dependencies_are_not_runtime_dependencies(manifest: dict[str, typ.Any], name: str) -> None:
    """Test and doctest helpers must not enter the published runtime graph."""
    assert name not in manifest["dependencies"]
    for target in manifest.get("target", {}).values():
        assert name not in target.get("dependencies", {})


def test_tracing_subscriber_remains_available_to_tests(manifest: dict[str, typ.Any]) -> None:
    """Examples, benchmarks and tests retain their tracing subscriber."""
    subscriber = manifest["dev-dependencies"]["tracing-subscriber"]
    assert "env-filter" in subscriber["features"]


def test_prometheus_exporter_remains_available_to_non_loom_doctests(manifest: dict[str, typ.Any]) -> None:
    """The documented integration uses a development-only HTTP exporter."""
    exporter = manifest["target"]["cfg(not(loom))"]["dev-dependencies"][
        "metrics-exporter-prometheus"
    ]
    assert "http-listener" in exporter["features"]
    assert not exporter.get("optional", False)
    assert "use metrics_exporter_prometheus::PrometheusBuilder;" in (
        ROOT / "src" / "metrics.rs"
    ).read_text()


def test_metrics_feature_does_not_enable_exporter(manifest: dict[str, typ.Any]) -> None:
    """Runtime metrics emit through the facade without installing an exporter."""
    assert manifest["features"]["metrics"] == ["dep:metrics"]


def test_request_stream_reader_enables_tokio_util_io_directly(manifest: dict[str, typ.Any]) -> None:
    """Request streaming must not rely on exporter feature unification."""
    assert "io" in manifest["dependencies"]["tokio-util"]["features"]
    assert "use tokio_util::io::StreamReader;" in (
        ROOT / "src" / "request" / "mod.rs"
    ).read_text()


def test_itoa_remains_available_to_resp_example(manifest: dict[str, typ.Any]) -> None:
    """The RESP example can format integers without a runtime dependency."""
    assert "itoa" in manifest["dev-dependencies"]
    example = ROOT / "examples" / "resp_codec_impl" / "encode.rs"
    assert "itoa::Buffer::new()" in example.read_text()
