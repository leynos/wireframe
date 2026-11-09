//! Unit and property tests for the shared documentation example loader.

use std::collections::HashSet;

use proptest::prelude::*;

use super::{
    PublishedDocument,
    document_paths,
    load_documents,
    load_published_examples,
    parse_document,
    rustdoc_document_paths,
};

const SAMPLE: &str = "Prose.\n\n<!-- tested-example: guide-001 -->\n```rust\nfn main() {}\n```\n";

fn parse(source: &str) -> Result<super::ParsedDocument, String> {
    parse_document(&PublishedDocument::new("guide.md", source), &HashSet::new())
}

#[test]
fn parsing_reports_the_example_identity_and_source() {
    let parsed = parse(SAMPLE).expect("the sample document is valid");

    assert_eq!(parsed.examples.len(), 1);
    let example = &parsed.examples[0];
    assert_eq!(example.id, "guide-001");
    assert_eq!(example.document, "guide.md");
    assert_eq!(example.language, "rust");
    assert_eq!(example.source, "fn main() {}\n");
    assert_eq!(example.start_line, 4);
    assert_eq!(parsed.identifiers, vec![("guide-001".to_owned(), 3)]);
}

#[test]
fn loader_inventory_matches_the_rustdoc_module_list() {
    let mut loader: Vec<_> = document_paths().collect();
    let mut rustdoc: Vec<_> = rustdoc_document_paths().collect();
    loader.sort_unstable();
    rustdoc.sort_unstable();

    assert_eq!(loader, rustdoc, "the two document lists must agree");
}

#[test]
fn published_inventory_yields_examples_for_every_document() {
    let examples = load_published_examples().expect("the published inventory parses");

    for document in document_paths() {
        assert!(
            examples.iter().any(|example| example.document == document),
            "{document} should contribute at least one example"
        );
    }
}

#[test]
fn documents_load_in_order_and_share_identifiers() {
    let first = "<!-- tested-example: first-001 -->\n```rust\nfn main() {}\n```\n";
    let second = "<!-- tested-example: second-001 -->\n~~~rust\nfn main() {}\n~~~\n";

    let examples = load_documents(&[("first.md", first), ("second.md", second)])
        .expect("distinct identifiers are accepted");

    assert_eq!(
        examples
            .iter()
            .map(|example| example.id.as_str())
            .collect::<Vec<_>>(),
        vec!["first-001", "second-001"]
    );
}

#[test]
fn duplicates_across_documents_are_rejected_by_the_merge_boundary() {
    let first = "<!-- tested-example: shared-001 -->\n```rust\nfn main() {}\n```\n";
    let duplicate = "<!-- tested-example: shared-001 -->\n~~~rust\nfn main() {}\n~~~\n";

    let error = load_documents(&[("first.md", first), ("second.md", duplicate)])
        .expect_err("a duplicate identifier must fail loading");

    assert!(
        error.contains("duplicate tested-example identifier `shared-001`"),
        "unexpected loader error: {error}"
    );
}

#[test]
fn a_rejected_document_reports_no_claims() {
    let source = "<!-- tested-example: guide-001 -->\n```\ntext\n```\n";
    let known = HashSet::new();

    let error = parse_document(&PublishedDocument::new("guide.md", source), &known)
        .expect_err("a fence without a language must fail");

    assert!(
        error.contains("no language identifier"),
        "unexpected loader error: {error}"
    );
    assert!(known.is_empty(), "the caller's identifier set is untouched");
}

proptest! {
    #[test]
    fn marked_fences_round_trip_their_source(
        body in prop::collection::vec("[A-Za-z0-9 ._/-]{0,30}", 0..5),
        fence_length in 3usize..6,
        indentation in 0usize..4,
    ) {
        let ticks = "`".repeat(fence_length);
        let padding = " ".repeat(indentation);
        let source = format!(
            "<!-- tested-example: sample-001 -->\n{padding}{ticks}rust\n{}\n{padding}{ticks}\n",
            body.join("\n"),
        );
        let expected = if body.is_empty() {
            String::new()
        } else {
            format!("{}\n", body.join("\n"))
        };

        let parsed = parse(&source).expect("a well-formed fence parses");
        prop_assert_eq!(parsed.examples.len(), 1);
        prop_assert_eq!(&parsed.examples[0].source, &expected);
        prop_assert_eq!(parsed.examples[0].start_line, 2);
    }
}
