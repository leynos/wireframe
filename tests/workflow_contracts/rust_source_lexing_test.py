"""Pin how comments and literals are blanked before any scanning runs.

Every scanner in this suite sees only the text this module returns, so each
shadowing form a contract file can contain is pinned here with its exact
replacement. A regression would let a commented-out definition, or a string
that merely mentions a name, count as evidence that something is compiled.
"""

from __future__ import annotations

import pytest
from hypothesis import given, settings
from hypothesis import strategies as st

from rust_source_lexing import (
    drives_trybuild,
    trybuild_shortfall,
    without_comments_and_literals,
)

#: A trybuild driver that compiles a case, spelled without quotes so it can be
#: embedded in every shadowing form below.
DRIVER = "let cases = trybuild::TestCases::new(); cases.pass(FIXTURE);"
#: The two halves of that driver, kept apart so a shadow can sit between them.
CONSTRUCTION = "let cases = trybuild::TestCases::new();"
FIXTURE_CALL = "cases.pass(FIXTURE);"
#: Ways text can contain a driver without it ever being compiled.
SHADOWS = st.sampled_from([
    "line_comment",
    "block_comment",
    "nested_comment",
    "string",
    "escaped_string",
    "raw_string",
])
#: How deeply a block comment nests.
DEPTHS = st.integers(min_value=1, max_value=5)
#: How many escaped quotes precede the payload inside a string.
ESCAPES = st.integers(min_value=1, max_value=5)
#: How long a raw string's hash fence is.
FENCES = st.integers(min_value=1, max_value=6)

PROPERTY_SETTINGS = settings(max_examples=50, deadline=None)


def _shadow(
    kind: str,
    payload: str,
    *,
    depth: int,
    escapes: int,
    fences: int,
) -> str:
    """Return ``payload`` hidden inside the shadowing form ``kind``.

    Fixed examples would only pin the shapes written by hand. The nesting
    depth, the run of escaped quotes and the fence length are parameters so a
    counter that mishandles one of them is found by generation rather than by
    whichever case an author thought of.
    """
    match kind:
        case "line_comment":
            return "\n".join(f"// {line}" for line in payload.splitlines())
        case "block_comment":
            return f"/* {payload} */"
        case "nested_comment":
            return f"{'/* ' * depth}{payload}{' */' * depth}"
        case "string":
            return f'let text = "{payload}";'
        case "escaped_string":
            escaped = '\\"' * escapes
            return f'let text = "{escaped} {payload}";'
        case _:
            fence = "#" * fences
            return f'let text = r{fence}"{payload}"{fence};'


@pytest.mark.parametrize(
    ("source", "expected"),
    [
        pytest.param("a // b", "a  ", id="line-comment"),
        pytest.param("/* x */y", " y", id="block-comment"),
        pytest.param("/* a /* b */ c */z", " z", id="nested-block-comment"),
        pytest.param('let s = "a";', 'let s = "";', id="string"),
        pytest.param('let s = r#"a"#;', 'let s = "";', id="raw-string"),
        pytest.param("'a'", "' '", id="char-literal"),
        pytest.param("'\\''", "' '", id="escaped-quote-char-literal"),
        pytest.param('"a\\"b"', '""', id="escaped-quote-string"),
        pytest.param('r##"a"#b"##', '""', id="hashed-raw-string"),
        pytest.param("x /* unterminated", "x  ", id="unterminated-comment"),
    ],
)
def test_comments_and_literals_become_inert_text(
    source: str, expected: str
) -> None:
    """Each shadowing form is replaced, so scanning cannot read it as code."""
    assert without_comments_and_literals(source) == expected, (
        f"{source!r} must be blanked to {expected!r}"
    )


def test_a_lifetime_is_not_a_character_literal() -> None:
    """A lifetime marker must survive, or the rest of the file is swallowed."""
    assert without_comments_and_literals("&'a str") == "&'a str", (
        "a lifetime is not a character literal"
    )


@PROPERTY_SETTINGS
@given(shadow=SHADOWS, depth=DEPTHS, escapes=ESCAPES, fences=FENCES)
def test_a_shadow_between_live_fragments_leaves_both_visible(
    shadow: str, depth: int, escapes: int, fences: int
) -> None:
    """Shadowed tokens are inert; live tokens on either side stay visible."""
    shadowed = _shadow(
        shadow, DRIVER, depth=depth, escapes=escapes, fences=fences
    )
    assert not drives_trybuild(shadowed), (
        f"shadowed text must not count as a driver: {shadowed!r}"
    )
    source = f"{CONSTRUCTION}\n{shadowed}\n{FIXTURE_CALL}\n"
    assert drives_trybuild(source), (
        f"live code either side of {shadow!r} must still drive trybuild: "
        f"{source!r}"
    )


def test_a_fixture_call_on_another_receiver_is_not_a_driver() -> None:
    """A bound driver counts only when it is the object asked to run a case."""
    source = (
        "let cases = trybuild::TestCases::new();\n"
        "let harness = FixtureHarness::new();\n"
        "harness.pass(FIXTURE);\n"
    )
    assert trybuild_shortfall(source) == (
        "constructs trybuild::TestCases but runs no fixture through it"
    ), "a call on another receiver must not be read as the driver's own"
