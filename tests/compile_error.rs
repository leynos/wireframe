//! Compile-time trait-bound and public-default tests.

#![cfg(not(coverage))]

/// Check public type contracts and reject a zero fixture budget at compile time.
#[test]
fn compile_tests() {
    let t = trybuild::TestCases::new();
    t.pass("tests/ui/server_from_app.rs");
    t.pass("tests/ui/server_factory_forms.rs");
    t.pass("tests/ui/wireframe_result_default_no_protocol.rs");
    t.pass("tests/ui/client_const_api.rs");
    t.pass("tests/ui/codec_shared_const_api.rs");
    t.pass("tests/ui/const_config_docs_example.rs");
    t.pass("tests/ui/fragment_const_api.rs");
    t.pass("tests/ui/app_const_api.rs");
    t.pass("tests/ui/server_const_api.rs");
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
