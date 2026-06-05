//! Shared helpers for formal-tooling metadata tests.

use crate::repo_access::read_repo_file;

pub(crate) type FormalToolingResult<T = ()> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub(crate) const KANI_VERSION_PATH: &str = "tools/kani/VERSION";
pub(crate) const VERUS_VERSION_PATH: &str = "tools/verus/VERSION";
pub(crate) const VERUS_CHECKSUMS_PATH: &str = "tools/verus/SHA256SUMS";
pub(crate) const PROVER_TOOLS_REF_PATH: &str = "tools/rust-prover-tools/REF";
pub(crate) const MAKEFILE_PATH: &str = "Makefile";

pub(crate) struct MakefileContent<'a>(pub(crate) &'a str);

impl MakefileContent<'_> {
    pub(crate) fn has_phony_target(&self, target: impl AsRef<str>) -> bool {
        let target = target.as_ref();
        self.0.lines().any(|line| {
            line.strip_prefix(".PHONY:").is_some_and(|targets| {
                targets
                    .split_whitespace()
                    .any(|candidate| candidate == target)
            })
        })
    }

    pub(crate) fn target_recipe(&self, target: impl AsRef<str>) -> Option<String> {
        let target = target.as_ref();
        let target_prefix = format!("{target}:");
        let mut lines = self
            .0
            .lines()
            .skip_while(|line| !line.starts_with(&target_prefix));
        lines.next()?;
        let recipe_lines = lines
            .take_while(|line| line.is_empty() || line.starts_with('\t'))
            .collect::<Vec<_>>();
        Some(recipe_lines.join("\n"))
    }
}

pub(crate) struct ChecksumsContent<'a>(pub(crate) &'a str);

impl ChecksumsContent<'_> {
    pub(crate) fn contains_archive(&self, archive_name: impl AsRef<str>) -> bool {
        let archive_name = archive_name.as_ref();
        self.0.lines().any(|line| {
            let mut fields = line.split_whitespace();
            let digest = fields.next();
            let archive = fields.next();
            digest.is_some_and(is_sha256_hex)
                && archive == Some(archive_name)
                && fields.next().is_none()
        })
    }
}

pub(crate) struct ProverToolsRef<'a>(pub(crate) &'a str);

impl ProverToolsRef<'_> {
    pub(crate) fn ref_value(&self) -> Option<&str> {
        self.0
            .lines()
            .find_map(|line| line.strip_prefix("ref: ").map(str::trim))
    }

    pub(crate) fn as_str(&self) -> &str { self.0 }
}

pub(crate) fn read_trimmed_repo_file(
    path: impl AsRef<camino::Utf8Path>,
) -> FormalToolingResult<String> {
    Ok(read_repo_file(path)?.trim().to_owned())
}

pub(crate) fn kani_version() -> FormalToolingResult<String> {
    read_trimmed_repo_file(KANI_VERSION_PATH)
}

pub(crate) fn verus_version() -> FormalToolingResult<String> {
    read_trimmed_repo_file(VERUS_VERSION_PATH)
}

pub(crate) fn verus_checksums() -> FormalToolingResult<String> {
    read_repo_file(VERUS_CHECKSUMS_PATH)
}

pub(crate) fn prover_tools_ref_metadata() -> FormalToolingResult<String> {
    read_repo_file(PROVER_TOOLS_REF_PATH)
}

pub(crate) fn makefile() -> FormalToolingResult<String> { read_repo_file(MAKEFILE_PATH) }

pub(crate) fn is_three_part_numeric_version(version: impl AsRef<str>) -> bool {
    let version = version.as_ref();
    let parts: Vec<&str> = version.split('.').collect();
    parts.len() == 3
        && parts.iter().all(|part| {
            !part.is_empty() && part.chars().all(|character| character.is_ascii_digit())
        })
}

pub(crate) fn verus_linux_archive_name(version: impl AsRef<str>) -> String {
    let version = version.as_ref();
    format!("verus-{version}-x86-linux.zip")
}

pub(crate) fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64 && value.chars().all(|character| character.is_ascii_hexdigit())
}
