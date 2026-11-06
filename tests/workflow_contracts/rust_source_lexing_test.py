"""Pin how comments and literals are blanked before any scanning runs.

Every scanner in this suite sees only the text this module returns, so each
shadowing form a contract file can contain is pinned here with its exact
replacement. A regression would let a commented-out definition, or a string
that merely mentions a name, count as evidence that something is compiled.
"""

from __future__ import annotations

import pytest

from rust_source_lexing import without_comments_and_literals


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
