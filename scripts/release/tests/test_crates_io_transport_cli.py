"""Specify bounded crates.io probe and downloaded-byte transport."""

import io
import json
import typing as typ
import urllib.error
import urllib.request
import urllib.response
from email.message import Message

import crates_io_registry_http
import pytest
from crates_io_registry_http import RegistryUrlError
from crates_io_transport_cli import (
    DOWNLOAD_ATTEMPTS,
    EXIT_OK,
    TransportTiming,
    main,
)
from crates_io_transport_support import (
    CRATE_NAME,
    VERSION,
    FakeOpener,
    FakeResponse,
    HandlerListOwner,
    ResponseWithMessage,
    download_args,
    http_error_response,
    missing_response,
    probe_args,
)

if typ.TYPE_CHECKING:
    import pathlib

REGISTRY_PREFIX = "https://crates.io/api/v1/crates/"


@pytest.mark.parametrize(
    "url",
    [
        pytest.param("https://attacker.example/forged-response", id="unallowlisted"),
        pytest.param("http://crates.io/api/v1", id="plain-http"),
        pytest.param("https://user@crates.io/api/v1", id="userinfo"),
        pytest.param("https://crates.io:443/api/v1", id="explicit-port"),
        pytest.param("https://crates.io/api/v1#fragment", id="fragment"),
        pytest.param("https://crates.io:abc/api/v1", id="non-numeric-port"),
        pytest.param("https://crates.io:99999/api/v1", id="out-of-range-port"),
        pytest.param("https://[::1/api/v1", id="unbracketed-ipv6"),
    ],
)
def test_build_request_rejects_every_disallowed_initial_url(url: str) -> None:
    """Reject every URL-policy violation at the public request builder."""
    with pytest.raises(RegistryUrlError):
        crates_io_registry_http.build_request(url, headers={})


def _install_redirect(monkeypatch: pytest.MonkeyPatch, location: str) -> None:
    """Make the shared opener answer every HTTPS request with one redirect."""
    opener = typ.cast("HandlerListOwner", crates_io_registry_http._HTTP_OPENER)
    https_handler = next(
        handler
        for handler in opener.handlers
        if isinstance(handler, urllib.request.HTTPSHandler)
    )

    def redirect_response(
        request: urllib.request.Request,
    ) -> urllib.response.addinfourl:
        """Return a 302 response to the configured ``Location`` value.

        Parameters
        ----------
        request : urllib.request.Request
            Request whose URL becomes the response origin.

        Returns
        -------
        urllib.response.addinfourl
            A redirect to the supplied test location.

        Examples
        --------
        >>> redirect_response(urllib.request.Request("https://crates.io")).code
        302
        """
        headers = Message()
        headers["Location"] = location
        response = urllib.response.addinfourl(
            io.BytesIO(), headers, request.full_url, code=302
        )
        typ.cast("ResponseWithMessage", response).msg = "Found"
        return response

    monkeypatch.setattr(https_handler, "https_open", redirect_response)


@pytest.mark.parametrize(
    "location",
    [
        pytest.param("https://attacker.example/forged-response", id="unallowlisted"),
        pytest.param("http://crates.io/forged", id="plain-http"),
        pytest.param("https://user@crates.io/forged", id="userinfo"),
        pytest.param("https://crates.io:443/forged", id="explicit-port"),
        pytest.param("https://crates.io/forged#fragment", id="fragment"),
        pytest.param("https://crates.io:abc/forged", id="non-numeric-port"),
        pytest.param("https://crates.io:99999/forged", id="out-of-range-port"),
        pytest.param("https://[::1/forged", id="unbracketed-ipv6"),
    ],
)
def test_default_opener_rejects_every_bad_redirect_target(
    monkeypatch: pytest.MonkeyPatch, location: str
) -> None:
    """Classify every rejected redirect target as a registry URL error."""
    _install_redirect(monkeypatch, location)
    request = crates_io_registry_http.build_request(
        f"{REGISTRY_PREFIX}{CRATE_NAME}/{VERSION}", headers={}
    )

    with pytest.raises(RegistryUrlError):
        crates_io_registry_http.default_opener(request, timeout=1.0)


