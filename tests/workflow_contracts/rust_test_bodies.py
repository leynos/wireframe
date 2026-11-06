"""Read what one named test does, rather than what its file contains.

An override's exact matcher names a test, so the test that has to compile the
cases is the named one. Asking whether the *file* drives trybuild is a weaker
question: a sibling test can construct ``trybuild::TestCases`` while the
selected test does nothing, and the override then bounds no work while the
compilation still runs under the default deadline.

This module answers the narrow question. It finds the crate-root ``#[test]``
item with the given name and returns the text between that function's own
braces, using the token vocabulary and scope state of
``rust_source_scanning``. Comments and literals are blanked first, so a driver
mentioned only in a comment or a string is not evidence.
"""

from __future__ import annotations

import re

from rust_source_lexing import drives_trybuild, without_comments_and_literals
from rust_source_scanning import (
    ITEM_TOKEN,
    ScanState,
    enter_block,
    enter_module,
    leave_block,
    note_function,
)


def _brace_depth_delta(token: re.Match[str]) -> int:
    """Return how much one token changes the enclosing brace depth."""
    return {"open": 1, "close": -1}.get(token.lastgroup, 0)


def _closing_brace(cleaned: str, open_index: int) -> int | None:
    """Return the index of the brace closing the one at ``open_index``."""
    depth = 0
    for token in ITEM_TOKEN.finditer(cleaned, open_index):
        depth += _brace_depth_delta(token)
        if depth == 0:
            return token.start()
    return None


def _braced_body(cleaned: str, open_index: int) -> str:
    """Return the text between the braces that open at ``open_index``.

    An unterminated body yields the rest of the source rather than raising:
    the contract reports a missing driver either way, and a partial file is
    still worth reading rather than crashing the whole gate.
    """
    end = _closing_brace(cleaned, open_index)
    if end is None:
        return cleaned[open_index + 1 :]
    return cleaned[open_index + 1 : end]


def test_function_body(source: str, name: str) -> str | None:
    """Return the body of the crate-root ``#[test] fn name``, or ``None``.

    The body is the text between the function's own braces, so a caller can
    ask what that one test does rather than what the file contains. A
    definition inside an inline module, an ``impl`` or another function is not
    the crate-root test an exact matcher selects, so it yields ``None``.

    Parameters
    ----------
    source : str
        Rust source text to scan.
    name : str
        Bare function name the override's exact matcher states.

    Returns
    -------
    str | None
        The body text, or ``None`` when no crate-root test has that name.

    Examples
    --------
    >>> test_function_body("#[test]\\nfn a() { work(); }", "a")
    ' work(); '
    >>> test_function_body("mod m { #[test] fn a() {} }", "a") is None
    True
    """
    cleaned = without_comments_and_literals(source)
    state = ScanState()
    is_awaiting_body = False
    for token in ITEM_TOKEN.finditer(cleaned):
        if is_awaiting_body and token.lastgroup == "open":
            return _braced_body(cleaned, token.start())
        match token.lastgroup:
            case "module":
                enter_module(state, token)
                is_awaiting_body = False
            case "open":
                enter_block(state)
            case "close":
                leave_block(state)
            case "semicolon":
                state.is_cfg_pending = False
                is_awaiting_body = False
            case "cfg_attribute":
                state.is_cfg_pending = True
            case "test":
                state.is_test_pending = True
            case "function":
                is_awaiting_body = note_function(state, token) == name
    return None


def drives_trybuild_in_test(source: str, name: str) -> bool:
    """Return whether the named crate-root test's own body drives trybuild.

    A sibling test that constructs ``trybuild::TestCases`` does not make the
    override's test compile anything, so the question is scoped to the selected
    function rather than the file. A driver reached through a helper function
    is not recognized; the contract reports that as missing rather than
    assuming it, which fails loudly instead of passing silently.

    Parameters
    ----------
    source : str
        Rust source text to scan.
    name : str
        Bare function name the override's exact matcher states.

    Returns
    -------
    bool
        ``True`` only when that test's body constructs ``trybuild::TestCases``.

    Examples
    --------
    >>> drives_trybuild_in_test(
    ...     "#[test]\\nfn a() { trybuild::TestCases::new(); }", "a")
    True
    >>> drives_trybuild_in_test(
    ...     "#[test]\\nfn a() {}\\n#[test]\\nfn b() {"
    ...     " trybuild::TestCases::new(); }", "a")
    False
    """
    body = test_function_body(source, name)
    return body is not None and drives_trybuild(body)