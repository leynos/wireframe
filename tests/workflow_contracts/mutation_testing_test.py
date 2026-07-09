"""Contract tests for the mutation-testing caller workflow.

The executable logic lives in the ``leynos/shared-actions`` reusable
workflow, which carries its own unit and integration tests; wireframe's
caller is declarative configuration. These tests parse the caller with
PyYAML and pin the contract it must uphold, so drift (repointing the pin
at a branch, widening permissions, or losing the issue-571
configuration) fails CI on the pull request rather than surfacing in a
scheduled or manual run.

Run via ``make test-workflow-contracts``.
"""

from __future__ import annotations

from pathlib import Path

import yaml

WORKFLOW_PATH = (
    Path(__file__).resolve().parents[2] / ".github" / "workflows" / "mutation-testing.yml"
)

#: The repo-wide leynos/shared-actions pin. Originally documented in
#: docs/execplans/adopt-shared-mutation-workflow.md (the merge commit of
#: leynos/shared-actions PR #319); bumped to the single estate-wide pin
#: that carries the CodeScene coverage `mode: check` gate
#: (leynos/shared-actions PR #334). Every shared-actions reference in the
#: repo shares this SHA; bump them together.
PINNED_SHA = "927edd45ae77be4251a8a18ca9eb5613a2e32cbd"

EXPECTED_USES = (
    "leynos/shared-actions/.github/workflows/mutation-cargo.yml@" + PINNED_SHA
)

SCAFFOLDING_EXCLUDES = (
    "src/test_helpers.rs",
    "src/connection/test_support.rs",
    "src/codec/examples.rs",
)


def _load() -> dict[str, object]:
    """Parse the workflow file."""
    return yaml.safe_load(WORKFLOW_PATH.read_text(encoding="utf-8"))


def _triggers(workflow: dict[str, object]) -> dict[str, object]:
    """Return the ``on:`` mapping (PyYAML parses the bare key as True)."""
    triggers = workflow.get("on", workflow.get(True))
    assert isinstance(triggers, dict), "the workflow must declare an on: mapping"
    return triggers


def _mutation_job(workflow: dict[str, object]) -> dict[str, object]:
    """Return the single calling job."""
    jobs = workflow.get("jobs")
    assert isinstance(jobs, dict), "the workflow must declare a jobs mapping"
    assert jobs, "the workflow must declare at least one job"
    assert list(jobs) == ["mutation"], (
        f"expected a single job named 'mutation', found {sorted(jobs)}"
    )
    return jobs["mutation"]


def test_uses_reference_is_pinned_to_the_documented_sha() -> None:
    """The job must call the shared workflow at the exact documented SHA."""
    uses = _mutation_job(_load()).get("uses")
    assert uses is not None, "jobs.mutation.uses is missing"
    path, _, ref = uses.partition("@")
    assert path == "leynos/shared-actions/.github/workflows/mutation-cargo.yml", (
        f"jobs.mutation.uses must reference mutation-cargo.yml, got {path!r}"
    )
    assert len(ref) == 40, (
        f"jobs.mutation.uses must pin a full 40-character commit SHA, "
        f"not a branch or tag: {ref!r}"
    )
    assert all(c in "0123456789abcdef" for c in ref), (
        f"jobs.mutation.uses must pin a lowercase hex commit SHA, "
        f"not a branch or tag: {ref!r}"
    )
    assert uses == EXPECTED_USES, (
        f"jobs.mutation.uses pins {ref!r}; the execution plan documents {PINNED_SHA!r} — "
        "bump the plan and this test together with the workflow"
    )


def test_job_permissions_are_exactly_least_privilege() -> None:
    """The job grants contents: read and id-token: write, nothing broader."""
    permissions = _mutation_job(_load()).get("permissions")
    assert permissions == {"contents": "read", "id-token": "write"}, (
        "jobs.mutation.permissions must be exactly "
        f"{{'contents': 'read', 'id-token': 'write'}}, got {permissions!r}"
    )


def test_workflow_default_permissions_are_empty() -> None:
    """The workflow-level default token scope is empty."""
    workflow = _load()
    assert workflow.get("permissions") == {}, (
        f"top-level permissions must be an empty mapping, got "
        f"{workflow.get('permissions')!r}"
    )


def test_concurrency_serializes_per_ref_without_cancelling() -> None:
    """Runs queue per ref instead of cancelling one another."""
    concurrency = _load().get("concurrency")
    assert isinstance(concurrency, dict), "the workflow must declare concurrency"
    assert concurrency.get("group") == "mutation-testing-${{ github.ref }}", (
        f"concurrency.group must key on the triggering ref, got "
        f"{concurrency.get('group')!r}"
    )
    assert concurrency.get("cancel-in-progress") is False, (
        f"concurrency.cancel-in-progress must be false, got "
        f"{concurrency.get('cancel-in-progress')!r}"
    )


def test_triggers_keep_schedule_and_plain_dispatch() -> None:
    """The daily schedule stays; dispatch has no legacy branch input."""
    triggers = _triggers(_load())
    schedule = triggers.get("schedule")
    assert schedule == [{"cron": "30 4 * * *"}], (
        f"on.schedule must be the daily 04:30 UTC cron, got {schedule!r}"
    )
    assert "workflow_dispatch" in triggers, "on.workflow_dispatch is missing"
    dispatch = triggers.get("workflow_dispatch") or {}
    inputs = dispatch.get("inputs") or {}
    assert "branch" not in inputs, (
        "on.workflow_dispatch must not declare a branch input; the Actions "
        "run-workflow control selects the ref"
    )


def test_with_block_carries_the_caller_configuration() -> None:
    """The caller passes the companion crate, excludes, and feature args."""
    with_block = _mutation_job(_load()).get("with")
    assert isinstance(with_block, dict), "jobs.mutation.with is missing"
    assert "extra-crate-dirs" not in with_block, (
        "the wireframe_testing target stays disabled until its doctests "
        "compile (#578); restore the extra-crate-dirs assertion when "
        "re-enabling it"
    )
    assert with_block.get("shard-count") == 8, (
        f"with.shard-count must be 8 (Stage E: six shards lost one leg "
        f"to the job ceiling), got {with_block.get('shard-count')!r}"
    )
    assert with_block.get("extra-args") == "--all-features", (
        f"with.extra-args must be '--all-features' so feature-gated tests run "
        f"(#571), got {with_block.get('extra-args')!r}"
    )
    excludes = with_block.get("exclude-globs")
    assert isinstance(excludes, str), "with.exclude-globs is missing"
    for scaffold in SCAFFOLDING_EXCLUDES:
        assert scaffold in excludes.split(","), (
            f"with.exclude-globs must cover {scaffold} (#571), got {excludes!r}"
        )