@pytest.mark.parametrize(
    "url",
    [
        pytest.param("https://attacker.example/api/v1", id="unallowlisted-host"),
        pytest.param("http://crates.io/api/v1", id="plain-http"),
        pytest.param("https://user@crates.io/api/v1", id="userinfo"),
        pytest.param("https://crates.io:443/api/v1", id="explicit-port"),
        pytest.param("https://crates.io/api/v1#fragment", id="fragment"),
        pytest.param("https://crates.io:abc/api/v1", id="non-numeric-port"),
        pytest.param("https://crates.io:99999/api/v1", id="out-of-range-port"),
        pytest.param("https://[::1/api/v1", id="unbracketed-ipv6"),
    ],
)
def test_is_registry_url_rejects_out_of_policy_and_malformed_urls(url: str) -> None:
    """Answer ``False`` for every URL outside the registry policy.

    The malformed cases matter because urllib raises ``ValueError`` from
    ``urlsplit`` and from the ``port`` property rather than rejecting them.
    """
    assert not crates_io_registry_http.is_registry_url(url), (
        f"{url!r} must not be treated as an allowlisted registry URL"
    )


def test_probe_saves_http_status_and_body_without_retry(
    tmp_path: pathlib.Path,
) -> None:
    """Record a response once, including an HTTPError that carries a status."""
    status = tmp_path / "status.txt"
    body = tmp_path / "body.json"
    opener = FakeOpener(
        missing_response(
            "https://crates.io/api/v1/crates/wireframe/1.2.3",
            b'{"errors":[]}',
        )
    )

    result = main(probe_args(status, body), opener=opener)

    assert result == EXIT_OK, "a recorded HTTP response should complete the probe"
    assert len(opener.requests) == 1, "the registry probe must not retry"
    assert status.read_text(encoding="ascii").strip() == "404", (
        "save the registry's HTTP status"
    )
    assert body.read_bytes() == b'{"errors":[]}', "save the response body unchanged"


def test_probe_uses_the_adapter_user_agent_and_bounded_timeout(
    tmp_path: pathlib.Path,
) -> None:
    """Keep registry identity and the request timeout at the adapter boundary."""
    opener = FakeOpener(FakeResponse(200, b'{"version":{}}'))
    result = main(probe_args(tmp_path / "status", tmp_path / "body"), opener=opener)

    assert result == EXIT_OK, "a successful probe should record its response"
    request, timeout = opener.requests[0]
    user_agent = request.get_header("User-agent")
    assert user_agent is not None, "set the registry client User-Agent"
    assert user_agent.startswith("wireframe"), (
        "identify the registry client with the required User-Agent"
    )
    assert timeout > 0, "bound the request with a positive timeout"


def test_download_retries_transient_responses_then_stops_after_success(
    tmp_path: pathlib.Path,
) -> None:
    """Retry a 5xx response but never repeat a successful archive download."""
    delays: list[float] = []
    opener = FakeOpener(
        http_error_response(
            "https://static.crates.io/crates/wireframe/wireframe-1.2.3.crate",
            503,
            b"busy",
        ),
        FakeResponse(200, b"exact registry archive"),
    )
    archive = tmp_path / "candidate.crate"
    status = tmp_path / "status.txt"
    report = tmp_path / "download-report.json"
    clock_samples = iter((1_000_000_000, 21_123_000_000))

    result = main(
        download_args(archive, status, report),
        opener=opener,
        timing=TransportTiming(
            sleeper=delays.append,
            monotonic_clock_ns=lambda: next(clock_samples),
        ),
    )

    assert result == EXIT_OK, "the download step records its final response"
    assert len(opener.requests) == 2, "retry the transient response once before success"
    assert delays == [10.0], "wait the configured delay between retries"
    assert archive.read_bytes() == b"exact registry archive", (
        "retain the downloaded registry bytes unchanged"
    )
    assert status.read_text(encoding="ascii").strip() == "200", (
        "record the successful download status"
    )
    diagnostics = json.loads(report.read_text(encoding="ascii"))
    assert diagnostics["attempts"] == 2, "record the retry count"
    assert diagnostics["status_class"] == "2xx", "record only the status class"
    assert diagnostics["elapsed_ms"] == 20_123, "convert injected nanoseconds to ms"


