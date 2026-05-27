//! Fixture world for formal-tooling behavioural scenarios.

use rstest::fixture;

use crate::formal_tooling_support::{
    ChecksumsContent,
    FormalToolingResult as FixtureResult,
    MakefileContent,
    ProverToolsRef,
    is_three_part_numeric_version,
    kani_version,
    makefile,
    prover_tools_ref_metadata,
    verus_checksums,
    verus_linux_archive_name,
    verus_version,
};

pub type TestResult = FixtureResult<()>;

/// BDD world holding formal-verification tooling metadata.
#[derive(Debug, Default)]
pub struct FormalToolingWorld {
    kani_version: Option<String>,
    verus_version: Option<String>,
    verus_checksums: Option<String>,
    prover_tools_ref_metadata: Option<String>,
    makefile: Option<String>,
}

#[rustfmt::skip]
#[fixture]
pub fn formal_tooling_world() -> FormalToolingWorld {
    std::hint::black_box(());
    FormalToolingWorld::default()
}

impl FormalToolingWorld {
    /// Load formal-tooling metadata and the root Makefile for later checks.
    ///
    /// # Errors
    ///
    /// Returns an error when any required repository file cannot be read.
    pub fn load(&mut self) -> TestResult {
        self.kani_version = Some(kani_version()?);
        self.verus_version = Some(verus_version()?);
        self.verus_checksums = Some(verus_checksums()?);
        self.prover_tools_ref_metadata = Some(prover_tools_ref_metadata()?);
        self.makefile = Some(makefile()?);
        Ok(())
    }

    fn required<'a>(value: Option<&'a String>, label: &str) -> Result<&'a str, String> {
        value
            .map(String::as_str)
            .ok_or_else(|| format!("{label} metadata not loaded"))
    }

    fn loaded_kani_version(&self) -> Result<&str, String> {
        Self::required(self.kani_version.as_ref(), "Kani version")
    }

    fn loaded_verus_version(&self) -> Result<&str, String> {
        Self::required(self.verus_version.as_ref(), "Verus version")
    }

    fn loaded_verus_checksums(&self) -> Result<&str, String> {
        Self::required(self.verus_checksums.as_ref(), "Verus checksum")
    }

    fn loaded_prover_tools_ref_metadata(&self) -> Result<&str, String> {
        Self::required(
            self.prover_tools_ref_metadata.as_ref(),
            "rust-prover-tools reference",
        )
    }

    fn loaded_makefile(&self) -> Result<&str, String> {
        Self::required(self.makefile.as_ref(), "Makefile")
    }

    /// Verify all formal-tooling pins are present and shaped as expected.
    ///
    /// # Errors
    ///
    /// Returns an error when required metadata is absent or malformed.
    pub fn verify_tool_metadata_pins(&self) -> TestResult {
        let kani_version = self.loaded_kani_version()?;
        if !is_three_part_numeric_version(kani_version) {
            return Err(format!("Kani version should be MAJOR.MINOR.PATCH: {kani_version}").into());
        }
        if self.loaded_verus_version()?.is_empty() {
            return Err("Verus version should not be empty".into());
        }
        let metadata = ProverToolsRef(self.loaded_prover_tools_ref_metadata()?);
        if !metadata
            .as_str()
            .contains("repository: https://github.com/leynos/rust-prover-tools.git")
        {
            return Err("rust-prover-tools metadata should name the repository".into());
        }
        if metadata.ref_value().is_none() {
            return Err("rust-prover-tools metadata should expose a ref".into());
        }
        Ok(())
    }

    /// Verify Verus checksums contain the archive for the configured target.
    ///
    /// # Errors
    ///
    /// Returns an error when the expected Linux archive is absent.
    pub fn verify_verus_checksum_manifest(&self) -> TestResult {
        let archive_name = verus_linux_archive_name(self.loaded_verus_version()?);
        let checksums = ChecksumsContent(self.loaded_verus_checksums()?);
        if !checksums.contains_archive(&archive_name) {
            return Err(format!("missing checksum entry for {archive_name}").into());
        }
        Ok(())
    }

    /// Verify all formal-tooling Makefile targets are present and phony.
    ///
    /// # Errors
    ///
    /// Returns an error when any expected target is missing.
    pub fn verify_makefile_targets(&self) -> TestResult {
        let makefile = MakefileContent(self.loaded_makefile()?);
        for target in [
            "install-kani",
            "check-kani-version",
            "install-verus",
            "run-verus",
        ] {
            if !makefile.has_phony_target(target) || makefile.target_recipe(target).is_none() {
                return Err(format!("Makefile should expose `{target}`").into());
            }
        }
        Ok(())
    }

    /// Verify Makefile targets delegate to `prover-tools`.
    ///
    /// # Errors
    ///
    /// Returns an error when a target omits the pinned entry point.
    pub fn verify_makefile_targets_delegate_to_prover_tools(&self) -> TestResult {
        let makefile = MakefileContent(self.loaded_makefile()?);
        for target in [
            "install-kani",
            "check-kani-version",
            "install-verus",
            "run-verus",
        ] {
            let recipe = makefile
                .target_recipe(target)
                .ok_or_else(|| format!("Makefile should expose `{target}`"))?;
            if !recipe.contains("$(PROVER_TOOLS)") {
                return Err(format!("`{target}` should delegate through prover-tools").into());
            }
        }
        Ok(())
    }
}
