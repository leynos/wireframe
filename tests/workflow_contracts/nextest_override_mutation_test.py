"""Prove the override contract rejects the ways an override can go wrong.

The checked-in configuration exercises only the accepted shape, so a validator
that ignored a missing matcher, a non-exact matcher, or an uncovered trybuild
target would pass against it either way. Each mutation below breaks exactly one
property of the accepted configuration and must be rejected.
"""

from __future__ import annotations

import typing as typ
from copy import deepcopy
from pathlib import Path

import pytest

from cargo_targets import CargoTestTarget
from nextest_contract_support import (
    TRYSBUILD_SOURCE,
    assert_no_override_violations,
    read_source,
    repository_config,
    repository_targets,
)
from nextest_policy import override_target_violations
from rust_source_lexing import drives_trybuild
from rust_source_scanning import CrateGate, has_crate_scope_gate

if typ.TYPE_CHECKING:
    from collections.abc import Callable, Sequence


def _non_trybuild_target(targets: Sequence[CargoTestTarget]) -> CargoTestTarget:
    """Return a discovered target that does not drive trybuild."""
    return next(
        target
        for target in targets
        if not drives_trybuild(read_source(target.src_path))
        and not has_crate_scope_gate(read_source(target.src_path), CrateGate.LOOM)
    )


def _set_filter(
    config: dict[str, typ.Any], _targets: Sequence[CargoTestTarget], text: str
) -> None:
    """Replace the one override's filter with ``text``."""
    config["profile"]["default"]["overrides"][0]["filter"] = text


def _without_binary(
    config: dict[str, typ.Any], targets: Sequence[CargoTestTarget]
) -> None:
    """Drop the binary matcher from the filter."""
    _set_filter(config, targets, "package(=wireframe) & test(=compile_tests)")


def _without_package(
    config: dict[str, typ.Any], targets: Sequence[CargoTestTarget]
) -> None:
    """Drop the package matcher from the filter."""
    _set_filter(config, targets, "binary(=compile_error) & test(=compile_tests)")


def _without_test(
    config: dict[str, typ.Any], targets: Sequence[CargoTestTarget]
) -> None:
    """Drop the test matcher from the filter."""
    _set_filter(config, targets, "package(=wireframe) & binary(=compile_error)")


def _wrong_package(
    config: dict[str, typ.Any], targets: Sequence[CargoTestTarget]
) -> None:
    """Name a package that discovers no such target."""
    _set_filter(
        config,
        targets,
        "package(=other) & binary(=compile_error) & test(=compile_tests)",
    )


def _non_exact_binary(
    config: dict[str, typ.Any], targets: Sequence[CargoTestTarget]
) -> None:
    """State the binary matcher as a substring rather than an exact match."""
    _set_filter(
        config,
        targets,
        "package(=wireframe) & binary(compile_error) & test(=compile_tests)",
    )


def _extra_override(
    config: dict[str, typ.Any], targets: Sequence[CargoTestTarget]
) -> None:
    """Append an override for a target that does not drive trybuild."""
    target = _non_trybuild_target(targets)
    config["profile"]["default"]["overrides"].append({
        "filter": (
            f"package(={target.package}) & binary(={target.name})"
            " & test(=some_test)"
        ),
        "slow-timeout": {
            "period": "60s",
            "terminate-after": 8,
            "grace-period": "5s",
        },
    })


_OVERRIDE_MUTATIONS: typ.Final = {
    "no_binary": _without_binary,
    "no_package": _without_package,
    "no_test": _without_test,
    "wrong_package": _wrong_package,
    "non_exact": _non_exact_binary,
    "extra_override": _extra_override,
}


def _uncovered_trybuild_target(
    targets: Sequence[CargoTestTarget],
) -> tuple[tuple[CargoTestTarget, ...], dict[Path, str]]:
    """Add a trybuild-driving target that the one override does not cover."""
    synthetic = CargoTestTarget(
        package="wireframe",
        name="trybuild_extra",
        kinds=("test",),
        src_path=Path("/synthetic/trybuild_extra.rs"),
    )
    return (*targets, synthetic), {synthetic.src_path: TRYSBUILD_SOURCE}


def _cfg_gated_test(
    targets: Sequence[CargoTestTarget],
) -> tuple[tuple[CargoTestTarget, ...], dict[Path, str]]:
    """Gate the checked-in ``compile_tests`` behind a disabled feature."""
    target = next(target for target in targets if target.name == "compile_error")
    source = read_source(target.src_path)
    gated = source.replace(
        "#[test]\nfn compile_tests()",
        '#[cfg(feature = "never")]\n#[test]\nfn compile_tests()',
    )
    assert gated != source, "the mutation must gate the compile_tests function"
    return targets, {target.src_path: gated}


