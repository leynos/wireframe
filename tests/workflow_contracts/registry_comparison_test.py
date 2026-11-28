"""Cases for the comparison between the actionlint registry and the labels in use.

The contract in ``ci_runner_placement_test`` reads the live registry. These
cases hold the comparison itself, with no file read, to an exact label, a
trailing ``*`` glob and both ways a registry can be wrong, so a comparison that
filled one list from the other, or treated every entry as a match, fails here.
"""

from __future__ import annotations

import pytest
from registry_comparison import registry_mismatches


@pytest.mark.parametrize(
    ("registered", "in_use", "unregistered", "unused"),
    [
        pytest.param({"a-1"}, {"a-1"}, [], [], id="exact-match"),
        pytest.param({"a-*"}, {"a-1", "a-2"}, [], [], id="glob-covers-several"),
        pytest.param({"a-*"}, {"a-1"}, [], [], id="glob-covers-one"),
        pytest.param(set(), set(), [], [], id="nothing-declared-nothing-used"),
        pytest.param(set(), {"a-1"}, ["a-1"], [], id="empty-registry"),
        pytest.param({"a"}, {"a-1"}, ["a-1"], ["a"], id="exact-is-not-a-prefix"),
        pytest.param({"b-*"}, {"a-1"}, ["a-1"], ["b-*"], id="glob-for-another-family"),
        pytest.param({"a-*"}, {"a"}, ["a"], ["a-*"], id="glob-needs-its-hyphen"),
        pytest.param({"a-1", "a-2"}, {"a-1"}, [], ["a-2"], id="stale-exact-entry"),
        pytest.param({"a-*"}, set(), [], ["a-*"], id="glob-with-no-use"),
        pytest.param({"a-*b"}, {"a-xb"}, ["a-xb"], ["a-*b"], id="mid-string-glob"),
    ],
)
def test_the_registry_comparison_reports_both_mismatch_lists(
    registered: set[str],
    in_use: set[str],
    unregistered: list[str],
    unused: list[str],
) -> None:
    """Report the unregistered labels and the unused entries, each on its own."""
    assert registry_mismatches(registered, in_use) == (unregistered, unused), (
        f"registry {sorted(registered)} against labels {sorted(in_use)} must "
        f"report unregistered {unregistered} and unused {unused}"
    )
