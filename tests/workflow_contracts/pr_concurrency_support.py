"""Readers for the pull-request concurrency contract.

A superseded pull-request run costs the same minutes as the run that
replaced it. GitHub cancels one only when the workflow declares a
concurrency group and asks for it, so the fact is a property of every
workflow a pull request can start, not of any one job.

Loading is ``workflow_loader``'s, which refuses a duplicated mapping key,
and trigger reading is ``codescene_placement_reader.triggers``, which reads
every shape GitHub accepts under both spellings of ``on:``. Neither is
repeated here.

The readers here are pure: :func:`is_pull_request_startable` and
:func:`concurrency_violations` take a parsed document, so the contract
can drive them with a synthetic workflow the repository does not
contain. A rule exercised only over files that already satisfy it
passes whether or not it works.
"""

from __future__ import annotations

import typing as typ

from codescene_placement_reader import triggers
from workflow_loader import repository_workflows

if typ.TYPE_CHECKING:
    from workflow_loader import Document

#: The trigger a pull request starts. ``pull_request_target`` runs with
#: the base repository's token; the workflows on it here push commits
#: and merge, and cancelling one mid-write is not a saving.
PULL_REQUEST_TRIGGER: typ.Final = "pull_request"

#: The only accepted concurrency group. A pull request shares a group with
#: its own later pushes; every other event falls back to its run id, so a
#: push to main or a dispatch is never replaced while it waits. A fallback
#: of ``github.ref`` would put every push to main in one group, where a
#: third push replaces a pending second run that was meant to complete.
GROUP: typ.Final = (
    "${{ github.workflow }}-${{ github.event.pull_request.number || github.run_id }}"
)

#: The only accepted ``cancel-in-progress`` value. A literal ``true``
#: would also cancel a push to main or a dispatch that shared a group,
#: so the contract requires the guarded expression, not a truthy setting.
CANCEL_IN_PROGRESS: typ.Final = "${{ github.event_name == 'pull_request' }}"


def is_pull_request_startable(document: Document) -> bool:
    """Report whether a pull request can start this workflow.

    A workflow whose triggers cannot be read raises ``ValueError`` from
    ``triggers`` rather than being classified as out of scope.

    Parameters
    ----------
    document : Document
        A parsed workflow document.

    Returns
    -------
    bool
        True when the document declares the ``pull_request`` trigger.

    Examples
    --------
    >>> is_pull_request_startable({True: ["push", "pull_request"]})
    True
    """
    return PULL_REQUEST_TRIGGER in triggers(document)


def concurrency_violations(document: Document) -> list[str]:
    """Return every way a document fails the concurrency contract.

    Parameters
    ----------
    document : Document
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


def _normalized(value: str) -> str:
    """Collapse runs of whitespace, so a folded scalar compares equal."""
    return " ".join(value.split())


def _group_violations(group: object) -> list[str]:
    """Return the violations of the concurrency group itself.

    The group is compared whole. ``github.run_id`` is accepted only as the
    fallback after the pull-request number: as the whole key it makes every
    run unique, so nothing is ever cancelled, and ahead of the number it
    would win for a pull request too.

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
    if _normalized(group) != GROUP:
        return [f"sets the concurrency group to {group!r} and not {GROUP}"]
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
    if not isinstance(cancel, str) or _normalized(cancel) != CANCEL_IN_PROGRESS:
        return [f"sets cancel-in-progress to {cancel!r} and not {CANCEL_IN_PROGRESS}"]
    return []


def pull_request_workflows() -> dict[str, Document]:
    """Load the workflows a pull request can start.

    Returns
    -------
    dict[str, Document]
        Each pull-request-startable workflow, keyed by file name.
    """
    return {
        name: document
        for name, document in repository_workflows().items()
        if is_pull_request_startable(document)
    }
