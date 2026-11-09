//! Properties of the comment-stripping scan over generated source text.
//!
//! The regression table pins the spellings that were hard to get right. These
//! properties cover the two structures whose extent depends on nesting rather
//! than on a single spelling: a block comment may contain further block
//! comments, and a raw string may quote anything, including the markers that
//! would otherwise start a comment. Both must be consumed whole, so a construct
//! inside them is not reported and a construct after them still is.

use proptest::prelude::*;

use super::violations_in;

/// The construct a comment or a literal must hide, or must not hide.
///
/// The keyword and the rest are assembled rather than written contiguously so
/// this helper's own source does not read as prohibited to the scan it
/// exercises.
const HIDDEN_CONSTRUCT: &str = concat!("unsafe", " fn hidden() {}");

/// A block comment holding the construct, nested `depth` levels deep.
fn nested_comment(depth: usize) -> String {
    let mut comment = HIDDEN_CONSTRUCT.to_owned();
    for _ in 0..depth {
        comment = format!("/* {comment} */");
    }
    comment
}

/// A raw string with `hashes` delimiters holding `body`.
fn raw_string(hashes: usize, body: &str) -> String {
    let delimiter = "#".repeat(hashes);
    format!("r{delimiter}\"{body}\"{delimiter}")
}

/// Evidence fragments that would start or end a comment if they were code.
fn comment_markers() -> impl Strategy<Value = &'static str> {
    prop::sample::select(vec!["//", "/*", "*/", "text", " ", "\n"])
}

proptest! {
    #[test]
    fn comment_text_never_reports_a_violation(depth in 1usize..5) {
        let source = format!("{}\nfn safe() {{}}\n", nested_comment(depth));

        let violations = violations_in(&source, "src/lib.rs");

        prop_assert!(
            violations.is_empty(),
            "a nested comment reported {violations:?}",
        );
    }

    #[test]
    fn a_raw_string_never_hides_the_code_after_it(
        hashes in 0usize..4,
        body in prop::collection::vec(comment_markers(), 0..8),
    ) {
        let source = format!(
            "let note = {};\n{HIDDEN_CONSTRUCT}\n",
            raw_string(hashes, &body.concat()),
        );

        let violations = violations_in(&source, "src/lib.rs");

        prop_assert!(
            violations
                .iter()
                .any(|violation| violation.contains("an unsafe function")),
            "the code after a raw string was not reported: {violations:?}",
        );
    }

    #[test]
    fn a_raw_string_alone_reports_nothing(
        hashes in 0usize..4,
        body in prop::collection::vec(comment_markers(), 0..8),
    ) {
        let source = format!("let note = {};\n", raw_string(hashes, &body.concat()));

        let violations = violations_in(&source, "src/lib.rs");

        prop_assert!(
            violations.is_empty(),
            "a raw string reported {violations:?}",
        );
    }
}
