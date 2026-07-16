"""Test exact phrase-policy enforcement."""

import importlib
from pathlib import Path
import subprocess
import types

from hypothesis import HealthCheck, given, settings, strategies as st
import pytest

SCRIPTS = Path(__file__).resolve().parents[1]
PROHIBITED = "hand" + "-written"
TITLE_PROHIBITED = "Hand" + "-written"


@pytest.fixture
def checker(monkeypatch: pytest.MonkeyPatch) -> types.ModuleType:
    """Import the standalone phrase checker from the scripts directory."""
    monkeypatch.syspath_prepend(str(SCRIPTS))
    importlib.invalidate_caches()
    return importlib.import_module("typos_rollout_check")


def initialize(path: Path, files: dict[str, str]) -> None:
    """Create and stage a small tracked-file fixture."""
    for relative, content in files.items():
        target = path / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(content)
    subprocess.run(["git", "init", "--quiet"], cwd=path, check=True)
    subprocess.run(["git", "add", "."], cwd=path, check=True)


def policy_files(*, local_phrase: str = "") -> dict[str, str]:
    """Return minimal generated, shared, and local policy documents."""
    return {
        "typos.toml": (
            f"# Policy for {PROHIBITED} corrections.\n"
            '[files]\nextend-exclude = ["*.md", "!README.md"]\n\n'
            '[default]\nextend-ignore-re = ["`[^`\\\\n]+`"]\n'
        ),
        ".typos-oxendict-base.toml": (
            f'[phrases.corrections]\n"{PROHIBITED}" = "handwritten"\n'
        ),
        "typos.local.toml": local_phrase,
    }


class TestPhrasePolicyChecker:
    """Exercise the phrase checker's policy, scanning, and CLI boundaries."""

    def test_load_policy_combines_phrase_and_generated_scan_policy(
        self, checker: types.ModuleType, tmp_path: Path
    ) -> None:
        """Load shared phrases and generated scan settings."""
        files = policy_files(
            local_phrase='[phrases.corrections]\n"fit-for-purpose" = "suitable"\n'
        )
        initialize(tmp_path, files)

        policy = checker.load_policy(tmp_path)

        assert policy.phrase_corrections == (
            ("fit-for-purpose", "suitable"),
            (PROHIBITED, "handwritten"),
        ), "shared and local phrase corrections were not combined"
        assert policy.ignore_patterns == (r"`[^`\n]+`",), (
            "generated ignore patterns were not loaded"
        )
        assert policy.excluded_files == ("*.md", "!README.md"), (
            "generated file exclusions were not loaded"
        )

        (tmp_path / ".typos-oxendict-base.toml").unlink()
        with pytest.raises(FileNotFoundError, match=r"docs/developers-guide\.md"):
            checker.load_policy(tmp_path)

    def test_checker_preserves_boundaries_masking_and_exclusions(
        self, checker: types.ModuleType, tmp_path: Path
    ) -> None:
        """Report phrases only when boundaries and policy allow them."""
        initialize(
            tmp_path,
            {
                "README.md": (
                    f"{PROHIBITED}\n{TITLE_PROHIBITED} prose\n"
                    + "pre-hand"
                    + "-written\n"
                    + f"`{PROHIBITED}`\n"
                ),
                "skip.md": f"{PROHIBITED}\n",
                **policy_files(),
            },
        )

        findings = checker.check_phrase_corrections(
            tmp_path, checker.load_policy(tmp_path)
        )

        assert [(item.line, item.phrase) for item in findings] == [
            (1, PROHIBITED),
            (2, TITLE_PROHIBITED),
        ], "phrase boundaries, masking, or exclusions changed"

    def test_checker_skips_tracked_symlinks(
        self, checker: types.ModuleType, tmp_path: Path
    ) -> None:
        """Do not scan a tracked symlink whose target is outside the repository."""
        initialize(tmp_path, policy_files())
        external = tmp_path.parent / f"{tmp_path.name}-external.txt"
        external.write_text(f"{PROHIBITED}\n")
        (tmp_path / "external.txt").symlink_to(external)
        subprocess.run(["git", "add", "external.txt"], cwd=tmp_path, check=True)

        findings = checker.check_phrase_corrections(
            tmp_path, checker.load_policy(tmp_path)
        )

        assert findings == (), "a tracked symlink escaped the repository boundary"

    def test_checker_fails_when_tracked_text_disappears(
        self, checker: types.ModuleType, tmp_path: Path
    ) -> None:
        """Surface a tracked file that cannot be read instead of skipping it."""
        initialize(
            tmp_path,
            {"README.md": "Readable before removal.\n", **policy_files()},
        )
        (tmp_path / "README.md").unlink()

        with pytest.raises(FileNotFoundError, match="README.md"):
            checker.check_phrase_corrections(tmp_path, checker.load_policy(tmp_path))

    def test_checker_fails_when_tracked_text_is_not_utf8(
        self, checker: types.ModuleType, tmp_path: Path
    ) -> None:
        """Surface undecodable tracked text instead of silently skipping it."""
        initialize(tmp_path, {"README.md": "Initially UTF-8.\n", **policy_files()})
        (tmp_path / "README.md").write_bytes(b"\xff\xfe")

        with pytest.raises(UnicodeDecodeError):
            checker.check_phrase_corrections(tmp_path, checker.load_policy(tmp_path))

    def test_main_reports_location_and_exit_two(
        self,
        checker: types.ModuleType,
        tmp_path: Path,
        capsys: pytest.CaptureFixture[str],
    ) -> None:
        """Return two and preserve the established path diagnostic."""
        initialize(
            tmp_path,
            {"README.md": f"Prefer {PROHIBITED}.\n", **policy_files()},
        )

        assert checker.main(["--repository", str(tmp_path)]) == 2, (
            "the command accepted a prohibited phrase"
        )
        assert capsys.readouterr().out == (
            f"README.md:1:8: {PROHIBITED} -> handwritten\n"
        ), "the diagnostic omitted its source location or correction"


