"""Describe the crates one release candidate would publish, and bind their bytes.

Packaging happens once, on a read-only runner. The record written there names
every crate in publish order with the digest of its archive, and the commit
those bytes came from. Later jobs compare against the record, so a different
commit, a rebuilt archive, or a missing crate stops the release.

Behavioural coverage lives in
``scripts/release/tests/test_workspace_candidate.py``.
"""

import dataclasses
import gzip
import json
import pathlib
import re
import shutil
import tarfile
import tomllib
import typing as typ
import zlib

import release_digest
import release_digest_files

_COMMIT = re.compile(r"[0-9a-f]{40}")
_VCS_INFO = ".cargo_vcs_info.json"

#: What a damaged ``.crate`` can raise while it is opened, decompressed, or its
#: VCS record decoded. Each becomes a :class:`CandidateError` naming the file.
_UNREADABLE_ARCHIVE = (
    tarfile.TarError,
    gzip.BadGzipFile,
    zlib.error,
    EOFError,
    json.JSONDecodeError,
    UnicodeError,
)


class CandidateError(ValueError):
    """Report a candidate that cannot be recorded or no longer matches."""


@dataclasses.dataclass(frozen=True, slots=True)
class CrateArchive:
    """Name one packaged crate and the digest of its archive."""

    name: str
    version: str
    sha256: str

    @property
    def filename(self) -> str:
        """Return the name Cargo gives the archive.

        Examples
        --------
        >>> CrateArchive("wireframe", "1.0.0", "ab").filename
        'wireframe-1.0.0.crate'
        """
        return f"{self.name}-{self.version}.crate"


@dataclasses.dataclass(frozen=True, slots=True)
class Candidate:
    """Carry the commit, the tag, and the crates of one release candidate.

    ``tag`` is empty for a rehearsal, which has no tag to publish.
    """

    commit: str
    tag: str
    crates: tuple[CrateArchive, ...]

    @property
    def version(self) -> str:
        """Return the one version every crate in the workspace release carries."""
        return self.crates[0].version


@dataclasses.dataclass(frozen=True, slots=True)
class Expectation:
    """Name the commit and tag a run is entitled to release."""

    commit: str
    tag: str


def _load_toml(path: pathlib.Path) -> dict[str, typ.Any]:
    """Parse one TOML file.

    Raises
    ------
    CandidateError
        If the file is not UTF-8 or not valid TOML.
    """
    try:
        return tomllib.loads(path.read_text(encoding="utf-8"))
    except (tomllib.TOMLDecodeError, UnicodeError) as error:
        message = f"{path.name} is not valid TOML: {error}"
        raise CandidateError(message) from error


def publish_order(workspace_root: pathlib.Path) -> tuple[str, ...]:
    """Return the crates to publish, in the order ``lading.toml`` gives.

    Raises
    ------
    CandidateError
        If ``lading.toml`` is not valid TOML, or does not declare
        ``publish.order`` as a non-empty list of crate names.
    """
    publish = _load_toml(workspace_root / "lading.toml").get("publish", {})
    order = publish.get("order") if isinstance(publish, dict) else None
    if not _is_crate_list(order):
        message = "lading.toml must declare publish.order as a list of crate names"
        raise CandidateError(message)
    return tuple(order)


def _is_crate_list(order: object) -> typ.TypeIs[list[str]]:
    """Return whether ``order`` is a non-empty list of names.

    Examples
    --------
    >>> _is_crate_list(["wireframe"]), _is_crate_list([]), _is_crate_list("a")
    (True, False, False)
    """
    return (
        isinstance(order, list)
        and bool(order)
        and all(isinstance(name, str) for name in order)
    )


def workspace_version(workspace_root: pathlib.Path) -> str:
    """Return the version the root package declares for the workspace release.

    Raises
    ------
    CandidateError
        If ``Cargo.toml`` is not valid TOML, or declares no string
        ``package.version``.
    """
    manifest = _load_toml(workspace_root / "Cargo.toml")
    try:
        version = manifest["package"]["version"]
    except (KeyError, TypeError) as error:
        message = "Cargo.toml declares no package.version"
        raise CandidateError(message) from error
    if not isinstance(version, str):
        message = "Cargo.toml declares package.version as something other than text"
        raise CandidateError(message)
    return version


def archive_commit(archive: pathlib.Path) -> str:
    """Return the commit Cargo recorded inside a package archive.

    Parameters
    ----------
    archive
        A ``.crate`` file written by ``cargo package``.

    Returns
    -------
    str
        The commit the package was built from.

    Raises
    ------
    CandidateError
        If the archive is not a readable gzipped tarball, records no commit,
        carries a VCS record that is not JSON, or was packaged from a tree with
        uncommitted changes.
    """
    try:
        git = _recorded_git(archive)
    except _UNREADABLE_ARCHIVE as error:
        message = f"{archive.name} is not a readable package archive: {error}"
        raise CandidateError(message) from error
    if git.get("dirty"):
        message = f"{archive.name} was packaged from uncommitted changes"
        raise CandidateError(message)
    return str(git.get("sha1", ""))


