"""Compare release digests using only in-memory values.

Where a digest arrives from an untrusted source, :func:`constant_time_equal`
compares it without revealing, through timing, how much of the expected value
a guess matched. File reads and hashing live in :mod:`release_digest_files`.

Behavioural coverage lives in ``scripts/release/tests/test_release_digest.py``.
"""

import hmac


class DigestError(ValueError):
    """Report that a digest could not be read or did not match."""


class DigestMismatchError(DigestError):
    """Report bytes whose digest is not the one recorded for them."""


def constant_time_equal(left: str, right: str) -> bool:
    """Compare two digest strings without exiting early on a mismatch.

    Parameters
    ----------
    left
        One digest, usually the freshly computed one.
    right
        The other digest, usually the one a registry or a sidecar recorded.

    Returns
    -------
    bool
        Whether the two are equal, compared as bytes.

    Notes
    -----
    :func:`hmac.compare_digest` is used rather than ``==``. A published digest
    is not a secret, so this hides nothing here; it keeps the comparison
    independent of how many leading characters agree, which matters once the
    same helper is reused for values that are not public. Case is ignored,
    because ``sha256sum`` writes lowercase while other producers emit
    uppercase.

    Examples
    --------
    >>> constant_time_equal("AB" * 32, "ab" * 32)
    True
    >>> constant_time_equal("ab" * 32, "ab" * 31 + "cd")
    False
    """
    return hmac.compare_digest(left.lower().encode(), right.lower().encode())
