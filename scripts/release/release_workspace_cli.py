"""Record, check, survey, and verify a workspace release candidate.

- ``record`` describes the packaged archives, and retains them beside the record.
- ``check`` requires archives to match a record, before anything is published.
- ``survey`` asks crates.io about every crate and says whether to publish.
- ``verify`` downloads the published archives and compares their digests.

No command takes a credential: lading publishes, in a step of its own.
Behavioural coverage lives in
``scripts/release/tests/test_release_workspace_cli.py``.
"""

import argparse
import collections.abc as cabc
import dataclasses
import pathlib
import sys
import tempfile
import time

import crates_io_registry_http
import crates_io_transport_cli
import release_crates_io_adapter
import release_reconcile_crates
import release_registry_response
import verify_crates_io_bytes
import workspace_candidate
from release_reconcile_crates import RegistryState
from workspace_candidate import Candidate, CandidateError, CrateArchive, Expectation

EXIT_OK = 0
EXIT_REFUSED = 1

PUBLISH = "publish"
SKIP = "skip"


class SurveyError(RuntimeError):
    """Report a registry state that does not permit the release."""


@dataclasses.dataclass(frozen=True, slots=True)
class Registry:
    """Carry the network, delay, and clock boundaries the commands use.

    ``monotonic_clock_ns`` times each download against its budget, so a test
    can drive verification without reading the real clock.
    """

    opener: cabc.Callable[..., crates_io_registry_http.HttpResponse]
    sleeper: cabc.Callable[[float], None]
    monotonic_clock_ns: cabc.Callable[[], int]


def _live_registry() -> Registry:
    """Return the allowlisted opener, a real delay, and the monotonic clock."""
    return Registry(
        crates_io_registry_http.default_opener, time.sleep, time.monotonic_ns
    )


def observe_state(crate: CrateArchive, registry: Registry) -> RegistryState:
    """Classify what crates.io holds for one crate.

    An unreachable registry and an unreadable answer are both indeterminate,
    never absent, so neither can be mistaken for permission to publish.
    """
    with tempfile.TemporaryDirectory() as scratch:
        status_path = pathlib.Path(scratch) / "status"
        body_path = pathlib.Path(scratch) / "body"
        result = crates_io_transport_cli.probe(
            crates_io_transport_cli.ProbeRequest(
                crate.name, crate.version, status_path, body_path, registry.opener
            )
        )
        if result != crates_io_transport_cli.EXIT_OK:
            return RegistryState.INDETERMINATE
        try:
            observation = release_registry_response.observe_registry_response(
                int(status_path.read_text(encoding="ascii")),
                body_path.read_text(encoding="utf-8"),
            )
        except (
            UnicodeError,
            release_registry_response.RegistryResponseError,
        ) as error:
            print(f"{crate.name}: unreadable registry answer: {error}", file=sys.stderr)
            return RegistryState.INDETERMINATE
    return release_reconcile_crates.classify_registry_state(
        observation, version=crate.version, intended_sha256=crate.sha256
    )


def plan_publication(states: cabc.Mapping[str, RegistryState]) -> str:
    """Return whether a workspace still has a crate to publish.

    Parameters
    ----------
    states
        Registry state of every crate, by name.

    Returns
    -------
    str
        ``publish`` when any crate is absent, so an interrupted release
        resumes, or ``skip`` when every crate is already published.

    Raises
    ------
    SurveyError
        If any crate conflicts, is yanked, or could not be observed.

    Examples
    --------
    >>> plan_publication({"a": RegistryState.IDENTICAL, "b": RegistryState.ABSENT})
    'publish'
    >>> plan_publication({"a": RegistryState.IDENTICAL})
    'skip'
    """
    refused = {
        name: state.value
        for name, state in states.items()
        if release_reconcile_crates.plan_crates_action(state) == "fail"
    }
    if refused:
        message = f"the registry does not permit this release: {refused}"
        raise SurveyError(message)
    return PUBLISH if RegistryState.ABSENT in states.values() else SKIP


def plan_rehearsal(states: cabc.Mapping[str, RegistryState]) -> str:
    """Return whether a release of this workspace can be rehearsed.

    A pull request usually carries the last published version with newer
    content, which is a conflict a release would refuse. There is no release to
    rehearse then, so the answer is ``skip`` rather than an error.

    Examples
    --------
    >>> plan_rehearsal({"a": RegistryState.CONFLICT, "b": RegistryState.CONFLICT})
    'skip'
    >>> plan_rehearsal({"a": RegistryState.ABSENT, "b": RegistryState.ABSENT})
    'publish'
    """
    try:
        return plan_publication(states)
    except SurveyError:
        return SKIP


