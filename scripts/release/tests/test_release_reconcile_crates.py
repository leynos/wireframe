"""Behavioural coverage for crates.io registry reconciliation.

Before the release publishes a crate it asks the registry what is already
there, and classifies the answer into one of five states. The classification
is the safety property: only a byte-identical published version may be skipped,
and a yank must be judged before identity so a withdrawn release is never
reported as merely present.

The fixtures use both real response shapes. The version endpoint answers with
``{"version": {…}}`` and the crate endpoint with ``{"versions": [ … ]}``;
drafting for only one of them would have raised on the endpoint the release
actually calls, so both are covered.

Run via ``make test-release-scripts``.
"""

import json

import pytest
from hypothesis import given, settings
from hypothesis import strategies as st
from release_reconcile_crates import (
    RegistryState,
    classify_registry_state,
    plan_crates_action,
)
from release_registry_response import RegistryResponseError, observe_registry_response

#: A well-formed digest, matching what the registry reports for a published
#: version. Which 64 characters they are does not matter; that both sides use
#: the same string does.
INTENDED = "f" * 64
OTHER = "e" * 64

#: The version under test, and one the registry has already published.
VERSION = "1.0.0"
EARLIER = "0.9.0"
NOT_FOUND = 404
OK = 200

#: One SHA-256 digest, lowercase hexadecimal, generated rather than fixed.
#:
#: LEM-3's non-vacuity clause asks for generated digests specifically, so a
#: conflict cannot be produced by a classifier that merely compares against a
#: constant this file happens to contain. The alphabet is narrowed to
#: hexadecimal because the record is a digest; the *values* are not narrowed,
#: which is what makes the matching and the non-matching case both reachable
#: from one draw.
_DIGEST = st.text(alphabet="0123456789abcdef", min_size=64, max_size=64)


def _version_body(*, checksum: str, yanked: bool, num: str = VERSION) -> str:
    """Render the version endpoint's response, as crates.io sends it."""
    return json.dumps({"version": {"num": num, "checksum": checksum, "yanked": yanked}})


def _crate_body(*records: tuple[str, str, bool]) -> str:
    """Render the crate endpoint's response, as crates.io sends it.

    The stated total matches the records supplied, which is what the live
    endpoint was observed to do. :func:`_truncated_body` renders the case
    where it does not.

    Returns
    -------
    str
        The response body, as JSON text.
    """
    return json.dumps({
        "crate": {"num_versions": len(records)},
        "versions": [{"num": num, "checksum": c, "yanked": y} for num, c, y in records],
    })


def _truncated_body(*records: tuple[str, str, bool]) -> str:
    """Render a crate response whose stated total exceeds the records it sent."""
    parsed = json.loads(_crate_body(*records))
    parsed["crate"]["num_versions"] = len(records) + 1
    return json.dumps(parsed)


def _classify(
    body: str | None, *, intended: str = INTENDED, status: int = OK
) -> RegistryState:
    """Classify a response for this release's own version and digest.

    Every test here asks about the same version, so repeating that argument at
    each call site only obscures which part of the input the test varies. The
    parameters that a test does vary stay explicit at the call site.

    Returns
    -------
    RegistryState
        The state the response was classified as.
    """
    observation = observe_registry_response(status, body)
    return classify_registry_state(
        observation, version=VERSION, intended_sha256=intended
    )


def test_absent_version_is_absent() -> None:
    """Require a 404 to classify as absent, so the release proceeds to publish."""
    state = _classify(None, status=NOT_FOUND)
    assert state is RegistryState.ABSENT, "a 404 must classify as absent"
    assert plan_crates_action(state) == "publish", (
        "an absent version must be planned for publication"
    )


def test_identical_version_is_identical() -> None:
    """Require a published version with the intended digest to be identical."""
    state = _classify(_version_body(checksum=INTENDED, yanked=False))
    assert state is RegistryState.IDENTICAL, (
        "a published version with the intended digest must classify as identical"
    )
    assert plan_crates_action(state) == "skip", (
        "an identical version must be planned as a skip"
    )


def test_crate_endpoint_shape_is_also_understood() -> None:
    """Require the plural ``versions`` shape to classify identically.

    The release calls both endpoints. Handling one shape only would raise on
    the other, which is a failure the singular-shape tests cannot see.
    """
    state = _classify(_crate_body((VERSION, INTENDED, False)))
    assert state is RegistryState.IDENTICAL, (
        "the plural versions shape must classify the same way as the singular"
    )


def test_published_version_with_a_different_digest_conflicts() -> None:
    """Require a published version carrying other bytes to conflict.

    This is the case that must never skip: the version exists, but it is not
    what this release intends to publish.
    """
    state = _classify(_version_body(checksum=OTHER, yanked=False))
    assert state is RegistryState.CONFLICT, (
        "a published version carrying other bytes must conflict"
    )
    assert plan_crates_action(state) == "fail", (
        "a conflict must stop the release rather than publish or skip"
    )


