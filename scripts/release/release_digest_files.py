"""Hash release files for the policies that compare their digests.

This adapter owns blockwise file hashing, so policy callers receive digests
rather than paths, and large archives are never read whole.

Behavioural coverage lives in
``scripts/release/tests/test_release_digest_files.py``.
"""

import hashlib
import typing as typ

if typ.TYPE_CHECKING:
    import pathlib


def digest_of_file(path: pathlib.Path) -> str:
    """Return the lowercase SHA-256 of a file, reading it in bounded blocks.

    Returns
    -------
    str
        The lowercase hexadecimal SHA-256 digest of the file.

    Examples
    --------
    >>> import tempfile
    >>> from pathlib import Path
    >>> with tempfile.TemporaryDirectory() as tmp:
    ...     sample = Path(tmp) / "hello.txt"
    ...     _ = sample.write_bytes(b"wireframe")
    ...     digest_of_file(sample)
    '96f3aab252e6597318e3b52d4a4cfb5a857ee8ff7cae5f3f22440f00f9eb3a88'
    """
    with path.open("rb") as handle:
        return digest_of_stream(handle)


def digest_of_stream(source: typ.IO[bytes]) -> str:
    """Return the lowercase SHA-256 of the bytes ``source`` yields from here on.

    Returns
    -------
    str
        The lowercase hexadecimal SHA-256 digest of the remaining bytes.

    Notes
    -----
    The read starts at the stream's current position, so a caller that has
    already consumed part of a file must seek back to its start first. Reading
    a stream the caller already holds is what lets an adapter hash and parse
    one open file: hashing a pathname and then reopening it leaves a window in
    which the path can name different bytes.

    Examples
    --------
    >>> import io
    >>> digest_of_stream(io.BytesIO(b"wireframe"))
    '96f3aab252e6597318e3b52d4a4cfb5a857ee8ff7cae5f3f22440f00f9eb3a88'
    """
    hasher = hashlib.sha256()
    for block in iter(lambda: source.read(1024 * 1024), b""):
        hasher.update(block)
    return hasher.hexdigest()
