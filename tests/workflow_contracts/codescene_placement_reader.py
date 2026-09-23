"""Reading machinery for the CodeScene placement contract (CV-005).

This module gets facts out of a workflow tree and holds no opinion about
them; the assertions live in ``ci_codescene_placement_test``, and the
readings are proved against constructed trees in
``codescene_placement_reader_test``. Every reader takes its documents as an
argument, so a fixture tree exercises exactly the code the contract runs.

Loading is ``workflow_loader``'s, which refuses a duplicated mapping key.
Three readings here refuse rather than guess, because each guess had a
failure mode that no green run would show.

``triggers`` reads ``on:`` as a scalar, a sequence or a mapping, under both
the string key and YAML 1.1's boolean ``True``, and refuses anything else. A
mapping-only reader stringifies ``on: [push, pull_request]`` into one key
that matches no trigger, and a reader that returns nothing for an unknown
shape lets the workflow escape every pull-request clause.

Which ``uses:`` values are local calls is ``workflow_calls``'s question.

``pull_request_closure`` follows those calls transitively, because a
workflow declaring only ``workflow_call`` still runs on a pull request when
a pull-request workflow calls it, and ``secrets: inherit`` hands it the
token.
"""

from __future__ import annotations

import typing as typ

from workflow_calls import local_callee

if typ.TYPE_CHECKING:
    from collections.abc import Mapping

    from workflow_loader import Document

#: Triggers that start a workflow for a pull request, or on the way to one.
#: ``pull_request_target`` runs with write permissions, which makes it more
#: dangerous, not less; ``merge_group`` runs the checks a pull request needs
#: to leave the merge queue, so a red one blocks the merge; ``workflow_run``
#: runs after a pull-request workflow, with the repository's secrets; and a
#: review, a review comment or a comment on the pull request
#: (``pull_request_review``, ``pull_request_review_comment``,
#: ``issue_comment``) each start a workflow for it. ``workflow_dispatch`` is
#: not here: a dispatch is not a pull request.
PULL_REQUEST_TRIGGERS: typ.Final = frozenset(
    {
        "pull_request",
        "pull_request_target",
        "merge_group",
        "workflow_run",
        "pull_request_review",
        "pull_request_review_comment",
        "issue_comment",
    }
)


def triggers(document: Document) -> dict[object, object]:
    """Return a workflow's triggers as a mapping of name to configuration.

    Parameters
    ----------
    document
        One parsed workflow document.

    Returns
    -------
    dict[object, object]
        Trigger name to its configuration; a scalar or sequence form maps
        each name to ``None``.

    Raises
    ------
    ValueError
        When ``on:`` is missing, is declared under both the string key and
        YAML 1.1's boolean ``True``, or is not a scalar, sequence or mapping of
        names, since a workflow whose triggers cannot be read cannot be
        classified as outside the pull-request lane either.

    Examples
    --------
    >>> triggers({True: ["push", "pull_request"]})
    {'push': None, 'pull_request': None}
    """
    if "on" in document and True in document:
        # GitHub merges the two, so reading either alone is blind to the
        # other's triggers.
        message = "a workflow declaring both `on:` and `'on':` is refused"
        raise ValueError(message)
    for key in ("on", True):
        if key not in document:
            continue
        match document[key]:
            case dict() as mapping:
                return mapping
            case str() as event:
                return {event: None}
            case list() as events if all(
                isinstance(event, str) for event in events
            ):
                return dict.fromkeys(events)
            case other:
                message = f"unsupported `on:` shape {other!r}"
                raise ValueError(message)
    message = "a workflow with no `on:` block cannot be classified"
    raise ValueError(message)


def jobs(document: Document) -> dict[object, Document]:
    """Return a workflow's jobs, skipping any that are not mappings.

    Parameters
    ----------
    document
        One parsed workflow document.

    Returns
    -------
    dict[object, Document]
        Job id to job definition.
    """
    raw = document.get("jobs")
    if not isinstance(raw, dict):
        return {}
    return {name: job for name, job in raw.items() if isinstance(job, dict)}


