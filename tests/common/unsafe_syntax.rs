//! Detect prohibited `unsafe` syntax in a source text.
//!
//! The unsafe-code contract reads source files as text rather than compiling
//! every feature combination, so the detector has to state what it counts. It
//! matches each construct spelling literally after collapsing runs of
//! whitespace, so a construct split across a line break is still reported. It
//! does not strip comments or string literals, so prose that spells a construct
//! out is reported: the check fails closed rather than quietly ignoring a real
//! one. Files that exist to name the constructs — this module and the contract
//! test — are exempted by the caller, which is why the list is explicit rather
//! than a pattern.

/// Constructs the workspace forbids.
///
/// The adjacent-brace spelling is listed separately because collapsing
/// whitespace cannot insert a space that was never there. The attribute form
/// covers Rust 2024's unsafe attributes, such as `#[unsafe(no_mangle)]` and
/// `#[unsafe(export_name = "...")]`, which carry no `unsafe` keyword of their
/// own.
pub(crate) const UNSAFE_CONSTRUCTS: &[&str] = &[
    "unsafe impl",
    "unsafe fn",
    "unsafe trait",
    "unsafe extern",
    "unsafe {",
    "unsafe{",
    "#[unsafe(",
];

/// Unsafe-code violations in one source text.
///
/// An `unsafe_code` allowance is always a violation: `forbid` admits no
/// override, so the allowance cannot take effect and its presence means a root
/// is trying to weaken the prohibition. A scoped expectation and an unsafe
/// construct are violations too, because the workspace contains no sanctioned
/// unsafe code.
pub(crate) fn violations_in(source: &str, label: &str) -> Vec<String> {
    let normalized_source = collapse_whitespace(source);
    let mut violations = Vec::new();
    if lint_attribute_names_unsafe_code(&normalized_source, "allow(") {
        violations.push(format!(
            "{label} uses an `unsafe_code` allowance; the workspace forbids unsafe code"
        ));
    }
    if lint_attribute_names_unsafe_code(&normalized_source, "expect(") {
        violations.push(format!(
            "{label} carries an `unsafe_code` expectation; the workspace forbids unsafe code"
        ));
    }
    violations.extend(
        UNSAFE_CONSTRUCTS
            .iter()
            .filter(|construct| normalized_source.contains(**construct))
            .map(|construct| {
                format!("{label} contains `{construct}` but the workspace forbids unsafe code")
            }),
    );
    violations
}

/// Collapse every run of whitespace to a single space.
///
/// `check-fmt` only formats workspace members, so a source in the standalone
/// `tests/feature_gate` package can be committed with `unsafe` and the rest of
/// a construct on separate lines. Normalizing first means the scan sees the
/// spelling the compiler would, whether the tokens were split across a line
/// break or an attribute list was wrapped by `rustfmt`.
fn collapse_whitespace(source: &str) -> String {
    source.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Whether any `kind` lint attribute in `source` lists `unsafe_code`.
///
/// `kind` is an attribute name with its opening parenthesis, such as `"allow("`.
/// The lint list can name several lints, so `#[allow(dead_code, unsafe_code)]`
/// counts as well as `#[allow(unsafe_code)]`. Each comma-separated element is
/// compared whole, so an `unsafe_code` that appears inside a nested attribute —
/// as in `#![expect(#![forbid(unsafe_code)] dead_code)]` — is not mistaken for
/// a lint this attribute names.
fn lint_attribute_names_unsafe_code(source: &str, kind: &str) -> bool {
    source.match_indices(kind).any(|(index, _)| {
        source
            .get(index + kind.len()..)
            .and_then(|rest| rest.split_once(')'))
            .is_some_and(|(arguments, _)| {
                arguments
                    .split(',')
                    .any(|lint| lint.trim() == "unsafe_code")
            })
    })
}

#[cfg(test)]
mod unsafe_syntax_tests {
    //! Regressions for the spellings the detector must catch.

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
    #[case::construct("unsafe fn helper() {}", "contains `unsafe fn`")]
    #[case::construct_split_across_lines("unsafe\nfn helper() {}", "contains `unsafe fn`")]
    #[case::adjacent_brace("unsafe{ std::ptr::null::<u8>() };", "contains `unsafe{`")]
    #[case::unsafe_attribute("#[unsafe(no_mangle)]", "contains `#[unsafe(`")]
    #[case::unsafe_attribute_with_argument(
        "#[unsafe(export_name = \"entry\")]",
        "contains `#[unsafe(`"
    )]
    #[case::nested_prohibition("#![expect(#![forbid(unsafe_code)] dead_code)]", "")]
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
}