#: The override's test compiles nothing while a sibling drives trybuild, so a
#: check that read trybuild use across the whole file would accept it.
NO_OP_SELECTED_SOURCE = """\
#[test]
fn compile_tests() {}

#[test]
fn rstest_bdd_v06_contracts() {
    let cases = trybuild::TestCases::new();
    cases.pass("tests/ui/example.rs");
}
"""

#: The selected test constructs a driver but never runs a fixture through it,
#: so the compilation the override bounds never happens under that test.
UNRUN_FIXTURES_SOURCE = """\
#[test]
fn compile_tests() {
    let _ = trybuild::TestCases::new();
}
"""

#: The selected test binds a driver but runs its fixture through another
#: object, so the driver itself compiles nothing.
FIXTURE_ELSEWHERE_SOURCE = """\
#[test]
fn compile_tests() {
    let cases = trybuild::TestCases::new();
    let harness = FixtureHarness::new();
    harness.pass("tests/ui/example.rs");
}
"""


def _crate_root_cfg(
    targets: Sequence[CargoTestTarget],
) -> tuple[tuple[CargoTestTarget, ...], dict[Path, str]]:
    """Gate the whole target behind a crate-root feature that stays off."""
    target = next(target for target in targets if target.name == "compile_error")
    source = read_source(target.src_path)
    gated = source.replace(
        "#![cfg(not(coverage))]",
        '#![cfg(not(coverage))]\n#![cfg(feature = "never")]',
        1,
    )
    assert gated != source, "the mutation must add a crate-root feature gate"
    return targets, {target.src_path: gated}


def _no_op_selected_test(
    targets: Sequence[CargoTestTarget],
) -> tuple[tuple[CargoTestTarget, ...], dict[Path, str]]:
    """Empty the selected test's body, keeping a sibling's trybuild driver."""
    target = next(target for target in targets if target.name == "compile_error")
    return targets, {target.src_path: NO_OP_SELECTED_SOURCE}


def _unrun_fixtures(
    targets: Sequence[CargoTestTarget],
) -> tuple[tuple[CargoTestTarget, ...], dict[Path, str]]:
    """Keep the driver construction and drop every fixture call."""
    target = next(target for target in targets if target.name == "compile_error")
    return targets, {target.src_path: UNRUN_FIXTURES_SOURCE}


def _fixture_on_another_receiver(
    targets: Sequence[CargoTestTarget],
) -> tuple[tuple[CargoTestTarget, ...], dict[Path, str]]:
    """Bind a driver and run the fixture through a different object."""
    target = next(target for target in targets if target.name == "compile_error")
    return targets, {target.src_path: FIXTURE_ELSEWHERE_SOURCE}


#: The selected test declares its driver inside a nested function it never
#: calls. The item compiles, the driver never runs, and the text still looks
#: like a trybuild case to any check that reads the body as a whole.
NESTED_UNCALLED_DRIVER_SOURCE = """\
#[test]
fn compile_tests() {
    fn helper() {
        let cases = trybuild::TestCases::new();
        cases.pass("tests/ui/example.rs");
    }
}
"""


def _nested_uncalled_driver(
    targets: Sequence[CargoTestTarget],
) -> tuple[tuple[CargoTestTarget, ...], dict[Path, str]]:
    """Confine the driver to a nested function the test never calls."""
    target = next(target for target in targets if target.name == "compile_error")
    return targets, {target.src_path: NESTED_UNCALLED_DRIVER_SOURCE}


#: Mutations that doctor the target list or the source text rather than the
#: configuration, so the test can hand them different state.
_SOURCE_MUTATIONS: typ.Final = {
    "uncovered_trybuild_target": _uncovered_trybuild_target,
    "cfg_gated_test": _cfg_gated_test,
    "crate_root_cfg": _crate_root_cfg,
    "no_op_selected_test": _no_op_selected_test,
    "unrun_fixtures": _unrun_fixtures,
    "fixture_on_another_receiver": _fixture_on_another_receiver,
    "nested_uncalled_driver": _nested_uncalled_driver,
}

