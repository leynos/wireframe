"""Prove the ``dylint.toml`` contract accepts only narrow, evidenced exclusions.

The policy helpers live in ``dylint_policy``; these tests hold the real
``dylint.toml`` to it and drive each rejected shape with a constructed file, so
a mutation fails for the stated reason rather than by accident. The
property-based cases cover the whole path grammar, the evidence block and the
nested-table walk rather than the named examples alone.
"""

from __future__ import annotations

import pytest
from dylint_policy import (
    CRATE_WIDE_EXCLUSION_KEYS,
    EXCLUSION_EVIDENCE,
    MODULE_PATH_RE,
    REPOSITORY_WIDE_TUNING_KEYS,
    assert_dylint_policy,
)
from dev_fast_routing_test import ROOT
from hypothesis import HealthCheck, given, settings
from hypothesis import strategies as st

DYLINT_CONFIG_PATH = ROOT / "dylint.toml"


def test_dylint_config_forbids_crate_wide_exclusions() -> None:
    """``dylint.toml`` exists, parses, and keeps exclusions module-scoped."""
    assert DYLINT_CONFIG_PATH.is_file(), "the repository needs a root dylint.toml"
    assert_dylint_policy(DYLINT_CONFIG_PATH.read_text(encoding="utf-8"))


def test_dylint_policy_accepts_a_justified_module_exclusion() -> None:
    """The documented exception shape must remain admissible."""
    source = (
        "[no_std_fs_operations]\n"
        "# Reproduced by `make lint-whitaker` on 2026-11-04 from\n"
        "# `no_std_fs_operations`.\n"
        "# External constraint: the upstream codec owns the handle.\n"
        "# Removal tracked by #578.\n"
        'excluded_paths = ["wireframe::legacy_io"]\n'
    )

    assert_dylint_policy(source)


@pytest.mark.parametrize(
    ("source", "expected"),
    [
        pytest.param(
            '[dylint]\nexcluded_crates = ["wireframe"]\n',
            "crate-wide",
            id="excluded_crates",
        ),
        pytest.param(
            '[no_std_fs_operations]\nexcluded_paths = ["wireframe"]\n',
            "module path",
            id="bare_crate_name",
        ),
        pytest.param(
            '[no_std_fs_operations]\nexcluded_paths = ["wireframe::"]\n',
            "module path",
            id="trailing_separator",
        ),
        pytest.param(
            '[no_std_fs_operations]\nexcluded_paths = ["."]\n',
            "module path",
            id="repository_root",
        ),
        pytest.param(
            '[no_std_fs_operations]\nexcluded_paths = ["crates/*"]\n',
            "module path",
            id="glob",
        ),
        pytest.param(
            "[no_std_fs_operations]\n"
            "# Reproduced by a run. External constraint: none.\n"
            "# Removal tracked by #578.\n"
            'excluded_paths = "."\n',
            "module path",
            id="scalar_repository_root",
        ),
        pytest.param(
            '[no_std_fs_operations]\nexcluded_paths = ["wireframe::legacy"]\n',
            "needs a comment",
            id="missing_justification",
        ),
        pytest.param(
            "[no_std_fs_operations]\n"
            "# Reproduced by a run. External constraint: none.\n"
            'excluded_paths = ["wireframe::legacy"]\n',
            "needs a comment",
            id="missing_owning_issue",
        ),
        pytest.param(
            'no_std_fs_operations.excluded_paths = ["wireframe::legacy"]\n',
            "needs a comment",
            id="dotted_key_missing_justification",
        ),
        pytest.param(
            'no_std_fs_operations = { excluded_paths = ["wireframe::legacy"] }\n',
            "needs a comment",
            id="inline_table_missing_justification",
        ),
        pytest.param(
            "[module_max_lines]\n"
            "# Reproduced by a run. External constraint: none.\n"
            "# Removal tracked by #578.\n"
            'excluded_paths = ["wireframe::legacy"]\n',
            "no_std_fs_operations",
            id="excluded_paths_in_another_table",
        ),
        pytest.param(
            '[no_std_fs_operations]\n"excluded_paths" = ["wireframe::legacy"]\n',
            "needs a comment",
            id="quoted_key_missing_justification",
        ),
        pytest.param(
            "[no_std_fs_operations]\n"
            "# Reproduced by a run. External constraint: none.\n"
            "# Removal tracked by #578.\n"
            'excluded_paths = ["wireframe::legacy"]\n',
            "diagnostic identifier",
            id="missing_diagnostic_identifier",
        ),
        pytest.param(
            "[module_max_lines]\nthresholds = { complexity = 20 }\n",
            "tuning",
            id="thresholds",
        ),
        pytest.param(
            "[no_expect_outside_tests]\n"
            'additional_test_attributes = ["async_std::test"]\n',
            "tuning",
            id="additional_test_attributes",
        ),
        pytest.param(
            '[dylint]\nlocale = "en-GB"\n',
            "tuning",
            id="locale",
        ),
    ],
)
def test_dylint_policy_rejects_unsafe_exceptions(source: str, expected: str) -> None:
    """Each rejected shape must fail for its own stated reason."""
    with pytest.raises(AssertionError, match=expected):
        assert_dylint_policy(source)


