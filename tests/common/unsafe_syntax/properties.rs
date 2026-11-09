//! Properties of the comment-stripping scan over generated source text.
//!
//! The regression table pins the spellings that were hard to get right. These
//! properties cover the two structures whose extent depends on nesting rather
//! than on a single spelling: a block comment may contain further block
//! comments, and a raw string may quote anything, including the markers that
//! would otherwise start a comment.
//!
//! The two are treated differently, and the difference is the point. A comment
//! is consumed whole, so a construct written inside one is not reported. A
//! string literal is preserved as text, because the scan matches spellings on
//! purpose: `let note = "unsafe fn";` is reported. What a literal must not do is
//! close early and expose or swallow the code around it.

use proptest::prelude::*;

use super::violations_in;

/// The construct the scan must report when it appears in code.
const HIDDEN_CONSTRUCT: &str = concat!("unsafe", " fn hidden() {}");

/// A construct that only ever appears inside a comment.
const COMMENTED_CONSTRUCT: &str = concat!("unsafe", " trait Hidden {}");

/// A block comment holding `text`, nested `depth` levels deep.
fn nested_comment(depth: usize, text: &str) -> String {
    let mut comment = text.to_owned();
    for _ in 0..depth {
        comment = format!("/* {comment} */");
    }
    comment
}

/// Evidence fragments that would start or end a comment, or a string.
fn comment_markers() -> impl Strategy<Value = &'static str> {
    prop::sample::select(vec!["//", "/*", "*/", "text", " ", "\n", "\""])
}

/// A raw string quoting comment markers, a quote, and a partial delimiter.
///
/// The delimiter always has at least one hash and is longer than any run of
/// hashes that follows a quote in the body, so the body stays literal text
/// rather than closing the string early.
fn raw_string_quoting_markers() -> impl Strategy<Value = String> {
    (
        1usize..4,
        prop::collection::vec(comment_markers(), 0..8),
        0usize..3,
    )
        .prop_map(|(hashes, body, suffix)| {
            let delimiter = "#".repeat(hashes);
            let partial_hashes = "#".repeat(suffix.min(hashes - 1));
            let partial = format!("\"{partial_hashes}");
            format!(
                "r{delimiter}\"{body}{partial}\"{delimiter}",
                body = body.join("")
            )
        })
}

proptest! {
    #[test]
    fn comment_text_never_reports_a_violation(depth in 1usize..5) {
        let source = format!(
            "{}\nfn safe() {{}}\n",
            nested_comment(depth, COMMENTED_CONSTRUCT),
        );

        let violations = violations_in(&source, "src/lib.rs");

        prop_assert!(
            violations.is_empty(),
            "a nested comment reported {violations:?}",
        );
    }

    #[test]
    fn a_nested_comment_hides_only_its_own_text(depth in 1usize..5) {
        let source = format!(
            "{}\n{HIDDEN_CONSTRUCT}\n",
            nested_comment(depth, COMMENTED_CONSTRUCT),
        );

        let violations = violations_in(&source, "src/lib.rs");

        // Every finding must concern the function written after the comment; a
        // finding about a trait could only come from the comment's own text.
        prop_assert!(
            violations
                .iter()
                .all(|violation| violation.contains("an unsafe function")),
            "only the code after the comment may be reported: {violations:?}",
        );
        prop_assert!(
            violations
                .iter()
                .any(|violation| violation.contains("an unsafe function")),
            "the code after the comment was not reported: {violations:?}",
        );
    }

    #[test]
    fn a_raw_string_never_hides_the_code_after_it(string in raw_string_quoting_markers()) {
        let source = format!("let note = {string};\n{HIDDEN_CONSTRUCT}\n");

        let violations = violations_in(&source, "src/lib.rs");

        prop_assert!(
            violations
                .iter()
                .any(|violation| violation.contains("an unsafe function")),
            "the code after a raw string was not reported: {violations:?}",
        );
    }

    #[test]
    fn a_raw_string_alone_reports_nothing(string in raw_string_quoting_markers()) {
        let source = format!("let note = {string};\n");

        let violations = violations_in(&source, "src/lib.rs");

        prop_assert!(
            violations.is_empty(),
            "a raw string reported {violations:?}",
        );
    }
}
