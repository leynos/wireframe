"""Capture registry HTTP responses and retry archive downloads.

Request construction and redirect-target validation belong to
``crates_io_registry_http``; this module owns response capture, archive
polling, and bounded transport file formats.
"""

import dataclasses
import http.client
import json
import typing as typ
import urllib.error
import urllib.request

import crates_io_registry_http
import release_crates_io_adapter

if typ.TYPE_CHECKING:
    import collections.abc as cabc
    import pathlib

HTTP_SERVER_ERROR_MIN = 500
HTTP_INFORMATIONAL_STATUS_MIN = 100
HTTP_STATUS_LIMIT = 600

type Opener = cabc.Callable[..., crates_io_registry_http.HttpResponse]
type Sleeper = cabc.Callable[[float], None]


@dataclasses.dataclass(frozen=True, slots=True)
class DownloadPollPolicy:
    """Set the retry bounds for one archive download."""

    attempts: int
    delay_seconds: float
    timeout_seconds: float


@dataclasses.dataclass(frozen=True, slots=True)
class DownloadPollResult:
    """Return the last archive response and bounded retry observations."""

    response: tuple[int, bytes] | None
    attempts: int
    status_class: str
    last_error: OSError | http.client.HTTPException | None


def request_bytes(
    opener: cabc.Callable[..., crates_io_registry_http.HttpResponse],
    request: urllib.request.Request,
    *,
    timeout: float,
) -> tuple[int, bytes]:
    """Open one request and return its HTTP status and complete response body.

    Parameters
    ----------
    opener : collections.abc.Callable
        Opener configured for the registry request boundary.
    request : urllib.request.Request
        Request to open.
    timeout : float
        Maximum time to wait for the response.

    Returns
    -------
    tuple[int, bytes]
        HTTP status and response bytes, including an ``HTTPError`` response.
    """
    try:
        response = opener(request, timeout=timeout)
    except urllib.error.HTTPError as error:
        try:
            return error.code, error.read()
        finally:
            error.close()
    try:
        return int(response.status), response.read()
    finally:
        response.close()


def _download_attempt(
    opener: Opener, request: urllib.request.Request, timeout: float
) -> tuple[
    tuple[int, bytes] | None,
    OSError | http.client.HTTPException | None,
]:
    """Return one complete archive response or its transport exception.

    Returns
    -------
    tuple[tuple[int, bytes] | None, OSError | http.client.HTTPException | None]
        The complete response and no error, or no response and the transport
        exception from the request.

    Examples
    --------
    >>> def offline(*_args, **_kwargs):
    ...     raise OSError("offline")
    >>> response, error = _download_attempt(
    ...     offline, urllib.request.Request("https://static.crates.io/a.crate"), 1.0
    ... )
    >>> response is None and isinstance(error, OSError)
    True
    """
    try:
        return request_bytes(opener, request, timeout=timeout), None
    except (OSError, http.client.HTTPException) as error:
        return None, error


def _download_status_class(status: int) -> str:
    """Classify an HTTP status with a fixed low-cardinality label.

    Returns
    -------
    str
        A fixed HTTP status class such as ``2xx``, or ``unknown``.

    Examples
    --------
    >>> _download_status_class(206)
    '2xx'
    >>> _download_status_class(0)
    'unknown'

    """
    if HTTP_INFORMATIONAL_STATUS_MIN <= status < HTTP_STATUS_LIMIT:
        return f"{status // 100}xx"
    return "unknown"


def _retryable_download_status(status: int) -> bool:
    """Identify absence and server errors that may settle on another poll.

    Returns
    -------
    bool
        Whether another bounded archive poll may produce a usable response.

    Examples
    --------
    >>> _retryable_download_status(404)
    True
    >>> _retryable_download_status(422)
    False

    """
    return status in release_crates_io_adapter.ABSENT_STATUSES or (
        HTTP_SERVER_ERROR_MIN <= status < HTTP_STATUS_LIMIT
    )


