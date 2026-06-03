//! Compile-time trait-bound and public-default tests.

#[test]
fn compile_tests() {
    let t = trybuild::TestCases::new();
    t.pass("tests/ui/wireframe_result_default_no_protocol.rs");
    t.compile_fail("tests/ui/wireframe_result_default_rejects_unit_protocol.rs");
}
