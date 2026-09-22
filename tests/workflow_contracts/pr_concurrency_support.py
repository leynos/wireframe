"""Readers for the pull-request concurrency contract.

A superseded pull-request run costs the same minutes as the run that
replaced it. GitHub cancels one only when the workflow declares a
concurrency group and asks for it, so the fact is a property of every
workflow a pull request can start, not of any one job.

The readers here are pure: :func:`trigger_names` and
:func:`concurrency_violations` take a parsed document, so the contract
can drive them with a synthetic workflow the repository does not
contain. A rule exercised only over files that already satisfy it
passes whether or not it works.
"""

import typing as typ
from pathlib import Path

import yaml

ROOT: typ.Final = Path(__file__).resolve().parents[2]

#: GitHub accepts either extension, so a sweep that scans one is a gap.
WORKFLOW_SUFFIXES: typ.Final = (".yml", ".yaml")

#: The trigger a pull request starts. ``pull_request_target`` runs with
#: the base repository's token; the workflows on it here push commits
#: and merge, and cancelling one mid-write is not a saving.
PULL_REQUEST_TRIGGER: typ.Final = "pull_request"

#: The only accepted ``cancel-in-progress`` value. A literal ``true``
#: would cancel a push to main or a scheduled run sharing the group, so
#: the contract requires the guarded expression, not a truthy setting.
CANCEL_IN_PROGRESS: typ.Final = "${{ github.event_name == 'pull_request' }}"

#: A group keyed on the run identifier is unique per run, so it
#: serializes nothing and can never cancel a predecessor.
RUN_ID_EXPRESSION: typ.Final = "github.run_id"


class WorkflowShapeError(AssertionError):
    """A workflow document is not the shape these readers can read.

    Derives from :class:`AssertionError` so a malformed workflow reads
    as a failed expectation rather than an unexpected crash, and names
    the workflow instead of a Python attribute. Each subclass carries
    its own message, so a caller cannot weaken one by passing a vaguer
    string at the raise site.
    """


class MissingTriggersError(WorkflowShapeError):
    """A workflow declared no triggers under either key."""

    def __init__(self) -> None:
        """Name the missing key rather than the Python that read it."""
        super().__init__("a workflow must declare triggers")


class UnreadableTriggersError(WorkflowShapeError):
    """A trigger value was none of the three shapes GitHub accepts."""

    def __init__(self) -> None:
        """Name the three shapes, so the fix is in the message."""
        super().__init__("triggers must be a string, a list or a mapping")


class NotAMappingError(WorkflowShapeError):
    """A workflow document did not parse to a mapping."""

    def __init__(self, name: str) -> None:
        """Name the workflow whose document is malformed.

        Parameters
        ----------
        name : str
            The workflow file name.
        """
        super().__init__(f"{name} must parse to a mapping")


class UnparsableWorkflowError(WorkflowShapeError):
    """A workflow document is not parsable YAML."""

    def __init__(self, name: str) -> None:
        """Name the workflow that failed to parse.

        Parameters
        ----------
        name : str
            The workflow file name.
        """
        super().__init__(f"{name} is not parsable YAML")


def trigger_names(document: dict[str, object]) -> frozenset[str]:
    """Return the event names a workflow document declares.

    YAML 1.1 reads an unquoted ``on:`` key as the boolean ``True``, so a
    reader that looks only under the string key finds no triggers at all
    and silently reports a workflow as startable by nothing.

    An unreadable trigger value raises :class:`WorkflowShapeError` from
    :func:`_event_names`.

    Parameters
    ----------
    document : dict[str, object]
        A parsed workflow document.

    Returns
    -------
    frozenset[str]
        The declared event names.

    Raises
    ------
    MissingTriggersError
        If the document declares no triggers under either key.
    """
    for key in ("on", True):
        if key in document:
            return _event_names(document[key])
    raise MissingTriggersError


