"""Drive bounded crates.io probe and download commands.

The registry HTTP adapter owns request construction and redirect policy; the
transport HTTP adapter captures responses. This CLI writes observations for
offline policy commands and injects testable external boundaries. Publication
belongs to lading, so no command here takes a credential.
"""

import argparse
import collections.abc as cabc
import dataclasses
import http.client
import pathlib
import sys
import time
import urllib.parse

import crates_io_registry_http
import crates_io_transport_http
import release_crates_io_adapter

EXIT_OK = 0
EXIT_TRANSPORT_ERROR = 1
EXIT_USAGE = 64
#: The registry answered, but the response could not be written locally.
#: The value is ``EX_IOERR`` from ``sysexits.h``.
EXIT_RECORD_ERROR = 74

REQUEST_TIMEOUT_SECONDS = 30.0
DOWNLOAD_ATTEMPTS = 12
DOWNLOAD_DELAY_SECONDS = 10.0
REGISTRY_API = "https://crates.io/api/v1"


type Opener = cabc.Callable[..., crates_io_registry_http.HttpResponse]
type Sleeper = cabc.Callable[[float], None]
type MonotonicClock = cabc.Callable[[], int]


@dataclasses.dataclass(frozen=True, slots=True)
class TransportTiming:
    """Carry the delay and monotonic clock used by bounded downloads."""

    sleeper: Sleeper = time.sleep
    monotonic_clock_ns: MonotonicClock = time.monotonic_ns


@dataclasses.dataclass(frozen=True, slots=True)
class ProbeRequest:
    """Carry registry observation inputs and output paths."""

    crate: str
    version: str
    status_path: pathlib.Path
    body_path: pathlib.Path
    opener: Opener


@dataclasses.dataclass(frozen=True, slots=True)
class DownloadRequest:
    """Carry archive inputs, paths, and injected opener, sleeper, and clock."""

    crate: str
    version: str
    archive_path: pathlib.Path
    status_path: pathlib.Path
    opener: Opener
    sleeper: Sleeper
    monotonic_clock_ns: MonotonicClock
    report_path: pathlib.Path | None = None


def _probe_url(crate: str, version: str) -> str:
    """Build a URL for one encoded crate version observation."""
    encoded_crate = urllib.parse.quote(crate, safe="-_")
    encoded_version = urllib.parse.quote(version, safe="-._")
    return f"{REGISTRY_API}/crates/{encoded_crate}/{encoded_version}"


def probe(request_data: ProbeRequest) -> int:
    """Record one probe and fail closed without an HTTP status.

    A transport failure and a local write failure are reported separately, so
    a full disk or a bad output path is never mistaken for an unreachable
    registry.

    Parameters
    ----------
    request_data : ProbeRequest
        Crate identity, output paths, and opener.

    Returns
    -------
    int
        ``EXIT_OK``, ``EXIT_TRANSPORT_ERROR`` when the registry gave no HTTP
        status, or ``EXIT_RECORD_ERROR`` when the response could not be saved.
    """
    request = crates_io_registry_http.build_request(
        _probe_url(request_data.crate, request_data.version),
        headers=release_crates_io_adapter.required_headers(),
    )
    try:
        status, body = crates_io_transport_http.request_bytes(
            request_data.opener, request, timeout=REQUEST_TIMEOUT_SECONDS
        )
    except (OSError, http.client.HTTPException) as error:
        print(f"crates.io probe returned no HTTP status: {error}", file=sys.stderr)
        return EXIT_TRANSPORT_ERROR
    try:
        crates_io_transport_http.save_response(
            request_data.status_path, request_data.body_path, status, body
        )
    except OSError as error:
        print(
            f"crates.io probe answered with HTTP {status}, "
            f"but the response could not be recorded: {error}",
            file=sys.stderr,
        )
        return EXIT_RECORD_ERROR
    return EXIT_OK


