"""Exercise registry transport boundaries that must fail closed."""

import http.client
import json
import typing as typ
import urllib.error
import urllib.request

import pytest
from crates_io_registry_http import RegistryUrlError, _AllowlistedRequestHandler
from crates_io_transport_cli import (
    DOWNLOAD_ATTEMPTS,
    EXIT_OK,
    EXIT_RECORD_ERROR,
    EXIT_TRANSPORT_ERROR,
    TransportTiming,
    main,
)
from crates_io_transport_support import (
    FakeOpener,
    FakeResponse,
    download_args,
    probe_args,
)

if typ.TYPE_CHECKING:
    import pathlib

@pytest.mark.parametrize(
    ("url", "hook"),
    [
        pytest.param(
            "https://attacker.example/forged-response",
            "https_request",
            id="unallowlisted-host",
        ),
        pytest.param("ftp://crates.io/forged-response", "ftp_request", id="ftp"),
    ],
)
def test_request_handler_rejects_an_unallowlisted_redirect_target(
    url: str, hook: str
) -> None:
    """Recheck the URL urllib opens after following a registry redirect."""
    handler = _AllowlistedRequestHandler(frozenset({"crates.io"}))
    request = urllib.request.Request(
        url
    )

    with pytest.raises(RegistryUrlError):
        getattr(handler, hook)(request)


def test_rejected_redirect_fails_closed(tmp_path: pathlib.Path) -> None:
    """Treat rejected redirect targets as an unavailable registry observation."""
    opener = FakeOpener(RegistryUrlError())

    result = main(probe_args(tmp_path / "status", tmp_path / "body"), opener=opener)

    assert result == EXIT_TRANSPORT_ERROR, "reject a forged registry probe"
    assert len(opener.requests) == 1, "do not retry a rejected probe redirect"


def test_probe_fails_closed_when_no_http_status_exists(tmp_path: pathlib.Path) -> None:
    """Do not turn a connection failure into an absent registry version."""
    status = tmp_path / "status.txt"
    body = tmp_path / "body.json"
    opener = FakeOpener(urllib.error.URLError("offline"))

    result = main(probe_args(status, body), opener=opener)

    assert result == EXIT_TRANSPORT_ERROR, "a probe without status must fail closed"
    assert len(opener.requests) == 1, "the failed probe must not retry"
    assert not status.exists(), "a missing status must not be written as HTTP 404"


def test_incomplete_probe_fails_closed(tmp_path: pathlib.Path) -> None:
    """Do not classify a truncated probe body as a registry observation."""
    status = tmp_path / "status"
    body = tmp_path / "body"
    opener = FakeOpener(http.client.IncompleteRead(b"", 1))

    result = main(probe_args(status, body), opener=opener)

    assert result == EXIT_TRANSPORT_ERROR, "an incomplete probe has no usable status"
    assert len(opener.requests) == 1, "a registry probe is single-shot"
    assert not status.exists(), "a partial body must not create a status observation"


def test_download_retries_incomplete_http_responses_to_its_budget(
    tmp_path: pathlib.Path,
) -> None:
    """Write the unavailable sentinel after bounded retries of truncated bodies."""
    archive = tmp_path / "archive.crate"
    status = tmp_path / "status"
    report = tmp_path / "download-report.json"
    delays: list[float] = []
    opener = FakeOpener(*[
        http.client.IncompleteRead(b"", 1) for _ in range(DOWNLOAD_ATTEMPTS)
    ])

    result = main(
        download_args(archive, status, report),
        opener=opener,
        timing=TransportTiming(sleeper=delays.append),
    )

    assert result == EXIT_OK, "the offline verifier handles the sentinel"
    assert len(opener.requests) == DOWNLOAD_ATTEMPTS, "honour the fixed retry budget"
    assert len(delays) == DOWNLOAD_ATTEMPTS - 1, "do not wait after the final attempt"
    assert status.read_text(encoding="ascii").strip() == "0", (
        "represent an incomplete response without an HTTP status"
    )
    assert archive.read_bytes() == b"", "do not retain a partial archive"
    diagnostics = json.loads(report.read_text(encoding="ascii"))
    assert diagnostics["attempts"] == DOWNLOAD_ATTEMPTS, "record bounded attempts"
    assert diagnostics["status_class"] == "unknown", "hide absent status details"
    assert diagnostics["elapsed_ms"] >= 0, "record non-negative elapsed time"


@pytest.mark.parametrize("unwritable", ["status", "body"])
def test_probe_reports_a_local_write_failure_apart_from_transport(
    tmp_path: pathlib.Path,
    capsys: pytest.CaptureFixture[str],
    unwritable: str,
) -> None:
    """Report an unwritable output path as a recording failure, not as offline."""
    missing_dir = tmp_path / "missing-directory"
    paths = {"status": tmp_path / "status", "body": tmp_path / "body"}
    paths[unwritable] = missing_dir / unwritable
    opener = FakeOpener(FakeResponse(200, b'{"version":{}}'))

    result = main(probe_args(paths["status"], paths["body"]), opener=opener)

    stderr = capsys.readouterr().err
    assert result == EXIT_RECORD_ERROR, "a local write failure has its own exit code"
    assert "could not be recorded" in stderr, "name the local write failure"
    assert "HTTP 200" in stderr, "report the status the registry did return"
    assert "returned no HTTP status" not in stderr, (
        "a write failure must not be reported as a transport failure"
    )
