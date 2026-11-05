"""Prove the pure nextest and timeout policy readings against constructed input.

The repository's own configuration exercises only the accepted shapes, so a
parser that mishandled a rejected shape would pass against it either way. Each
refusal is driven here with a constructed value, and each accepted shape has a
positive case beside it.
"""

from __future__ import annotations

import pytest

from nextest_policy import FilterSyntaxError, parse_target_filter
from rust_source_scanning import (
    defines_test_function,
    drives_trybuild,
    is_loom_gated,
    qualified_test_names,
)
from timeout_policy import (
    DurationSyntaxError,
    coverage_action_name,
    parse_duration,
)

CANONICAL_FILTER = (
    "package(=wireframe) & binary(=compile_error) & test(=compile_tests)"
)


def test_the_canonical_filter_parses() -> None:
    """The reviewed filter yields its three exact matcher values."""
    parsed = parse_target_filter(CANONICAL_FILTER)
    assert (parsed.package, parsed.binary, parsed.test) == (
        "wireframe",
        "compile_error",
        "compile_tests",
    ), "the canonical filter must yield its three exact matcher values"


@pytest.mark.parametrize(
    "text",
    [
        pytest.param(
            "package(=wireframe) | binary(=compile_error) & test(=compile_tests)",
            id="or-operator",
        ),
        pytest.param(
            "not(package(=wireframe)) & binary(=compile_error) & test(=compile_tests)",
            id="not-predicate",
        ),
        pytest.param(
            "package(wireframe) & binary(=compile_error) & test(=compile_tests)",
            id="non-exact-package",
        ),
        pytest.param(
            "package(=wireframe) & binary(~compile_error) & test(=compile_tests)",
            id="regex-binary",
        ),
        pytest.param(
            "package(=wireframe) & binary(=compile_error)",
            id="missing-test",
        ),
        pytest.param(
            "package(=wireframe) & binary(=compile_error) & test(=compile_tests)"
            " & kind(=lib)",
            id="extra-predicate",
        ),
        pytest.param(
            "package(=wireframe) & package(=other) & binary(=compile_error)"
            " & test(=compile_tests)",
            id="repeated-package",
        ),
        pytest.param("", id="empty"),
    ],
)
def test_a_filter_outside_the_exact_conjunction_is_refused(text: str) -> None:
    """Anything but three exact matchers joined by ``&`` fails closed."""
    with pytest.raises(FilterSyntaxError):
        parse_target_filter(text)


@pytest.mark.parametrize(
    ("source", "expected"),
    [
        pytest.param(
            "#[test]\nfn compile_tests() {}\n", True, id="plain"
        ),
        pytest.param(
            "#[test]\n#[cfg(feature = \"x\")]\nfn compile_tests() {}\n",
            True,
            id="further-attribute",
        ),
        pytest.param(
            "#[test]\nfn other() {}\n", False, id="different-name"
        ),
        pytest.param(
            "fn compile_tests() {}\n", False, id="no-test-attribute"
        ),
        pytest.param(
            "// #[test]\n// fn compile_tests() {}\n",
            False,
            id="line-commented-out",
        ),
        pytest.param(
            "/* #[test]\nfn compile_tests() {} */\n",
            False,
            id="block-commented-out",
        ),
        pytest.param(
            "mod contracts {\n    #[test]\n    fn compile_tests() {}\n}\n",
            False,
            id="nested-module",
        ),
        pytest.param(
            "mod outer {\n    mod inner {\n        #[test]\n"
            "        fn compile_tests() {}\n    }\n}\n",
            False,
            id="doubly-nested-module",
        ),
        pytest.param(
            "impl Harness {\n    #[test]\n    fn compile_tests() {}\n}\n",
            False,
            id="impl-block",
        ),
        pytest.param(
            "mod contracts {\n    #[test]\n    fn compile_tests() {}\n}\n"
            "#[test]\nfn compile_tests() {}\n",
            True,
            id="root-and-nested",
        ),
    ],
)
def test_test_function_detection_is_module_aware(
    source: str, expected: bool
) -> None:
    """Only a crate-root ``#[test]`` has the name an exact matcher selects."""
    assert defines_test_function(source, "compile_tests") is expected, (
        "only a crate-root #[test] may satisfy the exact matcher"
    )


