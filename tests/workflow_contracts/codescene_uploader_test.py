"""Contract tests for the deprecated CodeScene uploader checksum inputs.

The shared uploader's committed ``cli-manifest.json`` is the trust anchor for
the CodeScene CLI archive. From shared-actions ``f68e8e2e`` the action
*rejects* a non-empty ``installer-checksum`` with a hard failure rather than
ignoring it, and ``archive-checksum`` can only ever repeat the digest the
manifest already carries. A workflow still passing the old input therefore
breaks its upload step the moment the repository variable feeding it holds a
value, and the variable itself has no consumer left.

Four concerns are asserted, each in its own test so that a failure names the
defect rather than a bundle:

* no workflow passes ``installer-checksum``;
* no workflow reads or refreshes ``CODESCENE_CLI_SHA256``;
* every uploader reference is pinned to the one approved full SHA;
* the dispatch workflow that refreshed the variable does not exist.

Each assertion ranges over a set that is checked for content first. A rule
over an empty collection is satisfied by deleting the thing it guards, so
the workflow set and the uploader reference set are both proved non-empty
before they are proved compliant.
"""

from __future__ import annotations

import typing as typ
from pathlib import Path

REPOSITORY_ROOT: typ.Final = Path(__file__).resolve().parents[2]
WORKFLOW_DIRECTORY: typ.Final = REPOSITORY_ROOT / ".github" / "workflows"

#: The uploader revision this repository trusts. Named as an allowlist rather
#: than matched as "any full SHA": the pin is the manifest, so a different
#: revision is a different trust anchor whatever the shape of its identifier.
UPLOADER_PIN: typ.Final = "a5765019912a8ab6882b12db049c7cde635f3a85"
UPLOADER_ACTION: typ.Final = (
    "leynos/shared-actions/.github/actions/upload-codescene-coverage"
)
DEPRECATED_INPUT: typ.Final = "installer-checksum"
#: The repository variable that held the CodeScene installer script's digest.
DEPRECATED_DIGEST_VARIABLE: typ.Final = "CODESCENE_CLI_SHA256"
#: The dispatch workflow whose only output was that variable.
DIGEST_REFRESH_WORKFLOW: typ.Final = "get-codescene-sha.yml"


def _workflow_texts() -> dict[str, str]:
    """Return every workflow file's raw text, keyed by file name.

    Read as text rather than parsed. The strings refused below read the same
    in a comment as in a value, and a comment carrying one is an instruction
    to a later reader to reintroduce the other. Both extensions are read,
    because GitHub runs a workflow written either way.

    Returns
    -------
    dict[str, str]
        Workflow file name to file contents.
    """
    texts = {
        path.name: path.read_text(encoding="utf-8")
        for path in sorted(WORKFLOW_DIRECTORY.iterdir())
        if path.suffix in {".yml", ".yaml"}
    }
    assert texts, (
        "no workflow files were read, so every assertion over them would hold vacuously"
    )
    return texts


def test_no_workflow_passes_the_deprecated_installer_checksum() -> None:
    """Refuse the input the uploader rejects outright.

    A non-empty value exits the action with a hard failure. Trunk stayed
    green only because the repository variable feeding it happened to be
    empty, so the defect was latent rather than absent and fires the moment
    the variable is set.
    """
    offending = sorted(
        name for name, text in _workflow_texts().items() if DEPRECATED_INPUT in text
    )

    assert not offending, (
        f"{DEPRECATED_INPUT} is rejected when non-empty; pass nothing, because "
        "the uploader's committed manifest is the trust anchor and "
        f"archive-checksum can only repeat its digest: {offending}"
    )


def test_no_workflow_reads_the_deprecated_digest_variable() -> None:
    """Leave no workflow reading or refreshing a value nothing consumes.

    ``installer-checksum`` was the variable's only consumer. A workflow still
    reading it feeds a rejected input, which fails only when the uploader
    runs; one still refreshing it maintains dead state, which never fails at
    all and so is invisible without this rule.
    """
    offending = sorted(
        name
        for name, text in _workflow_texts().items()
        if DEPRECATED_DIGEST_VARIABLE in text
    )

    assert not offending, (
        f"no workflow may read or refresh {DEPRECATED_DIGEST_VARIABLE}; the "
        f"uploader pins the CLI through its own manifest: {offending}"
    )


def test_the_digest_refresh_workflow_is_absent() -> None:
    """Keep the dispatch that maintained the dead variable out of the tree."""
    assert not (WORKFLOW_DIRECTORY / DIGEST_REFRESH_WORKFLOW).exists(), (
        f"{DIGEST_REFRESH_WORKFLOW} refreshes a variable nothing reads; it "
        "must not exist"
    )


def test_every_uploader_call_is_on_the_approved_revision() -> None:
    """Hold every uploader call to the revision carrying the manifest.

    A full-SHA rule alone is satisfied by any revision, including the ones
    whose unpinned ``cs-coverage`` could not parse its own coverage report.
    The approved pin is therefore named, and the reference set is proved
    non-empty first so that deleting the calls cannot satisfy the rule.
    """
    references = sorted(
        f"{name}:{line.strip()}"
        for name, text in _workflow_texts().items()
        for line in text.splitlines()
        if f"{UPLOADER_ACTION}@" in line
    )

    assert references, "this repository must call the CodeScene uploader"
    wrong = [
        reference
        for reference in references
        if f"{UPLOADER_ACTION}@{UPLOADER_PIN}" not in reference
    ]
    assert not wrong, f"every uploader call must be pinned to {UPLOADER_PIN}: {wrong}"
