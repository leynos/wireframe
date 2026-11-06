"""Pin when a Rust source counts as a trybuild driver.

An override bounds a test's deadline only because that test compiles
trybuild fixtures. Two shapes satisfy the eye but compile nothing: a body
that merely names ``trybuild::TestCases``, and a driver that is constructed
but never asked to run a case. Both leave the compilation to run under the
default deadline while the contract stays green, so each is rejected here.

The file-level question decides which targets need an override at all; the
scoped question decides whether the override's own test does the work. The
tables below keep the two apart so a regression names the one it broke.
"""

from __future__ import annotations

import pytest

from rust_source_lexing import drives_trybuild
from rust_test_bodies import drives_trybuild_in_test


@pytest.mark.parametrize(
    ("source", "expected"),
    [
        pytest.param(
            'let t = trybuild::TestCases::new(); t.pass("a.rs");',
            True,
            id="plain",
        ),
        pytest.param(
            'let t = trybuild :: TestCases :: new (); t . pass ("a.rs");',
            True,
            id="spaced",
        ),
        pytest.param(
            'let t = trybuild::TestCases::new(); t.compile_fail("a.rs");',
            True,
            id="compile-fail",
        ),
        pytest.param("let t = other::TestCases::new();", False, id="other-crate"),
        pytest.param("let t: trybuild::TestCases;", False, id="type-only"),
        pytest.param(
            "let t = trybuild::TestCases::new();", False, id="construct-only"
        ),
        pytest.param('t.pass("a.rs");', False, id="fixture-only"),
        pytest.param(
            '// let t = trybuild::TestCases::new(); t.pass("a.rs");',
            False,
            id="line-comment",
        ),
        pytest.param(
            '/* let t = trybuild::TestCases::new(); t.pass("a.rs"); */',
            False,
            id="block-comment",
        ),
        pytest.param(
            'let path = "trybuild::TestCases::new() t.pass(a.rs)";',
            False,
            id="string-literal",
        ),
    ],
)
def test_trybuild_detection(source: str, expected: bool) -> None:
    """A driver counts only when it is constructed and runs a fixture."""
    assert drives_trybuild(source) is expected, (
        "trybuild detection must ignore comments and literals"
    )


@pytest.mark.parametrize(
    ("source", "expected"),
    [
        pytest.param(
            "#[test]\nfn compile_tests() {\n"
            "    let t = trybuild::TestCases::new();\n"
            '    t.pass("tests/ui/a.rs");\n}\n',
            True,
            id="own-body",
        ),
        pytest.param(
            "#[test]\nfn compile_tests() {\n"
            "    { let inner = 1; }\n"
            "    let t = trybuild::TestCases::new();\n"
            '    t.pass("tests/ui/a.rs");\n}\n',
            True,
            id="nested-block-before-driver",
        ),
        pytest.param(
            "#[test]\nfn compile_tests() {\n"
            '    let brace = "}";\n'
            "    let t = trybuild::TestCases::new();\n"
            '    t.pass("tests/ui/a.rs");\n}\n',
            True,
            id="string-containing-brace",
        ),
        pytest.param(
            "#[test]\nfn compile_tests() {\n"
            "    mod inner { fn helper() {} }\n"
            "    let t = trybuild::TestCases::new();\n"
            '    t.pass("tests/ui/a.rs");\n}\n',
            True,
            id="inline-module-before-driver",
        ),
        pytest.param(
            "#[test]\nfn other() {}\n#[test]\nfn compile_tests() {\n"
            "    helper();\n}\nfn helper() {\n"
            "    let t = trybuild::TestCases::new();\n"
            '    t.pass("tests/ui/a.rs");\n}\n',
            False,
            id="helper-only",
        ),
        pytest.param(
            "#[test]\nfn compile_tests() {}\n#[test]\nfn sibling() {\n"
            "    let t = trybuild::TestCases::new();\n"
            '    t.pass("tests/ui/a.rs");\n}\n',
            False,
            id="sibling-only",
        ),
        pytest.param(
            "#[test]\nfn compile_tests() {}\n",
            False,
            id="empty-body",
        ),
        pytest.param(
            "#[test]\nfn compile_tests() {\n"
            "    let t: trybuild::TestCases;\n}\n",
            False,
            id="type-only-in-body",
        ),
        pytest.param(
            "#[test]\nfn compile_tests() {\n"
            "    let t = trybuild::TestCases::new();\n}\n",
            False,
            id="constructs-without-running",
        ),
        pytest.param(
            "#[test]\nfn compile_tests() {\n"
            "    // trybuild::TestCases\n}\n",
            False,
            id="commented-out-in-body",
        ),
        pytest.param(
            "mod m {\n    #[test]\n    fn compile_tests() {\n"
            "        let t = trybuild::TestCases::new();\n"
            '        t.pass("tests/ui/a.rs");\n    }\n}\n',
            False,
            id="nested-module",
        ),
    ],
)
def test_trybuild_use_is_scoped_to_the_selected_test(
    source: str, expected: bool
) -> None:
    """Only the override's own test has to drive trybuild."""
    assert drives_trybuild_in_test(source, "compile_tests") is expected, (
        "a sibling or helper driver must not stand in for the selected test"
    )