BOUNDARY_CHARACTERS = st.sampled_from(("", " ", "\n", ".", "a", "_", "-"))


@given(left=BOUNDARY_CHARACTERS, right=BOUNDARY_CHARACTERS)
# Each example uses the same immutable imported module, so fixture reuse is safe.
@settings(suppress_health_check=[HealthCheck.function_scoped_fixture])
def test_phrase_boundaries_hold_for_generated_neighbours(
    checker: types.ModuleType, left: str, right: str
) -> None:
    """Recognize a prohibited phrase exactly when both boundaries permit it."""
    text = f"{left}{PROHIBITED}{right}"
    findings = tuple(
        checker._phrase_findings(
            Path("README.md"),
            text,
            checker._masked(text, ()),
            ((PROHIBITED, "handwritten"),),
        )
    )
    has_valid_boundaries = left not in {"a", "_", "-"} and right not in {
        "a",
        "_",
        "-",
    }

    assert bool(findings) is has_valid_boundaries
    if findings:
        finding = findings[0]
        assert (finding.line, finding.column) == (
            text.count("\n", 0, len(left)) + 1,
            len(left.rsplit("\n", maxsplit=1)[-1]) + 1,
        )


@given(content=st.text(alphabet=" abcdefghijklmnopqrstuvwxyz-", max_size=40))
# Each example uses the same immutable imported module, so fixture reuse is safe.
@settings(suppress_health_check=[HealthCheck.function_scoped_fixture])
def test_generated_ignored_spans_remain_masked(
    checker: types.ModuleType, content: str
) -> None:
    """Prevent prohibited phrases inside generated ignored spans from leaking."""
    text = f"`{content}{PROHIBITED}{content}`"
    masked = checker._masked(text, (r"`[^`\n]+`",))

    assert len(masked) == len(text), "masking changed source offsets"
    assert (
        tuple(
            checker._phrase_findings(
                Path("README.md"),
                text,
                masked,
                ((PROHIBITED, "handwritten"),),
            )
        )
        == ()
    ), "an ignored phrase escaped masking"
