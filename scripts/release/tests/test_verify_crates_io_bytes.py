"""Specify pure classification of typed crates.io download observations."""

import pytest
import release_digest
from verify_crates_io_bytes import (
    ArchiveAbsentError,
    ArchiveMismatchError,
    DownloadAvailability,
    DownloadObservation,
    VerificationError,
    plan_verification,
    verify_observation,
)

EXPECTED = "ab" * 32


@pytest.mark.parametrize(
    ("observation", "verdict"),
    [
        (DownloadObservation(DownloadAvailability.PRESENT, EXPECTED), "verified"),
        (DownloadObservation(DownloadAvailability.PRESENT, "cd" * 32), "mismatch"),
        (DownloadObservation(DownloadAvailability.PRESENT), "error"),
        (DownloadObservation(DownloadAvailability.ABSENT), "absent"),
        (DownloadObservation(DownloadAvailability.INDETERMINATE), "error"),
    ],
)
def test_verdict_follows_only_typed_observation_and_digest(
    observation: DownloadObservation, verdict: str
) -> None:
    """Classify the adapter's facts without interpreting HTTP values."""
    assert (
        plan_verification(observation=observation, expected_sha256=EXPECTED) == verdict
    ), "the verdict must depend only on the typed observation and expected digest"


def test_matching_observation_returns_its_digest() -> None:
    """Return the digest a pure comparison has verified."""
    observation = DownloadObservation(DownloadAvailability.PRESENT, EXPECTED)

    assert (
        verify_observation(
            observation=observation,
            crate="wireframe",
            version="1.0.0",
            expected_sha256=EXPECTED,
        )
        == EXPECTED
    ), "a matching observation must return the digest it verified"


def test_absence_is_reported_without_becoming_a_digest_mismatch() -> None:
    """Keep missing publication distinct from changed bytes."""
    with pytest.raises(ArchiveAbsentError):
        verify_observation(
            observation=DownloadObservation(DownloadAvailability.ABSENT),
            crate="wireframe",
            version="1.0.0",
            expected_sha256=EXPECTED,
        )


def test_mismatch_is_catchable_as_a_digest_error() -> None:
    """Preserve the shared digest error hierarchy for byte disagreement."""
    with pytest.raises(ArchiveMismatchError) as caught:
        verify_observation(
            observation=DownloadObservation(DownloadAvailability.PRESENT, "cd" * 32),
            crate="wireframe",
            version="1.0.0",
            expected_sha256=EXPECTED,
        )
    assert isinstance(caught.value, release_digest.DigestMismatchError), (
        "archive mismatch errors must preserve the shared digest-error hierarchy"
    )


def test_uncertain_observation_cannot_be_verified() -> None:
    """Stop when the adapter cannot establish presence or absence."""
    with pytest.raises(VerificationError):
        verify_observation(
            observation=DownloadObservation(DownloadAvailability.INDETERMINATE),
            crate="wireframe",
            version="1.0.0",
            expected_sha256=EXPECTED,
        )
