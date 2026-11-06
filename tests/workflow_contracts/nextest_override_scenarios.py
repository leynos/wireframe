"""Build nextest override scenarios that are broken in exactly one way.

The checked-in configuration exercises a single accepted shape, so the
override policy is only as strong as the defects a test can construct. Each
builder here breaks one property — an exact matcher, a crate-root definition,
the trybuild driver, the target coverage, or the target resolution — and states
the diagnostic that must appear. The property tests pair these builders with
generated names, so a policy that recognizes only the checked-in fixture
fails.

Every defect lives in one builder registered in ``_DEFECT_BUILDERS``, and
``OVERRIDE_DEFECTS`` is derived from that registry, so a defect cannot be
listed without also being constructible.
"""

from __future__ import annotations

import dataclasses
import typing as typ
from pathlib import Path

from cargo_targets import CargoTestTarget
from hypothesis import strategies as st

if typ.TYPE_CHECKING:
    from collections.abc import Callable

#: The parts of an override scenario the policy is asked to judge: the
#: discovered targets, the source text to serve for each, the configuration,
#: and the diagnostic the scenario must provoke.
Scenario = tuple[
    list[CargoTestTarget], dict[Path, str], dict[str, typ.Any], str
]

#: The accepted counterpart: the same targets, sources and configuration with
#: no defect to report.
AcceptedScenario = tuple[
    list[CargoTestTarget], dict[Path, str], dict[str, typ.Any]
]


@dataclasses.dataclass(frozen=True)
class OverrideNames:
    """The generated names one override scenario is built from."""

    package: str
    binary: str
    function: str


def empty_test_source(function: str) -> str:
    """Return a ``#[test]`` function with an empty body."""
    return f"#[test]\nfn {function}() {{}}"


def trybuild_source(function: str) -> str:
    """Return a crate-root trybuild driver defining ``function``."""
    return (
        "#[test]\n"
        f"fn {function}() {{\n"
        "    let cases = trybuild::TestCases::new();\n"
        '    cases.compile_fail("tests/ui/example.rs");\n'
        "}\n"
    )


def test_target(package: str, binary: str) -> CargoTestTarget:
    """Return a discovered test target with a unique source path."""
    return CargoTestTarget(
        package=package,
        name=binary,
        kinds=("test",),
        src_path=Path("/fixture") / f"{binary}.rs",
    )


def override_filter(package: str, binary: str, function: str) -> str:
    """Return the exact conjunction an override must state."""
    return f"package(={package}) & binary(={binary}) & test(={function})"


def override_config(*filters: str) -> dict[str, typ.Any]:
    """Return a nextest configuration whose overrides state ``filters``."""
    return {
        "profile": {
            "default": {"overrides": [{"filter": text} for text in filters]}
        }
    }


def accepted_override(
    package: str, binary: str, function: str
) -> AcceptedScenario:
    """Return the smallest override scenario the policy must accept."""
    target = test_target(package, binary)
    return (
        [target],
        {target.src_path: trybuild_source(function)},
        override_config(override_filter(package, binary, function)),
    )


def _rewritten_source(
    names: OverrideNames,
    rewrite: Callable[[str], str],
    message: str,
) -> Scenario:
    """Return a one-target scenario whose trybuild source is rewritten."""
    target = test_target(names.package, names.binary)
    return (
        [target],
        {target.src_path: rewrite(trybuild_source(names.function))},
        override_config(
            override_filter(names.package, names.binary, names.function)
        ),
        message,
    )


def _nested(names: OverrideNames) -> Scenario:
    """A definition inside an inline module is not at the crate root."""
    return _rewritten_source(
        names,
        lambda source: f"mod inner {{\n{source}}}\n",
        "does not define at its crate root",
    )


def _commented_out(names: OverrideNames) -> Scenario:
    """A commented-out definition is never compiled."""
    return _rewritten_source(
        names,
        lambda source: "".join(f"// {line}\n" for line in source.splitlines()),
        "does not define at its crate root",
    )


def _no_trybuild(names: OverrideNames) -> Scenario:
    """A test function that never drives trybuild compiles no cases."""
    return _rewritten_source(
        names,
        lambda _source: empty_test_source(names.function),
        "does not drive trybuild::TestCases",
    )


def _cfg_gated(names: OverrideNames) -> Scenario:
    """A definition a ``cfg`` attribute can remove is not always compiled."""
    return _rewritten_source(
        names,
        lambda source: source.replace(
            f"fn {names.function}()",
            f'#[cfg(feature = "never")]\nfn {names.function}()',
        ),
        "cfg attribute",
    )


def _uncovered(names: OverrideNames) -> Scenario:
    """A trybuild target the one override does not name stays uncovered."""
    target = test_target(names.package, names.binary)
    other = test_target(names.package, f"{names.binary}_extra")
    source = trybuild_source(names.function)
    return (
        [target, other],
        {target.src_path: source, other.src_path: source},
        override_config(
            override_filter(names.package, names.binary, names.function)
        ),
        f"trybuild target {names.package}/{other.name} has no nextest override",
    )


def _extra_override(names: OverrideNames) -> Scenario:
    """An override for a target that is not a trybuild driver proves nothing."""
    target = test_target(names.package, names.binary)
    other = test_target(names.package, f"{names.binary}_extra")
    return (
        [target, other],
        {
            target.src_path: trybuild_source(names.function),
            other.src_path: empty_test_source(names.function),
        },
        override_config(
            override_filter(names.package, names.binary, names.function),
            override_filter(names.package, other.name, names.function),
        ),
        f"override target {names.package}/{other.name} is not an uncovered "
        "trybuild target",
    )


def _ambiguous(names: OverrideNames) -> Scenario:
    """Two discovered targets with the same name make the matcher ambiguous."""
    target = test_target(names.package, names.binary)
    duplicate = CargoTestTarget(
        package=names.package,
        name=names.binary,
        kinds=("test",),
        src_path=Path("/fixture") / f"{names.binary}_duplicate.rs",
    )
    source = trybuild_source(names.function)
    return (
        [target, duplicate],
        {target.src_path: source, duplicate.src_path: source},
        override_config(
            override_filter(names.package, names.binary, names.function)
        ),
        "resolves to 2 discovered test targets",
    )


#: One builder per single defect the override resolution must report.
_DEFECT_BUILDERS: dict[str, Callable[[OverrideNames], Scenario]] = {
    "nested": _nested,
    "commented_out": _commented_out,
    "no_trybuild": _no_trybuild,
    "cfg_gated": _cfg_gated,
    "uncovered": _uncovered,
    "extra_override": _extra_override,
    "ambiguous": _ambiguous,
}

#: Single defects the override resolution must report.
OVERRIDE_DEFECTS = st.sampled_from(sorted(_DEFECT_BUILDERS))


def defective_override(
    package: str, binary: str, function: str, defect: str
) -> Scenario:
    """Return one scenario broken in exactly one way, with its message.

    Parameters
    ----------
    package, binary, function
        Names the scenario is generated from.
    defect
        A key of ``OVERRIDE_DEFECTS``.

    Returns
    -------
    Scenario
        The targets, sources, configuration and expected diagnostic.

    Raises
    ------
    AssertionError
        When ``defect`` has no builder, so a stale strategy cannot pass
        silently.
    """
    try:
        build = _DEFECT_BUILDERS[defect]
    except KeyError:
        message = f"unknown override defect {defect!r}"
        raise AssertionError(message) from None
    return build(OverrideNames(package, binary, function))