def test_download_retries_absence_only_to_its_fixed_attempt_budget(
    tmp_path: pathlib.Path,
) -> None:
    """Keep eventual archive propagation inside a finite polling budget."""
    download_url = (
        "https://static.crates.io/crates/wireframe/wireframe-1.2.3.crate"
    )
    opener = FakeOpener(*[
        http_error_response(download_url, 404, b"absent")
        for _ in range(DOWNLOAD_ATTEMPTS)
    ])
    delays: list[float] = []
    archive = tmp_path / "candidate.crate"
    status = tmp_path / "status.txt"
    report = tmp_path / "download-report.json"
    clock_samples = iter((4_000_000_000, 11_000_000_000))

    result = main(
        download_args(archive, status, report),
        opener=opener,
        timing=TransportTiming(
            sleeper=delays.append,
            monotonic_clock_ns=lambda: next(clock_samples),
        ),
    )

    assert result == EXIT_OK, "the transport records exhausted polling for verification"
    assert len(opener.requests) == DOWNLOAD_ATTEMPTS, "stop at the fixed attempt budget"
    assert len(delays) == DOWNLOAD_ATTEMPTS - 1, "do not delay after the final attempt"
    assert status.read_text(encoding="ascii").strip() == "404", (
        "retain the final absence status"
    )
    assert archive.read_bytes() == b"absent", "retain the final response body"
    diagnostics = json.loads(report.read_text(encoding="ascii"))
    assert diagnostics["attempts"] == DOWNLOAD_ATTEMPTS, "record all attempts"
    assert diagnostics["elapsed_ms"] == 7_000, "time exhausted polling exactly"


def test_download_report_clamps_a_backwards_clock_to_zero(
    tmp_path: pathlib.Path,
) -> None:
    """Keep elapsed download telemetry non-negative when a clock regresses."""
    archive = tmp_path / "candidate.crate"
    status = tmp_path / "status.txt"
    report = tmp_path / "download-report.json"
    clock_samples = iter((2_000_000_000, 1_000_000_000))
    opener = FakeOpener(FakeResponse(200, b"archive"))

    result = main(
        download_args(archive, status, report),
        opener=opener,
        timing=TransportTiming(monotonic_clock_ns=lambda: next(clock_samples)),
    )

    assert result == EXIT_OK, "a completed download should be recorded"
    diagnostics = json.loads(report.read_text(encoding="ascii"))
    assert diagnostics["elapsed_ms"] == 0, "clamp elapsed time at zero"


def test_download_records_no_status_after_transient_network_exhaustion(
    tmp_path: pathlib.Path,
) -> None:
    """Save an indeterminate sentinel so the offline verifier fails closed."""
    failures = [urllib.error.URLError("offline") for _ in range(DOWNLOAD_ATTEMPTS)]
    opener = FakeOpener(*failures)
    archive = tmp_path / "candidate.crate"
    status = tmp_path / "status.txt"

    result = main(
        download_args(archive, status),
        opener=opener,
        timing=TransportTiming(sleeper=lambda _delay: None),
    )

    assert result == EXIT_OK, "the offline verifier consumes the transport sentinel"
    assert status.read_text(encoding="ascii").strip() == "0", (
        "record missing HTTP status as indeterminate"
    )
    assert archive.read_bytes() == b"", "do not invent a response body"


def test_download_stops_after_a_rejected_redirect_target(
    tmp_path: pathlib.Path,
) -> None:
    """Do not retry an allowlist rejection that cannot change between attempts."""
    delays: list[float] = []
    opener = FakeOpener(RegistryUrlError())
    archive = tmp_path / "candidate.crate"
    status = tmp_path / "status.txt"
    report = tmp_path / "download-report.json"

    result = main(
        download_args(archive, status, report),
        opener=opener,
        timing=TransportTiming(sleeper=delays.append),
    )

    assert result == EXIT_OK, "the rejected download still records a sentinel"
    assert len(opener.requests) == 1, "do not retry a deterministic rejection"
    assert not delays, "do not delay when the rejection cannot settle"
    assert status.read_text(encoding="ascii").strip() == "0", (
        "record the rejected download as indeterminate"
    )
    assert archive.read_bytes() == b"", "do not invent a response body"
    diagnostics = json.loads(report.read_text(encoding="ascii"))
    assert diagnostics["attempts"] == 1, "report the single attempt"
