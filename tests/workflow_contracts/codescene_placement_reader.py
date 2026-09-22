"""Reading machinery for the CodeScene placement contract (CV-005).

This module gets facts out of a workflow tree and holds no opinion about
them; the assertions live in ``ci_codescene_placement_test``, and the
readings are proved against constructed trees in
``codescene_placement_reader_test``. Every reader takes its documents as an
argument, so a fixture tree exercises exactly the code the contract runs.

Four readings here refuse rather than guess, because each guess had a
failure mode that no green run would show.

``load_workflow`` refuses a duplicated mapping key. PyYAML keeps the last
value and says nothing, so a lane declaring ``runs-on`` or ``env`` twice
parses into a document that has silently discarded half of what GitHub was
asked to run.

``triggers`` reads ``on:`` as a scalar, a sequence or a mapping, under both
the string key and YAML 1.1's boolean ``True``, and refuses anything else. A
mapping-only reader stringifies ``on: [push, pull_request]`` into one key
that matches no trigger, and a reader that returns nothing for an unknown
shape lets the workflow escape every pull-request clause.

``local_callee`` matches a reusable-workflow call by shape rather than by an
enumerated prefix list: whatever resolves to a file under this repository's
``.github/workflows/`` is local, whether it is written ``./``, ``$/`` or
fully qualified with this repository's own ``owner/repo``.

``pull_request_closure`` follows those calls transitively, because a
workflow declaring only ``workflow_call`` still runs on a pull request when
a pull-request workflow calls it, and ``secrets: inherit`` hands it the
token.
"""

from __future__ import annotations

import typing as typ
from pathlib import Path

import yaml

#: Both spellings GitHub accepts for a workflow file's extension, compared
#: lowercased so that ``CI.YML`` is not skipped in silence.
WORKFLOW_SUFFIXES: typ.Final = frozenset({".yml", ".yaml"})

#: This repository as a fully qualified ``uses:`` value names it, lowercased.
REPOSITORY: typ.Final = "leynos/wireframe"

#: Where this repository's workflows live, as a ``uses:`` value names them.
WORKFLOW_PREFIX: typ.Final = ".github/workflows/"

#: Prefixes that, once stripped, leave a path inside this repository. ``./``
#: is the documented local form; ``$/`` is accepted for the same reason as the
#: qualified form: over-reading a call only widens the closure the
#: prohibitions run over, while under-reading one hides a workflow from them.
LOCAL_PREFIXES: typ.Final = ("./", "$/", f"{REPOSITORY}/")

#: Triggers that start a workflow for a pull request. ``pull_request_target``
#: runs with write permissions, which makes it more dangerous, not less.
PULL_REQUEST_TRIGGERS: typ.Final = frozenset(
    {"pull_request", "pull_request_target"}
)

Document = dict[object, object]


class DuplicateKeyError(yaml.constructor.ConstructorError):
    """A mapping declared the same key twice."""


class StrictLoader(yaml.SafeLoader):
    """A ``SafeLoader`` that refuses duplicate mapping keys.

    Examples
    --------
    >>> yaml.load("a: 1\\nb: 2\\n", Loader=StrictLoader)
    {'a': 1, 'b': 2}
    """

    def construct_mapping(
        self, node: yaml.MappingNode, deep: bool = False
    ) -> dict[object, object]:
        """Build a mapping, refusing a key that appears twice."""
        seen: set[object] = set()
        for key_node, _ in node.value:
            key = self.construct_object(key_node, deep=deep)
            if key in seen:
                raise DuplicateKeyError(
                    "while constructing a mapping",
                    node.start_mark,
                    f"found duplicate key {key!r}",
                    key_node.start_mark,
                )
            seen.add(key)
        return super().construct_mapping(node, deep=deep)


def load_workflow(text: str) -> Document:
    """Parse one workflow strictly.

    Examples
    --------
    >>> load_workflow("on: push\\njobs: {}\\n")
    {True: 'push', 'jobs': {}}

    Raises
    ------
    DuplicateKeyError
        When any mapping in the document declares a key twice.
    """
    document = yaml.load(text, Loader=StrictLoader)
    return document if isinstance(document, dict) else {}


def read_workflows(directory: Path) -> dict[str, Document]:
    """Parse every workflow in a directory, keyed by file name.

    Raises
    ------
    DuplicateKeyError
        When any workflow declares a mapping key twice.
    """
    paths = sorted(
        path
        for path in directory.iterdir()
        if path.is_file() and path.suffix.lower() in WORKFLOW_SUFFIXES
    )
    return {
        path.name: load_workflow(path.read_text(encoding="utf-8"))
        for path in paths
    }