#: Each mutation and the fragment of the violation that names its own cause.
#: Several rows also trigger the uncovered-target check, because a filter the
#: parser refuses claims no target. Matching only "some assertion failed" would
#: therefore pass even if the property the row breaks were no longer checked, so
#: the expected cause is pinned rather than the bare fact of a failure.
_MUTATION_VIOLATIONS: typ.Final = {
    "no_binary": "filter omits the binary matcher",
    "no_package": "filter omits the package matcher",
    "no_test": "filter omits the test matcher",
    "wrong_package": "resolves to 0 discovered test targets",
    "non_exact": "is not an exact",
    "extra_override": "is not an uncovered trybuild target",
    "uncovered_trybuild_target": "has no nextest override",
    "cfg_gated_test": "can exclude from the run",
    "crate_root_cfg": "crate-root cfg attribute",
    "no_op_selected_test": "does not drive trybuild::TestCases",
    "unrun_fixtures": "does not drive trybuild::TestCases",
    "nested_uncalled_driver": "does not drive trybuild::TestCases",
}


def _mutate_override_config(
    mutation: str,
    config: dict[str, typ.Any],
    targets: Sequence[CargoTestTarget],
) -> None:
    """Apply one override-config mutation in place."""
    try:
        mutate = _OVERRIDE_MUTATIONS[mutation]
    except KeyError:
        message = f"unknown override mutation {mutation!r}"
        raise AssertionError(message) from None
    mutate(config, targets)


def _source_reader_with(
    extra: dict[Path, str],
) -> Callable[[Path], str]:
    """Return a reader that serves ``extra`` before the filesystem."""

    def read(path: Path) -> str:
        """Serve the injected source when one matches, else read from disk."""
        return extra[path] if path in extra else read_source(path)

    return read


@pytest.mark.parametrize(
    ("mutation", "expected"),
    [
        pytest.param(mutation, expected, id=mutation)
        for mutation, expected in _MUTATION_VIOLATIONS.items()
    ],
)
def test_override_mutations_are_rejected(mutation: str, expected: str) -> None:
    """Each mutation alone leaves an override unproved or a target uncovered."""
    config = deepcopy(repository_config())
    targets: tuple[CargoTestTarget, ...] = repository_targets()
    if mutation in _SOURCE_MUTATIONS:
        targets, extra_sources = _SOURCE_MUTATIONS[mutation](targets)
    else:
        extra_sources = {}
        _mutate_override_config(mutation, config, targets)
    with pytest.raises(AssertionError, match=expected):
        assert_no_override_violations(
            config, targets, _source_reader_with(extra_sources)
        )


def test_a_sibling_trybuild_driver_does_not_cover_the_selected_test() -> None:
    """The override must name the test that compiles, not just any test."""
    targets, extra_sources = _no_op_selected_test(repository_targets())
    violations = override_target_violations(
        repository_config(), targets, _source_reader_with(extra_sources)
    )
    assert any(
        "does not drive trybuild::TestCases" in violation
        for violation in violations
    ), f"the empty selected test must be reported, got {violations!r}"


def test_a_selected_test_that_runs_no_fixture_is_rejected() -> None:
    """Constructing a driver without running a case compiles nothing."""
    targets, extra_sources = _unrun_fixtures(repository_targets())
    violations = override_target_violations(
        repository_config(), targets, _source_reader_with(extra_sources)
    )
    assert any(
        "does not drive trybuild::TestCases" in violation
        and "runs no fixture" in violation
        for violation in violations
    ), f"a driver with no fixture call must be reported, got {violations!r}"


def test_a_fixture_call_on_another_receiver_is_rejected() -> None:
    """A driver is evidence only when it is the object asked to run a case."""
    targets, extra_sources = _fixture_on_another_receiver(repository_targets())
    violations = override_target_violations(
        repository_config(), targets, _source_reader_with(extra_sources)
    )
    assert any(
        "does not drive trybuild::TestCases" in violation
        and "runs no fixture through it" in violation
        for violation in violations
    ), f"a call on another receiver must be reported, got {violations!r}"


def test_non_list_overrides_are_rejected() -> None:
    """A scalar ``profile.default.overrides`` cannot be iterated as overrides."""
    config = {"profile": {"default": {"overrides": {"filter": "x"}}}}
    assert override_target_violations(config, (), read_source) == [
        "profile.default.overrides is not a list"
    ], "a non-list overrides value must be reported"


def test_non_string_override_filter_is_rejected() -> None:
    """A filter that is not text is reported rather than parsed."""
    config = {"profile": {"default": {"overrides": [{"filter": 7}]}}}
    assert override_target_violations(config, (), read_source) == [
        "override 0 has no string filter"
    ], "a non-string filter must be reported"
