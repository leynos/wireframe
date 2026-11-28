"""Adapt crates.io transport responses into typed release observations.

The registry replies in two JSON shapes and uses HTTP status codes to report
absence and transient failures. This adapter owns that transport vocabulary:
it maps status and schema into domain observations, leaving
:mod:`release_reconcile_crates` to decide what those facts permit.

Keeping the two apart is what lets each be reviewed on its own terms. This
module is where a schema surprise belongs — a record without a checksum, a
body that is neither shape, a history that disagrees with its own stated
total. The policy module never has to guess how the registry spells things,
and this one never has to know what a release intends to do.

The shapes, both taken from live responses rather than documentation:

- ``GET /api/v1/crates/<crate>/<version>`` answers ``{"version": {...}}`` —
  one record, for the version asked about.
- ``GET /api/v1/crates/<crate>`` answers ``{"versions": [{...}, ...]}``
  alongside ``crate`` — the crate's **entire publication history**. Probed on
  2026-10-23, ``serde`` returned all 316 of its versions and
  ``crate.num_versions`` said 316.

Nothing here performs network I/O; the caller records the response and passes
it in. Coverage lives in ``scripts/release/tests/test_release_reconcile_crates.py``.

"""

import json

from release_reconcile_crates import (
    RegistryAbsent,
    RegistryIndeterminate,
    RegistryObservation,
    RegistrySingleVersion,
    RegistryVersion,
    RegistryVersionHistory,
)

#: Top-level key in a *crate* response: an array of every version.
VERSIONS_KEY = "versions"

#: Top-level key in a *version* response: one version's record.
VERSION_KEY = "version"

#: Top-level key naming the crate metadata object in a crate response.
CRATE_KEY = "crate"

#: HTTP status for a successful registry response.
HTTP_OK = 200

#: HTTP status the registry uses when a version key is absent.
HTTP_NOT_FOUND = 404

#: Count of versions the crate endpoint states the history holds.
NUM_VERSIONS_KEY = "num_versions"

#: The per-version key carrying the version number the record describes.
NUM_KEY = "num"

#: The per-version key holding the authoritative SHA-256 of the ``.crate``.
CHECKSUM_KEY = "checksum"

#: The per-version key marking a version its maintainers withdrew.
YANKED_KEY = "yanked"


class RegistryResponseError(ValueError):
    """Report a response body that is not the JSON shape the registry sends."""


def _raw_records(parsed: object) -> tuple[bool, list[object]]:
    """Return the version records from a parsed crates.io response.

    Parameters
    ----------
    parsed
        A decoded ``crates.io`` response.

    Returns
    -------
    tuple[bool, list[object]]
        Whether this is a history response and its raw version records.

    Raises
    ------
    RegistryResponseError
        If ``parsed`` is neither shape.

    Notes
    -----
    A reader written for one shape rejects the other, so both are accepted
    here and folded to one list. This is not defensive over-acceptance: each
    branch is a documented response from an endpoint the release calls, and
    the checksum lives in the same field in both.
    """
    if not isinstance(parsed, dict):
        msg = f"registry response is not a JSON object: {parsed!r}"
        raise RegistryResponseError(msg)
    single = parsed.get(VERSION_KEY)
    if isinstance(single, dict):
        return False, [single]
    many = parsed.get(VERSIONS_KEY)
    if isinstance(many, list):
        return True, many
    msg = (
        f"registry response carries neither a {VERSION_KEY!r} object nor a "
        f"{VERSIONS_KEY!r} list: {parsed!r}"
    )
    raise RegistryResponseError(msg)


def _record_fields(versions: list[object]) -> tuple[RegistryVersion, ...]:
    """Return the fields this code reads from each version record.

    Parameters
    ----------
    versions
        The version records from a decoded crates.io response.

    Returns
    -------
    tuple[RegistryVersion, ...]
        One record per entry, in the order the registry listed them.

    Raises
    ------
    RegistryResponseError
        If a version record is not an object or carries no usable number or
        checksum.

    Notes
    -----
    A missing or non-string ``num`` is rejected rather than represented as
    absence. In a history answer, an unnumbered record could be the intended
    version; treating it as absent would plan a duplicate publication.
    """
    records: list[RegistryVersion] = []
    for version in versions:
        if not isinstance(version, dict):
            msg = f"registry version record is not an object: {version!r}"
            raise RegistryResponseError(msg)
        records.append(_version_record(version))
    return tuple(records)


