"""Contract tests for the Whitaker installer cache key.

Separate from ``ci_runner_placement_test`` because it asks a different
question. Placement is about where a lane runs; this is about what a lane is
allowed to restore once it gets there. They travel together only because
moving the lane to a second runner environment is what made the key wrong.

The failure this guards against is not a cache miss. A miss is cheap and
self-correcting. The failure is a *hit* that should have been a miss: the
lane restores a compiled ``whitaker-installer`` built against one image and
runs it on another. ``whitaker-installer`` requires glibc 2.39, so a binary
from a newer image fails on an older one at run time, inside the Install
Whitaker step, with a loader error rather than anything naming the cache.
Nothing in the restore reports it, because from `actions/cache`'s point of
view the restore succeeded.

Both environments are Ubuntu 24.04 today, so the key would not collide yet.
It carries ``runner.environment`` now because the lane gained a second
environment in the same change, and the moment those images diverge the
correct behaviour has to already be in place: by the time the divergence is
visible, the poisoned entry is already cached.

Mutation proof; each applied alone and reverted:

- dropping the ``${{ runner.environment }}`` segment fails
  ``test_the_whitaker_cache_key_is_exactly_the_reviewed_one``, which is the
  regression this module exists for;
- dropping the ``${{ env.WHITAKER_INSTALLER_VERSION }}`` segment fails the
  same test, which is what makes the check a whole-key comparison rather
  than a search for one segment;
- renaming the step fails ``test_the_reviewed_cache_step_exists_exactly_once``
  rather than silently checking nothing, which is the failure a
  ``next(... , None)`` lookup would have produced;
- pointing the step's ``uses`` at a mutable ref fails
  ``test_the_cache_step_is_pinned_to_a_commit``;
- deleting ``WHITAKER_INSTALLER_VERSION`` from the job's environment fails
  ``test_the_cache_key_reads_a_version_the_job_actually_sets`` alone. Nothing
  else notices, because the key stays well formed: the segment interpolates
  to the empty string and every installer version quietly shares one entry.

The third of those fails three tests rather than one, and that is the
intended shape: a renamed step makes every assertion here unable to find its
subject, and they say so instead of passing.

Run via ``make test-workflow-contracts``.
"""

from __future__ import annotations

import re
import typing as typ

import pytest
from runner_placement_policy import WHITAKER_CACHE_KEY, WHITAKER_CACHE_STEP
from runner_placement_reader import WORKFLOW_DIR, jobs, step_named

#: The lane that installs Whitaker, and so the lane that caches it.
WHITAKER_JOB: typ.Final = ("ci.yml", "build-test")

#: The environment variable the key's version segment reads.
VERSION_VARIABLE: typ.Final = "WHITAKER_INSTALLER_VERSION"

#: A third-party action reference pinned to a full commit SHA.
PINNED_USES: typ.Final = re.compile(r"^[\w.-]+/[\w.-]+@[0-9a-f]{40}$")

pytestmark = pytest.mark.skipif(
    not WORKFLOW_DIR.is_dir(),
    reason="workflow files not present in this working copy",
)


def test_the_reviewed_cache_step_exists_exactly_once() -> None:
    """Scenario: the cache step is renamed, removed, or duplicated.

    Invariant: exactly one step carries the reviewed name. Every other
    assertion here finds the step by that name, so a rename would leave them
    checking nothing at all while still passing, which is the quiet way a
    contract stops being a contract.
    """
    step = step_named(WHITAKER_JOB, WHITAKER_CACHE_STEP)
    assert "with" in step, (
        f"the {WHITAKER_CACHE_STEP!r} step should configure a cache, but "
        "declares no with: block"
    )


def test_the_whitaker_cache_key_is_exactly_the_reviewed_one() -> None:
    """Scenario: a segment is dropped from the installer cache key.

    Invariant: the key equals the reviewed string, segment for segment.

    Equality rather than containment, because every segment is load-bearing
    and a containment check only ever notices the one segment it was written
    to look for. ``runner.environment`` separates the Ubicloud and
    GitHub-hosted runs of this lane, whose images may diverge in glibc;
    ``runner.arch`` stops an arm64 lane restoring an amd64 binary; and the
    version segment is what stops a bumped installer restoring its
    predecessor.

    The consequence of getting this wrong is a successful restore of a binary
    that cannot run, which surfaces two steps later as a loader error naming
    nothing to do with caching.
    """
    step = step_named(WHITAKER_JOB, WHITAKER_CACHE_STEP)
    declared = (step.get("with") or {}).get("key")
    assert declared == WHITAKER_CACHE_KEY, (
        "the Whitaker installer cache key must be exactly the reviewed key\n"
        f"  expected: {WHITAKER_CACHE_KEY}\n"
        f"  found:    {declared}"
    )


def test_the_cache_key_reads_a_version_the_job_actually_sets() -> None:
    """Scenario: the key reads a version variable the job does not define.

    Invariant: the job declares ``WHITAKER_INSTALLER_VERSION``. An
    unset variable interpolates to the empty string rather than failing, so
    the key would still be well formed and every lane would share one entry
    across every installer version. The key and the environment are pinned in
    separate places, so nothing but this holds them together.
    """
    assert f"${{{{ env.{VERSION_VARIABLE} }}}}" in WHITAKER_CACHE_KEY, (
        f"the reviewed key should read {VERSION_VARIABLE} from the job "
        "environment"
    )
    environment = jobs()[WHITAKER_JOB].get("env") or {}
    assert VERSION_VARIABLE in environment, (
        f"{WHITAKER_JOB[0]}:{WHITAKER_JOB[1]} must set {VERSION_VARIABLE}; "
        "an unset variable interpolates to the empty string and every "
        "installer version would then share one cache entry"
    )
    assert str(environment[VERSION_VARIABLE]).strip(), (
        f"{VERSION_VARIABLE} must have a value; an empty one is the same "
        "defect as an absent one, and harder to see"
    )


def test_the_cache_step_is_pinned_to_a_commit() -> None:
    """Scenario: the cache action is repointed at a mutable ref.

    Invariant: it is pinned to a full commit SHA. This step restores an
    executable that a later step runs, so the action that performs the
    restore is as much a supply-chain surface as the binary it restores.
    """
    uses = str(step_named(WHITAKER_JOB, WHITAKER_CACHE_STEP).get("uses"))
    assert PINNED_USES.match(uses), (
        f"the {WHITAKER_CACHE_STEP!r} step restores an executable, so it must "
        f"be pinned to a full commit SHA; found {uses!r}"
    )
