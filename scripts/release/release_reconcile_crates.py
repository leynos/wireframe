"""Decide what a publication run may do from typed registry observations.

The crates.io adapter turns HTTP responses and JSON into one of three domain
observations: an absent version, an indeterminate answer, or a readable set of
version records. This module compares those facts with the candidate the run
intends to publish. It knows neither status codes nor response schemas, so the
decision can be tested without transport or serialization details.

The registry returns a crate's whole history, not just its latest version.
Classification therefore filters records for the intended version before
judging yank state or digest identity. A yanked target takes precedence over a
matching digest; every state other than an identical version or an absent one
stops publication.

Run via the release workflow's crates.io reconciliation; behavioural coverage
lives in ``scripts/release/tests/test_release_reconcile_crates.py``.
"""

import dataclasses
import enum
import typing as typ

if typ.TYPE_CHECKING:
    import collections.abc as cabc


@dataclasses.dataclass(frozen=True, slots=True)
class RegistryVersion:
    """Carry the domain facts needed to classify one published version."""

    num: str
    checksum: str
    is_yanked: bool


@dataclasses.dataclass(frozen=True, slots=True)
class RegistryAbsent:
    """State that the registry adapter observed no requested version."""


@dataclasses.dataclass(frozen=True, slots=True)
class RegistryIndeterminate:
    """State that the adapter could not establish whether a version exists."""


@dataclasses.dataclass(frozen=True, slots=True)
class RegistrySingleVersion:
    """Carry one observed record for the version requested."""

    record: RegistryVersion


@dataclasses.dataclass(frozen=True, slots=True)
class RegistryVersionHistory:
    """Carry the readable version records the registry observed for a crate."""

    records: tuple[RegistryVersion, ...]


RegistryObservation = (
    RegistryAbsent
    | RegistryIndeterminate
    | RegistrySingleVersion
    | RegistryVersionHistory
)


class RegistryState(enum.Enum):
    """What the registry reports about the version this run intends to publish."""

    #: No such version. Publishing is the only way forward.
    ABSENT = "absent"

    #: The published version's checksum equals the one this run would upload.
    IDENTICAL = "identical"

    #: A version exists under this number with different bytes.
    CONFLICT = "conflict"

    #: The version exists and has been yanked.
    YANKED = "yanked"

    #: The response cannot be interpreted, so nothing may be concluded.
    INDETERMINATE = "indeterminate"


def _records_for_version(
    records: cabc.Iterable[RegistryVersion], version: str
) -> list[RegistryVersion]:
    """Return the observed records describing ``version``.

    Returns
    -------
    list[RegistryVersion]
        Every observation whose version number matches ``version``.

    Examples
    --------
    >>> records = [
    ...     RegistryVersion("0.9.0", "ab", True),
    ...     RegistryVersion("1.0.0", "cd", False),
    ... ]
    >>> _records_for_version(records, "1.0.0")
    [RegistryVersion(num='1.0.0', checksum='cd', is_yanked=False)]
    >>> _records_for_version(records, "2.0.0")
    []
    """
    return [record for record in records if record.num == version]


def _classify_present_records(
    records: cabc.Iterable[RegistryVersion], intended_sha256: str
) -> RegistryState:
    """Classify the records already selected for the intended version.

    A yank outranks everything. Otherwise the version is identical only when
    every record for it carries the intended digest: one matching record must
    not vouch for another that differs.

    Returns
    -------
    RegistryState
        The state represented by the selected records.

    Examples
    --------
    >>> same = RegistryVersion("1.0.0", "ab", False)
    >>> _classify_present_records([same], "ab") is RegistryState.IDENTICAL
    True
    >>> other = RegistryVersion("1.0.0", "cd", False)
    >>> _classify_present_records([same, other], "ab") is RegistryState.CONFLICT
    True
    """
    observed = tuple(records)
    if any(record.is_yanked for record in observed):
        return RegistryState.YANKED
    if all(record.checksum == intended_sha256 for record in observed):
        return RegistryState.IDENTICAL
    return RegistryState.CONFLICT


def classify_registry_state(
    observation: RegistryObservation, *, version: str, intended_sha256: str
) -> RegistryState:
    """Classify a typed registry observation against the intended release.

    Parameters
    ----------
    observation
        Domain facts produced by the registry response adapter.
    version
        The version this run intends to publish, spelled as the registry
        spells it.
    intended_sha256
        SHA-256 this run would publish, lowercase hexadecimal.

    Returns
    -------
    RegistryState
        The state, from :class:`RegistryState`.

    Notes
    -----
    A readable history that omits the intended version means it is absent. A
    single-version answer about another number conflicts, because it does not
    answer the question the caller asked. Yank takes precedence over digest
    equality so a withdrawn version is never treated as complete.

    Examples
    --------
    >>> classify_registry_state(
    ...     RegistryAbsent(), version="1.0.0", intended_sha256="ab"
    ... )
    <RegistryState.ABSENT: 'absent'>
    >>> matching = RegistrySingleVersion(RegistryVersion("1.0.0", "ab", False))
    >>> classify_registry_state(matching, version="1.0.0", intended_sha256="ab")
    <RegistryState.IDENTICAL: 'identical'>
    >>> classify_registry_state(
    ...     RegistryIndeterminate(), version="1.0.0", intended_sha256="ab"
    ... )
    <RegistryState.INDETERMINATE: 'indeterminate'>
    """
    match observation:
        case RegistryAbsent():
            return RegistryState.ABSENT
        case RegistryIndeterminate():
            return RegistryState.INDETERMINATE
        case RegistrySingleVersion(record=record):
            if record.num != version:
                return RegistryState.CONFLICT
            return _classify_present_records((record,), intended_sha256)
        case RegistryVersionHistory(records=records):
            matching = _records_for_version(records, version)
            if not matching:
                return RegistryState.ABSENT
            return _classify_present_records(matching, intended_sha256)


def plan_crates_action(state: RegistryState) -> str:
    """Return the action to take for a classified registry state.

    Returns
    -------
    str
        ``publish``, ``skip``, or ``fail`` for the observed state.

    Only an identical version skips. Absence publishes; every other state
    refuses to proceed. A retry policy belongs to the caller, which knows its
    attempt budget.

    Examples
    --------
    >>> plan_crates_action(RegistryState.ABSENT)
    'publish'
    >>> plan_crates_action(RegistryState.IDENTICAL)
    'skip'
    >>> plan_crates_action(RegistryState.CONFLICT)
    'fail'
    >>> plan_crates_action(RegistryState.YANKED)
    'fail'
    >>> plan_crates_action(RegistryState.INDETERMINATE)
    'fail'
    """
    if state is RegistryState.ABSENT:
        return "publish"
    if state is RegistryState.IDENTICAL:
        return "skip"
    return "fail"
