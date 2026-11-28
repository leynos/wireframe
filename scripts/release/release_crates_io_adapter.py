"""Translate crates.io download responses at their API boundary.

This adapter owns the public download URL, required request headers, download
status mapping, absence-body signature, and file hashing. It is the I/O
boundary: it reads the saved archive from disk. The verifier receives only
:class:`verify_crates_io_bytes.DownloadObservation`; it never reads a path or
interprets transport values.

Behavioural coverage lives in
``scripts/release/tests/test_release_crates_io_adapter.py``.
"""

import typing as typ

import release_digest_files
from verify_crates_io_bytes import DownloadAvailability, DownloadObservation

if typ.TYPE_CHECKING:
    import pathlib

#: Public host serving published crate archives without a token.
DOWNLOAD_HOST = "https://static.crates.io"

#: Statuses the download host returns for an absent key.
ABSENT_STATUSES = frozenset({403, 404})

#: Status the download host returns with archive bytes.
SUCCESS_STATUS = 200

#: Prefix of the XML refusal body observed for a missing archive key.
S3_ERROR_PREFIX = b"<?xml"


def download_url(crate: str, version: str) -> str:
    """Return the token-free URL for one published crate archive.

    Returns
    -------
    str
        The public static.crates.io download URL.

    Examples
    --------
    >>> download_url("wireframe", "0.1.0-beta4")
    'https://static.crates.io/crates/wireframe/wireframe-0.1.0-beta4.crate'
    """
    return f"{DOWNLOAD_HOST}/crates/{crate}/{crate}-{version}.crate"


def required_headers() -> dict[str, str]:
    """Return the request headers required by the public crates.io endpoint.

    Returns
    -------
    dict[str, str]
        Headers identifying the release verifier to the public endpoint.

    Examples
    --------
    >>> required_headers()["User-Agent"].startswith("wireframe")
    True
    """
    return {
        "User-Agent": "wireframe-release-verifier (https://github.com/leynos/wireframe)"
    }


def observe_download_file(status: int, archive: pathlib.Path) -> DownloadObservation:
    """Map an HTTP response and saved body into a domain observation.

    Returns
    -------
    DownloadObservation
        The observed presence, absence, or uncertainty and its digest when
        bytes were present.

    Raises
    ------
    OSError
        If a 200 response's saved archive cannot be opened, read or hashed.
        The error propagates, so an unreadable body is never mistaken for an
        absent or present archive.

    Examples
    --------
    >>> import tempfile
    >>> from pathlib import Path
    >>> with tempfile.TemporaryDirectory() as tmp:
    ...     absent = Path(tmp) / "absent.crate"
    ...     _ = absent.write_bytes(b"<?xml version='1.0'?>")
    ...     observe_download_file(200, absent).availability
    <DownloadAvailability.ABSENT: 'absent'>
    >>> observe_download_file(503, Path("missing.crate")).availability
    <DownloadAvailability.INDETERMINATE: 'indeterminate'>
    """
    if status in ABSENT_STATUSES:
        return DownloadObservation(DownloadAvailability.ABSENT)
    if status != SUCCESS_STATUS:
        return DownloadObservation(DownloadAvailability.INDETERMINATE)
    with archive.open("rb") as body:
        prefix = body.read(len(S3_ERROR_PREFIX))
    if not prefix or prefix == S3_ERROR_PREFIX:
        return DownloadObservation(DownloadAvailability.ABSENT)
    digest = release_digest_files.digest_of_file(archive)
    return DownloadObservation(DownloadAvailability.PRESENT, digest)
