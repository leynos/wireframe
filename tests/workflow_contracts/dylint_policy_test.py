"""Prove the ``dylint.toml`` contract accepts only narrow, evidenced exclusions.

The policy helpers live in ``dylint_policy``; these tests hold the real
``dylint.toml`` to it and drive each rejected shape with a constructed file, so
a mutation fails for the stated reason rather than by accident.
"""

from __future__ import annotations

import pytest
from dylint_policy import assert_dylint_policy
from dev_fast_routing_test import ROOT

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