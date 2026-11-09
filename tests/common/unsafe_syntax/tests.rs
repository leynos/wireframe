//! Regressions for the spellings the detector must catch.
//!
//! The table is kept beside the detector rather than in the contract test so a
//! spelling can be added without touching the workspace-wide assertions. Each
//! case states the source text and the violation fragment it must produce; an
//! empty fragment means the text is clean.

use rstest::rstest;

use super::violations_in;

#[rstest]
#[case::allowance("#![allow(unsafe_code)]", "uses an `unsafe_code` allowance")]
#[case::allowance_after_another_lint(
    "#[allow(dead_code, unsafe_code)]",
    "uses an `unsafe_code` allowance"
)]
#[case::expectation(
    "#[expect(\n    unsafe_code,\n)]",
    "carries an `unsafe_code` expectation"
)]
#[case::expectation_after_another_lint(
    "#[expect(dead_code, unsafe_code)]",
    "carries an `unsafe_code` expectation"
)]
#[case::allowance_with_space_before_parenthesis(
    "#[allow (unsafe_code)]",
    "uses an `unsafe_code` allowance"
)]
#[case::expectation_split_before_parenthesis(
    "#[expect\n(unsafe_code)]",
    "carries an `unsafe_code` expectation"
)]
#[case::construct("unsafe fn helper() {}", "contains an unsafe function")]
#[case::construct_split_across_lines("unsafe\nfn helper() {}", "contains an unsafe function")]
#[case::construct_after_block_comment(
    "unsafe/* SAFETY */ fn helper() {}",
    "contains an unsafe function"
)]
#[case::construct_after_line_comment(
    "unsafe // SAFETY\nfn helper() {}",
    "contains an unsafe function"
)]
#[case::adjacent_brace("unsafe{ std::ptr::null::<u8>() };", "contains an unsafe block")]
#[case::spaced_brace("unsafe { std::ptr::null::<u8>() };", "contains an unsafe block")]
#[case::unsafe_trait("unsafe trait Marker {}", "contains an unsafe trait")]
#[case::unsafe_implementation(
    "unsafe impl Marker for Value {}",
    "contains an unsafe trait implementation"
)]
#[case::unsafe_extern(
    "unsafe extern \"C\" { fn foreign(); }",
    "contains an unsafe extern item"
)]
#[case::unsafe_attribute("#[unsafe(no_mangle)]", "contains an unsafe attribute")]
#[case::unsafe_attribute_with_argument(
    "#[unsafe(export_name = \"entry\")]",
    "contains an unsafe attribute"
)]
#[case::unsafe_attribute_split_before_parenthesis(
    "#[unsafe\n(no_mangle)]",
    "contains an unsafe attribute"
)]
#[case::unsafe_attribute_after_block_comment(
    "#[unsafe/* SAFETY */(no_mangle)]",
    "contains an unsafe attribute"
)]
#[case::unsafe_attribute_after_an_item(
    "fn first() {}\n#[unsafe(no_mangle)]\npub fn second() {}",
    "contains an unsafe attribute"
)]
#[case::unsafe_attribute_after_a_comment(
    "fn first() {}\n// declare the symbol\n#[unsafe(no_mangle)]\npub fn second() {}",
    "contains an unsafe attribute"
)]
#[case::unsafe_attribute_inside_cfg_attr(
    "#[cfg_attr(feature = \"ffi\", unsafe(no_mangle))]",
    "contains an unsafe attribute"
)]
#[case::nested_prohibition("#![expect(#![forbid(unsafe_code)] dead_code)]", "")]
#[case::comment_only_prose("// unsafe fn helper() {}", "")]
#[case::nested_block_comment_spelling_a_construct(
    "/* outer /* unsafe fn hidden() {} */ still a comment */ fn safe() {}",
    ""
)]
#[case::raw_string_alone("let pattern = r\"a/*b\";", "")]
#[case::code_after_a_raw_string_with_quotes_and_comment_markers(
    "let sql = r#\"SELECT \"x\" -- /* not a comment */\"#; unsafe fn helper() {}",
    "contains an unsafe function"
)]
#[case::string_containing_comment_markers("let pattern = \"src/**/*.rs\";", "")]
#[case::identifier_containing_the_keyword("let unsafe_count = 1;", "")]
#[case::code_after_a_string_that_contains_a_comment_marker(
    "let url = \"http://example.com\"; unsafe fn helper() {}",
    "contains an unsafe function"
)]
#[case::string_literal_spelling_a_construct(
    "let note = \"unsafe fn\";",
    "contains an unsafe function"
)]
#[case::clean("fn helper() {}", "")]
fn reports_prohibited_text(#[case] source: &str, #[case] expected: &str) {
    let violations = violations_in(source, "src/lib.rs");
    if expected.is_empty() {
        assert!(
            violations.is_empty(),
            "unexpected violations: {violations:?}"
        );
    } else {
        assert!(
            violations
                .iter()
                .any(|violation| violation.contains(expected)),
            "no violation matched {expected:?}: {violations:?}"
        );
    }
}

/// Every violation a single file can raise is reported, in a stable order.
///
/// The table above matches one fragment at a time, so it cannot show that a
/// source raising several findings reports all of them, or that the order a
/// reviewer reads is the order the scan produces. The label is fixed so the
/// expected text is a snapshot of the whole output rather than a shape.
#[test]
fn multi_violation_output_is_stable() {
    let source = concat!(
        "#![allow(unsafe_code)]\n",
        "#[expect(unsafe_code)]\n",
        "unsafe fn helper() {}\n",
        "unsafe trait Marker {}\n",
        "fn block() { unsafe { std::ptr::null::<u8>() }; }\n",
    );

    assert_eq!(
        violations_in(source, "tests/feature_gate/src/optional.rs"),
        [
            "tests/feature_gate/src/optional.rs uses an `unsafe_code` allowance; the workspace \
             forbids unsafe code",
            "tests/feature_gate/src/optional.rs carries an `unsafe_code` expectation; the \
             workspace forbids unsafe code",
            "tests/feature_gate/src/optional.rs contains an unsafe block but the workspace \
             forbids unsafe code",
            "tests/feature_gate/src/optional.rs contains an unsafe function but the workspace \
             forbids unsafe code",
            "tests/feature_gate/src/optional.rs contains an unsafe trait but the workspace \
             forbids unsafe code",
        ],
        "the whole multi-violation output must stay stable"
    );
}
