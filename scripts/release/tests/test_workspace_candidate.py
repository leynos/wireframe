"""Specify how a release candidate is recorded, checked, and refused."""

import gzip
import hashlib
import io
import tarfile
import typing as typ

import pytest
from hypothesis import given
from hypothesis import strategies as st
from workspace_candidate import (
    Candidate,
    CandidateError,
    CrateArchive,
    Expectation,
    archive_commit,
    check_candidate,
    publish_order,
    read_candidate,
    record_candidate,
    require_expectation,
    workspace_version,
    write_candidate,
)
from workspace_candidate_support import (
    COMMIT,
    CRATES,
    OTHER_COMMIT,
    VERSION,
    write_archive,
    write_release,
    write_workspace,
)

if typ.TYPE_CHECKING:
    import collections.abc as cabc
    import pathlib

TAGGED = Expectation(COMMIT, f"v{VERSION}")
REHEARSAL = Expectation(COMMIT, "")


def test_the_record_names_every_crate_in_publish_order(tmp_path: pathlib.Path) -> None:
    """Each crate carries the release version and its own archive's digest."""
    packages = write_release(tmp_path)

    candidate = record_candidate(tmp_path, packages, TAGGED)

    assert [crate.name for crate in candidate.crates] == list(CRATES)
    assert (candidate.commit, candidate.tag) == (COMMIT, f"v{VERSION}")
    for crate in candidate.crates:
        contents = (packages / crate.filename).read_bytes()
        assert crate.sha256 == hashlib.sha256(contents).hexdigest()


def test_a_missing_archive_is_refused(tmp_path: pathlib.Path) -> None:
    """A crate that was not packaged cannot be left out of the release."""
    packages = write_release(tmp_path)
    (packages / f"{CRATES[1]}-{VERSION}.crate").unlink()

    with pytest.raises(CandidateError, match="was not packaged"):
        record_candidate(tmp_path, packages, TAGGED)


@pytest.mark.parametrize(
    ("git", "reason"),
    [
        pytest.param({"sha1": OTHER_COMMIT}, "was packaged from", id="other-commit"),
        pytest.param({}, "was packaged from no commit", id="no-sha"),
        pytest.param(
            {"sha1": COMMIT, "dirty": True}, "uncommitted changes", id="dirty-tree"
        ),
        pytest.param(None, "records no commit", id="no-record"),
    ],
)
def test_an_archive_from_anything_but_the_commit_is_refused(
    tmp_path: pathlib.Path, git: dict[str, object] | None, reason: str
) -> None:
    """The bytes must come from the exact commit being released."""
    packages = write_release(tmp_path)
    write_archive(packages, CRATES[0], git)

    with pytest.raises(CandidateError, match=reason):
        record_candidate(tmp_path, packages, TAGGED)


@pytest.mark.parametrize("commit", ["", "main", COMMIT[:12], COMMIT.upper()])
def test_a_vague_commit_is_refused(commit: str) -> None:
    """Two equally vague names would compare equal and prove nothing."""
    with pytest.raises(CandidateError, match="full lowercase SHA-1"):
        require_expectation(Expectation(commit, ""), VERSION)


@given(version=st.from_regex(r"[0-9]\.[0-9]\.[0-9](-[a-z]+[0-9])?", fullmatch=True))
def test_a_tag_must_name_the_workspace_version(version: str) -> None:
    """Only the version's own tag, or no tag at all, is accepted."""
    require_expectation(Expectation(COMMIT, ""), version)
    require_expectation(Expectation(COMMIT, f"v{version}"), version)
    for tag in (version, f"v{version}.0", "v"):
        with pytest.raises(CandidateError, match="does not name version"):
            require_expectation(Expectation(COMMIT, tag), version)


def test_an_unchanged_candidate_passes_the_check(tmp_path: pathlib.Path) -> None:
    """A record read back from disk still matches its archives."""
    packages = write_release(tmp_path)
    path = tmp_path / "candidate.json"
    write_candidate(record_candidate(tmp_path, packages, TAGGED), path)

    check_candidate(read_candidate(path), packages, TAGGED)


@pytest.mark.parametrize(
    "expected",
    [
        pytest.param(Expectation(OTHER_COMMIT, f"v{VERSION}"), id="other-commit"),
        pytest.param(REHEARSAL, id="rehearsal-record-for-a-release"),
    ],
)
def test_a_record_for_another_run_is_refused(
    tmp_path: pathlib.Path, expected: Expectation
) -> None:
    """A rehearsal's record can never stand in for a release's."""
    packages = write_release(tmp_path)
    candidate = record_candidate(tmp_path, packages, TAGGED)

    with pytest.raises(CandidateError, match="another commit or tag"):
        check_candidate(candidate, packages, expected)


def test_changed_bytes_are_refused(tmp_path: pathlib.Path) -> None:
    """A rebuilt archive that differs is not the candidate."""
    packages = write_release(tmp_path)
    candidate = record_candidate(tmp_path, packages, TAGGED)
    write_archive(packages, CRATES[1], {"sha1": COMMIT}, payload=b"changed")

    with pytest.raises(CandidateError, match="not the recorded"):
        check_candidate(candidate, packages, TAGGED)


