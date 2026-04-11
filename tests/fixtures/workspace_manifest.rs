//! Fixture world for workspace-manifest behavioural scenarios.

use std::{env, process::Command};

use camino::Utf8PathBuf;
use cap_std::{ambient_authority, fs_utf8::Dir};
use rstest::fixture;

type FixtureResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;
pub type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

/// BDD world holding the root manifest and `cargo metadata` output.
#[derive(Debug, Default)]
pub struct WorkspaceManifestWorld {
    manifest: Option<String>,
    metadata: Option<String>,
}

#[fixture]
pub fn workspace_manifest_world() -> WorkspaceManifestWorld {
    std::hint::black_box(());
    WorkspaceManifestWorld::default()
}

impl WorkspaceManifestWorld {
    fn repo_root() -> FixtureResult<Utf8PathBuf> {
        Utf8PathBuf::from_path_buf(env::current_dir()?).map_err(|path| {
            format!(
                "repository root path is not valid UTF-8: {}",
                path.display()
            )
            .into()
        })
    }

    fn repo_dir() -> FixtureResult<Dir> {
        Ok(Dir::open_ambient_dir(
            Self::repo_root()?,
            ambient_authority(),
        )?)
    }

    fn package_id() -> FixtureResult<String> {
        Ok(format!(
            "path+file://{}#wireframe@0.3.0",
            Self::repo_root()?
        ))
    }

    /// Load the repository manifest and workspace metadata for later checks.
    ///
    /// # Errors
    ///
    /// Returns an error when the manifest cannot be read or `cargo metadata`
    /// fails.
    pub fn load(&mut self) -> TestResult {
        let manifest = Self::repo_dir()?.read_to_string("Cargo.toml")?;
        let output = Command::new("cargo")
            .args(["metadata", "--no-deps", "--format-version", "1"])
            .current_dir(Self::repo_root()?)
            .output()?;
        if !output.status.success() {
            return Err(format!(
                "`cargo metadata` failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )
            .into());
        }

        self.manifest = Some(manifest);
        self.metadata = Some(String::from_utf8(output.stdout)?);
        Ok(())
    }

    fn manifest(&self) -> Result<&str, String> {
        self.manifest
            .as_deref()
            .ok_or_else(|| "workspace manifest not loaded".to_owned())
    }

    fn metadata(&self) -> Result<&str, String> {
        self.metadata
            .as_deref()
            .ok_or_else(|| "workspace metadata not loaded".to_owned())
    }

    /// Verify the root manifest declares the staged hybrid workspace contract.
    ///
    /// # Errors
    ///
    /// Returns an error when the manifest is missing an expected workspace
    /// clause.
    pub fn verify_staged_hybrid_workspace_manifest(&self) -> TestResult {
        let manifest = self.manifest()?;
        for expected in [
            "[workspace]",
            "members = [\".\"]",
            "default-members = [\".\"]",
            "resolver = \"3\"",
        ] {
            if !manifest.contains(expected) {
                return Err(format!("expected `{expected}` in root Cargo.toml").into());
            }
        }
        Ok(())
    }

    /// Verify the root package remains part of the workspace membership.
    ///
    /// # Errors
    ///
    /// Returns an error when the metadata omits the root package.
    pub fn verify_root_is_workspace_member(&self) -> TestResult {
        let metadata = self.metadata()?;
        if !metadata.contains(&Self::package_id()?) {
            return Err("workspace metadata did not include the root package".into());
        }
        Ok(())
    }

    /// Verify the root package is the only default workspace member.
    ///
    /// # Errors
    ///
    /// Returns an error when the metadata widens default-member coverage.
    pub fn verify_root_is_only_default_member(&self) -> TestResult {
        let metadata = self.metadata()?;
        let expected = format!(
            "\"workspace_default_members\":[\"{}\"]",
            Self::package_id()?
        );
        if !metadata.contains(&expected) {
            return Err(
                "workspace metadata did not keep the root package as the only default member"
                    .into(),
            );
        }
        Ok(())
    }

    /// Verify the staged rollout has not yet added the verification crate.
    ///
    /// # Errors
    ///
    /// Returns an error when the 10.1.2 crate appears too early or the helper
    /// crate unexpectedly disappears from workspace metadata.
    pub fn verify_verification_crate_is_absent(&self) -> TestResult {
        let metadata = self.metadata()?;
        if metadata.contains("wireframe-verification") {
            return Err(
                "verification crate should not join the workspace until roadmap item 10.1.2".into(),
            );
        }
        if !metadata.contains("wireframe_testing#0.3.0") {
            return Err(
                "workspace metadata should continue to report the in-repo helper crate".into(),
            );
        }
        Ok(())
    }
}
