"""Hold the CodeScene token's environment to the uploading job (CV-005).

The token lives in the `codescene` environment, whose deployment policy admits
`main` alone. So every job that invokes the uploader declares that
environment, no other job does, and no workflow a pull request can start
declares it in any job: a declaration there would let branch code ask for the
token.
"""

from __future__ import annotations

import typing as typ

from codescene_placement_reader import jobs, pull_request_closure

if typ.TYPE_CHECKING:
    import collections.abc as cabc

    from workflow_loader import Document

ENVIRONMENT: typ.Final[str] = "codescene"
UPLOAD_ACTION: typ.Final[str] = "upload-codescene-coverage"
MISSING: typ.Final[str] = f"the uploading job must declare `environment: {ENVIRONMENT}`"
STRAY: typ.Final[str] = f"declares `{ENVIRONMENT}` but uploads nothing"
REACHABLE: typ.Final[str] = (
    f"is reachable from a pull request and declares `{ENVIRONMENT}`"
)


def environment_name(job: Document) -> str | None:
    """Return the environment a job declares, from either accepted form.

    Parameters
    ----------
    job : dict[str, object]
        The job to read.

    Returns
    -------
    str | None
        The environment's name, or None when the job declares none.

    Examples
    --------
    >>> environment_name({"environment": "codescene"})
    'codescene'
    >>> environment_name({"environment": {"name": "codescene", "url": "x"}})
    'codescene'
    >>> environment_name({}) is None
    True

    """
    match job.get("environment"):
        case str() as name:
            return name
        case {"name": str() as name}:
            return name
        case _:
            return None


def uploads(job: Document) -> bool:
    """Return whether a job has a step invoking the uploader.

    Parameters
    ----------
    job : dict[str, object]
        The job to read.

    Returns
    -------
    bool
        True when some step's `uses` names the upload action.

    """
    declared = job.get("steps") or []
    return any(
        isinstance(step, dict) and UPLOAD_ACTION in str(step.get("uses", ""))
        for step in declared
    )


def _placed(
    documents: dict[str, Document], names: cabc.Iterable[str]
) -> list[tuple[str, Document]]:
    """Return every job in the named workflows with its location.

    Parameters
    ----------
    documents : dict[str, Document]
        Every parsed workflow, by file name.
    names : Iterable[str]
        The workflows to read.

    Returns
    -------
    list[tuple[str, Document]]
        `"workflow:job"` and the job, for each job.

    """
    return [
        (f"{name}:{job_id}", job)
        for name in sorted(names)
        for job_id, job in jobs(documents[name]).items()
    ]


def environment_violations(documents: dict[str, Document]) -> list[str]:
    """Report every departure from the `codescene` environment placement.

    Parameters
    ----------
    documents : dict[str, Document]
        Every parsed workflow, by file name.

    Returns
    -------
    list[str]
        One message per violation; empty when the placement holds.

    """
    placed = _placed(documents, documents)
    uploading = [(where, job) for where, job in placed if uploads(job)]
    if not uploading:
        return ["no workflow job invokes the CodeScene uploader"]
    problems = [
        f"{where}: {MISSING}"
        for where, job in uploading
        if environment_name(job) != ENVIRONMENT
    ]
    problems.extend(
        f"{where} {STRAY}"
        for where, job in placed
        if not uploads(job) and environment_name(job) == ENVIRONMENT
    )
    problems.extend(
        f"{where} {REACHABLE}"
        for where, job in _placed(documents, pull_request_closure(documents))
        if environment_name(job) == ENVIRONMENT
    )
    return problems