def test_yanked_version_is_yanked_not_identical() -> None:
    """Require a yank to be reported as a yank, even when the bytes match.

    Yank is judged before identity deliberately. A yanked version that happens
    to carry the intended digest is still withdrawn, and reporting it as
    "identical" would let the release treat a withdrawn crate as complete.
    """
    state = _classify(_version_body(checksum=INTENDED, yanked=True))
    assert state is RegistryState.YANKED, (
        "a yank must be judged before the digest and reported as a yank"
    )
    assert plan_crates_action(state) == "fail", (
        "a withdrawn version must stop the release even when the bytes match"
    )


def test_the_intended_record_is_found_wherever_it_sits_in_the_history() -> None:
    """Require the history to be searched, not read by position.

    The crate endpoint returns every version, newest first, so the intended
    one is only first when it is the most recent publication. A classifier
    reading only the leading record would report a conflict for a release
    whose version is present but not first.
    """
    state = _classify(_crate_body((EARLIER, OTHER, False), (VERSION, INTENDED, False)))
    assert state is RegistryState.IDENTICAL, (
        "a matching record must be found wherever it sits in the list"
    )


def test_a_yanked_older_version_does_not_poison_a_newer_release() -> None:
    """Require a yank to be judged for the intended version alone.

    Live evidence: ``GET /api/v1/crates/serde`` answers with all 316 of its
    versions at once, and 3 of those are yanked. A classifier that asked "is
    any version in this history yanked?" would therefore classify every future
    serde release as ``YANKED`` and stop the release for ever. The same body
    scanned for a digest would likewise let an older release's bytes answer for
    a newer version.
    """
    state = _classify(
        _crate_body((VERSION, INTENDED, False), (EARLIER, INTENDED, True))
    )
    assert state is RegistryState.IDENTICAL, (
        "a yank on an older release must not be reported as this one's state"
    )


def test_one_matching_record_does_not_vouch_for_a_different_one() -> None:
    """Require every record for the intended version to match.

    A classifier asking whether *any* record matches would call the version
    identical, and skip the upload, even though the registry also holds other
    bytes under the same number.
    """
    state = _classify(
        _crate_body((VERSION, INTENDED, False), (VERSION, OTHER, False))
    )
    assert state is RegistryState.CONFLICT, (
        "a differing record for the same version must be reported as a conflict"
    )


def test_the_yank_of_the_intended_version_is_still_reported() -> None:
    """Require the per-version filter not to hide a genuine yank.

    The control for the row above: narrowing the question must not make the
    answer unreachable. The same body classifies ``YANKED`` when the intended
    version is the withdrawn one.
    """
    state = _classify(
        _crate_body((EARLIER, INTENDED, False), (VERSION, INTENDED, True))
    )
    assert state is RegistryState.YANKED, (
        "the intended version's own yank must still stop the release"
    )


def test_a_version_the_history_does_not_list_is_absent() -> None:
    """Publish a version the crate has never released.

    The crate exists, so the endpoint answers 200 with its history; the
    version is not in it. That is the ordinary first publication of a new
    version, and reading it as a conflict would stop every release.
    """
    state = _classify(_crate_body((EARLIER, OTHER, False)))
    assert state is RegistryState.ABSENT, (
        "a history without the intended version must classify as absent"
    )
    assert plan_crates_action(state) == "publish", (
        "an unlisted version is the case publication exists for"
    )


def test_a_truncated_history_is_indeterminate() -> None:
    """Refuse a history that disagrees with its own stated total.

    Filtering by version makes truncation dangerous in a new way: a body
    missing the intended record is indistinguishable from a version never
    published, which plans a publish over whatever the registry holds. The
    stated count is what tells the two apart.
    """
    state = _classify(_truncated_body((EARLIER, OTHER, False)))
    assert state is RegistryState.INDETERMINATE, (
        "a history whose record count contradicts its stated total must not "
        "be read as evidence that the version is absent"
    )
    assert plan_crates_action(state) != "publish", (
        "a truncated history must not plan a publish"
    )


def test_a_version_answer_about_another_number_conflicts() -> None:
    """Refuse a single-version answer describing a different version.

    The version endpoint is asked for one number and answers with one record.
    A record for another number means the response does not describe what was
    asked about, and nothing may be concluded about the intended version.
    """
    state = _classify(_version_body(checksum=INTENDED, yanked=False, num=EARLIER))
    assert state is RegistryState.CONFLICT, (
        "an answer about a different version must not be read as this one's"
    )