def steps(document: Document) -> list[Document]:
    """Return every mapping step in every job, in document order.

    Parameters
    ----------
    document
        One parsed workflow document.

    Returns
    -------
    list[Document]
        Every step that is a mapping, across all jobs.
    """
    return [
        step
        for job in jobs(document).values()
        for step in (job.get("steps") or [])
        if isinstance(step, dict)
    ]


def calls(document: Document) -> list[Document]:
    """Return every step, plus every job that is itself a workflow call.

    A job calling a reusable workflow carries ``uses:`` on the job and has no
    steps, so a reader of steps alone misses the one shape that can run
    another repository's workflow.

    Parameters
    ----------
    document
        One parsed workflow document.

    Returns
    -------
    list[Document]
        The calling jobs first, then every step.
    """
    return [job for job in jobs(document).values() if "uses" in job] + steps(
        document
    )


def pull_request_closure(documents: Mapping[str, Document]) -> list[str]:
    """Return every workflow a pull request can reach, callees included.

    Parameters
    ----------
    documents
        Every workflow in the tree, keyed by file name.

    Returns
    -------
    list[str]
        File names of the pull-request roots and every workflow they call in
        this checkout, transitively, sorted.

    Raises
    ------
    ValueError
        When any workflow's triggers cannot be read.

    Examples
    --------
    >>> pull_request_closure({
    ...     "ci.yml": {True: "pull_request", "jobs": {
    ...         "r": {"uses": "./.github/workflows/lib.yml"}}},
    ...     "lib.yml": {True: "workflow_call", "jobs": {}},
    ... })
    ['ci.yml', 'lib.yml']
    """
    pending = [
        name
        for name, document in documents.items()
        if PULL_REQUEST_TRIGGERS & set(triggers(document))
    ]
    reached: set[str] = set()
    # A visited set rather than recursion: two reusable workflows calling
    # each other would otherwise loop, and a contract that hangs is worse
    # than one that is wrong.
    while pending:
        name = pending.pop()
        if name in reached or name not in documents:
            continue
        reached.add(name)
        pending.extend(
            callee
            for job in jobs(documents[name]).values()
            if (callee := local_callee(str(job.get("uses", ""))))
        )
    return sorted(reached)


def mentions(node: object, needle: str, *, ignore_case: bool = False) -> bool:
    """Return whether a string occurs in any key or scalar of a node.

    The whole tree is walked rather than the scopes a value is meant to live
    in, because the point of every caller is that it must not appear at all.

    Parameters
    ----------
    node
        Any node of a parsed workflow document.
    needle
        The text to look for.
    ignore_case
        Compare case-insensitively, as for a host name.

    Returns
    -------
    bool
        True when the needle occurs in any key or scalar under the node.

    Examples
    --------
    >>> mentions({"env": {"T": "${{ secrets.CS_ACCESS_TOKEN }}"}}, "CS_")
    True
    >>> mentions({"run": "curl https://API.CodeScene.io"}, "codescene.io",
    ...          ignore_case=True)
    True
    """
    if isinstance(node, dict):
        return any(
            mentions(key, needle, ignore_case=ignore_case)
            or mentions(value, needle, ignore_case=ignore_case)
            for key, value in node.items()
        )
    if isinstance(node, list):
        return any(
            mentions(item, needle, ignore_case=ignore_case) for item in node
        )
    text = str(node)
    return needle.lower() in text.lower() if ignore_case else needle in text


def external_secret_inheritors(document: Document) -> list[str]:
    """Return jobs that hand every secret to a workflow outside this checkout.

    ``secrets: inherit`` to a local callee is visible, because the callee is
    in the closure and read like any other workflow. The same line pointed at
    another repository, or at this one by ``@ref``, forwards the token to a
    document this contract cannot read, without naming it anywhere a sweep
    for the name could find.

    Parameters
    ----------
    document
        One parsed workflow document.

    Returns
    -------
    list[str]
        Ids of the jobs that inherit secrets into an unreadable workflow.
    """
    return [
        str(name)
        for name, job in jobs(document).items()
        if job.get("secrets") == "inherit"
        and "uses" in job
        and local_callee(str(job["uses"])) is None
    ]
