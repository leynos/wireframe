//! Shared helpers for formal-tooling metadata tests.

use std::env;

use camino::Utf8PathBuf;
use cap_std::{ambient_authority, fs_utf8::Dir};

pub(crate) type FormalToolingResult<T = ()> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub(crate) const KANI_VERSION_PATH: &str = "tools/kani/VERSION";
pub(crate) const VERUS_VERSION_PATH: &str = "tools/verus/VERSION";
pub(crate) const VERUS_CHECKSUMS_PATH: &str = "tools/verus/SHA256SUMS";
pub(crate) const PROVER_TOOLS_REF_PATH: &str = "tools/rust-prover-tools/REF";
pub(crate) const MAKEFILE_PATH: &str = "Makefile";

pub(crate) fn repo_root() -> FormalToolingResult<Utf8PathBuf> {
    Utf8PathBuf::from_path_buf(env::current_dir()?).map_err(|path| {
        format!(
            "repository root path is not valid UTF-8: {}",
            path.display()
        )
        .into()
    })
}

pub(crate) fn repo_dir() -> FormalToolingResult<Dir> {
    Ok(Dir::open_ambient_dir(repo_root()?, ambient_authority())?)
}

pub(crate) fn read_repo_file(path: &str) -> FormalToolingResult<String> {
    Ok(repo_dir()?.read_to_string(path)?)
}

pub(crate) fn read_trimmed_repo_file(path: &str) -> FormalToolingResult<String> {
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

pub(crate) fn is_three_part_numeric_version(version: &str) -> bool {
    let mut parts = version.split('.');
    let has_three_numeric_parts = parts
        .by_ref()
        .take(3)
        .all(|part| !part.is_empty() && part.chars().all(|character| character.is_ascii_digit()));
    has_three_numeric_parts && parts.next().is_none()
}

pub(crate) fn verus_linux_archive_name(version: &str) -> String {
    format!("verus-{version}-x86-linux.zip")
}

pub(crate) fn checksums_contain_archive(checksums: &str, archive_name: &str) -> bool {
    checksums.lines().any(|line| {
        let mut fields = line.split_whitespace();
        let digest = fields.next();
        let archive = fields.next();
        digest.is_some_and(is_sha256_hex)
            && archive == Some(archive_name)
            && fields.next().is_none()
    })
}

fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64 && value.chars().all(|character| character.is_ascii_hexdigit())
}

pub(crate) fn prover_tools_ref_value(metadata: &str) -> Option<&str> {
    metadata
        .lines()
        .find_map(|line| line.strip_prefix("ref: ").map(str::trim))
}

pub(crate) fn has_phony_target(makefile: &str, target: &str) -> bool {
    makefile.lines().any(|line| {
        line.strip_prefix(".PHONY:").is_some_and(|targets| {
            targets
                .split_whitespace()
                .any(|candidate| candidate == target)
        })
    })
}

pub(crate) fn target_recipe(makefile: &str, target: &str) -> Option<String> {
    let target_prefix = format!("{target}:");
    let mut lines = makefile
        .lines()
        .skip_while(|line| !line.starts_with(&target_prefix));
    lines.next()?;
    let recipe_lines = lines
        .take_while(|line| line.is_empty() || line.starts_with('\t'))
        .collect::<Vec<_>>();
    Some(recipe_lines.join("\n"))
}
