"""Compare a typed download observation with the trusted crate digest.

The crates.io adapter maps response status and downloaded bytes into
:class:`DownloadObservation`. This module makes no requests, reads no files,
and knows no HTTP response codes; it decides only whether the observed digest
matches the release identity.

Behavioural coverage lives in
``scripts/release/tests/test_verify_crates_io_bytes.py``.
"""

import dataclasses
import enum

import release_digest


class DownloadAvailability(enum.Enum):
    """Describe whether the adapter observed bytes, absence, or uncertainty."""

    #: A readable archive and its digest were observed.
    PRESENT = "present"

    #: The adapter observed the registry's absent-key response.
    ABSENT = "absent"

    #: The response did not establish presence or absence.
    INDETERMINATE = "indeterminate"


@dataclasses.dataclass(frozen=True, slots=True)
class DownloadObservation:
    """Carry the availability and digest facts needed for pure verification."""

    availability: DownloadAvailability
    sha256: str | None = None


class VerificationError(RuntimeError):
    """Report that an observed archive cannot be verified."""


class MissingObservedDigestError(VerificationError):
    """Report a present observation with no digest value."""

    def __init__(self) -> None:
        """Build the diagnosis for an internally inconsistent observation.

        Examples
        --------
        >>> str(MissingObservedDigestError())
        'a verified observation carries no digest'
        """
        super().__init__("a verified observation carries no digest")


class ArchiveAbsentError(VerificationError):
    """Report that no archive is available under the expected key."""

    def __init__(self, crate: str, version: str) -> None:
        """Build the diagnosis for an absent archive key.

        Examples
        --------
        >>> str(ArchiveAbsentError("crate", "1.0.0"))
        'crate 1.0.0 is not published: the download host holds no such key'
        """
        super().__init__(
            f"{crate} {version} is not published: the download host holds no such key"
        )


class ArchiveMismatchError(VerificationError, release_digest.DigestMismatchError):
    """Report that published bytes differ from the trusted identity digest."""

    def __init__(
        self, crate: str, version: str, observed: str | None, expected: str
    ) -> None:
        """Build the diagnosis for bytes that differ from the trusted digest.

        Examples
        --------
        >>> str(ArchiveMismatchError("crate", "1.0.0", "ab", "cd"))
        'crate 1.0.0 published as ab, but the identity record expects cd'
        """
        super().__init__(
            f"{crate} {version} published as {observed}, but the identity "
            f"record expects {expected}"
        )


class UnverifiableObservationError(VerificationError):
    """Report a download response that establishes neither bytes nor absence."""

    def __init__(self, crate: str, version: str) -> None:
        """Build the diagnosis for an indeterminate download observation.

        Examples
        --------
        >>> str(UnverifiableObservationError("crate", "1.0.0")).startswith(
        ...     "the download observation cannot verify crate 1.0.0"
        ... )
        True
        """
        super().__init__(
            f"the download observation cannot verify {crate} {version}; "
            "the response established neither readable bytes nor absence"
        )


def plan_verification(*, observation: DownloadObservation, expected_sha256: str) -> str:
    """Return the verdict for an in-memory download observation.

    Returns
    -------
    str
        ``verified``, ``absent``, ``mismatch``, or ``error``.

    Examples
    --------
    >>> present = DownloadObservation(DownloadAvailability.PRESENT, "aa")
    >>> plan_verification(observation=present, expected_sha256="aa")
    'verified'
    >>> plan_verification(
    ...     observation=DownloadObservation(DownloadAvailability.ABSENT),
    ...     expected_sha256="aa",
    ... )
    'absent'
    >>> plan_verification(
    ...     observation=DownloadObservation(DownloadAvailability.INDETERMINATE),
    ...     expected_sha256="aa",
    ... )
    'error'
    """
    match observation.availability:
        case DownloadAvailability.ABSENT:
            return "absent"
        case DownloadAvailability.INDETERMINATE:
            return "error"
        case DownloadAvailability.PRESENT:
            if observation.sha256 is None:
                return "error"
            if release_digest.constant_time_equal(observation.sha256, expected_sha256):
                return "verified"
            return "mismatch"


def verify_observation(
    *,
    observation: DownloadObservation,
    crate: str,
    version: str,
    expected_sha256: str,
) -> str:
    """Return the verified digest or raise for absent, uncertain, or wrong bytes.

    Returns
    -------
    str
        The observed digest when it matches the trusted identity record.

    Raises
    ------
    ArchiveAbsentError
        If the adapter observed no archive.
    ArchiveMismatchError
        If the observed digest differs from the identity record.
    UnverifiableObservationError
        If the observation cannot establish readable archive bytes.

    Examples
    --------
    >>> verify_observation(  # doctest: +ELLIPSIS
    ...     observation=DownloadObservation(DownloadAvailability.PRESENT, "aa"),
    ...     crate="wireframe", version="1.0.0", expected_sha256="aa",
    ... )
    'aa'
    >>> verify_observation(
    ...     observation=DownloadObservation(DownloadAvailability.ABSENT),
    ...     crate="wireframe", version="1.0.0", expected_sha256="aa",
    ... )
    Traceback (most recent call last):
        ...
    verify_crates_io_bytes.ArchiveAbsentError: wireframe 1.0.0 ...
    """
    verdict = plan_verification(
        observation=observation, expected_sha256=expected_sha256
    )
    match verdict:
        case "verified":
            return _verified_digest(observation)
        case "absent":
            raise ArchiveAbsentError(crate, version)
        case "mismatch":
            raise ArchiveMismatchError(
                crate, version, observation.sha256, expected_sha256
            )
        case _:
            raise UnverifiableObservationError(crate, version)


def _verified_digest(observation: DownloadObservation) -> str:
    """Return the digest carried by an observation marked verified.

    Returns
    -------
    str
        The digest a prior comparison established as correct.

    Raises
    ------
    MissingObservedDigestError
        If the observation was marked present without carrying a digest.

    Examples
    --------
    >>> _verified_digest(DownloadObservation(DownloadAvailability.PRESENT, "ab"))
    'ab'
    """
    if observation.sha256 is None:
        raise MissingObservedDigestError
    return observation.sha256
