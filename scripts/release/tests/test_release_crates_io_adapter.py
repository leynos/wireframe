"""Test HTTP and local-file adaptation for crates.io byte verification."""

import hashlib
import typing as typ

import pytest

from release_crates_io_adapter import (
    ABSENT_STATUSES,
    download_url,
    observe_download_file,
    required_headers,
)
from verify_crates_io_bytes import DownloadAvailability

if typ.TYPE_CHECKING:
    import pathlib

ARCHIVE_BYTES = bytes([31, 139, 8, 0]) + b"crate payload"


def test_download_url_and_headers_name_the_public_endpoint() -> None:
    """Build the token-free archive URL with a required user agent."""
    assert download_url("wireframe", "1.0.0") == (
        "https://static.crates.io/crates/wireframe/wireframe-1.0.0.crate"
    ), "the archive URL must use the versioned crates.io filename"
    assert required_headers()["User-Agent"].startswith("wireframe-release-verifier"), (
        "the request must identify the release verifier"
    )


def test_absent_statuses_map_to_absent_observations(
    tmp_path: pathlib.Path,
) -> None:
    """Map both CDN absence statuses without reading an archive path."""
    missing_path = tmp_path / "not-read.crate"

    assert all(
        observe_download_file(status, missing_path).availability
        is DownloadAvailability.ABSENT
        for status in ABSENT_STATUSES
    ), "both measured absence statuses must map to absent observations"


def test_error_status_is_indeterminate(tmp_path: pathlib.Path) -> None:
    """Refuse a transient response as evidence of absence or identity."""
    observation = observe_download_file(503, tmp_path / "not-read.crate")

    assert observation.availability is DownloadAvailability.INDETERMINATE, (
        "a transient HTTP failure must remain indeterminate"
    )


def test_saved_access_denied_body_is_absent(tmp_path: pathlib.Path) -> None:
    """Recognize the measured XML refusal body even when saved by curl."""
    archive = tmp_path / "missing.crate"
    _ = archive.write_bytes(b"<?xml version='1.0'?><Error/>")

    assert (
        observe_download_file(200, archive).availability is DownloadAvailability.ABSENT
    ), "the saved access-denied body must override the nominal success status"


def test_gzip_archive_is_hashed_by_the_adapter(tmp_path: pathlib.Path) -> None:
    """Hash saved archive bytes and return a typed present observation."""
    archive = tmp_path / "published.crate"
    _ = archive.write_bytes(ARCHIVE_BYTES)

    observation = observe_download_file(200, archive)

    assert observation.availability is DownloadAvailability.PRESENT, (
        "a successful archive response must produce a present observation"
    )
    assert observation.sha256 == hashlib.sha256(ARCHIVE_BYTES).hexdigest(), (
        "the adapter must hash the exact saved archive bytes"
    )


def test_unreadable_success_body_raises_rather_than_observing(
    tmp_path: pathlib.Path,
) -> None:
    """A 200 whose saved body cannot be read is an error, not an observation."""
    with pytest.raises(FileNotFoundError):
        observe_download_file(200, tmp_path / "never-saved.crate")


def test_empty_success_body_is_absent(tmp_path: pathlib.Path) -> None:
    """Treat an empty 200 body as absence, with no digest to compare."""
    archive = tmp_path / "empty.crate"
    _ = archive.write_bytes(b"")

    observation = observe_download_file(200, archive)

    assert observation.availability is DownloadAvailability.ABSENT, (
        "an empty 200 body is absence, not an archive"
    )
    assert observation.sha256 is None, "an absent observation must carry no digest"


def test_single_byte_non_xml_body_is_present(tmp_path: pathlib.Path) -> None:
    """Treat any non-empty body without the XML prefix as archive bytes."""
    archive = tmp_path / "tiny.crate"
    _ = archive.write_bytes(b"x")

    observation = observe_download_file(200, archive)

    assert observation.availability is DownloadAvailability.PRESENT, (
        "a non-empty body without the XML refusal prefix is an archive"
    )
    assert observation.sha256 == hashlib.sha256(b"x").hexdigest(), (
        "the adapter must hash the single saved byte"
    )
