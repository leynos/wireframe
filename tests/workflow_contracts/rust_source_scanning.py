"""Read Rust test modules well enough to prove a nextest exact matcher.

Nextest names a test by its module path, so ``test(=compile_tests)`` selects a
crate-root ``#[test] fn compile_tests`` and nothing else. Proving that from
source means knowing which ``fn`` items are tests, which module each sits in,
and which of the surrounding text is actually code: a commented-out definition
or a string literal containing the name is not evidence. The text is cleaned
by ``rust_source_lexing`` before any of this scanning runs.

The scanning here is deliberately small. It handles the layouts a contract file
can meaningfully inspect — inline ``mod`` blocks, item braces and item
attributes — and ignores everything else. It is not a Rust parser, and it says
so: a definition it cannot see is reported as missing rather than assumed.

The token vocabulary and the scope state are public because
``rust_test_bodies`` walks the same tokens to ask what one test's body does;
keeping one tokenizer means the two questions cannot drift apart.
"""

from __future__ import annotations

import enum
import re
import typing as typ

from rust_source_lexing import without_comments_and_literals

if typ.TYPE_CHECKING:
    from collections.abc import Iterator

#: Tokens that name a test or change the scope around it. ``mod NAME {``
#: consumes its opening brace so the module block is counted once. The Loom and
#: coverage gates are tokens so each can be attributed to the scope that
#: encloses it, and a ``cfg`` attribute is a token so a test it can remove from
#: the run is recognizable. The generic inner-``cfg`` token comes after the two
#: documented gates, so those keep their own names and only an undocumented
#: crate-root gate is reported as such.
ITEM_TOKEN: re.Pattern[str] = re.compile(
    r"""
    (?P<loom_gate>\#\!\s*\[\s*cfg\s*\(\s*loom\s*\)\s*\])
    | (?P<coverage_gate>
        \#\!\s*\[\s*cfg\s*\(\s*not\s*\(\s*coverage\s*\)\s*\)\s*\])
    | (?P<inner_cfg_attribute>\#\!\s*\[\s*cfg\s*[\(\=])
    | (?P<cfg_attribute>\#\s*\[\s*cfg\s*[\(\=])
    | (?P<module>\bmod\s+(?P<module_name>[A-Za-z_][A-Za-z0-9_]*)\s*\{)
    | (?P<open>\{)
    | (?P<close>\})
    | (?P<semicolon>;)
    | (?P<test>\#\s*\[\s*test\s*\])
    | (?P<function>\bfn\s+(?P<function_name>[A-Za-z_][A-Za-z0-9_]*))
    """,
    re.VERBOSE,
)


class ScanState:
    """Track open blocks and the attributes pending on the next item."""

    def __init__(self) -> None:
        """Start with no open blocks and no attributes pending on an item."""
        self.blocks: list[str | None] = []
        #: Whether each open block is itself behind a ``cfg`` attribute, so a
        #: test nested inside one is conditionally excluded too.
        self.block_cfg: list[bool] = []
        self.is_test_pending = False
        self.is_cfg_pending = False


def enter_module(state: ScanState, token: re.Match[str]) -> None:
    """Record an inline module and clear any pending item attributes."""
    state.blocks.append(token.group("module_name"))
    state.block_cfg.append(state.is_cfg_pending)
    state.is_test_pending = False
    state.is_cfg_pending = False


def enter_block(state: ScanState) -> None:
    """Record a block that is not a module, such as an ``impl`` or ``fn``."""
    state.blocks.append(None)
    state.block_cfg.append(state.is_cfg_pending)
    state.is_test_pending = False
    state.is_cfg_pending = False


def leave_block(state: ScanState) -> None:
    """Close the innermost block, if any."""
    if state.blocks:
        state.blocks.pop()
        state.block_cfg.pop()
    state.is_test_pending = False
    state.is_cfg_pending = False


def note_function(state: ScanState, token: re.Match[str]) -> str | None:
    """Return the qualified name of a pending test, or ``None`` for other items."""
    if not state.is_test_pending:
        return None
    state.is_test_pending = False
    modules = [block for block in state.blocks if block is not None]
    if len(modules) != len(state.blocks):
        return None
    return "::".join([*modules, token.group("function_name")])


