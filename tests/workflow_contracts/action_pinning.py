"""Readers for the workflow action-pinning contract.

CodeQL's default-setup ``actions/unpinned-tag`` query flags a third-party
action or reusable workflow whose ref is not a full commit hash. A mutable tag
can be repointed after review, so the code that runs need not be the code that
was reviewed, and a workflow dispatch lane runs it with a writable token.

The policy reproduced here is the rule's, for the shapes this repository uses.
A 40- or 64-character hexadecimal ref is a commit; ``actions``, ``github`` and
``advanced-security`` are trusted owners; a ``./`` local action or a ``$/``
same-repository reference is inherently pinned; and a ``docker://`` image must
carry a ``sha256:`` digest. Anything else is a violation. Action lock files and
GitHub immutable releases also satisfy the rule, but this repository uses
neither, so the contract does not model them.

Loading is ``workflow_loader``'s, which refuses a duplicated mapping key. The
readers take parsed documents, so the contract can drive them with synthetic
workflows the repository does not contain.
"""

from __future__ import annotations

import dataclasses
import re
import typing as typ

from workflow_loader import repository_workflows

if typ.TYPE_CHECKING:
    from collections.abc import Mapping

    from workflow_loader import Document

#: Owners whose action and reusable-workflow refs the CodeQL rule trusts to be
#: mutable, recorded in its ``trustedActionsOwnerDataModel``.
TRUSTED_OWNERS: typ.Final = frozenset({"actions", "github", "advanced-security"})

#: A full commit hash. Repositories using the SHA-256 object format have 64
#: hexadecimal characters; SHA-1 repositories have 40.
COMMIT_REF_RE: typ.Final = re.compile(r"^[A-Fa-f0-9]{40}(?:[A-Fa-f0-9]{24})?$")

#: A container image pinned by digest rather than by a mutable tag.
CONTAINER_DIGEST_RE: typ.Final = re.compile(r"^sha256:[A-Fa-f0-9]{64}$")

#: A local (``./``) or same-repository (``$/``) reference is resolved at the
#: calling workflow's own commit, so it can never be repointed.
SELF_REFERENCE_PREFIXES: typ.Final = ("./", "$/")


@dataclasses.dataclass(frozen=True)
class ActionReference:
    """One ``uses:`` reference and where the workflow declares it.

    Attributes
    ----------
    workflow
        The workflow file name, such as ``ci.yml``.
    job
        The job identifier holding the reference.
    step
        The step's ``name`` or its one-based position when it is unnamed;
        ``"(job)"`` for a reusable workflow the job itself calls.
    uses
        The ``uses:`` value exactly as declared.
    """

    workflow: str
    job: str
    step: str
    uses: str

    def describe(self) -> str:
        """Return a location a reviewer can find in the workflow file.

        Returns
        -------
        str
            The workflow, job, step and reference, separated by spaces.
        """
        return (
            f"{self.workflow}: job={self.job} step={self.step} "
            f"uses={self.uses}"
        )


def action_references(workflows: Mapping[str, Document]) -> list[ActionReference]:
    """Return every ``uses:`` reference in the given workflows.

    Both positions GitHub accepts are read: a job-level ``uses:`` calls a
    reusable workflow, and a step-level ``uses:`` calls an action.

    Parameters
    ----------
    workflows
        Parsed workflow documents, keyed by file name.

    Returns
    -------
    list[ActionReference]
        The references, in workflow, job and step order.

    Examples
    --------
    >>> action_references(
    ...     {"x.yml": {"jobs": {"build": {"steps": [{"uses": "Owner/action@v1"}]}}}}
    ... )
    [ActionReference(workflow='x.yml', job='build', step='step 1', uses='Owner/action@v1')]
    """
    return [
        reference
        for workflow_name, document in workflows.items()
        for reference in _document_references(workflow_name, document)
    ]


def _document_references(
    workflow_name: str, document: Document
) -> list[ActionReference]:
    """Return the references declared by one workflow document."""
    jobs = document.get("jobs")
    if not isinstance(jobs, dict):
        return []
    references: list[ActionReference] = []
    for job_name, job in jobs.items():
        if not isinstance(job, dict):
            continue
        job_uses = job.get("uses")
        if isinstance(job_uses, str):
            references.append(
                ActionReference(workflow_name, str(job_name), "(job)", job_uses)
            )
        references.extend(_step_references(workflow_name, str(job_name), job))
    return references


def _step_references(
    workflow_name: str, job_name: str, job: Mapping[object, object]
) -> list[ActionReference]:
    """Return the references declared by one job's steps."""
    steps = job.get("steps")
    if not isinstance(steps, list):
        return []
    references: list[ActionReference] = []
    for position, step in enumerate(steps, start=1):
        if not isinstance(step, dict):
            continue
        uses = step.get("uses")
        if not isinstance(uses, str):
            continue
        name = step.get("name")
        label = name if isinstance(name, str) else f"step {position}"
        references.append(ActionReference(workflow_name, job_name, label, uses))
    return references


def pinning_problem(uses: str) -> str | None:
    """Return why a ``uses:`` reference is unpinned, or ``None`` when it is pinned.

    Parameters
    ----------
    uses
        A workflow's ``uses:`` value.

    Returns
    -------
    str | None
        A reviewer-facing problem, or ``None`` when the reference conforms.

    Examples
    --------
    >>> pinning_problem("actions/checkout@v4") is None
    True
    >>> pinning_problem("thollander/actions-comment-pull-request@v3") is not None
    True
    """
    reference = uses.strip()
    if reference.startswith(SELF_REFERENCE_PREFIXES):
        return None
    if reference.startswith("docker://"):
        return _container_problem(reference)
    callee, separator, ref = reference.rpartition("@")
    if not _has_ref(callee, separator, ref):
        return f"{reference!r} declares no ref"
    owner = callee.split("/", 1)[0]
    if owner in TRUSTED_OWNERS or COMMIT_REF_RE.fullmatch(ref):
        return None
    return (
        f"{reference!r} uses the mutable ref {ref!r} on the untrusted owner "
        f"{owner!r}; pin it to a full commit hash"
    )


def _has_ref(callee: str, separator: str, ref: str) -> bool:
    """Report whether a ``uses:`` value names both a callee and a ref."""
    return bool(separator and callee and ref)


def _container_problem(reference: str) -> str | None:
    """Return why a ``docker://`` reference is unpinned, or ``None``."""
    _, separator, digest = reference.rpartition("@")
    if separator and CONTAINER_DIGEST_RE.fullmatch(digest):
        return None
    return f"{reference!r} must be pinned by a sha256 image digest"


def unpinned_references(
    workflows: Mapping[str, Document],
) -> list[ActionReference]:
    """Return every reference in the given workflows that is not pinned.

    Parameters
    ----------
    workflows
        Parsed workflow documents, keyed by file name.

    Returns
    -------
    list[ActionReference]
        The offending references, each with its workflow location.
    """
    return [
        reference
        for reference in action_references(workflows)
        if pinning_problem(reference.uses) is not None
    ]


def repository_unpinned_references() -> list[ActionReference]:
    """Return every unpinned reference in this repository's workflows.

    Returns
    -------
    list[ActionReference]
        The offending references, each with its workflow location.
    """
    return unpinned_references(repository_workflows())