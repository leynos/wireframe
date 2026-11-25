"""Generated properties for crates.io version-history reconciliation.

History properties hold the behaviour across record order, unrelated older
versions, and independently generated checksums. The table-driven examples
remain in ``test_release_reconcile_crates.py``; this module isolates the
bounded generated histories so both suites stay easy to scan.

Run via ``make test-release-publication``.
"""

import itertools

from hypothesis import given, settings
from hypothesis import strategies as st
from release_reconcile_crates import (
    RegistryState,
    RegistryVersion,
    RegistryVersionHistory,
    classify_registry_state,
)

INTENDED = "f" * 64
VERSION = "1.0.0"

_DIGEST = st.text(alphabet="0123456789abcdef", min_size=64, max_size=64)
_OTHER_RECORD = st.builds(
    RegistryVersion,
    num=st.sampled_from(("0.9.0", "1.1.0", "2.0.0")),
    checksum=_DIGEST,
    is_yanked=st.booleans(),
)


@st.composite
def _history_with_target(
    draw: st.DrawFn, *, checksum: str, is_yanked: bool
) -> list[RegistryVersion]:
    """Generate a history with the target among unrelated records."""
    others = draw(st.lists(_OTHER_RECORD, max_size=5))
    index = draw(st.integers(min_value=0, max_value=len(others)))
    target = RegistryVersion(VERSION, checksum, is_yanked)
    return [*others[:index], target, *others[index:]]


def _classify(records: list[RegistryVersion]) -> RegistryState:
    """Classify a generated history against its intended candidate."""
    return classify_registry_state(
        RegistryVersionHistory(tuple(records)),
        version=VERSION,
        intended_sha256=INTENDED,
    )


@settings(max_examples=100, deadline=None, derandomize=True)
@given(records=_history_with_target(checksum=INTENDED, is_yanked=False))
def test_generated_history_finds_the_matching_version_anywhere(
    records: list[RegistryVersion],
) -> None:
    """Find the matching target even when unrelated records surround it."""
    assert _classify(records) is RegistryState.IDENTICAL, (
        "an exact target record must remain identifiable at every history index"
    )


@settings(max_examples=100, deadline=None, derandomize=True)
@given(records=_history_with_target(checksum=INTENDED, is_yanked=True))
def test_generated_yanked_target_outranks_matching_digest(
    records: list[RegistryVersion],
) -> None:
    """Let the target's yank state outrank digest equality in every history."""
    assert _classify(records) is RegistryState.YANKED, (
        "a matching digest cannot make a yanked target publishable"
    )


@settings(max_examples=100, deadline=None, derandomize=True)
@given(
    checksum=_DIGEST.filter(lambda digest: digest != INTENDED),
    others=st.lists(_OTHER_RECORD, max_size=5),
)
def test_generated_nonmatching_target_conflicts(
    checksum: str, others: list[RegistryVersion]
) -> None:
    """Classify every unyanked target with other bytes as a conflict."""
    records = [*others, RegistryVersion(VERSION, checksum, is_yanked=False)]
    assert _classify(records) is RegistryState.CONFLICT, (
        "unrelated records cannot turn different target bytes into a match"
    )


@settings(max_examples=100, deadline=None, derandomize=True)
@given(
    checksum=_DIGEST,
    is_yanked=st.booleans(),
    others=st.lists(_OTHER_RECORD, max_size=4),
)
def test_registry_history_classification_is_order_independent(
    checksum: str, *, is_yanked: bool, others: list[RegistryVersion]
) -> None:
    """Preserve the classification under every order of a short history."""
    records = [*others, RegistryVersion(VERSION, checksum, is_yanked)]
    expected = _classify(records)
    for ordering in itertools.permutations(records):
        assert _classify(list(ordering)) is expected, (
            "registry history order must not change the target version's state"
        )
