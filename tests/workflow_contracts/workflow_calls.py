"""Classify a ``uses:`` value as a call into this repository's workflows.

Two questions, kept apart because the contract answers them differently.
``local_callee`` says which checked-out workflow file a call runs, so the
pull-request closure can follow it. ``qualified_self_call`` says whether a
call names this repository's workflows at an ``@ref``: GitHub runs that at
the named ref, not at the pull request's head, so the closure cannot read
what runs and the contract refuses the form. Both match by shape: a prefix
is stripped and the remainder must be a file directly under
``.github/workflows/``.
"""

from __future__ import annotations

import typing as typ
from pathlib import PurePosixPath

if typ.TYPE_CHECKING:
    from workflow_loader import Document

#: This repository as a fully qualified ``uses:`` value names it, lowercased.
REPOSITORY: typ.Final = "leynos/wireframe"

#: Where this repository's workflows live, as a ``uses:`` value names them.
WORKFLOW_PREFIX: typ.Final = ".github/workflows/"

#: Prefixes that, once stripped, leave a path inside this checkout. ``./``
#: is the documented local form; ``$/`` is accepted because over-reading a
#: call only widens the closure the prohibitions run over, while
#: under-reading one hides a workflow from them. The qualified
#: ``owner/repo/...@ref`` form is not here: it runs the file at the named
#: ref, which this checkout does not hold, so it is refused, not followed.
LOCAL_PREFIXES: typ.Final = ("./", "$/")


def _workflow_file(path: str) -> str | None:
    """Return the file name when a path names a workflow file directly."""
    workflow_path = PurePosixPath(path)
    if workflow_path.as_posix() != path:
        return None
    if workflow_path.parent != PurePosixPath(WORKFLOW_PREFIX):
        return None
    return workflow_path.name


def local_callee(uses: str) -> str | None:
    """Return the workflow in this checkout a ``uses:`` value runs.

    Parameters
    ----------
    uses
        A job's or step's ``uses:`` value.

    Returns
    -------
    str | None
        The file name under ``.github/workflows/`` when the call runs that
        file as checked out, or ``None`` for anything else, including this
        repository's own workflows called by ``owner/repo`` and ``@ref``.

    Examples
    --------
    >>> local_callee("./.github/workflows/release.yml")
    'release.yml'
    >>> local_callee(f"{REPOSITORY}/.github/workflows/x.yml@main") is None
    True
    >>> local_callee("./.github/actions/setup") is None
    True
    """
    candidate = uses.strip()
    if "@" in candidate:
        return None
    for prefix in LOCAL_PREFIXES:
        if candidate.startswith(prefix):
            candidate = candidate[len(prefix) :]
            break
    return _workflow_file(candidate)


def qualified_self_call(uses: str) -> bool:
    """Return whether a ``uses:`` value calls this repository at a ref.

    GitHub runs such a call at the named ref, not at the pull request's
    head, so the file in this checkout is not the file that runs and the
    closure cannot read it. The contract refuses the form outright, whether
    it is written with this repository's ``owner/repo`` or with a local
    ``./`` or ``$/`` prefix and an ``@ref``.

    Parameters
    ----------
    uses
        A job's ``uses:`` value.

    Returns
    -------
    bool
        True when the value names a file under this repository's
        ``.github/workflows/`` and carries an ``@ref``; the repository is
        compared case-insensitively.

    Examples
    --------
    >>> qualified_self_call(f"{REPOSITORY}/.github/workflows/x.yml@main")
    True
    >>> qualified_self_call("$/.github/workflows/x.yml@main")
    True
    >>> qualified_self_call("./.github/workflows/x.yml")
    False
    """
    candidate, _, ref = uses.strip().partition("@")
    if not ref:
        return False
    for prefix in (*LOCAL_PREFIXES, f"{REPOSITORY}/"):
        if candidate.lower().startswith(prefix):
            return _workflow_file(candidate[len(prefix) :]) is not None
    return False


def qualified_self_callers(document: Document) -> list[str]:
    """Return jobs that call this repository's workflows by ``@ref``.

    Parameters
    ----------
    document
        One parsed workflow document.

    Returns
    -------
    list[str]
        Ids of the jobs whose ``uses:`` is a qualified same-repository call.
    """
    return [
        str(name)
        for name, job in (document.get("jobs") or {}).items()
        if isinstance(job, dict)
        and qualified_self_call(str(job.get("uses", "")))
    ]
