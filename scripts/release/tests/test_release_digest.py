"""Pin the digest comparison that decides whether published bytes match."""

import pytest
from hypothesis import given
from hypothesis import strategies as st
from release_digest import constant_time_equal

HEX_DIGESTS = st.binary(min_size=32, max_size=32).map(bytes.hex)


@given(digest=HEX_DIGESTS)
def test_a_digest_equals_itself_in_either_case(digest: str) -> None:
    """Producers differ on case, and the comparison must not."""
    assert constant_time_equal(digest.upper(), digest)


@given(left=HEX_DIGESTS, right=HEX_DIGESTS)
def test_equality_agrees_with_the_plain_comparison(left: str, right: str) -> None:
    """Avoiding an early exit must not change the answer."""
    assert constant_time_equal(left, right) is (left == right)


@pytest.mark.parametrize("truncated", ["", "ab" * 31])
def test_a_truncated_digest_never_matches(truncated: str) -> None:
    """A prefix of the recorded digest is not the recorded digest."""
    assert not constant_time_equal(truncated, "ab" * 32)