def test_a_missing_archive_fails_the_check(tmp_path: pathlib.Path) -> None:
    """Every recorded crate must still be present."""
    packages = write_release(tmp_path)
    candidate = record_candidate(tmp_path, packages, TAGGED)
    (packages / candidate.crates[0].filename).unlink()

    with pytest.raises(CandidateError, match="is missing"):
        check_candidate(candidate, packages, TAGGED)


@pytest.mark.parametrize(
    "contents", ["", "[]", "{}", '{"commit": "a", "tag": "", "crates": [{}]}']
)
def test_a_malformed_record_is_refused(tmp_path: pathlib.Path, contents: str) -> None:
    """A truncated or foreign file is never read as a candidate."""
    path = tmp_path / "candidate.json"
    path.write_text(contents)

    with pytest.raises(CandidateError, match="not a candidate record"):
        read_candidate(path)


def test_a_record_without_crates_is_refused(tmp_path: pathlib.Path) -> None:
    """An empty release would verify vacuously."""
    path = tmp_path / "candidate.json"
    write_candidate(Candidate(COMMIT, "", ()), path)

    with pytest.raises(CandidateError, match="names no crates"):
        read_candidate(path)


def test_a_workspace_without_an_order_is_refused(tmp_path: pathlib.Path) -> None:
    """The release order is never guessed."""
    write_workspace(tmp_path)
    (tmp_path / "lading.toml").write_text("[preflight]\n")

    with pytest.raises(CandidateError, match=r"publish\.order"):
        publish_order(tmp_path)


def test_the_filename_is_the_one_cargo_writes() -> None:
    """The archive name joins the crate and its version."""
    assert CrateArchive("a_b", "1.0.0", "").filename == "a_b-1.0.0.crate"


def _non_json_vcs_record(path: pathlib.Path) -> None:
    """Write an archive whose VCS record is not JSON."""
    stem = path.name.removesuffix(".crate")
    member = tarfile.TarInfo(f"{stem}/.cargo_vcs_info.json")
    contents = b"{not json"
    member.size = len(contents)
    with tarfile.open(path, "w:gz") as package:
        package.addfile(member, io.BytesIO(contents))


def _truncated_archive(path: pathlib.Path) -> None:
    """Cut a valid archive short, as an interrupted copy would."""
    whole = write_archive(path.parent, CRATES[0], {"sha1": COMMIT}).read_bytes()
    path.write_bytes(whole[: len(whole) // 2])


@pytest.mark.parametrize(
    "damage",
    [
        pytest.param(lambda path: path.write_bytes(b"not an archive"), id="not-gzip"),
        pytest.param(
            lambda path: path.write_bytes(gzip.compress(b"not a tarball")),
            id="not-tar",
        ),
        pytest.param(_truncated_archive, id="truncated"),
        pytest.param(_non_json_vcs_record, id="vcs-record-not-json"),
    ],
)
def test_a_corrupt_archive_is_refused(
    tmp_path: pathlib.Path, damage: cabc.Callable[[pathlib.Path], object]
) -> None:
    """A damaged ``.crate`` is a refused candidate, not a traceback."""
    archive = tmp_path / f"{CRATES[0]}-{VERSION}.crate"
    damage(archive)

    with pytest.raises(CandidateError, match="not a readable package archive"):
        archive_commit(archive)


@pytest.mark.parametrize(
    "contents",
    [
        pytest.param("[publish\n", id="invalid-toml"),
        pytest.param('publish = "wireframe"\n', id="publish-not-a-table"),
        pytest.param('[publish]\norder = "wireframe"\n', id="order-not-a-list"),
        pytest.param("[publish]\norder = [1, 2]\n", id="order-not-names"),
    ],
)
def test_a_malformed_lading_manifest_is_refused(
    tmp_path: pathlib.Path, contents: str
) -> None:
    """A manifest lading could not read is never read as a release order."""
    write_workspace(tmp_path)
    (tmp_path / "lading.toml").write_text(contents)

    with pytest.raises(CandidateError, match=r"lading\.toml"):
        publish_order(tmp_path)


@pytest.mark.parametrize(
    "contents",
    [
        pytest.param('[package]\nname = "wireframe"\n', id="no-version"),
        pytest.param('package = "wireframe"\n', id="package-not-a-table"),
        pytest.param("[workspace]\n", id="no-package"),
        pytest.param("[package]\nversion = 1\n", id="version-not-text"),
        pytest.param("[package\n", id="invalid-toml"),
    ],
)
def test_a_manifest_without_a_package_version_is_refused(
    tmp_path: pathlib.Path, contents: str
) -> None:
    """The release version is read from the root package, never guessed."""
    (tmp_path / "Cargo.toml").write_text(contents)

    with pytest.raises(CandidateError, match=r"Cargo\.toml"):
        workspace_version(tmp_path)
