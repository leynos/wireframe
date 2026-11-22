//! Tests for the list-container rule that keeps hidden examples out of guides.
//!
//! A fence or marker behind a list marker, a tab or four spaces would be
//! skipped by the top-level walk, so the loader fails closed and names the
//! line. These cases pin the accepted indentation, the rejected prefixes and
//! the diagnostics that tell an author how to recover.

use proptest::prelude::*;
use rstest::rstest;

use super::parse;

/// Diagnostic shared by every container-hidden example.
const CONTAINER_REMEDY: &str = concat!(
    "tested examples inside a list item or indented block are not scanned; ",
    "move the fence out of the list and indent it by at most three spaces"
);

#[rstest]
#[case::list_continuation(
    "Prose.\n\n1. An example:\n\n    ```rust\n    fn main() {}\n    ```\n",
    5
)]
#[case::indented_marker(
    "Prose.\n\n    <!-- tested-example: guide-001 -->\n    ```rust\n    fn main() {}\n    ```\n",
    3
)]
#[case::hidden_fence_after_marker(
    "<!-- tested-example: guide-001 -->\n    ```rust\n    fn main() {}\n    ```\n",
    2
)]
#[case::tab_continuation("\t```rust\n\tfn main() {}\n\t```\n", 1)]
#[case::bullet_fence("- ```rust\n", 1)]
#[case::bullet_marker("- <!-- tested-example: guide-001 -->\n", 1)]
#[case::ordered_fence("10. ```rust\n", 1)]
#[case::quote_then_bullet("> - ```rust\n", 1)]
#[case::bullet_then_quote("- > ```rust\n", 1)]
#[case::deep_quoted_indent("Prose.\n\n>     ```rust\n", 3)]
#[case::tilde_indent("    ~~~rust\n", 1)]
#[case::tilde_bullet("* ~~~rust\n", 1)]
#[case::plus_bullet("+ ```rust\n", 1)]
#[case::paren_ordered("1) ```rust\n", 1)]
fn container_hidden_examples_are_rejected(#[case] source: &str, #[case] line: usize) {
    let error = parse(source).expect_err("a container-hidden example must fail the scan");

    assert_eq!(error, format!("guide.md:{line}: {CONTAINER_REMEDY}"));
}

#[rstest]
#[case::ten_digit_marker("1234567890. ```rust\n")]
#[case::dashes("---\n")]
#[case::stars("***\n")]
#[case::arrow("--> ```rust\n")]
#[case::plain_indented_code("Prose.\n\n    let example = \"plain\";\n")]
fn lines_without_a_hidden_example_are_accepted(#[case] source: &str) {
    parse(source).expect("the line must not be mistaken for a hidden example");
}

#[rstest]
#[case::one_space(" ")]
#[case::two_spaces("  ")]
#[case::three_spaces("   ")]
fn marked_top_level_fences_accept_up_to_three_leading_spaces(#[case] padding: &str) {
    for marker in ['`', '~'] {
        let fence = marker.to_string().repeat(3);
        let source = format!(
            "<!-- tested-example: guide-001 -->\n{padding}{fence}rust\n{padding}fn main() \
             {{}}\n{padding}{fence}\n"
        );
        let parsed = parse(&source).expect("a top-level fence may be indented up to three spaces");
        assert_eq!(parsed.examples.len(), 1);
        assert_eq!(
            parsed.examples[0].source,
            format!("{padding}fn main() {{}}\n")
        );
    }
}

#[test]
fn list_continuations_and_example_bodies_keep_their_source() {
    let continuation = concat!(
        "- An example:\n\n",
        "  <!-- tested-example: guide-001 -->\n",
        "  ```rust\n",
        "  fn main() {}\n",
        "  ```\n",
    );
    let parsed = parse(continuation).expect("a two-space continuation is still scanned");
    assert_eq!(parsed.examples[0].source, "  fn main() {}\n");

    let body = concat!(
        "    ```rust\n",
        "\t<!-- tested-example: inner-001 -->\n",
        "- ```text\n",
        "1. ~~~rust\n",
    );
    let source = format!("<!-- tested-example: guide-001 -->\n```markdown\n{body}```\n");
    let parsed = parse(&source).expect("an open example hides later lines from the walk");

    assert_eq!(parsed.examples.len(), 1);
    assert_eq!(parsed.examples[0].source, body);
    assert_eq!(parsed.identifiers, vec![("guide-001".to_owned(), 1)]);
}

/// Strategies for the prefixes the loader treats as a list container.
fn container_prefix() -> impl Strategy<Value = String> {
    prop_oneof![
        (4usize..9).prop_map(|spaces| " ".repeat(spaces)),
        Just("\t".to_owned()),
        "[*+-] ",
        "[1-9][0-9]{0,8}[.)] ",
        "[*+-] > ",
        "> [*+-] ",
    ]
}

proptest! {
    #[test]
    fn container_prefixed_fences_are_rejected_on_their_own_line(
        prefix in container_prefix(),
        fence_length in 3usize..6,
        use_tilde in any::<bool>(),
    ) {
        let marker = if use_tilde { '~' } else { '`' };
        let fence = marker.to_string().repeat(fence_length);
        let source = format!("Prose.\n\n{prefix}{fence}rust\n");

        let error = parse(&source).expect_err("a container-hidden fence must fail the scan");
        prop_assert_eq!(error, format!("guide.md:3: {CONTAINER_REMEDY}"));
    }
}