def test_an_unrecognized_body_is_indeterminate() -> None:
    """Require an unreadable answer to be indeterminate rather than absent.

    Indeterminate must not plan a publish. A registry that answered something
    unexpected has not told us the version is absent, and treating silence as
    absence is how a release publishes over existing bytes.
    """
    state = _classify("not json at all")
    assert state is RegistryState.INDETERMINATE, (
        "an unreadable answer must be indeterminate, not absent"
    )
    assert plan_crates_action(state) != "publish", (
        "an indeterminate state must not plan a publication"
    )


def test_a_version_with_no_checksum_is_refused() -> None:
    """Require a record missing its checksum to be refused, not classified.

    The distinction from the unparsable-body case below is deliberate. A body
    that is not JSON at all is a transient or proxied answer and is classified
    ``INDETERMINATE`` so a caller may retry. A well-formed crates.io response
    whose version record carries no checksum is a shape this code cannot read,
    and the module documents that as a raised
    :class:`RegistryResponseError` rather than a state — the registry does not
    omit the field, so silence here means the assumption about its schema is
    wrong, and a state would swallow that.
    """
    with pytest.raises(RegistryResponseError):
        _classify(json.dumps({"version": {"num": VERSION, "yanked": False}}))


@pytest.mark.parametrize(
    "body",
    [
        json.dumps({"version": {"checksum": INTENDED, "yanked": False}}),
        json.dumps({
            "crate": {"num_versions": 1},
            "versions": [{"checksum": INTENDED, "yanked": False}],
        }),
    ],
    ids=["single-version", "history"],
)
def test_a_record_without_a_version_number_is_refused(body: str) -> None:
    """Refuse an unnumbered record rather than treating it as absent."""
    with pytest.raises(RegistryResponseError, match="no usable number"):
        _classify(body)


@pytest.mark.parametrize("status", [500, 502, 503, 429])
def test_a_server_error_is_indeterminate(status: int) -> None:
    """Require a retryable server status not to be read as absence."""
    state = _classify(None, status=status)
    assert state is RegistryState.INDETERMINATE, (
        "a retryable server status must not be read as absence"
    )
    assert plan_crates_action(state) == "fail", (
        "a server error must stop the release rather than publish over it"
    )


def test_only_identical_skips() -> None:
    """Require ``skip`` to be reachable for exactly one state.

    Stated as a property over the whole enum rather than for named states, so
    a state added later cannot quietly acquire a skipping plan.
    """
    skipping = {state for state in RegistryState if plan_crates_action(state) == "skip"}
    assert skipping == {RegistryState.IDENTICAL}, (
        "only the identical state may plan a skip"
    )
    publishing = {
        state for state in RegistryState if plan_crates_action(state) == "publish"
    }
    assert publishing == {RegistryState.ABSENT}, (
        "only the absent state may plan a publication"
    )


@pytest.mark.parametrize(
    ("intended", "state"),
    [
        # A digest the registry does not report is a conflict.
        ("d" * 64, RegistryState.CONFLICT),
        # A caller passing a non-digest gets the same answer. Recorded, not
        # endorsed: this function does not validate its argument, and the
        # argument's shape is owned by the identity record it is read from.
        ("not-a-digest", RegistryState.CONFLICT),
    ],
)
def test_a_digest_the_registry_does_not_report_conflicts(
    intended: str, state: RegistryState
) -> None:
    """Require any non-matching intended digest to produce a conflict.

    The second case pins that a malformed argument is *not* separately
    rejected. A caller reading the identity record is responsible for its
    shape; adding a check here would not remove that responsibility, only
    split one validation across two places.
    """
    assert (
        _classify(_version_body(checksum=INTENDED, yanked=False), intended=intended)
        is state
    ), "any intended digest the registry does not report must conflict"


@settings(max_examples=200, deadline=None, derandomize=True)
@given(published=_DIGEST, intended=_DIGEST)
def test_identity_turns_on_the_digest_alone(published: str, intended: str) -> None:
    """Require identity to follow digest equality for any pair of digests.

    LEM-3 says only an identical checksum permits a skip. Stated over generated
    digests, that is one two-sided claim: equal digests must classify
    ``IDENTICAL`` and unequal ones must classify ``CONFLICT``. Both directions
    are asserted in one test on purpose. Splitting them would let a classifier
    that always answers ``IDENTICAL`` pass the matching half, and one that
    always answers ``CONFLICT`` pass the other, so neither half alone is
    evidence that the comparison happens at all.

    The published checksum and the intended digest are drawn independently, so
    Hypothesis supplies both equal and unequal pairs. ``derandomize`` keeps the
    shrink reproducible in CI, and ``deadline=None`` because the first example
    pays the import cost of the JSON reader.
    """
    state = _classify(
        _version_body(checksum=published, yanked=False), intended=intended
    )
    expected = (
        RegistryState.IDENTICAL if published == intended else RegistryState.CONFLICT
    )
    assert state is expected, (
        f"identity must turn on digest equality alone: published {published!r} "
        f"against intended {intended!r} gave {state!r}"
    )
