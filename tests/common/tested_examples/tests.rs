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

/// Parse `source` as `guide.md` with no identifiers claimed by earlier documents.
fn parse(source: &str) -> Result<super::ParsedDocument, String> {
    parse_document(&PublishedDocument::new("guide.md", source), &HashSet::new())
}

/// Build a one-example document that uses `identifier`.
fn document_with(identifier: &str) -> String {
    format!("<!-- tested-example: {identifier} -->\n```rust\nfn main() {{}}\n```\n")
}

#[test]
fn parsing_reports_the_example_identity_and_source() {
    let parsed = parse(SAMPLE).expect("the sample document is valid");

    assert_eq!(parsed.examples.len(), 1);
    let example = parsed
        .examples
        .first()
        .expect("the sample yields one example");
    assert_eq!(example.id, "guide-001");
    assert_eq!(example.document, "guide.md");
    assert_eq!(example.language, "rust");
    assert_eq!(example.source, "fn main() {}\n");
    assert_eq!(example.start_line, 4);
    assert_eq!(parsed.identifiers, vec![("guide-001".to_owned(), 3)]);
}

#[test]
fn a_successful_parse_leaves_the_callers_identifiers_untouched() {
    let known = HashSet::from(["earlier-001".to_owned()]);

    let parsed = parse_document(&PublishedDocument::new("guide.md", SAMPLE), &known)
        .expect("the sample document is valid");

    assert_eq!(parsed.identifiers, vec![("guide-001".to_owned(), 3)]);
    assert_eq!(
        known,
        HashSet::from(["earlier-001".to_owned()]),
        "the caller merges identifiers only after a successful parse"
    );
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
    let first = document_with("first-001");
    let second = document_with("second-001");

    let examples = load_documents(&[("first.md", first.as_str()), ("second.md", second.as_str())])
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
fn multiple_examples_load_in_document_and_source_order() {
    let first = concat!(
        "Prose.\n\n",
        "<!-- tested-example: first-001 -->\n```rust\nfn first() {}\n```\n\n",
        "<!-- tested-example: first-002 -->\n```rust,no_run\nfn second() {}\n```\n",
    );
    let second = concat!(
        "# Second\n\n",
        "<!-- tested-example: second-001 -->\n```text\nalpha\nbeta\n```\n",
    );

    let examples = load_documents(&[("first.md", first), ("second.md", second)])
        .expect("distinct identifiers are accepted");

    let observed: Vec<(&str, &str, &str, &str, usize)> = examples
        .iter()
        .map(|example| {
            (
                example.id.as_str(),
                example.document.as_str(),
                example.language.as_str(),
                example.source.as_str(),
                example.start_line,
            )
        })
        .collect();
    #[rustfmt::skip]
    let expected = vec![
        ("first-001", "first.md", "rust", "fn first() {}\n", 4),
        ("first-002", "first.md", "rust", "fn second() {}\n", 9),
        ("second-001", "second.md", "text", "alpha\nbeta\n", 4),
    ];
    assert_eq!(observed, expected);
}

#[test]
fn duplicates_across_documents_are_rejected_by_the_merge_boundary() {
    let first = document_with("shared-001");
    let duplicate = document_with("shared-001");

    let error = load_documents(&[
        ("first.md", first.as_str()),
        ("second.md", duplicate.as_str()),
    ])
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

#[test]
fn a_fence_inside_a_block_quote_is_rejected() {
    let source = "Prose.\n\n> ```rust\n> fn quoted() {}\n> ```\n";

    let error = parse(source).expect_err("a quoted fence must fail the scan");

    assert!(
        error.contains("inside a block quote are not scanned"),
        "unexpected loader error: {error}"
    );
}

#[test]
fn a_marker_inside_a_block_quote_is_rejected() {
    let source = concat!(
        "Prose.\n\n",
        "> <!-- tested-example: guide-001 -->\n",
        "> ```rust\n> fn quoted() {}\n> ```\n",
    );

    let error = parse(source).expect_err("a quoted marker must fail the scan");

    assert!(
        error.contains("inside a block quote are not scanned"),
        "unexpected loader error: {error}"
    );
}

#[test]
fn quoted_markdown_inside_an_example_is_extracted_verbatim() {
    // A top-level fence may show block-quoted Markdown; only a quote that
    // contains the example itself is rejected.
    let source = concat!(
        "<!-- tested-example: guide-001 -->\n",
        "```markdown\n",
        "> ```text\n> example\n> ```\n",
        "```\n",
    );

    let parsed = parse(source).expect("a quoted body inside an example is allowed");

    assert_eq!(parsed.examples.len(), 1);
    assert_eq!(
        parsed
            .examples
            .first()
            .expect("the fence yields one example")
            .source,
        "> ```text\n> example\n> ```\n"
    );
}

proptest! {
    #[test]
    fn marked_fences_round_trip_their_source(
        body in prop::collection::vec("[A-Za-z0-9 ._/-]{0,30}", 0..5),
        fence_length in 3usize..6,
        indentation in 0usize..4,
        use_tilde in any::<bool>(),
    ) {
        let marker = if use_tilde { '~' } else { '`' };
        let fence = marker.to_string().repeat(fence_length);
        // A shorter run of the same character cannot close the fence, so it must
        // survive inside the extracted source.
        let interior = format!("{}\n{}", body.join("\n"), marker.to_string().repeat(fence_length - 1));
        let padding = " ".repeat(indentation);
        let source = format!(
            "<!-- tested-example: sample-001 -->\n{padding}{fence}rust\n{interior}\n{padding}{fence}\n",
        );

        let parsed = parse(&source).expect("a well-formed fence parses");
        prop_assert_eq!(parsed.examples.len(), 1);
        let example = parsed.examples.first().expect("the fence yields one example");
        prop_assert_eq!(&example.id, "sample-001");
        prop_assert_eq!(&example.source, &format!("{interior}\n"));
        prop_assert_eq!(example.start_line, 2);
        prop_assert_eq!(parsed.identifiers, vec![("sample-001".to_owned(), 1)]);
    }

    #[test]
    fn duplicate_identifiers_are_rejected_whatever_their_spelling(
        first in "[a-z][a-z0-9-]{0,10}",
        second in "[a-z][a-z0-9-]{0,10}",
    ) {
        let first_document = document_with(&first);
        let second_document = document_with(&second);
        let result = load_documents(&[
            ("first.md", first_document.as_str()),
            ("second.md", second_document.as_str()),
        ]);

        prop_assert_eq!(result.is_err(), first == second);
    }

    #[test]
    fn one_identifier_claimed_by_two_documents_is_rejected(identifier in "[a-z][a-z0-9-]{0,10}") {
        let first = document_with(&identifier);
        let second = document_with(&identifier);

        let error = load_documents(&[
            ("first.md", first.as_str()),
            ("second.md", second.as_str()),
        ])
        .expect_err("an identifier claimed by two documents must fail loading");

        prop_assert!(
            error.contains(&format!("duplicate tested-example identifier `{identifier}`")),
            "unexpected loader error: {}",
            error
        );
    }

    #[test]
    fn quoted_fences_are_rejected_at_any_quote_depth(depth in 1usize..5) {
        let quote = "> ".repeat(depth);
        let source = format!("Prose.\n\n{quote}```rust\n{quote}fn main() {{}}\n{quote}```\n");

        let error = parse(&source).expect_err("a quoted fence must fail the scan");
        prop_assert!(
            error.contains("inside a block quote are not scanned"),
            "unexpected loader error: {}",
            error
        );
    }
}