def _recorded_git(archive: pathlib.Path) -> dict[str, typ.Any]:
    """Return the ``git`` table of an archive's VCS record.

    Raises
    ------
    CandidateError
        If the archive holds no VCS record, or the record has no ``git`` table.
    """
    member = f"{archive.name.removesuffix('.crate')}/{_VCS_INFO}"
    with tarfile.open(archive, "r:gz") as package:
        try:
            recorded = package.extractfile(member)
        except KeyError:
            recorded = None
        if recorded is None:
            message = f"{archive.name} records no commit"
            raise CandidateError(message)
        record = json.load(recorded)
    git = record.get("git", {}) if isinstance(record, dict) else None
    if not isinstance(git, dict):
        message = f"{archive.name} records no commit"
        raise CandidateError(message)
    return git


def require_expectation(expected: Expectation, version: str) -> None:
    """Refuse a vague commit, or a tag that names a different version.

    Examples
    --------
    >>> require_expectation(Expectation("a" * 40, "v1.0.0"), "1.0.0")
    >>> require_expectation(Expectation("main", ""), "1.0.0")
    Traceback (most recent call last):
        ...
    workspace_candidate.CandidateError: the commit must be a full lowercase SHA-1
    """
    if _COMMIT.fullmatch(expected.commit) is None:
        message = "the commit must be a full lowercase SHA-1"
        raise CandidateError(message)
    if expected.tag not in {"", f"v{version}"}:
        message = f"tag {expected.tag!r} does not name version {version}"
        raise CandidateError(message)


def record_candidate(
    workspace_root: pathlib.Path,
    package_directory: pathlib.Path,
    expected: Expectation,
) -> Candidate:
    """Describe the archives ``cargo package`` left behind.

    Parameters
    ----------
    workspace_root
        Checkout holding ``lading.toml`` and the root manifest.
    package_directory
        Directory holding the ``.crate`` files.
    expected
        Commit and tag the archives must belong to.

    Returns
    -------
    Candidate
        Every crate in publish order, with its archive's digest.

    Raises
    ------
    CandidateError
        If a manifest is malformed, or an archive is missing, unreadable, or
        packaged from another commit.
    """
    version = workspace_version(workspace_root)
    require_expectation(expected, version)
    crates = []
    for name in publish_order(workspace_root):
        archive = package_directory / f"{name}-{version}.crate"
        if not archive.is_file():
            message = f"{archive.name} was not packaged"
            raise CandidateError(message)
        commit = archive_commit(archive)
        if commit != expected.commit:
            message = f"{archive.name} was packaged from {commit or 'no commit'}"
            raise CandidateError(message)
        digest = release_digest_files.digest_of_file(archive)
        crates.append(CrateArchive(name, version, digest))
    return Candidate(expected.commit, expected.tag, tuple(crates))


def retain_archives(
    candidate: Candidate, package_directory: pathlib.Path, destination: pathlib.Path
) -> None:
    """Copy exactly the recorded archives into ``destination``.

    A build directory may hold archives from earlier versions, so later jobs
    receive the recorded files rather than everything that matches a pattern.
    """
    destination.mkdir(parents=True, exist_ok=True)
    for crate in candidate.crates:
        shutil.copyfile(
            package_directory / crate.filename, destination / crate.filename
        )


def check_candidate(
    candidate: Candidate, package_directory: pathlib.Path, expected: Expectation
) -> None:
    """Require a record and the archives beside it to be this run's candidate.

    Raises
    ------
    CandidateError
        If the record names another commit or tag, or an archive is missing or
        does not have the recorded digest.
    """
    require_expectation(expected, candidate.version)
    if (candidate.commit, candidate.tag) != (expected.commit, expected.tag):
        message = "the candidate record belongs to another commit or tag"
        raise CandidateError(message)
    for crate in candidate.crates:
        archive = package_directory / crate.filename
        if not archive.is_file():
            message = f"{crate.filename} is missing"
            raise CandidateError(message)
        digest = release_digest_files.digest_of_file(archive)
        if not release_digest.constant_time_equal(digest, crate.sha256):
            message = f"{crate.filename} is {digest}, not the recorded {crate.sha256}"
            raise CandidateError(message)


def write_candidate(candidate: Candidate, path: pathlib.Path) -> None:
    """Write the record as sorted, newline-terminated JSON."""
    record = json.dumps(dataclasses.asdict(candidate), indent=2, sort_keys=True)
    path.write_text(record + "\n", encoding="utf-8")


def read_candidate(path: pathlib.Path) -> Candidate:
    """Read a record written by :func:`write_candidate`.

    Raises
    ------
    CandidateError
        If the file is not a candidate record naming at least one crate.
    """
    try:
        record = json.loads(path.read_text(encoding="utf-8"))
        crates = tuple(CrateArchive(**crate) for crate in record["crates"])
        candidate = Candidate(record["commit"], record["tag"], crates)
    except (KeyError, TypeError, UnicodeError, json.JSONDecodeError) as error:
        message = f"{path.name} is not a candidate record"
        raise CandidateError(message) from error
    if not crates:
        message = f"{path.name} names no crates"
        raise CandidateError(message)
    return candidate