def _event_names(triggers: object) -> frozenset[str]:
    """Normalize the three shapes GitHub accepts under ``on:``.

    Parameters
    ----------
    triggers : object
        The value of the workflow's trigger key.

    Returns
    -------
    frozenset[str]
        The declared event names.

    Raises
    ------
    UnreadableTriggersError
        If the value is none of the three accepted shapes.
    """
    match triggers:
        case str():
            return frozenset({triggers})
        case dict() | list():
            return frozenset(str(name) for name in triggers)
        case _:
            raise UnreadableTriggersError


def is_pull_request_startable(document: dict[str, object]) -> bool:
    """Report whether a pull request can start this workflow.

    Parameters
    ----------
    document : dict[str, object]
        A parsed workflow document.

    Returns
    -------
    bool
        True when the document declares the ``pull_request`` trigger.
    """
    return PULL_REQUEST_TRIGGER in trigger_names(document)


def concurrency_violations(document: dict[str, object]) -> list[str]:
    """Return every way a document fails the concurrency contract.

    Parameters
    ----------
    document : dict[str, object]
        A parsed workflow document.

    Returns
    -------
    list[str]
        One message per violation; empty when the document conforms.
    """
    concurrency = document.get("concurrency")
    if concurrency is None:
        return ["declares no concurrency: block"]
    if not isinstance(concurrency, dict):
        return ["declares a concurrency: that is not a mapping"]
    return _group_violations(concurrency.get("group")) + _cancel_violations(
        concurrency.get("cancel-in-progress")
    )


def _group_violations(group: object) -> list[str]:
    """Return the violations of the concurrency group itself.

    Parameters
    ----------
    group : object
        The declared ``group`` value.

    Returns
    -------
    list[str]
        One message per violation; empty when the group conforms.
    """
    if not isinstance(group, str) or not group.strip():
        return ["declares no concurrency group"]
    if RUN_ID_EXPRESSION in group:
        return [f"keys its concurrency group on {RUN_ID_EXPRESSION}"]
    return []


def _cancel_violations(cancel: object) -> list[str]:
    """Return the violations of the ``cancel-in-progress`` setting.

    Parameters
    ----------
    cancel : object
        The declared ``cancel-in-progress`` value.

    Returns
    -------
    list[str]
        One message per violation; empty when the setting conforms.
    """
    if cancel is None:
        return ["sets no cancel-in-progress"]
    if not isinstance(cancel, str) or " ".join(cancel.split()) != CANCEL_IN_PROGRESS:
        return [f"sets cancel-in-progress to {cancel!r} and not {CANCEL_IN_PROGRESS}"]
    return []


def workflow_documents() -> dict[str, dict[str, object]]:
    """Parse every workflow in the repository.

    An unparsable or non-mapping workflow raises a
    :class:`WorkflowShapeError` subclass from :func:`_parse`.

    Returns
    -------
    dict[str, dict[str, object]]
        Each workflow document, keyed by file name.
    """
    directory = ROOT / ".github" / "workflows"
    paths = sorted(
        path for suffix in WORKFLOW_SUFFIXES for path in directory.glob(f"*{suffix}")
    )
    return {path.name: _parse(path) for path in paths}


def _parse(path: Path) -> dict[str, object]:
    """Parse one workflow file.

    Parameters
    ----------
    path : Path
        The workflow file to read.

    Returns
    -------
    dict[str, object]
        The parsed document.

    Raises
    ------
    UnparsableWorkflowError
        If the file is not parsable YAML.
    NotAMappingError
        If the document does not parse to a mapping.
    """
    try:
        document = yaml.safe_load(path.read_text(encoding="utf-8"))
    except yaml.YAMLError as error:
        raise UnparsableWorkflowError(path.name) from error
    if not isinstance(document, dict):
        raise NotAMappingError(path.name)
    return document


def pull_request_workflows() -> dict[str, dict[str, object]]:
    """Load the workflows a pull request can start.

    Returns
    -------
    dict[str, dict[str, object]]
        Each pull-request-startable workflow, keyed by file name.
    """
    return {
        name: document
        for name, document in workflow_documents().items()
        if is_pull_request_startable(document)
    }
