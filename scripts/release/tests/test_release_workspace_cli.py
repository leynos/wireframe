"""Specify the workspace release commands against a scripted registry."""

import itertools
import json
import time
import typing as typ
import urllib.error

import crates_io_transport_cli
import pytest
from crates_io_transport_support import FakeOpener, FakeResponse, missing_response
from release_reconcile_crates import RegistryState
from release_workspace_cli import (
    EXIT_OK,
    EXIT_REFUSED,
    Registry,
    SurveyError,
    main,
    plan_publication,
    plan_rehearsal,
)
from workspace_candidate import read_candidate
from workspace_candidate_support import COMMIT, CRATES, VERSION, write_release

if typ.TYPE_CHECKING:
    import pathlib

TAG = f"v{VERSION}"
REFUSED_STATES = (
    RegistryState.CONFLICT,
    RegistryState.YANKED,
    RegistryState.INDETERMINATE,
)


class FakeClock:
    """Advance one millisecond per reading, and count the readings."""

    def __init__(self) -> None:
        """Start before the first reading."""
        self.readings = 0

    def __call__(self) -> int:
        """Return the next reading in nanoseconds."""
        self.readings += 1
        return self.readings * 1_000_000


class Release(typ.NamedTuple):
    """Carry the paths one recorded candidate occupies."""

    root: pathlib.Path
    packages: pathlib.Path
    candidate: pathlib.Path

    def record(self, tag: str) -> int:
        """Record the packaged archives, returning the exit code."""
        return main([
            "record",
            *("--candidate", str(self.candidate), "--commit", COMMIT, "--tag", tag),
            *("--package-directory", str(self.packages)),
            *("--workspace-root", str(self.root)),
        ])

    def digests(self) -> list[str]:
        """Return the recorded digests, in publish order."""
        return [crate.sha256 for crate in read_candidate(self.candidate).crates]

    def survey(self, *outcomes: FakeResponse | BaseException) -> tuple[int, str]:
        """Survey a scripted registry, returning the exit code and outputs."""
        output = self.root / "github-output"
        registry = Registry(FakeOpener(*outcomes), lambda _delay: None, FakeClock())
        arguments = ["survey", "--candidate", str(self.candidate)]
        arguments += ["--github-output", str(output)]
        result = main(arguments, registry=registry)
        return result, output.read_text() if output.exists() else ""

    def verify(
        self, *outcomes: FakeResponse | BaseException, clock: FakeClock | None = None
    ) -> int:
        """Verify against a scripted registry, returning the exit code."""
        registry = Registry(
            FakeOpener(*outcomes), lambda _delay: None, clock or FakeClock()
        )
        return main(["verify", "--candidate", str(self.candidate)], registry=registry)


@pytest.fixture
def release(tmp_path: pathlib.Path) -> Release:
    """Return a packaged workspace whose candidate is not yet recorded."""
    return Release(tmp_path, write_release(tmp_path), tmp_path / "retained" / "candidate.json")


def published(digest: str, *, version: str = VERSION) -> FakeResponse:
    """Return crates.io's answer for one published version."""
    record = {"num": version, "checksum": digest, "yanked": False}
    return FakeResponse(200, json.dumps({"version": record}).encode())


def absent() -> urllib.error.HTTPError:
    """Return crates.io's answer for a version that does not exist."""
    return missing_response("https://crates.io/api/v1/crates/wireframe/1", b"{}")


def test_only_the_recorded_archives_are_retained(release: Release) -> None:
    """An archive left over from another version is not part of the candidate."""
    (release.packages / "wireframe-0.0.1.crate").write_bytes(b"stale")
    release.record(TAG)
    retained = {path.name for path in release.candidate.parent.iterdir()}
    assert retained == {
        "candidate.json",
        *(f"{name}-{VERSION}.crate" for name in CRATES),
    }


def test_an_unpublished_release_is_published(release: Release) -> None:
    """Every crate is absent, so the run publishes the recorded version."""
    assert release.record(TAG) == EXIT_OK
    result, outputs = release.survey(absent(), absent())
    assert (result, outputs) == (EXIT_OK, f"action=publish\nversion={VERSION}\n")


def test_an_interrupted_release_resumes(release: Release) -> None:
    """A published first crate does not stop the second from being published."""
    release.record(TAG)
    result, outputs = release.survey(published(release.digests()[0]), absent())
    assert result == EXIT_OK
    assert "action=publish\n" in outputs


def test_a_completed_release_is_skipped(release: Release) -> None:
    """Re-running a finished release asks for no credential."""
    release.record(TAG)
    result, outputs = release.survey(*map(published, release.digests()))
    assert result == EXIT_OK
    assert "action=skip\n" in outputs


@pytest.mark.parametrize(
    "outcome",
    [
        pytest.param(published("ab" * 32), id="different-bytes"),
        pytest.param(FakeResponse(503, b"{}"), id="server-error"),
        pytest.param(FakeResponse(200, b"\xff"), id="undecodable"),
        pytest.param(FakeResponse(200, b"[]"), id="unexpected-shape"),
        pytest.param(urllib.error.URLError("offline"), id="offline"),
    ],
)
def test_a_release_refuses_anything_but_absence_or_identity(
    release: Release,
    outcome: FakeResponse | BaseException,
    capsys: pytest.CaptureFixture[str],
) -> None:
    """No uncertain or conflicting answer reaches the publishing step."""
    release.record(TAG)
    result, outputs = release.survey(absent(), outcome)
    assert result == EXIT_REFUSED
    assert "action" not in outputs
    assert CRATES[1] in capsys.readouterr().err


