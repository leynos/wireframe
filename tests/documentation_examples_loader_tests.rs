//! Failure contracts for the documented-example fence loader.

#![forbid(unsafe_code)]

// `pub` because this binary uses only part of the shared loader.
pub mod documentation_examples;

use documentation_examples::{hidden_rustdoc_line, parse_document};
use rstest::rstest;

#[rstest]
#[case::empty_marker("<!-- tested-example: -->\n", "identifier must not be empty")]
#[case::blank_marker("<!-- tested-example:  -->\n", "identifier must not be empty")]
#[case::unmarked_backticks("```rust\nlet x = 1;\n```\n", "missing a tested-example marker")]
#[case::unmarked_tildes("~~~rust\nlet x = 1;\n~~~\n", "missing a tested-example marker")]
#[case::unterminated("<!-- tested-example: a -->\n```rust\nlet x = 1;\n", "not terminated")]
#[case::no_fence("<!-- tested-example: a -->\n", "marker has no fence")]
#[case::indented_fence("1. Step:\n\n   ```rust\n   let x = 1;\n   ```\n", "column 0")]
#[case::prose_after_marker("<!-- tested-example: a -->\nprose\n", "expected an opening fence")]
#[case::no_language(
    "<!-- tested-example: a -->\n```\nlet x = 1;\n```\n",
    "declare a language"
)]
fn malformed_documented_examples_are_rejected(#[case] contents: &str, #[case] expected: &str) {
    let error = parse_document("fixture.md", contents)
        .expect_err("malformed documented example should be rejected");
    assert!(
        error.contains(expected),
        "expected '{expected}' in '{error}'"
    );
}

#[test]
fn a_marker_may_be_separated_from_its_fence_by_blank_lines() {
    let examples = parse_document(
        "fixture.md",
        "<!-- tested-example: spaced -->\n\n```rust,no_run\nlet x = 1;\n```\n",
    )
    .expect("a blank line keeps the marker attached");
    assert_eq!(examples.len(), 1);
    let example = examples.first().expect("one example");
    assert_eq!(example.id, "spaced");
    assert_eq!(example.language(), "rust");
    assert_eq!(example.body, "let x = 1;\n");
}

/// A marker-like line inside a marked fence is body text, not a new example,
/// and a longer fence stays open across the shorter one it quotes.
#[test]
fn quoted_markers_and_fences_stay_inside_the_body() {
    let contents = concat!(
        "<!-- tested-example: quoting -->\n",
        "````markdown\n",
        "<!-- tested-example: quoted -->\n",
        "```rust\n",
        "```\n",
        "````\n",
    );
    let examples = parse_document("fixture.md", contents).expect("the outer fence is well formed");
    assert_eq!(examples.len(), 1);
    assert_eq!(examples.first().expect("one example").id, "quoting");
}

#[rstest]
#[case::hidden_item("# fn main() {}\nlet x = 1;\n", Some("# fn main() {}"))]
#[case::hidden_indented("    # let y = 2;\n", Some("    # let y = 2;"))]
#[case::bare_hash("#\n", Some("#"))]
#[case::attribute("#[derive(Debug)]\nstruct A;\n", None)]
#[case::inner_attribute("#![allow(unused)]\n", None)]
#[case::comment("// # not hidden\nlet x = 1;\n", None)]
fn hidden_lines_are_detected(#[case] body: &str, #[case] expected: Option<&str>) {
    assert_eq!(hidden_rustdoc_line(body), expected);
}
