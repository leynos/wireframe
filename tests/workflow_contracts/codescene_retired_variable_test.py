"""The CodeScene CLI digest variable is retired; no workflow may name it.

Split from ``ci_codescene_placement_test`` to keep that module under the
400-line limit. The two share the strict loader, not the placement rule:
this one reads every workflow, not the pull-request closure.
"""

from __future__ import annotations

import typing as typ
from pathlib import Path

from workflow_loader import read_workflows

WORKFLOW_DIR: typ.Final = (
    Path(__file__).resolve().parents[2] / ".github" / "workflows"
)

#: The repository variable that fed the uploader's old ``installer-checksum``
#: input. Nothing may read or refresh it: the uploader takes its digest from a
#: committed manifest, and a stale variable is a value nobody maintains.
RETIRED_VARIABLE: typ.Final = "CODESCENE_CLI_SHA256"


def test_nothing_reads_or_refreshes_the_retired_variable() -> None:
    """Scenario: the CodeScene CLI digest variable outlives its reader.

    Invariant: no workflow in the repository mentions
    ``CODESCENE_CLI_SHA256``, at any scope, in a step input or in a script.

    The variable existed to feed the uploader's ``installer-checksum`` input.
    The uploader now takes its digest from a committed manifest, so the input
    is gone and the variable feeds nothing. A workflow that refreshes it
    spends a runner maintaining a value nobody reads, and one that passes it
    hands the uploader an input it refuses.

    Every workflow is read, not only the pull-request closure: a refresher on
    a schedule or a dispatch is exactly the shape this is meant to catch, and
    neither is reachable from a pull request.
    """
    offenders = sorted(
        name
        for name, document in read_workflows(WORKFLOW_DIR).items()
        if RETIRED_VARIABLE in str(document)
    )
    assert not offenders, (
        f"{RETIRED_VARIABLE} is retired and feeds nothing, but "
        f"{offenders} still mention it. The repository variable itself can "
        "be deleted once no workflow names it."
    )
