"""Drive the nextest discovery invariants across generated inputs.

The named fixture tests pin the accepted and rejected shapes the repository
actually uses. These property cases cover the same invariants over generated
matchers and generated Rust block structures, so a change that works only for
the fixtures fails here.

The strategies live beside the invariants they exercise so a failure names the
generated shape it rejected.
"""

from __future__ import annotations

import typing as typ
from pathlib import Path

import pytest
from cargo_targets import CargoTestTarget
from hypothesis import HealthCheck, given, settings
from hypothesis import strategies as st
from nextest_policy import (
    FilterSyntaxError,
    TargetFilter,
    override_target_violations,
    parse_target_filter,
)
from rust_source_scanning import qualified_test_names

#: A value nextest's exact-``=`` matcher accepts.
MATCHER_VALUE = st.from_regex(r"[A-Za-z0-9_.\-]{1,20}", fullmatch=True)
#: An identifier that can name a module or a function.
IDENTIFIER = st.from_regex(r"[A-Za-z_][A-Za-z0-9_]{0,12}", fullmatch=True)
#: A complete filter with independently generated matcher values.
MATCHER_FILTER = st.builds(
    TargetFilter, package=MATCHER_VALUE, binary=MATCHER_VALUE, test=MATCHER_VALUE
)
#: The conjunction orders nextest itself accepts.
MATCHER_KEYS = st.permutations(["package", "binary", "test"])
#: Separators nextest tolerates around ``&``.
SEPARATORS = st.sampled_from([" & ", "&", "  &  "])
#: Ways text can mention a definition without it being compiled.
SHADOWS = st.sampled_from(["line_comment", "block_comment", "string"])

PROPERTY_SETTINGS = settings(
    max_examples=25, deadline=None, suppress_health_check=[HealthCheck.too_slow]
)


def _test_function(function: str) -> str:
    """Return a ``#[test]`` function with an empty body."""
    return f"#[test]\nfn {function}() {{}}"


def _nest(body: str, modules: list[str]) -> str:
    """Wrap ``body`` in the given inline modules, the first one outermost."""
    for module in reversed(modules):
        body = f"mod {module} {{\n{body}\n}}"
    return body


@PROPERTY_SETTINGS
@given(expected=MATCHER_FILTER, order=MATCHER_KEYS, separator=SEPARATORS)
def test_any_order_of_exact_matchers_parses_to_the_same_filter(
    expected: TargetFilter, order: tuple[str, ...], separator: str
) -> None:
    """Nextest accepts any conjunction order, so the parser must too."""
    values = {
        "package": expected.package,
        "binary": expected.binary,
        "test": expected.test,
    }
    text = separator.join(f"{key}(={values[key]})" for key in order)
    assert parse_target_filter(text) == expected, f"{text!r} must parse to {expected!r}"


@PROPERTY_SETTINGS
@given(
    package=MATCHER_VALUE,
    binary=MATCHER_VALUE,
    test=MATCHER_VALUE,
    omitted=st.sets(st.sampled_from(["package", "binary", "test"]), max_size=2),
)
def test_a_filter_missing_any_matcher_is_rejected(
    package: str, binary: str, test: str, omitted: frozenset[str]
) -> None:
    """A conjunction without all three exact matchers must fail loudly."""
    values = {"package": package, "binary": binary, "test": test}
    text = " & ".join(
        f"{key}(={values[key]})" for key in sorted(values) if key not in omitted
    )
    if omitted:
        with pytest.raises(FilterSyntaxError, match="omits"):
            parse_target_filter(text)
    else:
        assert parse_target_filter(text) == TargetFilter(package, binary, test), (
            f"{text!r} names all three matchers and must parse"
        )


@PROPERTY_SETTINGS
@given(
    package=MATCHER_VALUE,
    binary=MATCHER_VALUE,
    test=MATCHER_VALUE,
    operator=st.sampled_from(["~", ""]),
)
def test_a_non_exact_test_matcher_is_rejected(
    package: str, binary: str, test: str, operator: str
) -> None:
    """A regex or substring matcher could name a target the author did not."""
    text = f"package(={package}) & binary(={binary}) & test({operator}{test})"
    with pytest.raises(FilterSyntaxError):
        parse_target_filter(text)


@PROPERTY_SETTINGS
@given(modules=st.lists(IDENTIFIER, max_size=3), function=IDENTIFIER)
def test_a_nested_definition_is_reported_at_its_module_path(
    modules: list[str], function: str
) -> None:
    """Every inline module prefixes the name nextest would list."""
    source = _nest(_test_function(function), modules)
    expected = "::".join([*modules, function])
    assert qualified_test_names(source) == frozenset({expected}), (
        f"{source!r} must yield {expected!r}"
    )


@PROPERTY_SETTINGS
@given(
    modules=st.lists(IDENTIFIER, max_size=2),
    function=IDENTIFIER,
    enclosing=IDENTIFIER,
)
def test_a_definition_outside_a_module_block_is_not_a_test(
    modules: list[str], function: str, enclosing: str
) -> None:
    """A ``#[test]`` in an ``impl`` or function body is not one nextest runs."""
    source = _nest(f"fn {enclosing}() {{ {_test_function(function)} }}", modules)
    assert qualified_test_names(source) == frozenset(), (
        f"{source!r} must not yield a test name"
    )


