//! Compile-time trait-bound and public-default tests.

#![cfg(not(coverage))]

use std::{error::Error, io, process::Command};
use camino::Utf8PathBuf;
use cap_std::{ambient_authority, fs_utf8::Dir};

/// Check public type contracts and reject a zero fixture budget at compile time.
#[test]
fn compile_tests() {
    let t = trybuild::TestCases::new();
    t.pass("tests/ui/server_from_app.rs");
    t.pass("tests/ui/server_factory_forms.rs");
    t.pass("tests/ui/wireframe_result_default_no_protocol.rs");
    t.pass("tests/ui/client_const_api.rs");
    t.pass("tests/ui/codec_shared_const_api.rs");
    t.pass("tests/ui/fragment_const_api.rs");
    t.pass("tests/ui/app_const_api.rs");
    t.compile_fail("tests/ui/server_rejects_non_send_factory.rs");
    t.compile_fail("tests/ui/wireframe_result_default_rejects_unit_protocol.rs");
    t.compile_fail("tests/ui/prepared_app_rejects_route.rs");
    t.compile_fail("tests/ui/zero_test_budget.rs");
    t.pass("tests/ui/client_error_wildcard_match.rs");
    t.compile_fail("tests/ui/client_error_exhaustive_match.rs");
    t.pass("tests/ui/wireframe_testing_exports.rs");
    // Trybuild forwards this build's features to the fixture, so the pool
    // accessors only exist here when `test-support` is enabled. `make test`
    // runs with `--all-features`, which is where this case gets its coverage.
    #[cfg(feature = "test-support")]
    t.pass("tests/ui/client_pool_test_support_api.rs");
}

/// Pin the v0.6 fallible scenario signature and strict missing-step validation.
#[test]
fn rstest_bdd_v06_contracts() -> Result<(), Box<dyn Error>> {
    stage_bdd_feature_files()?;
    let t = trybuild::TestCases::new();
    t.pass("tests/ui/rstest_bdd_v06/owned_fallible_fixture.rs");
    t.compile_fail("tests/ui/rstest_bdd_v06/strict_missing_step.rs");
    Ok(())
}

/// Stage feature inputs beneath the manifest directory created by trybuild.
fn stage_bdd_feature_files() -> Result<(), Box<dyn Error>> {
    let target = trybuild_target_directory()?;
    let target_dir = Dir::open_ambient_dir(target, ambient_authority())?;
    let destination = Utf8PathBuf::from("tests/trybuild/wireframe/rstest_bdd_v06");
    target_dir.create_dir_all(&destination)?;
    let source = Dir::open_ambient_dir(
        Utf8PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/ui/rstest_bdd_v06"),
        ambient_authority(),
    )?;
    for name in ["owned_fallible.feature", "missing_step.feature"] {
        target_dir.write(destination.join(name), source.read(name)?)?;
    }
    Ok(())
}

/// Resolve the target root the same way trybuild does, including Cargo config.
fn trybuild_target_directory() -> Result<Utf8PathBuf, Box<dyn Error>> {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let output = Command::new(cargo)
        .args(["metadata", "--no-deps", "--format-version=1", "--locked"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "cargo metadata failed while locating the trybuild target: {}",
            String::from_utf8_lossy(&output.stderr)
        ))
        .into());
    }
    let metadata: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    let target = metadata
        .get("target_directory")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| io::Error::other("cargo metadata omitted target_directory"))?;
    Ok(Utf8PathBuf::from(target))
}