def download(request_data: DownloadRequest) -> int:
    """Poll the archive endpoint inside the fixed attempt budget.

    Parameters
    ----------
    request_data : DownloadRequest
        Crate identity, output paths, opener, sleeper, and monotonic clock.

    Returns
    -------
    int
        ``EXIT_OK`` after recording a response or unavailable sentinel.

    Notes
    -----
    If every attempt lacks a response, write status ``0`` and empty bytes for
    the offline verifier to reject. A deterministic transport failure, such as
    a rejected redirect target, ends the budget after its first attempt.
    """
    request = crates_io_registry_http.build_request(
        release_crates_io_adapter.download_url(
            request_data.crate, request_data.version
        ),
        headers=release_crates_io_adapter.required_headers(),
    )
    result = crates_io_transport_http.DownloadPollResult(
        response=None, attempts=0, status_class="unknown", last_error=None
    )
    started_ns = request_data.monotonic_clock_ns()
    try:
        result = crates_io_transport_http.poll_archive(
            request_data.opener,
            request,
            crates_io_transport_http.DownloadPollPolicy(
                attempts=DOWNLOAD_ATTEMPTS,
                delay_seconds=DOWNLOAD_DELAY_SECONDS,
                timeout_seconds=REQUEST_TIMEOUT_SECONDS,
            ),
            request_data.sleeper,
        )
        if result.last_error is not None and result.response is None:
            print(
                f"crates.io archive download remained unavailable: {result.last_error}",
                file=sys.stderr,
            )
        _record_archive(request_data, result.response)
    finally:
        _record_report(request_data, result, started_ns)
    return EXIT_OK


def _record_archive(
    request_data: DownloadRequest, response: tuple[int, bytes] | None
) -> None:
    """Persist the polled response, or the unavailable sentinel without one."""
    status, body = (0, b"") if response is None else response
    crates_io_transport_http.save_response(
        request_data.status_path, request_data.archive_path, status, body
    )


def _record_report(
    request_data: DownloadRequest,
    result: crates_io_transport_http.DownloadPollResult,
    started_ns: int,
) -> None:
    """Write polling diagnostics when the caller asked for a report file."""
    if request_data.report_path is None:
        return
    # A clock that steps backwards must not produce a negative duration.
    elapsed_ms = max(0, (request_data.monotonic_clock_ns() - started_ns) // 1_000_000)
    crates_io_transport_http.save_download_report(
        request_data.report_path,
        attempts=result.attempts,
        status_class=result.status_class,
        elapsed_ms=elapsed_ms,
    )


def build_parser() -> argparse.ArgumentParser:
    """Build the argument parser for the transport commands.

    Returns
    -------
    argparse.ArgumentParser
        Parser for the probe and bounded download commands.
    """
    parser = argparse.ArgumentParser(prog="crates_io_transport_cli")
    commands = parser.add_subparsers(dest="command", required=True)
    registry_probe = commands.add_parser("probe", help="record one registry response")
    registry_probe.add_argument("--crate", required=True)
    registry_probe.add_argument("--version", required=True)
    registry_probe.add_argument("--status-file", required=True, type=pathlib.Path)
    registry_probe.add_argument("--body-file", required=True, type=pathlib.Path)
    archive_download = commands.add_parser("download", help="poll for registry bytes")
    archive_download.add_argument("--crate", required=True)
    archive_download.add_argument("--version", required=True)
    archive_download.add_argument("--archive", required=True, type=pathlib.Path)
    archive_download.add_argument("--status-file", required=True, type=pathlib.Path)
    archive_download.add_argument("--report-file", type=pathlib.Path)
    return parser


def main(
    argv: list[str] | None = None,
    *,
    opener: Opener | None = None,
    timing: TransportTiming | None = None,
) -> int:
    """Run one transport command using injected external boundaries.

    Parameters
    ----------
    argv : list[str] or None
        Command arguments; use process arguments when omitted.
    opener : Opener or None
        Registry opener; use the allowlisted opener when omitted.
    timing : TransportTiming
        Delay and monotonic clock for bounded archive polling.

    Returns
    -------
    int
        ``EXIT_OK``, ``EXIT_TRANSPORT_ERROR``, ``EXIT_RECORD_ERROR``, or
        ``EXIT_USAGE``.
    """
    try:
        arguments = build_parser().parse_args(argv)
    except SystemExit as error:
        return EXIT_OK if error.code == 0 else EXIT_USAGE
    selected_opener = opener or crates_io_registry_http.default_opener
    selected_timing = TransportTiming() if timing is None else timing
    if arguments.command == "probe":
        return probe(
            ProbeRequest(
                arguments.crate,
                arguments.version,
                arguments.status_file,
                arguments.body_file,
                selected_opener,
            )
        )
    return download(
        DownloadRequest(
            crate=arguments.crate,
            version=arguments.version,
            archive_path=arguments.archive,
            status_path=arguments.status_file,
            opener=selected_opener,
            sleeper=selected_timing.sleeper,
            monotonic_clock_ns=selected_timing.monotonic_clock_ns,
            report_path=arguments.report_file,
        )
    )


if __name__ == "__main__":
    raise SystemExit(main())