def _test_items(source: str) -> Iterator[tuple[str, bool]]:
    """Yield each test's qualified name and whether ``cfg`` can exclude it.

    ``#[test]`` is an item attribute, so a ``cfg`` attribute written in the
    same attribute block, or on any enclosing module, removes the test from
    the run. Tracking those two sources keeps the answer tied to scope rather
    than to whatever text happens to sit nearby.

    Parameters
    ----------
    source : str
        Rust source text to scan.

    Yields
    ------
    tuple[str, bool]
        The module-qualified name, and ``True`` when a ``cfg`` attribute can
        exclude the test.
    """
    state = ScanState()
    for token in ITEM_TOKEN.finditer(without_comments_and_literals(source)):
        # ``lastgroup`` names the alternative that matched, not its inner
        # capture: a module token reports ``module``, a function ``function``.
        match token.lastgroup:
            case "module":
                enter_module(state, token)
            case "open":
                enter_block(state)
            case "close":
                leave_block(state)
            case "semicolon":
                state.is_cfg_pending = False
            case "cfg_attribute":
                state.is_cfg_pending = True
            case "test":
                state.is_test_pending = True
            case "function":
                is_gated = state.is_cfg_pending or any(state.block_cfg)
                path = note_function(state, token)
                if path is not None:
                    yield path, is_gated


def qualified_test_names(source: str) -> frozenset[str]:
    """Return the module-qualified names of the tests nextest would list.

    Nextest names a test by its module path, so an inline ``mod contracts {
    #[test] fn compile_tests() }`` is ``contracts::compile_tests``. A
    ``#[test]`` inside any other block (an ``impl`` or a function body) is
    not a test nextest runs, so it contributes no name. Comments and literals
    are stripped first, so a commented-out definition contributes nothing.

    Parameters
    ----------
    source : str
        Rust source text to scan.

    Returns
    -------
    frozenset[str]
        Fully qualified test names, such as ``{"contracts::compile_tests"}``.

    Examples
    --------
    >>> sorted(qualified_test_names("#[test]\\nfn a() {}"))
    ['a']
    >>> sorted(qualified_test_names("mod m { #[test] fn b() {} }"))
    ['m::b']
    """
    return frozenset(name for name, _ in _test_items(source))


def is_conditionally_excluded_test(source: str, name: str) -> bool:
    """Return whether a ``cfg`` attribute can keep the named test out of a run.

    An attribute such as ``#[cfg(feature = "x")]`` on the test, or on a module
    that encloses it, means nextest may list nothing for the override's exact
    matcher. A ``cfg`` elsewhere in the file, including one on a statement
    inside another function, is not attributed to this test.

    Parameters
    ----------
    source : str
        Rust source text to scan.
    name : str
        Bare function name the override's exact matcher states.

    Returns
    -------
    bool
        ``True`` when a crate-root test of that name sits behind a ``cfg``.

    Examples
    --------
    >>> is_conditionally_excluded_test(
    ...     '#[cfg(feature = "x")]\\n#[test]\\nfn a() {}', "a")
    True
    >>> is_conditionally_excluded_test("#[test]\\nfn a() {}", "a")
    False
    """
    return any(
        is_gated
        for qualified, is_gated in _test_items(source)
        if qualified == name
    )


class CrateGate(enum.Enum):
    """An inner-attribute gate the scanner can recognize at crate scope."""

    LOOM = "loom_gate"
    COVERAGE = "coverage_gate"


def _crate_scope_tokens(source: str) -> Iterator[str]:
    """Yield the name of each item token that applies at crate scope.

    An inner attribute applies to the item that encloses it, so a token inside
    an inline module or any other block is not crate-level however it is
    indented. Tracking brace scope therefore beats matching indentation. Each
    scope question in this module walks the same tokens, so the rule lives here
    once rather than in a copy per question.
    """
    state = ScanState()
    for token in ITEM_TOKEN.finditer(without_comments_and_literals(source)):
        match token.lastgroup:
            case "module":
                enter_module(state, token)
            case "open":
                enter_block(state)
            case "close":
                leave_block(state)
            case group if not state.blocks:
                yield group


