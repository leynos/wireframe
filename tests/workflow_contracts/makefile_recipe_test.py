"""Pin the ``test-ui`` recipe to the compile-time harnesses it must run.

`make test-ui` is the only place CI runs the trybuild and Loom compile-time
suites, so the recipe itself is part of the contract. Matching the recipe text
is not enough: an `@echo` of a harness, a harness behind an inactive Make
conditional, a second `test-ui` rule that replaces the first, a harness moved
under a later target, or the Loom `--cfg=loom` flag moved onto the root command
all leave the text intact while the harness never runs. The checks here read the
recipe as commands and refuse each of those shapes.
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

#: The directives that open and close a conditional block. A whole rule written
#: inside one is not conditional *in its recipe*: the recipe reads clean while
#: Make skips the rule entirely, so the enclosing block is checked separately.
_CONDITIONAL_OPEN: re.Pattern[str] = re.compile(r"^(?:ifeq|ifneq|ifdef|ifndef)\b")
_CONDITIONAL_CLOSE: re.Pattern[str] = re.compile(r"^endif\b")


def conditional_depth(makefile: str, limit: int) -> int:
    """Return how many Make conditionals enclose the line at ``limit``.

    Only column-zero directives count. A tab-indented line is recipe text that
    Make hands to the shell, which is why the recipe check reads it instead.

    Parameters
    ----------
    makefile
        The whole Makefile text.
    limit
        Character index in ``makefile`` to stop reading at.

    Returns
    -------
    int
        The nesting depth of the enclosing conditional blocks.
    """
    depth = 0
    for line in makefile[:limit].splitlines():
        if line.startswith("\t"):
            continue
        if _CONDITIONAL_OPEN.match(line):
            depth += 1
        elif _CONDITIONAL_CLOSE.match(line):
            depth -= 1
    return depth

#: A ``test-ui`` rule at the start of a line. GNU Make warns about a duplicate
#: target and runs the last recipe, so the contract must see every rule.
_UI_TARGET_RULE: re.Pattern[str] = re.compile(r"^test-ui:", re.MULTILINE)


def _recipe_body(remainder: str) -> str:
    """Return the tab-indented recipe lines at the start of ``remainder``.

    ``remainder`` begins part-way through the target line, so its first line is
    dropped. GNU Make ignores blank lines and comment lines inside a recipe, so
    those do not end it; the recipe ends at the first line that is neither blank,
    a comment, nor tab-indented. That line may be another target with no blank
    line in between, whose commands are not part of this recipe.

    Parameters
    ----------
    remainder
        Makefile text from the end of the ``test-ui:`` matcher onwards.

    Returns
    -------
    str
        The recipe lines, each keeping its leading tab and newline.
    """
    lines = remainder.splitlines(keepends=True)[1:]
    body: list[str] = []
    for line in lines:
        if line.startswith("\t"):
            body.append(line)
        elif line.strip() and not line.lstrip().startswith("#"):
            break
    return "".join(body)


def ui_recipe(makefile: str) -> str:
    """Return the body of the single ``test-ui`` recipe in ``makefile``.

    Parameters
    ----------
    makefile
        The whole Makefile text.

    Returns
    -------
    str
        Recipe text from the ``test-ui:`` rule up to the first line that is not
        a recipe line.

    Raises
    ------
    AssertionError
        If the Makefile declares no ``test-ui`` rule, more than one, or a rule
        that sits inside a Make conditional. GNU Make replaces an earlier
        recipe with a later one, so reading the first rule would check a recipe
        the build does not run; a rule inside an inactive conditional never
        runs at all, however clean its recipe reads.
    """
    rules = list(_UI_TARGET_RULE.finditer(makefile))
    assert len(rules) == 1, (
        f"the Makefile must declare exactly one test-ui rule, found {len(rules)}"
    )
    assert conditional_depth(makefile, rules[0].start()) == 0, (
        "the test-ui rule must not sit inside a Make conditional"
    )
    return _recipe_body(makefile[rules[0].end() :])


def ui_target_body() -> str:
    """Return the body of the checked-in ``test-ui`` recipe.

    Returns
    -------
    str
        Recipe text from ``test-ui:`` up to the first line that is not a recipe
        line.
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


#: A ``test-ui`` rule whose harnesses sit under a later target, with no blank
#: line in between. Make ends the first recipe at the next target, so those
#: commands never run for ``make test-ui``.
_HARNESSES_UNDER_LATER_TARGET: str = (
    "test-ui:\n"
    "\t@echo announced\n"
    "test-feature-gate:\n"
    f"\t{ROOT_HARNESS_COMMAND}\n"
    f"\t{LOOM_HARNESS_COMMAND}\n"
)


def test_ui_recipe_stops_at_the_next_target() -> None:
    """A harness moved under a later target is not a ``test-ui`` command."""
    body = ui_recipe(_HARNESSES_UNDER_LATER_TARGET)
    assert body == "\t@echo announced\n", (
        "the recipe must end at the next target, not at the next blank line"
    )
    with pytest.raises(AssertionError):
        assert_ui_target_keeps_harnesses(body)


def test_ui_recipe_continues_past_blank_and_comment_lines() -> None:
    """Make ignores blank and comment lines inside a recipe, so the contract must."""
    makefile = (
        "test-ui:\n"
        f"\t{ROOT_HARNESS_COMMAND}\n"
        "\n"
        "# the Loom harness follows the root one\n"
        f"\t{LOOM_HARNESS_COMMAND}\n"
        "test-feature-gate:\n"
        "\t@true\n"
    )
    body = ui_recipe(makefile)
    assert body == f"\t{ROOT_HARNESS_COMMAND}\n\t{LOOM_HARNESS_COMMAND}\n", (
        "blank and comment lines must not end the recipe"
    )
    assert_ui_target_keeps_harnesses(body)


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


def test_conditional_depth_counts_only_enclosing_blocks() -> None:
    """A closed block and a tab-indented directive do not enclose the rule."""
    makefile = (
        "ifeq ($(CI),true)\n"
        "endif\n"
        "\tifeq (0,1)\n"
        "test-ui:\n"
        f"\t{ROOT_HARNESS_COMMAND}\n"
    )
    rule = makefile.index("test-ui:")
    assert conditional_depth(makefile, rule) == 0, (
        "a closed block and a recipe-text directive must not count"
    )


def test_ui_recipe_rejects_a_rule_inside_a_conditional() -> None:
    """A rule wrapped in an inactive conditional is refused, however clean its body."""
    makefile = (
        "ifeq (0,1)\n"
        "test-ui:\n"
        f"\t{ROOT_HARNESS_COMMAND}\n"
        f"\t{LOOM_HARNESS_COMMAND}\n"
        "endif\n"
    )
    with pytest.raises(AssertionError, match="inside a Make conditional"):
        ui_recipe(makefile)


def test_ui_target_rejects_an_inactive_conditional_around_the_rule() -> None:
    """The checked-in Makefile is refused once its rule is made conditional."""
    makefile = (REPO_ROOT / "Makefile").read_text()
    mutated = makefile.replace(
        "test-ui: ## Run trybuild compile-time API tests\n",
        "ifeq (0,1)\ntest-ui: ## Run trybuild compile-time API tests\n",
    ).replace(
        f"\t{LOOM_HARNESS_COMMAND}\n\n",
        f"\t{LOOM_HARNESS_COMMAND}\nendif\n\n",
    )
    assert mutated != makefile, "the mutation must wrap the rule"
    with pytest.raises(AssertionError, match="inside a Make conditional"):
        ui_recipe(mutated)
