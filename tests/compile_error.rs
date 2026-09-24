//! Compile-time trait-bound and public-default tests.

/// Check public type contracts and reject a zero fixture budget at compile time.
#[test]
fn compile_tests() {
    let t = trybuild::TestCases::new();
    t.pass("tests/ui/wireframe_result_default_no_protocol.rs");
    t.pass("tests/ui/client_const_api.rs");
    t.compile_fail("tests/ui/wireframe_result_default_rejects_unit_protocol.rs");
    t.compile_fail("tests/ui/prepared_app_rejects_route.rs");
    t.compile_fail("tests/ui/zero_test_budget.rs");
}