def _retryable_transport_error(
    error: OSError | http.client.HTTPException | None,
) -> bool:
    """Identify transport errors that another poll may settle.

    A rejected redirect target is deterministic: the allowlist decision cannot
    change between attempts, so retrying it only delays the same failure. Every
    other transport error stays retryable.

    Returns
    -------
    bool
        Whether another bounded archive poll may produce a usable response.

    Examples
    --------
    >>> _retryable_transport_error(urllib.error.URLError("offline"))
    True
    >>> _retryable_transport_error(
    ...     crates_io_registry_http.RegistryUrlError()
    ... )
    False

    """
    return not isinstance(error, crates_io_registry_http.RegistryUrlError)


def poll_archive(
    opener: Opener,
    request: urllib.request.Request,
    policy: DownloadPollPolicy,
    sleeper: Sleeper,
) -> DownloadPollResult:
    """Poll an archive with bounded retries for absence and server errors.

    Parameters
    ----------
    opener : Opener
        Allowlisted HTTP opener used for each attempt.
    request : urllib.request.Request
        Request for the public archive bytes.
    policy : DownloadPollPolicy
        Attempt, delay, and per-request timeout bounds.
    sleeper : Sleeper
        Injected delay used between retryable attempts.

    Returns
    -------
    DownloadPollResult
        The last response, attempt count, status class, and final transport
        error if the budget ended without an HTTP response.

    Examples
    --------
    >>> class Response:
    ...     status = 200
    ...     def read(self): return b"archive"
    ...     def close(self): pass
    >>> result = poll_archive(
    ...     lambda *_args, **_kwargs: Response(),
    ...     urllib.request.Request("https://static.crates.io/a.crate"),
    ...     DownloadPollPolicy(2, 0.0, 1.0),
    ...     lambda _delay: None,
    ... )
    >>> (result.response, result.attempts, result.status_class)
    ((200, b'archive'), 1, '2xx')
    """
    last_response: tuple[int, bytes] | None = None
    last_error: OSError | http.client.HTTPException | None = None
    status_class = "unknown"
    attempts = 0
    for attempt in range(policy.attempts):
        attempts = attempt + 1
        last_response, last_error = _download_attempt(
            opener, request, policy.timeout_seconds
        )
        if last_response is None:
            status_class = "unknown"
            if not _retryable_transport_error(last_error):
                break
            if attempt + 1 < policy.attempts:
                sleeper(policy.delay_seconds)
            continue
        last_error = None
        status_class = _download_status_class(last_response[0])
        if not _retryable_download_status(last_response[0]):
            break
        if attempt + 1 < policy.attempts:
            sleeper(policy.delay_seconds)
    return DownloadPollResult(last_response, attempts, status_class, last_error)


def save_response(
    status_path: pathlib.Path,
    body_path: pathlib.Path,
    status: int,
    body: bytes,
) -> None:
    """Write response bytes and status for the offline policy CLI.

    Parameters
    ----------
    status_path, body_path : pathlib.Path
        Destinations for the HTTP status and response body.
    status : int
        HTTP status to record.
    body : bytes
        Response bytes to preserve.

    """
    body_path.write_bytes(body)
    status_path.write_text(f"{status}\n", encoding="ascii")


def save_download_report(
    report_path: pathlib.Path,
    *,
    attempts: int,
    status_class: str,
    elapsed_ms: int,
) -> None:
    """Write bounded archive-polling diagnostics without response data.

    Parameters
    ----------
    report_path : pathlib.Path
        Destination for the machine-readable retry summary.
    attempts : int
        Number of HTTP attempts made by the bounded polling loop.
    status_class : str
        Last received HTTP status class or the fixed ``unknown`` value.
    elapsed_ms : int
        Total monotonic duration of the polling loop in milliseconds.
    """
    report_path.write_text(
        json.dumps(
            {
                "attempts": attempts,
                "status_class": status_class,
                "elapsed_ms": elapsed_ms,
            },
            sort_keys=True,
        )
        + "\n",
        encoding="ascii",
    )