def test_a_rehearsal_of_a_published_version_is_skipped(release: Release) -> None:
    """A pull request before the version bump has nothing to rehearse."""
    release.record("")
    result, outputs = release.survey(published("ab" * 32), published("cd" * 32))
    assert result == EXIT_OK
    assert "action=skip\n" in outputs


@pytest.mark.parametrize(
    "states", list(itertools.product(RegistryState, repeat=2))
)
def test_the_plans_agree_except_on_refusal(
    states: tuple[RegistryState, RegistryState],
) -> None:
    """A rehearsal differs from a release only where a release would refuse."""
    named = dict(zip(CRATES, states, strict=True))
    if any(state in REFUSED_STATES for state in states):
        with pytest.raises(SurveyError):
            plan_publication(named)
        assert plan_rehearsal(named) == "skip"
    else:
        assert plan_rehearsal(named) == plan_publication(named)


def test_published_bytes_that_match_are_verified(release: Release) -> None:
    """Each archive is downloaded and compared with its recorded digest."""
    release.record(TAG)
    archives = [
        FakeResponse(200, (release.packages / crate.filename).read_bytes())
        for crate in read_candidate(release.candidate).crates
    ]
    assert release.verify(*archives) == EXIT_OK


def test_verification_reads_only_the_injected_clock(
    release: Release, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Download budgets are timed by the registry's clock, not the real one."""
    release.record(TAG)
    archives = [
        FakeResponse(200, (release.packages / crate.filename).read_bytes())
        for crate in read_candidate(release.candidate).crates
    ]

    def real_clock() -> int:
        """Fail the test if verification reads the real clock."""
        pytest.fail("verification read the real monotonic clock")

    monkeypatch.setattr(time, "monotonic_ns", real_clock)
    clock = FakeClock()
    assert release.verify(*archives, clock=clock) == EXIT_OK
    assert clock.readings > 0


@pytest.mark.parametrize(
    "outcome",
    [
        pytest.param(FakeResponse(200, b"different"), id="different-bytes"),
        pytest.param(FakeResponse(422, b""), id="unusable-status"),
    ],
)
def test_published_bytes_that_differ_fail_verification(
    release: Release, outcome: FakeResponse
) -> None:
    """A release is not complete until the registry serves the candidate."""
    release.record(TAG)
    first = read_candidate(release.candidate).crates[0]
    archive = FakeResponse(200, (release.packages / first.filename).read_bytes())
    assert release.verify(archive, outcome) == EXIT_REFUSED


def test_the_check_command_refuses_a_rebuilt_archive(release: Release) -> None:
    """The publishing job repeats the check before it asks for a credential."""
    release.record(TAG)
    arguments = ["check", "--candidate", str(release.candidate), "--tag", TAG]
    arguments += ["--commit", COMMIT, "--package-directory", str(release.packages)]
    assert main(arguments) == EXIT_OK
    (release.packages / f"{CRATES[0]}-{VERSION}.crate").write_bytes(b"rebuilt")
    assert main(arguments) == EXIT_REFUSED


def test_a_tag_for_another_version_records_nothing(release: Release) -> None:
    """A mistyped tag stops before any candidate exists."""
    assert release.record("v9.9.9") == EXIT_REFUSED
    assert not release.candidate.exists()


def _stub_download(
    monkeypatch: pytest.MonkeyPatch, release: Release, *, status: str, result: int
) -> None:
    """Replace the download with one that leaves the recorded bytes behind."""

    def download(request: crates_io_transport_cli.DownloadRequest) -> int:
        """Write the candidate's own archive and ``status``, then return ``result``."""
        retained = release.packages / request.archive_path.name
        request.archive_path.write_bytes(retained.read_bytes())
        request.status_path.write_text(status, encoding="ascii")
        return result

    monkeypatch.setattr(crates_io_transport_cli, "download", download)


def test_a_failed_download_refuses_verification_by_its_result(
    release: Release,
    monkeypatch: pytest.MonkeyPatch,
    capsys: pytest.CaptureFixture[str],
) -> None:
    """A failing download is refused even when it left matching bytes behind."""
    release.record(TAG)
    _stub_download(
        monkeypatch,
        release,
        status="200\n",
        result=crates_io_transport_cli.EXIT_TRANSPORT_ERROR,
    )

    assert release.verify() == EXIT_REFUSED, "a failed download must not verify"
    assert "the download failed with exit code" in capsys.readouterr().err, (
        "name the failed download, not a digest mismatch"
    )


def test_a_download_without_a_numeric_status_is_refused(
    release: Release,
    monkeypatch: pytest.MonkeyPatch,
    capsys: pytest.CaptureFixture[str],
) -> None:
    """A status file that is not a number is a refusal, not a traceback."""
    release.record(TAG)
    _stub_download(
        monkeypatch, release, status="garbage\n", result=crates_io_transport_cli.EXIT_OK
    )

    assert release.verify() == EXIT_REFUSED, "an unreadable status must not verify"
    assert "recorded no HTTP status" in capsys.readouterr().err, (
        "name the unreadable status"
    )