def _version_record(version: dict[object, object]) -> RegistryVersion:
    """Validate and return the fields this adapter reads from one record.

    Returns
    -------
    RegistryVersion
        The version number, checksum, and yank state used by reconciliation.

    Raises
    ------
    RegistryResponseError
        If the record has no usable version number or checksum.

    Examples
    --------
    >>> _version_record({"num": "1.0.0", "checksum": "ab"})
    RegistryVersion(num='1.0.0', checksum='ab', is_yanked=False)
    """
    num = version.get(NUM_KEY)
    if not isinstance(num, str) or not num:
        msg = f"registry version record carries no usable number: {version!r}"
        raise RegistryResponseError(msg)
    checksum = version.get(CHECKSUM_KEY)
    if not isinstance(checksum, str) or not checksum:
        msg = f"registry version record carries no checksum: {version!r}"
        raise RegistryResponseError(msg)
    return RegistryVersion(
        num=num,
        checksum=checksum,
        is_yanked=bool(version.get(YANKED_KEY)),
    )


def _stated_total(parsed: object) -> int | None:
    """Return the version count a crate response claims to be listing."""
    if not isinstance(parsed, dict):
        return None
    crate = parsed.get(CRATE_KEY)
    if not isinstance(crate, dict):
        return None
    stated = crate.get(NUM_VERSIONS_KEY)
    return stated if isinstance(stated, int) else None


def _history_is_complete(
    *,
    is_history: bool,
    stated_total: int | None,
    records: tuple[RegistryVersion, ...],
) -> bool:
    """Report whether an advertised version history has every stated record.

    Returns
    -------
    bool
        ``True`` for a single-version response or a complete history.

    Examples
    --------
    >>> _history_is_complete(
    ...     is_history=True,
    ...     stated_total=1,
    ...     records=(RegistryVersion("1.0.0", "ab", is_yanked=False),),
    ... )
    True
    """
    if not is_history or stated_total is None:
        return True
    return stated_total == len(records)


def observe_registry_response(status: int, body: str | None) -> RegistryObservation:
    """Map an HTTP response into a domain observation.

    Parameters
    ----------
    status
        HTTP status returned by crates.io.
    body
        Response body as recorded, or ``None`` when the response had none.

    Returns
    -------
    RegistryObservation
        An absent, indeterminate, or readable version observation.

    Notes
    -----
    HTTP 404 is the registry's absence observation; every other non-200 status
    and an absent or non-JSON body are indeterminate. A body that parses but
    does not fit either documented shape is a schema surprise, which raises
    :class:`RegistryResponseError` rather than being folded into uncertainty.

    A body that is JSON but neither crates.io shape, or one carrying a version
    record this module cannot read, raises :class:`RegistryResponseError`.
    The parsing helpers raise it while unpacking the parsed body, where the
    response shape and records are actually read.

    Examples
    --------
    >>> observe_registry_response(404, None)
    RegistryAbsent()
    >>> observe_registry_response(503, "ignored")
    RegistryIndeterminate()
    >>> answer = observe_registry_response(
    ...     200, '{"version": {"num": "1.0.0", "checksum": "ab"}}'
    ... )
    >>> answer.record
    RegistryVersion(num='1.0.0', checksum='ab', is_yanked=False)
    """
    if status == HTTP_NOT_FOUND:
        return RegistryAbsent()
    if status != HTTP_OK or body is None:
        return RegistryIndeterminate()
    try:
        parsed = json.loads(body)
    except json.JSONDecodeError:
        return RegistryIndeterminate()
    return _observe_parsed_response(parsed)


def _observe_parsed_response(parsed: object) -> RegistryObservation:
    """Reconcile a decoded response body into its typed registry observation.

    Returns
    -------
    RegistryObservation
        A history, one version, or an indeterminate response when the
        advertised history is incomplete.

    Examples
    --------
    >>> isinstance(
    ...     _observe_parsed_response(
    ...         {"version": {"num": "1.0.0", "checksum": "ab"}}
    ...     ),
    ...     RegistrySingleVersion,
    ... )
    True
    """
    is_history, raw_records = _raw_records(parsed)
    records = _record_fields(raw_records)
    stated_total = _stated_total(parsed)
    if not _history_is_complete(
        is_history=is_history, stated_total=stated_total, records=records
    ):
        return RegistryIndeterminate()
    return _observation_for_records(records, is_history=is_history)


def _observation_for_records(
    records: tuple[RegistryVersion, ...], *, is_history: bool
) -> RegistryObservation:
    """Build the typed observation for successfully read version records.

    Returns
    -------
    RegistryObservation
        A version history or one version record, according to the response.

    Examples
    --------
    >>> record = RegistryVersion("1.0.0", "ab", is_yanked=False)
    >>> isinstance(
    ...     _observation_for_records((record,), is_history=False),
    ...     RegistrySingleVersion,
    ... )
    True
    """
    if is_history:
        return RegistryVersionHistory(records=records)
    return RegistrySingleVersion(record=records[0])
