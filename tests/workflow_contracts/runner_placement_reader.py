"""Reading machinery for the runner-placement contract.

This module knows how to get facts out of the workflow tree. It holds no
opinion about where anything should run: every reviewed decision lives in
``runner_placement_policy``, and the assertions that hold one against the
other live in ``ci_runner_placement_test``.

The three-way split exists because the contract passed this repository's
400-line maximum for a code file. Splitting by role rather than by size keeps
the boundary meaningful: what is derived from the tree, what a person decided,
and what enforces the one against the other.

Three readings here are less obvious than they look.

``workflow_paths`` reads both spellings of the extension. GitHub runs a
workflow written either way, so reading only ``*.yml`` would leave a lane in
``*.yaml`` outside every assertion in the contract, and the gap would be
invisible because nothing would fail.

``declaration_labels`` counts both arms of a conditional. A label reachable
only when a pull request comes from a fork is as much in use as one reachable
otherwise, and a registry check that missed it would demand a registration
for one arm and call the other stale.

``runner_value`` returns the *parsed* value rather than the file's text. A
folded scalar whose continuation is indented one level deeper keeps its line
break, so the parsed value carries a newline in the middle of an expression.
GitHub evaluates it regardless and the job runs, which is why the contract
reads the parse and not the source.
"""

from __future__ import annotations

import re
import typing as typ
from pathlib import Path

import yaml

REPO_ROOT: typ.Final = Path(__file__).resolve().parents[2]
WORKFLOW_DIR: typ.Final = REPO_ROOT / ".github" / "workflows"
ACTIONLINT_CONFIG: typ.Final = REPO_ROOT / ".github" / "actionlint.yaml"

#: Both spellings GitHub accepts for a workflow file's extension.
WORKFLOW_FILE_PATTERNS: typ.Final = ("*.yml", "*.yaml")

#: One expression, anchored end to end. ``[^'\n]`` in the arms and ``\S`` in
#: the guard keep a value carrying an embedded line break from matching here
#: as well, so the line-break failure is reported by its own test rather than
#: arriving as a confusing "expression not recognized".
RUNNER_EXPRESSION: typ.Final = re.compile(
    r"^\$\{\{ (?P<guard>\S+)"
    r" && '(?P<fork_arm>[^'\n]*)'"
    r" \|\| '(?P<default_arm>[^'\n]*)' \}\}$"
)

#: Every quoted literal in an expression, used to read the labels a lane can
#: actually select.
EXPRESSION_LITERAL: typ.Final = re.compile(r"'([^'\n]*)'")


def case_id(value: object) -> str:
    """Render one parametrized case identifier.

    A coordinate is a ``(workflow, job id)`` tuple, which pytest would
    otherwise render as an opaque index.

    Parameters
    ----------
    value
        One parametrized case: a ``(workflow, job id)`` coordinate, or any
        other value pytest was given.

    Returns
    -------
    str
        The coordinate's parts joined by ``-``, or ``str(value)`` otherwise.

    Examples
    --------
    >>> case_id(("ci.yml", "build-test"))
    'ci.yml-build-test'
    >>> case_id(30)
    '30'
    """
    if isinstance(value, tuple):
        return "-".join(str(item) for item in value)
    return str(value)


def workflow_paths() -> list[Path]:
    """Return every workflow document's path, in a stable order.

    Both spellings of the extension are read, because GitHub runs a workflow
    written either way.

    Returns
    -------
    list[Path]
        Every workflow document under ``.github/workflows``, sorted by path.

    Examples
    --------
    >>> names = [path.name for path in workflow_paths()]
    >>> "ci.yml" in names
    True
    """
    return sorted(
        path
        for pattern in WORKFLOW_FILE_PATTERNS
        for path in WORKFLOW_DIR.glob(pattern)
    )


def workflows() -> dict[str, dict[str, object]]:
    """Parse every workflow document, keyed by file name.

    Returns
    -------
    dict[str, dict[str, object]]
        Each workflow's parsed document, keyed by its file name.

    Examples
    --------
    >>> "ci.yml" in workflows()
    True
    """
    documents = {
        path.name: yaml.safe_load(path.read_text(encoding="utf-8"))
        for path in workflow_paths()
    }
    assert documents, "the repository should define at least one workflow"
    return documents


def jobs() -> dict[tuple[str, str], dict[str, object]]:
    """Return every job in the repository, keyed by ``(workflow, job id)``.

    Returns
    -------
    dict[tuple[str, str], dict[str, object]]
        Every job definition in the repository, keyed by its coordinate.

    Examples
    --------
    >>> ("ci.yml", "build-test") in jobs()
    True
    """
    keyed: dict[tuple[str, str], dict[str, object]] = {}
    for name, document in workflows().items():
        for job_id, definition in ((document or {}).get("jobs") or {}).items():
            keyed[(name, job_id)] = definition
    return keyed


