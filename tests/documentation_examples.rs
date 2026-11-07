//! Integration coverage for marked examples in published user-facing guides.

#[path = "common/tested_examples.rs"]
mod tested_examples;

use std::collections::HashSet;

use rstest::rstest;

#[test]
fn published_examples_load_from_the_marked_documents() {
    let examples = tested_examples::load_published_examples()
        .expect("every published example should have a unique valid marker");

    assert!(
        !examples.is_empty(),
        "the published guides should expose examples"
    );
    assert!(
        examples
            .iter()
            .all(|example| !example.source.trim().is_empty())
    );
}

#[rstest]
#[case("```rust\nfn main() {}\n```", "missing its marker")]
#[case(
    "<!-- tested-example: sample-1 -->\n```rust\nfn main() {}",
    "unterminated fence"
)]
#[case(
    "<!-- tested-example: sample-1 -->\n```\ntext\n```",
    "no language identifier"
)]
#[case(
    "<!-- tested-example: sample one -->\n```rust\nfn main() {}\n```",
    "identifier `sample one` is invalid"
)]
fn malformed_examples_fail_with_a_useful_reason(#[case] source: &str, #[case] expected: &str) {
    let document = tested_examples::PublishedDocument::new("fixture.md", source);
    let error = tested_examples::parse_document(&document, &mut HashSet::new())
        .expect_err("malformed examples should fail loading");

    assert!(error.contains(expected), "unexpected loader error: {error}");
}

#[test]
fn duplicate_identifiers_fail_across_documents() {
    let mut identifiers = std::collections::HashSet::new();
    let first = "<!-- tested-example: sample-1 -->\n```rust\nfn main() {}\n```";
    let duplicate = "<!-- tested-example: sample-1 -->\n~~~rust\nfn main() {}\n~~~";

    tested_examples::parse_document(
        &tested_examples::PublishedDocument::new("first.md", first),
        &mut identifiers,
    )
    .expect("first identifier should be accepted");
    let error = tested_examples::parse_document(
        &tested_examples::PublishedDocument::new("second.md", duplicate),
        &mut identifiers,
    )
    .expect_err("duplicate identifiers should fail loading");

    assert!(error.contains("duplicate tested-example identifier `sample-1`"));
}
