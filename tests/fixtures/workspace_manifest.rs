//! Fixture world for workspace-manifest behavioural scenarios.

use rstest::fixture;
use serde_json::Value;

use crate::workspace_manifest_support::{
    WorkspaceManifestResult as FixtureResult,
    cargo_metadata,
    has_manifest_line,
    helper_package_id,
    root_manifest,
    root_package_id,
    verification_package_id,
};

pub type TestResult = FixtureResult<()>;
const ROOT_PACKAGE_NAME: &str = "wireframe";
const HELPER_PACKAGE_NAME: &str = "wireframe_testing";
const VERIFICATION_PACKAGE_NAME: &str = "wireframe-verification";

/// BDD world holding the root manifest and `cargo metadata` output.
#[derive(Debug, Default)]
pub struct WorkspaceManifestWorld {
    manifest: Option<String>,
    metadata: Option<String>,
    helper_package_id: Option<String>,
    package_id: Option<String>,
    verification_package_id: Option<String>,
}

#[rustfmt::skip]
#[fixture]
pub fn workspace_manifest_world() -> WorkspaceManifestWorld {
    std::hint::black_box(());
    WorkspaceManifestWorld::default()
}

impl WorkspaceManifestWorld {
    /// Load the repository manifest and workspace metadata for later checks.
    ///
    /// # Errors
    ///
    /// Returns an error when the manifest cannot be read or `cargo metadata`
    /// fails.
    pub fn load(&mut self) -> TestResult {
        self.manifest = Some(root_manifest()?);
        self.metadata = Some(cargo_metadata()?);
        self.helper_package_id = Some(helper_package_id()?);
        self.package_id = Some(root_package_id()?);
        self.verification_package_id = Some(verification_package_id()?);
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

    fn package_id(&self) -> Result<&str, String> {
        self.package_id
            .as_deref()
            .ok_or_else(|| "workspace package id not loaded".to_owned())
    }

    fn helper_package_id(&self) -> Result<&str, String> {
        self.helper_package_id
            .as_deref()
            .ok_or_else(|| "helper package id not loaded".to_owned())
    }

    fn verification_package_id(&self) -> Result<&str, String> {
        self.verification_package_id
            .as_deref()
            .ok_or_else(|| "verification package id not loaded".to_owned())
    }

    fn metadata_json(&self) -> FixtureResult<Value> { Ok(serde_json::from_str(self.metadata()?)?) }

    fn metadata_array<'a>(metadata: &'a Value, field: &str) -> Result<&'a [Value], String> {
        metadata
            .get(field)
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .ok_or_else(|| format!("cargo metadata should expose {field} as an array"))
    }

    fn packages(metadata: &Value) -> Result<&[Value], String> {
        Self::metadata_array(metadata, "packages")
    }

    fn packages_include_name(metadata: &Value, name: &str) -> Result<bool, String> {
        Ok(Self::packages(metadata)?
            .iter()
            .any(|package| package.get("name").and_then(Value::as_str) == Some(name)))
    }

    fn verify_crate_is_workspace_member(&self, package_id: &str, package_name: &str) -> TestResult {
        let metadata = self.metadata_json()?;
        let workspace_members = Self::metadata_array(&metadata, "workspace_members")?;
        if !workspace_members
            .iter()
            .any(|member| member.as_str() == Some(package_id))
        {
            return Err(format!(
                "workspace metadata should include `{package_name}` in workspace_members"
            )
            .into());
        }
        if !Self::packages_include_name(&metadata, package_name)? {
            return Err(format!("cargo metadata packages should include `{package_name}`").into());
        }
        Ok(())
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
            "members = [\".\", \"crates/wireframe-verification\", \"wireframe_testing\"]",
            "default-members = [\".\"]",
            "resolver = \"3\"",
        ] {
            if !has_manifest_line(manifest, expected) {
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
        let package_id = self.package_id()?;
        let metadata = self.metadata_json()?;
        let workspace_members = Self::metadata_array(&metadata, "workspace_members")?;
        if !workspace_members
            .iter()
            .any(|member| member.as_str() == Some(package_id))
        {
            return Err(format!(
                "workspace metadata did not include the root package id in workspace_members: \
                 {workspace_members:?}"
            )
            .into());
        }
        if !Self::packages_include_name(&metadata, ROOT_PACKAGE_NAME)? {
            return Err(format!(
                "cargo metadata packages did not include the root package name \
                 `{ROOT_PACKAGE_NAME}`"
            )
            .into());
        }
        Ok(())
    }

    /// Verify the root package is the only default workspace member.
    ///
    /// # Errors
    ///
    /// Returns an error when the metadata widens default-member coverage.
    pub fn verify_root_is_only_default_member(&self) -> TestResult {
        let package_id = self.package_id()?;
        let metadata = self.metadata_json()?;
        let workspace_default_members =
            Self::metadata_array(&metadata, "workspace_default_members")?;
        if workspace_default_members.len() != 1
            || workspace_default_members.first().and_then(Value::as_str) != Some(package_id)
        {
            return Err(format!(
                "workspace metadata did not keep the root package as the only default member: \
                 {workspace_default_members:?}"
            )
            .into());
        }
        Ok(())
    }

    /// Verify the verification crate is now part of the workspace membership.
    ///
    /// # Errors
    ///
    /// Returns an error when the verification crate is missing from
    /// `workspace_members` or Cargo metadata.
    pub fn verify_verification_crate_is_workspace_member(&self) -> TestResult {
        self.verify_crate_is_workspace_member(
            self.verification_package_id()?,
            VERIFICATION_PACKAGE_NAME,
        )
    }

    /// Verify the testing helper crate is part of the workspace membership.
    ///
    /// # Errors
    ///
    /// Returns an error when the helper crate is missing from
    /// `workspace_members` or Cargo metadata.
    pub fn verify_helper_crate_is_workspace_member(&self) -> TestResult {
        self.verify_crate_is_workspace_member(self.helper_package_id()?, HELPER_PACKAGE_NAME)
    }
}
