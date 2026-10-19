//! Check that network-backed test helpers stay outside the Loom API surface.

#![cfg(loom)]

#[test]
fn loom_build_omits_non_loom_helpers() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/ui/loom_non_loom_exports.rs");
}