def triggers(document: Document) -> dict[object, object]:
    """Return a workflow's triggers as a mapping of name to configuration.

    Examples
    --------
    >>> triggers({True: ["push", "pull_request"]})
    {'push': None, 'pull_request': None}

    Raises
    ------
    ValueError
        When ``on:`` is missing or is not a scalar, sequence or mapping of
        names, since a workflow whose triggers cannot be read cannot be
        classified as outside the pull-request lane either.
    """
    for key in ("on", True):
        if key not in document:
            continue
        match document[key]:
            case dict() as mapping:
                return mapping
            case str() as event:
                return {event: None}
            case list() as events if all(
                isinstance(event, str) for event in events
            ):
                return dict.fromkeys(events)
            case other:
                message = f"unsupported `on:` shape {other!r}"
                raise ValueError(message)
    message = "a workflow with no `on:` block cannot be classified"
    raise ValueError(message)


def local_callee(uses: str) -> str | None:
    """Return the workflow in this repository a ``uses:`` value names.

    Examples
    --------
    >>> local_callee("./.github/workflows/release.yml")
    'release.yml'
    >>> local_callee("leynos/wireframe/.github/workflows/release.yml@main")
    'release.yml'
    >>> local_callee("other/repo/.github/workflows/x.yml@v1") is None
    True
    """
    candidate = uses.strip().split("@", 1)[0]
    for prefix in LOCAL_PREFIXES:
        if candidate.lower().startswith(prefix):
            candidate = candidate[len(prefix) :]
            break
    if not candidate.startswith(WORKFLOW_PREFIX):
        return None
    name = candidate.removeprefix(WORKFLOW_PREFIX)
    return name if name and "/" not in name else None


def jobs(document: Document) -> dict[object, Document]:
    """Return a workflow's jobs, skipping any that are not mappings."""
    raw = document.get("jobs")
    if not isinstance(raw, dict):
        return {}
    return {name: job for name, job in raw.items() if isinstance(job, dict)}


def steps(document: Document) -> list[Document]:
    """Return every mapping step in every job, in document order."""
    return [
        step
        for job in jobs(document).values()
        for step in (job.get("steps") or [])
        if isinstance(step, dict)
    ]


def calls(document: Document) -> list[Document]:
    """Return every step, plus every job that is itself a workflow call.

    A job calling a reusable workflow carries ``uses:`` on the job and has no
    steps, so a reader of steps alone misses the one shape that can run
    another repository's workflow.
    """
    return [job for job in jobs(document).values() if "uses" in job] + steps(
        document
    )


def pull_request_closure(documents: typ.Mapping[str, Document]) -> list[str]:
    """Return every workflow a pull request can reach, callees included.

    Examples
    --------
    >>> pull_request_closure({
    ...     "ci.yml": {True: "pull_request", "jobs": {
    ...         "r": {"uses": "./.github/workflows/lib.yml"}}},
    ...     "lib.yml": {True: "workflow_call", "jobs": {}},
    ... })
    ['ci.yml', 'lib.yml']
    """
    pending = [
        name
        for name, document in documents.items()
        if PULL_REQUEST_TRIGGERS & set(triggers(document))
    ]
    reached: set[str] = set()
    # A visited set rather than recursion: two reusable workflows calling
    # each other would otherwise loop, and a contract that hangs is worse
    # than one that is wrong.
    while pending:
        name = pending.pop()
        if name in reached or name not in documents:
            continue
        reached.add(name)
        pending.extend(
            callee
            for job in jobs(documents[name]).values()
            if (callee := local_callee(str(job.get("uses", ""))))
        )
    return sorted(reached)


def mentions(node: object, needle: str, *, ignore_case: bool = False) -> bool:
    """Return whether a string occurs in any key or scalar of a node.

    The whole tree is walked rather than the scopes a value is meant to live
    in, because the point of every caller is that it must not appear at all.

    Examples
    --------
    >>> mentions({"env": {"T": "${{ secrets.CS_ACCESS_TOKEN }}"}}, "CS_")
    True
    >>> mentions({"run": "curl https://API.CodeScene.io"}, "codescene.io",
    ...          ignore_case=True)
    True
    """
    if isinstance(node, dict):
        return any(
            mentions(key, needle, ignore_case=ignore_case)
            or mentions(value, needle, ignore_case=ignore_case)
            for key, value in node.items()
        )
    if isinstance(node, list):
        return any(
            mentions(item, needle, ignore_case=ignore_case) for item in node
        )
    text = str(node)
    return needle.lower() in text.lower() if ignore_case else needle in text


def external_secret_inheritors(document: Document) -> list[str]:
    """Return jobs that hand every secret to a workflow outside this tree.

    ``secrets: inherit`` to a local callee is visible, because the callee is
    in the closure and read like any other workflow. The same line pointed at
    another repository forwards the token to a document this contract cannot
    read, without naming it anywhere a sweep for the name could find.
    """
    return [
        str(name)
        for name, job in jobs(document).items()
        if job.get("secrets") == "inherit"
        and "uses" in job
        and local_callee(str(job["uses"])) is None
    ]