def has_crate_scope_gate(source: str, gate: CrateGate) -> bool:
    """Return whether an inner-attribute gate applies at crate scope.

    An inner attribute applies to the item that encloses it, so a gate inside
    an inline module gates that module rather than the crate, however it is
    indented. Tracking brace scope therefore beats matching indentation: a
    module-nested gate at column zero is not crate-level, and an indented
    crate-level gate still is. Comments and literals are stripped first, so a
    commented-out gate contributes nothing.

    Both gates ask the same scope question, so one walk answers for either;
    naming the gate with :class:`CrateGate` keeps each call site explicit
    without a second copy of the loop.

    Parameters
    ----------
    source : str
        Rust source text to scan.
    gate : CrateGate
        The gate to look for.

    Returns
    -------
    bool
        ``True`` only when the gate applies to the crate root.

    Examples
    --------
    >>> has_crate_scope_gate("#![cfg(loom)]\\n", CrateGate.LOOM)
    True
    >>> has_crate_scope_gate(
    ...     "mod m {\\n#![cfg(not(coverage))]\\n}\\n", CrateGate.COVERAGE)
    False
    """
    return any(group == gate.value for group in _crate_scope_tokens(source))


def has_undocumented_crate_cfg(source: str) -> bool:
    """Return whether a crate-root inner ``cfg`` other than the gates applies.

    The scanner recognizes two inner gates by name: ``#![cfg(loom)]`` and
    ``#![cfg(not(coverage))]``. Any other crate-root inner ``cfg`` — a feature
    gate, say — can leave the target with no tests at all when the condition is
    false, so an override naming it would match nothing and fall back to the
    default allowance. Outer ``#[cfg(...)]`` attributes are a different
    question, answered by :func:`is_conditionally_excluded_test`.

    Parameters
    ----------
    source : str
        Rust source text to scan.

    Returns
    -------
    bool
        ``True`` when a crate-root inner ``cfg`` is neither documented gate.

    Examples
    --------
    >>> has_undocumented_crate_cfg('#![cfg(feature = "x")]\\n')
    True
    >>> has_undocumented_crate_cfg("#![cfg(not(coverage))]\\n")
    False
    >>> has_undocumented_crate_cfg('mod m {\\n#![cfg(feature = "x")]\\n}\\n')
    False
    """
    return any(
        group == "inner_cfg_attribute" for group in _crate_scope_tokens(source)
    )


def defines_test_function(source: str, name: str) -> bool:
    """Return whether ``source`` defines a crate-root ``#[test] fn name``.

    Nextest matches ``test(=NAME)`` against the fully qualified name, so a
    definition inside an inline module is ``module::NAME`` and the exact
    matcher does not select it. Comments and literals are ignored, so a
    commented-out definition is not evidence.

    Parameters
    ----------
    source : str
        Rust source text to scan.
    name : str
        Bare function name the override's exact matcher states.

    Returns
    -------
    bool
        ``True`` only for a crate-root test definition.

    Examples
    --------
    >>> defines_test_function("#[test]\\nfn compile_tests() {}", "compile_tests")
    True
    >>> defines_test_function("mod m { #[test] fn c() {} }", "c")
    False
    """
    return name in qualified_test_names(source)


def describe_qualified_definitions(source: str, name: str) -> str:
    """Explain a definition the filter's exact name does not reach.

    Parameters
    ----------
    source : str
        Rust source text to scan.
    name : str
        Bare function name the override's exact matcher states.

    Returns
    -------
    str
        A clause naming the module-qualified definitions, or an empty string
        when no test with that bare name exists at all.
    """
    qualified = sorted(
        path
        for path in qualified_test_names(source)
        if path.rsplit("::", 1)[-1] == name
    )
    if not qualified:
        return ""
    return (
        "; it is defined only as "
        + ", ".join(qualified)
        + ", which nextest names by module path"
    )
