"""Pin the ``test-ui`` recipe to the compile-time harnesses it must run.

`make test-ui` is the only place CI runs the trybuild and Loom compile-time
suites, so the recipe itself is part of the contract. Matching the recipe text
is not enough: an `@echo` of a harness, a harness behind an inactive Make
conditional, a second `test-ui` rule that replaces the first, or the Loom
`--cfg=loom` flag moved onto the root command all leave the text intact while
the harness never runs. The checks here read the recipe as commands and refuse
each of those shapes.
"""

from __future__ import annotations

import re

import pytest

from runner_placement_reader import REPO_ROOT

ROOT_HARNESS_COMMAND = "$(CARGO) test --test compile_error"
LOOM_HARNESS_COMMAND = (
    'CARGO_ENCODED_RUSTFLAGS="--cfg=loom" $(CARGO) test -p wireframe_testing'
    " --test loom_api_surface --features runtime-benchmarks"
)

#: Make directives that can make a recipe line conditional. A harness behind
#: one of these may never run, so the contract refuses the recipe outright.
_MAKE_CONDITIONAL: re.Pattern[str] = re.compile(
    r"^(?:ifeq|ifneq|ifdef|ifndef|else|endif)\b"
)

#: A ``test-ui`` rule at the start of a line. GNU Make warns about a duplicate
#: target and runs the last recipe, so the contract must see every rule.
_UI_TARGET_RULE: re.Pattern[str] = re.compile(r"^test-ui:", re.MULTILINE)


def ui_recipe(makefile: str) -> str:
    """Return the body of the single ``test-ui`` recipe in ``makefile``.

    Parameters
    ----------
    makefile
        The whole Makefile text.

    Returns
    -------
    str
        Recipe text from the ``test-ui:`` rule up to the next blank line.

    Raises
    ------
    AssertionError
        If the Makefile declares no ``test-ui`` rule, or more than one. GNU
        Make replaces an earlier recipe with a later one, so reading the first
        rule would check a recipe the build does not run.
    """
    rules = list(_UI_TARGET_RULE.finditer(makefile))
    assert len(rules) == 1, (
        f"the Makefile must declare exactly one test-ui rule, found {len(rules)}"
    )
    return makefile[rules[0].end() :].split("\n\n", 1)[0]


def ui_target_body() -> str:
    """Return the body of the checked-in ``test-ui`` recipe.

    Returns
    -------
    str
        Recipe text from ``test-ui:`` up to the next blank line.
    """
    return ui_recipe((REPO_ROOT / "Makefile").read_text())


def executable_recipe_lines(target: str) -> list[str]:
    """Return the commands a Makefile recipe actually runs.

    Comments and ``echo`` lines document or narrate a command. Counting them
    would let an edit replace a harness invocation with an announcement of one
    and still satisfy the contract.

    Parameters
    ----------
    target : str
        Recipe text, with one command per line.

    Returns
    -------
    list[str]
        Executable commands, with the leading ``@`` removed.
    """
    stripped = (line.strip() for line in target.splitlines())
    commands = [
        line.removeprefix("@")
        for line in stripped
        if line and not line.startswith("#")
    ]
    return [command for command in commands if not command.startswith("echo")]


def has_make_conditional(target: str) -> bool:
    """Return whether a recipe contains a Make conditional directive.

    Parameters
    ----------
    target : str
        Recipe text, with one line per recipe line.

    Returns
    -------
    bool
        ``True`` when any line is a Make conditional directive.
    """
    return any(
        _MAKE_CONDITIONAL.match(line.strip()) for line in target.splitlines()
    )


def assert_ui_target_keeps_harnesses(target: str) -> None:
    """Require the ``test-ui`` recipe to execute both compile-time suites.

    Parameters
    ----------
    target : str
        Recipe text to inspect.

    Raises
    ------
    AssertionError
        If the recipe is conditional, or either harness command is missing
        from its executable lines.
    """
    assert not has_make_conditional(target), (
        "test-ui must not gate its harnesses behind a Make conditional"
    )
    commands = executable_recipe_lines(target)
    assert ROOT_HARNESS_COMMAND in commands, (
        "test-ui must run the root compile-error harness"
    )
    assert LOOM_HARNESS_COMMAND in commands, (
        "test-ui must run the Loom harness with its cfg flag on that command"
    )


def test_ui_target_keeps_both_harnesses() -> None:
    """The non-instrumented gate runs root and Loom compile-time suites."""
    assert_ui_target_keeps_harnesses(ui_target_body())


@pytest.mark.parametrize(
    "makefile",
    [
        pytest.param("all:\n\t@true\n", id="missing"),
        pytest.param(
            "test-ui:\n\t@true\n\ntest-ui:\n\t@echo nothing to run\n",
            id="duplicate",
        ),
    ],
)
def test_ui_target_requires_one_unambiguous_rule(makefile: str) -> None:
    """A missing or repeated ``test-ui`` rule is refused, not guessed at."""
    with pytest.raises(AssertionError, match="exactly one"):
        ui_recipe(makefile)


@pytest.mark.parametrize(
    ("original", "replacement"),
    [
        pytest.param(
            ROOT_HARNESS_COMMAND,
            f'@echo "{ROOT_HARNESS_COMMAND}"',
            id="echoed-root",
        ),
        pytest.param(
            LOOM_HARNESS_COMMAND,
            f'@echo "{LOOM_HARNESS_COMMAND}"',
            id="echoed-loom",
        ),
        pytest.param(
            ROOT_HARNESS_COMMAND,
            f"{LOOM_HARNESS_COMMAND.split(' ', 1)[0]} {ROOT_HARNESS_COMMAND}",
            id="loom-flag-moved",
        ),
    ],
)
def test_ui_target_rejects_ineffective_harness_commands(
    original: str, replacement: str
) -> None:
    """An announced or wrongly gated harness does not satisfy the UI contract."""
    target = ui_target_body()
    mutated = target.replace(f"\t{original}", f"\t{replacement}")
    assert mutated != target, "the mutation must change the recipe"
    with pytest.raises(AssertionError):
        assert_ui_target_keeps_harnesses(mutated)


def test_ui_target_rejects_conditional_harnesses() -> None:
    """A harness behind an inactive Make conditional never runs."""
    target = ui_target_body()
    mutated = target.replace(
        f"\t{ROOT_HARNESS_COMMAND}",
        f"\tifeq (0,1)\n\t{ROOT_HARNESS_COMMAND}",
    ).replace(
        f"\t{LOOM_HARNESS_COMMAND}",
        f"\t{LOOM_HARNESS_COMMAND}\n\tendif",
    )
    assert mutated != target, "the mutation must change the recipe"
    with pytest.raises(AssertionError):
        assert_ui_target_keeps_harnesses(mutated)