@PROPERTY_SETTINGS
@given(
    modules=st.lists(IDENTIFIER, max_size=2),
    function=IDENTIFIER,
    shadow=SHADOWS,
)
def test_text_that_only_shadows_a_definition_is_not_evidence(
    modules: list[str], function: str, shadow: str
) -> None:
    """A definition inside a comment or literal is not a compiled test."""
    definition = _nest(_test_function(function), modules)
    match shadow:
        case "line_comment":
            source = "\n".join(f"// {line}" for line in definition.splitlines())
        case "block_comment":
            source = f"/* {definition} */"
        case _:
            source = f'const TEXT: &str = "{definition}";'
    assert qualified_test_names(source) == frozenset(), (
        f"{source!r} must not yield a test name"
    )


def _override_filter(package: str, binary: str, function: str) -> str:
    """Return the exact conjunction an override must state."""
    return f"package(={package}) & binary(={binary}) & test(={function})"


def _trybuild_source(function: str) -> str:
    """Return a crate-root trybuild driver defining ``function``."""
    return (
        "#[test]\n"
        f"fn {function}() {{\n"
        "    let cases = trybuild::TestCases::new();\n"
        '    cases.compile_fail("tests/ui/example.rs");\n'
        "}\n"
    )


def _target(package: str, binary: str) -> CargoTestTarget:
    """Return a discovered test target with a unique source path."""
    return CargoTestTarget(
        package=package,
        name=binary,
        kinds=("test",),
        src_path=Path("/fixture") / f"{binary}.rs",
    )


def _override_config(*filters: str) -> dict[str, typ.Any]:
    """Return a nextest configuration whose overrides state ``filters``."""
    return {
        "profile": {
            "default": {"overrides": [{"filter": text} for text in filters]}
        }
    }


def _accepted_override(
    package: str, binary: str, function: str
) -> tuple[list[CargoTestTarget], dict[Path, str], dict[str, typ.Any]]:
    """Return the smallest override scenario the policy must accept."""
    target = _target(package, binary)
    return (
        [target],
        {target.src_path: _trybuild_source(function)},
        _override_config(_override_filter(package, binary, function)),
    )


@PROPERTY_SETTINGS
@given(package=IDENTIFIER, binary=IDENTIFIER, function=IDENTIFIER)
def test_an_override_naming_a_crate_root_trybuild_test_is_accepted(
    package: str, binary: str, function: str
) -> None:
    """An exact filter over the only trybuild target must report nothing."""
    targets, sources, config = _accepted_override(package, binary, function)
    assert override_target_violations(config, targets, sources.__getitem__) == [], (
        f"override for {package}/{binary}::{function} must be proved"
    )


#: Single defects the override resolution must report.
OVERRIDE_DEFECTS = st.sampled_from([
    "nested",
    "commented_out",
    "no_trybuild",
    "uncovered",
    "extra_override",
    "ambiguous",
])


def _defective_override(
    package: str, binary: str, function: str, defect: str
) -> tuple[list[CargoTestTarget], dict[Path, str], dict[str, typ.Any], str]:
    """Return one scenario broken in exactly one way, with its message."""
    target = _target(package, binary)
    source = _trybuild_source(function)
    plain = f"{_test_function(function)}\n"
    filter_text = _override_filter(package, binary, function)
    other = _target(package, f"{binary}_extra")
    if defect == "nested":
        sources = {target.src_path: f"mod inner {{\n{source}}}\n"}
        return [target], sources, _override_config(filter_text), (
            "does not define at its crate root"
        )
    if defect == "commented_out":
        commented = "".join(f"// {line}\n" for line in source.splitlines())
        return [target], {target.src_path: commented}, _override_config(filter_text), (
            "does not define at its crate root"
        )
    if defect == "no_trybuild":
        return [target], {target.src_path: plain}, _override_config(filter_text), (
            "does not drive trybuild::TestCases"
        )
    if defect == "uncovered":
        return [target, other], {
            target.src_path: source,
            other.src_path: source,
        }, _override_config(filter_text), (
            f"trybuild target {package}/{other.name} has no nextest override"
        )
    if defect == "extra_override":
        return [target, other], {
            target.src_path: source,
            other.src_path: plain,
        }, _override_config(
            filter_text, _override_filter(package, other.name, function)
        ), (
            f"override target {package}/{other.name} is not an uncovered "
            "trybuild target"
        )
    duplicate = CargoTestTarget(
        package=package,
        name=binary,
        kinds=("test",),
        src_path=Path("/fixture") / f"{binary}_duplicate.rs",
    )
    return [target, duplicate], {target.src_path: source}, _override_config(
        filter_text
    ), "resolves to 2 discovered test targets"


@PROPERTY_SETTINGS
@given(
    package=IDENTIFIER,
    binary=IDENTIFIER,
    function=IDENTIFIER,
    defect=OVERRIDE_DEFECTS,
)
def test_one_defect_in_the_override_resolution_is_reported(
    package: str, binary: str, function: str, defect: str
) -> None:
    """Each single defect must produce its own diagnostic, not a silent pass."""
    targets, sources, config, expected = _defective_override(
        package, binary, function, defect
    )
    violations = override_target_violations(config, targets, sources.__getitem__)
    assert any(expected in violation for violation in violations), (
        f"{defect!r} must report {expected!r}, got {violations!r}"
    )