#: A comment block that satisfies every evidence rule, used as the control that
#: isolates each property to the value or fragment under test.
COMPLETE_EVIDENCE = (
    "# Reproduced by `make lint-whitaker` on 2026-11-04.\n"
    "# `no_std_fs_operations` reported it.\n"
    "# External constraint: the upstream codec owns the handle.\n"
    "# Removal tracked by agreement.\n"
    "# Owning issue: #578.\n"
)
MODULE_SEGMENT = st.from_regex(r"[a-z_][a-z0-9_]{0,7}", fullmatch=True)
#: A generator over the module-path grammar the contract accepts.
MODULE_PATH = st.lists(MODULE_SEGMENT, min_size=2, max_size=4).map("::".join)
#: Strings that need no TOML escaping and fail the module-path grammar. Most
#: text falls outside that grammar, so the filter discards almost nothing.
UNSAFE_PATH = st.text(
    alphabet=st.characters(
        blacklist_categories=("Cc", "Cs"), blacklist_characters='"\\'
    ),
    max_size=20,
).filter(lambda value: not MODULE_PATH_RE.fullmatch(value))
#: Fragments that carry exactly one piece of required evidence each.
EVIDENCE_FRAGMENTS = {
    "reproduced by": "Reproduced by a recorded run.",
    "external constraint": "External constraint: upstream owns the handle.",
    "tracked by": "Removal tracked by agreement.",
    "diagnostic identifier": "`no_std_fs_operations` reported it.",
    "owning issue": "Owning issue: #578.",
}
EVIDENCE_ITEMS = (*EXCLUSION_EVIDENCE, "diagnostic identifier", "owning issue")
RESERVED_KEYS = (
    REPOSITORY_WIDE_TUNING_KEYS | CRATE_WIDE_EXCLUSION_KEYS | {"excluded_paths"}
)
BENIGN_KEY = st.from_regex(r"[a-z][a-z0-9_]{0,10}", fullmatch=True).filter(
    lambda key: key not in RESERVED_KEYS
)
TABLE_NAME = st.from_regex(r"[a-z][a-z0-9_]{0,10}", fullmatch=True)


def _exclusion(source_value: str) -> str:
    """Return the document that excludes ``source_value`` with full evidence."""
    return (
        "[no_std_fs_operations]\n"
        f"{COMPLETE_EVIDENCE}"
        f'excluded_paths = ["{source_value}"]\n'
    )


@settings(max_examples=25, deadline=None, suppress_health_check=[HealthCheck.too_slow])
@given(value=UNSAFE_PATH)
def test_any_value_outside_the_module_grammar_widens_the_exception(value: str) -> None:
    """Every path outside ``crate::module`` would exempt more than intended."""
    with pytest.raises(AssertionError, match="module path"):
        assert_dylint_policy(_exclusion(value))


@settings(max_examples=25, deadline=None, suppress_health_check=[HealthCheck.too_slow])
@given(path=MODULE_PATH)
def test_every_module_path_with_full_evidence_is_admitted(path: str) -> None:
    """The documented exception shape stays admissible across the grammar."""
    assert_dylint_policy(_exclusion(path))


@settings(max_examples=25, deadline=None, suppress_health_check=[HealthCheck.too_slow])
@given(
    included=st.sets(
        st.sampled_from(EVIDENCE_ITEMS), max_size=len(EVIDENCE_ITEMS) - 1
    )
)
def test_each_missing_piece_of_evidence_is_named(included: frozenset[str]) -> None:
    """Whatever the comment omits, the failure lists it, so review can fix it."""
    missing = [item for item in EVIDENCE_ITEMS if item not in included]
    comments = "".join(
        f"# {EVIDENCE_FRAGMENTS[item]}\n" for item in sorted(included)
    )
    source = (
        "[no_std_fs_operations]\n"
        f"{comments}"
        'excluded_paths = ["wireframe::legacy"]\n'
    )

    with pytest.raises(AssertionError) as failure:
        assert_dylint_policy(source)

    message = str(failure.value)
    for item in missing:
        assert item in message, f"the failure should name {item!r}: {message}"


@settings(max_examples=25, deadline=None, suppress_health_check=[HealthCheck.too_slow])
@given(table=TABLE_NAME, key=BENIGN_KEY, value=st.integers(min_value=0, max_value=99))
def test_a_nested_table_entry_with_a_benign_key_is_ignored(
    table: str, key: str, value: int
) -> None:
    """Descending into nested tables must not reject an ordinary setting."""
    assert_dylint_policy(f"[{table}]\n{key} = {value}\n")