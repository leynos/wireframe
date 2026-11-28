"""Pin file hashing against a reference implementation."""

import hashlib
import io
import typing as typ

from hypothesis import given
from hypothesis import strategies as st
from release_digest_files import digest_of_file, digest_of_stream

if typ.TYPE_CHECKING:
    import pathlib


def test_a_file_is_hashed_whole(tmp_path: pathlib.Path) -> None:
    """A file spanning several read blocks yields one digest of every byte."""
    contents = b"wireframe" * (1024 * 1024 // 4)
    archive = tmp_path / "wireframe.crate"
    _ = archive.write_bytes(contents)
    assert digest_of_file(archive) == hashlib.sha256(contents).hexdigest()


@given(skipped=st.binary(max_size=64), rest=st.binary(max_size=64))
def test_a_stream_is_hashed_from_its_current_position(
    skipped: bytes, rest: bytes
) -> None:
    """Bytes a caller has already consumed are not part of the digest."""
    stream = io.BytesIO(skipped + rest)
    _ = stream.read(len(skipped))
    assert digest_of_stream(stream) == hashlib.sha256(rest).hexdigest()