@pytest.mark.parametrize(
    ("source", "expected"),
    [
        pytest.param("#[test]\nfn a() {}\n", {"a"}, id="crate-root"),
        pytest.param(
            "mod m {\n    #[test]\n    fn b() {}\n}\n", {"m::b"}, id="nested"
        ),
        pytest.param(
            "mod a {\n    mod b {\n        #[test]\n        fn c() {}\n"
            "    }\n}\n",
            {"a::b::c"},
            id="doubly-nested",
        ),
        pytest.param(
            "// #[test]\n// fn commented() {}\n", set(), id="commented-out"
        ),
        pytest.param(
            "let c = '\\'';'{';\n#[test]\nfn after_escaped_quote() {}",
            {"after_escaped_quote"},
            id="escaped-quote-literal",
        ),
        pytest.param(
            "impl Harness {\n    #[test]\n    fn method() {}\n}\n",
            set(),
            id="not-a-test-item",
        ),
    ],
)
def test_test_function_paths_are_fully_qualified(
    source: str, expected: set[str]
) -> None:
    """The module path is established rather than assumed to be the bare name."""
    assert qualified_test_names(source) == frozenset(expected), (
        "test names must carry their module path"
    )


@pytest.mark.parametrize(
    ("source", "expected"),
    [
        pytest.param("let t = trybuild::TestCases::new();", True, id="plain"),
        pytest.param("let t = trybuild :: TestCases::new();", True, id="spaced"),
        pytest.param("let t = other::TestCases::new();", False, id="other-crate"),
        pytest.param(
            "// let t = trybuild::TestCases::new();", False, id="line-comment"
        ),
        pytest.param(
            "/* let t = trybuild::TestCases::new(); */", False, id="block-comment"
        ),
        pytest.param(
            'let path = "trybuild::TestCases::new()";',
            False,
            id="string-literal",
        ),
    ],
)
def test_trybuild_detection(source: str, expected: bool) -> None:
    """A live trybuild driver is recognized however its path is spaced."""
    assert drives_trybuild(source) is expected, (
        "trybuild detection must ignore comments and literals"
    )


@pytest.mark.parametrize(
    ("source", "expected"),
    [
        pytest.param("#![cfg(loom)]\n", True, id="crate-gate"),
        pytest.param("  #![cfg(loom)]\n", True, id="indented-crate-gate"),
        pytest.param(
            "mod m {\n#![cfg(loom)]\n}\n", False, id="nested-at-column-zero"
        ),
        pytest.param("#![cfg(not(coverage))]\n", False, id="coverage-gate"),
        pytest.param("#[cfg(loom)]\n", False, id="item-gate"),
        pytest.param("// #![cfg(loom)]\n", False, id="commented-out"),
    ],
)
def test_loom_gate_detection(source: str, expected: bool) -> None:
    """Only a live crate-level Loom gate excludes a target from the run."""
    assert is_loom_gated(source) is expected, (
        "only a live crate-level Loom gate excludes a target"
    )


@pytest.mark.parametrize(
    ("text", "expected"),
    [
        pytest.param("180s", 180.0, id="seconds"),
        pytest.param("10m", 600.0, id="minutes"),
        pytest.param("1h", 3600.0, id="hours"),
        pytest.param("1.5m", 90.0, id="fractional"),
    ],
)
def test_durations_are_read_in_seconds(text: str, expected: float) -> None:
    """The reviewed units convert to seconds."""
    assert parse_duration(text) == expected, f"{text!r} must convert to seconds"


@pytest.mark.parametrize(
    "text",
    [
        pytest.param("180", id="no-unit"),
        pytest.param("180ms", id="unknown-unit"),
        pytest.param("180d", id="day-unit"),
        pytest.param("0s", id="zero"),
        pytest.param("", id="empty"),
        pytest.param(None, id="not-a-string"),
    ],
)
def test_an_unreadable_duration_is_refused(text: object) -> None:
    """An unknown unit or a non-positive value is refused, not guessed at."""
    with pytest.raises(DurationSyntaxError):
        parse_duration(text)  # type: ignore[arg-type]


@pytest.mark.parametrize(
    ("uses", "expected"),
    [
        (
            "leynos/shared-actions/.github/actions/generate-coverage@abc123",
            "leynos/shared-actions/.github/actions/generate-coverage",
        ),
        ("actions/checkout@v4", "actions/checkout"),
    ],
)
def test_coverage_action_name_strips_the_version_pin(uses: str, expected: str) -> None:
    """A pinned invocation yields the action path alone."""
    actual = coverage_action_name({"uses": uses})
    assert actual == expected, f"{uses!r} should name {expected!r}, got {actual!r}"


@pytest.mark.parametrize(
    "uses",
    [
        "actions/checkout",
        "actions/checkout@",
        "@v4",
        "",
    ],
)
def test_coverage_action_name_rejects_an_invocation_without_a_pin(uses: str) -> None:
    """A ``uses`` value with no pin, an empty pin, or no name is refused."""
    with pytest.raises(AssertionError, match="action invocation"):
        coverage_action_name({"uses": uses})
