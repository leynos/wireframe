"""Protect dependency scopes in the root and verification manifests.

Check the root manifest against example, request-streaming and doctest source
uses, and ensure the verification crate retains its model checker without a
runtime dependency on the main crate.
"""

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
    assert name not in manifest["dependencies"], (
        f"{name} must not be a runtime dependency in Cargo.toml"
    )
    for target_name, target in manifest.get("target", {}).items():
        assert name not in target.get("dependencies", {}), (
            f"{name} must not be a runtime dependency for {target_name}"
        )


def test_tracing_subscriber_remains_available_to_tests(manifest: dict[str, typ.Any]) -> None:
    """Examples, benchmarks and tests retain their tracing subscriber."""
    subscriber = manifest["dev-dependencies"]["tracing-subscriber"]
    assert "env-filter" in subscriber["features"], (
        "tracing-subscriber must enable env-filter for benchmarks"
    )


def test_prometheus_exporter_remains_available_to_non_loom_doctests(manifest: dict[str, typ.Any]) -> None:
    """The documented integration uses a development-only HTTP exporter."""
    exporter = manifest["target"]["cfg(not(loom))"]["dev-dependencies"][
        "metrics-exporter-prometheus"
    ]
    assert "http-listener" in exporter["features"], (
        "metrics-exporter-prometheus must enable http-listener for doctests"
    )
    assert not exporter.get("optional", False), (
        "metrics-exporter-prometheus must be available to non-Loom doctests"
    )
    assert "use metrics_exporter_prometheus::PrometheusBuilder;" in (
        ROOT / "src" / "metrics.rs"
    ).read_text(), "src/metrics.rs must retain the documented Prometheus example"


def test_metrics_feature_does_not_enable_exporter(manifest: dict[str, typ.Any]) -> None:
    """Runtime metrics emit through the facade without installing an exporter."""
    assert manifest["features"]["metrics"] == ["dep:metrics"], (
        "the metrics feature must enable only the runtime metrics facade"
    )


def test_request_stream_reader_enables_tokio_util_io_directly(manifest: dict[str, typ.Any]) -> None:
    """Request streaming must not rely on exporter feature unification."""
    assert "io" in manifest["dependencies"]["tokio-util"]["features"], (
        "tokio-util must enable io directly for request streaming"
    )
    assert "use tokio_util::io::StreamReader;" in (
        ROOT / "src" / "request" / "mod.rs"
    ).read_text(), "src/request/mod.rs must retain its StreamReader import"


def test_itoa_remains_available_to_resp_example(manifest: dict[str, typ.Any]) -> None:
    """The RESP example can format integers without a runtime dependency."""
    assert "itoa" in manifest["dev-dependencies"], (
        "itoa must remain a dev dependency for the RESP example"
    )
    example = ROOT / "examples" / "resp_codec_impl" / "encode.rs"
    assert "itoa::Buffer::new()" in example.read_text(), (
        "examples/resp_codec_impl/encode.rs must retain its itoa use"
    )


def test_verification_crate_retains_model_checker_without_wireframe() -> None:
    """The model checker needs stateright but does not depend on wireframe."""
    path = ROOT / "crates" / "wireframe-verification" / "Cargo.toml"
    with path.open("rb") as cargo_file:
        verification = tomllib.load(cargo_file)
    dependencies = verification["dependencies"]
    assert "stateright" in dependencies, (
        "wireframe-verification must retain its stateright runtime dependency"
    )
    assert "wireframe" not in dependencies, (
        "wireframe-verification must not depend on the unused wireframe crate"
    )