def runner_value(definition: dict[str, object]) -> object | None:
    """Return a job's parsed ``runs-on`` value, if it declares one.

    Parameters
    ----------
    definition
        One job's parsed definition.

    Returns
    -------
    object | None
        The parsed ``runs-on`` value, or ``None`` when the job declares none.

    Examples
    --------
    >>> runner_value({"runs-on": "ubuntu-latest"})
    'ubuntu-latest'
    >>> runner_value({"uses": "./reusable.yml"}) is None
    True
    """
    return definition.get("runs-on")


def runner_declarations(definition: dict[str, object]) -> list[object]:
    """Return a job's ``runs-on`` declarations, which may be a list of labels.

    Parameters
    ----------
    definition
        One job's parsed definition.

    Returns
    -------
    list[object]
        Each ``runs-on`` declaration, empty when the job declares none.

    Examples
    --------
    >>> runner_declarations({"runs-on": ["self-hosted", "linux"]})
    ['self-hosted', 'linux']
    >>> runner_declarations({})
    []
    """
    value = runner_value(definition)
    if value is None:
        return []
    return value if isinstance(value, list) else [value]


def declaration_labels(declaration: object) -> set[str]:
    """Return every label one ``runs-on`` declaration can select.

    Both arms of an expression count. A label reachable only on the fork
    branch is as much in use as one reachable on the other.

    Parameters
    ----------
    declaration
        One ``runs-on`` declaration: a bare label or an expression.

    Returns
    -------
    set[str]
        Every label the declaration can select, both arms of an expression
        included.

    Examples
    --------
    >>> declaration_labels("ubuntu-latest")
    {'ubuntu-latest'}
    >>> sorted(declaration_labels("${{ x && 'ubuntu-latest' || 'other' }}"))
    ['other', 'ubuntu-latest']
    """
    text = str(declaration)
    if "${{" in text:
        return set(EXPRESSION_LITERAL.findall(text))
    return {text.strip()}


def job_labels(definition: dict[str, object]) -> set[str]:
    """Return every label one job can select, across all its declarations.

    Parameters
    ----------
    definition
        One job's parsed definition.

    Returns
    -------
    set[str]
        Every label the job can select, across all of its declarations.

    Examples
    --------
    >>> sorted(job_labels({"runs-on": ["self-hosted", "linux"]}))
    ['linux', 'self-hosted']
    """
    return {
        label
        for declaration in runner_declarations(definition)
        for label in declaration_labels(declaration)
    }


def labels_in_use() -> set[str]:
    """Return every label any lane in the repository can select.

    Returns
    -------
    set[str]
        Every label selectable by any job in any workflow.

    Examples
    --------
    >>> "ubuntu-latest" in labels_in_use()
    True
    """
    return {label for definition in jobs().values() for label in job_labels(definition)}


def registered_labels() -> set[str]:
    """Return the labels ``.github/actionlint.yaml`` registers.

    Returns
    -------
    set[str]
        Every label registered under ``self-hosted-runner``, empty when the
        configuration registers none.

    Examples
    --------
    >>> isinstance(registered_labels(), set)
    True
    """
    assert ACTIONLINT_CONFIG.exists(), (
        "this repository uses a runner label actionlint does not know, so "
        f"{ACTIONLINT_CONFIG.relative_to(REPO_ROOT)} must exist"
    )
    config = yaml.safe_load(ACTIONLINT_CONFIG.read_text(encoding="utf-8")) or {}
    return set((config.get("self-hosted-runner") or {}).get("labels") or [])


def job_steps(coordinate: tuple[str, str]) -> list[dict[str, object]]:
    """Return one job's steps, in order.

    Parameters
    ----------
    coordinate
        The job's ``(workflow, job id)`` coordinate.

    Returns
    -------
    list[dict[str, object]]
        The job's mapping steps, in declaration order.

    Examples
    --------
    >>> bool(job_steps(("ci.yml", "build-test")))
    True
    """
    steps = jobs()[coordinate].get("steps") or []
    assert isinstance(steps, list), (
        f"{coordinate[0]}:{coordinate[1]} should declare a list of steps"
    )
    return [step for step in steps if isinstance(step, dict)]


def step_named(coordinate: tuple[str, str], name: str) -> dict[str, object]:
    """Return the one step with the given ``name``.

    Exactly one, not the first: two steps sharing a name means the contract
    is asserting against whichever happens to come first, and which one that
    is can change without anything failing.

    Parameters
    ----------
    coordinate
        The job's ``(workflow, job id)`` coordinate.
    name
        The step's ``name``, which must be unique within the job.

    Returns
    -------
    dict[str, object]
        The single step carrying that name.

    Examples
    --------
    >>> "uses" in step_named(("ci.yml", "build-test"), "Cache Whitaker installer")
    True
    """
    matches = [step for step in job_steps(coordinate) if step.get("name") == name]
    assert len(matches) == 1, (
        f"expected exactly one step named {name!r} in "
        f"{coordinate[0]}:{coordinate[1]}, found {len(matches)}"
    )
    return matches[0]
