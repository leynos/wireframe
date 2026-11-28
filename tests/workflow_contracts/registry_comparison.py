"""Compare the actionlint registry with the runner labels the workflows use.

An entry is an exact label or ends in ``*``, as the estate's ``ubicloud-*``
does. Both ways a registry can be wrong are reported: a label no entry covers,
and an entry that covers no label in use.
"""

from __future__ import annotations

import typing as typ

if typ.TYPE_CHECKING:
    import collections.abc as cabc


def covers(entry: str, label: str) -> bool:
    """Say whether a registry entry names a label, exactly or by a ``*`` suffix."""
    prefix, star, rest = entry.partition("*")
    if not star:
        return entry == label
    return not rest and label.startswith(prefix)


def registry_mismatches(
    registered: cabc.Iterable[str], in_use: cabc.Iterable[str]
) -> tuple[list[str], list[str]]:
    """Return the unregistered labels, then the entries that cover no label."""
    entries = set(registered)
    labels = set(in_use)
    unregistered = sorted(
        label for label in labels if not any(covers(e, label) for e in entries)
    )
    unused = sorted(
        entry for entry in entries if not any(covers(entry, used) for used in labels)
    )
    return unregistered, unused
