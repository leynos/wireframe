"""Build workspaces and packaged archives for the candidate tests."""

import io
import json
import tarfile
import typing as typ

if typ.TYPE_CHECKING:
    import pathlib

COMMIT = "0123456789abcdef0123456789abcdef01234567"
OTHER_COMMIT = "f" * 40
VERSION = "1.2.3-alpha1"
CRATES = ("wireframe", "wireframe_testing")


def write_workspace(root: pathlib.Path, crates: tuple[str, ...] = CRATES) -> None:
    """Write the two files the candidate reads from a checkout."""
    order = ", ".join(f'"{name}"' for name in crates)
    (root / "lading.toml").write_text(f"[publish]\norder = [{order}]\n")
    (root / "Cargo.toml").write_text(
        f'[package]\nname = "wireframe"\nversion = "{VERSION}"\n'
    )


def write_archive(
    directory: pathlib.Path,
    name: str,
    git: dict[str, object] | None = None,
    *,
    payload: bytes = b"source",
) -> pathlib.Path:
    """Write one ``.crate`` whose recorded Git facts are ``git``.

    ``None`` omits the record, as Cargo does outside a repository.
    """
    stem = f"{name}-{VERSION}"
    files = {f"{stem}/src/lib.rs": payload}
    if git is not None:
        files[f"{stem}/.cargo_vcs_info.json"] = json.dumps({"git": git}).encode()
    archive = directory / f"{stem}.crate"
    with tarfile.open(archive, "w:gz") as package:
        for path, contents in files.items():
            member = tarfile.TarInfo(path)
            member.size = len(contents)
            package.addfile(member, io.BytesIO(contents))
    return archive


def write_release(root: pathlib.Path) -> pathlib.Path:
    """Write a workspace with every crate packaged, and return the directory."""
    write_workspace(root)
    packages = root / "package"
    packages.mkdir()
    for name in CRATES:
        write_archive(packages, name, {"sha1": COMMIT}, payload=name.encode())
    return packages