def survey(candidate: Candidate, registry: Registry) -> tuple[str, dict[str, str]]:
    """Observe every crate and decide what this run should do.

    Returns
    -------
    tuple[str, dict[str, str]]
        The action, and each crate's state for the log.

    Raises
    ------
    SurveyError
        If the run is tagged and the registry does not permit the release.
    """
    states = {crate.name: observe_state(crate, registry) for crate in candidate.crates}
    plan = plan_publication if candidate.tag else plan_rehearsal
    return plan(states), {name: state.value for name, state in states.items()}


def verify_published(candidate: Candidate, registry: Registry) -> None:
    """Download every crate and require the recorded digests.

    Raises
    ------
    verify_crates_io_bytes.VerificationError
        If an archive is absent, unavailable, or different.
    """
    with tempfile.TemporaryDirectory() as scratch:
        for crate in candidate.crates:
            archive = pathlib.Path(scratch) / crate.filename
            status_path = pathlib.Path(scratch) / f"{crate.filename}.status"
            crates_io_transport_cli.download(
                crates_io_transport_cli.DownloadRequest(
                    crate=crate.name,
                    version=crate.version,
                    archive_path=archive,
                    status_path=status_path,
                    opener=registry.opener,
                    sleeper=registry.sleeper,
                    monotonic_clock_ns=registry.monotonic_clock_ns,
                )
            )
            observation = release_crates_io_adapter.observe_download_file(
                int(status_path.read_text(encoding="ascii")), archive
            )
            verify_crates_io_bytes.verify_observation(
                observation=observation,
                crate=crate.name,
                version=crate.version,
                expected_sha256=crate.sha256,
            )
            print(f"{crate.filename}: published bytes match {crate.sha256}")


def build_parser() -> argparse.ArgumentParser:
    """Build the parser for the four commands."""
    parser = argparse.ArgumentParser(prog="release_workspace_cli")
    commands = parser.add_subparsers(dest="command", required=True)
    for name, summary in (
        ("record", "describe the packaged archives"),
        ("check", "require archives to match a record"),
        ("survey", "decide whether crates.io permits the release"),
        ("verify", "compare the published archives with a record"),
    ):
        command = commands.add_parser(name, help=summary)
        command.add_argument("--candidate", required=True, type=pathlib.Path)
        if name in {"record", "check"}:
            command.add_argument(
                "--package-directory", required=True, type=pathlib.Path
            )
            command.add_argument("--commit", required=True)
            command.add_argument("--tag", default="")
        if name == "record":
            command.add_argument(
                "--workspace-root", default=pathlib.Path(), type=pathlib.Path
            )
        if name == "survey":
            command.add_argument("--github-output", required=True, type=pathlib.Path)
    return parser


def _run(arguments: argparse.Namespace, registry: Registry) -> None:
    """Run one parsed command."""
    if arguments.command == "record":
        candidate = workspace_candidate.record_candidate(
            arguments.workspace_root,
            arguments.package_directory,
            Expectation(arguments.commit, arguments.tag),
        )
        workspace_candidate.retain_archives(
            candidate, arguments.package_directory, arguments.candidate.parent
        )
        workspace_candidate.write_candidate(candidate, arguments.candidate)
        return
    candidate = workspace_candidate.read_candidate(arguments.candidate)
    if arguments.command == "check":
        workspace_candidate.check_candidate(
            candidate,
            arguments.package_directory,
            Expectation(arguments.commit, arguments.tag),
        )
    elif arguments.command == "survey":
        action, states = survey(candidate, registry)
        print(f"registry states: {states}; action: {action}")
        with arguments.github_output.open("a", encoding="utf-8") as output:
            output.write(f"action={action}\nversion={candidate.version}\n")
    else:
        verify_published(candidate, registry)


def main(argv: list[str] | None = None, *, registry: Registry | None = None) -> int:
    """Run one command and return its exit code.

    Parameters
    ----------
    argv
        Command-line arguments. Defaults to ``sys.argv[1:]``.
    registry
        Network boundary. Defaults to crates.io.

    Returns
    -------
    int
        ``EXIT_OK``, or ``EXIT_REFUSED`` with the reason on standard error.
    """
    arguments = build_parser().parse_args(argv)
    try:
        _run(arguments, registry or _live_registry())
    except (
        CandidateError,
        SurveyError,
        verify_crates_io_bytes.VerificationError,
        OSError,
    ) as error:
        print(f"::error title=Release candidate::{error}", file=sys.stderr)
        return EXIT_REFUSED
    return EXIT_OK


if __name__ == "__main__":
    raise SystemExit(main())
