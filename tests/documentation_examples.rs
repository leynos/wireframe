//! Integration coverage for marked examples in published user-facing guides.

#[path = "common/repo_access.rs"]
mod repo_access;
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
    let identifiers: HashSet<_> = examples.iter().map(|example| example.id.as_str()).collect();
    assert_eq!(
        identifiers.len(),
        examples.len(),
        "every published example should carry a unique identifier"
    );
}

/// The gate is applied to the shipped file, not only to in-memory samples.
///
/// The test reads `docs/users-guide.md` from disk, removes one marker from that
/// real source, and requires the merge entrypoint used by the published
/// inventory to reject it with the document path and a useful reason. The
/// positive half of the boundary is
/// `published_examples_load_from_the_marked_documents`.
#[test]
fn the_gate_rejects_a_malformed_copy_of_a_shipped_document() {
    const MARKER: &str = "<!-- tested-example: users-guide-001 -->\n";
    let source = repo_access::read_repo_file("docs/users-guide.md")
        .expect("the shipped guide should be readable from the repository root");
    assert!(
        source.contains(MARKER),
        "the shipped guide should carry {MARKER}"
    );
    let mutated = source.replacen(MARKER, "", 1);

    let error = tested_examples::load_documents(&[("docs/users-guide.md", mutated.as_str())])
        .expect_err("a shipped document that lost its marker must fail the gate");

    assert!(
        error.contains("docs/users-guide.md:") && error.contains("missing its marker"),
        "unexpected gate diagnostic: {error}"
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
#[case(
    "<!-- tested-example:sample-1 -->\n```rust\nfn main() {}\n```",
    "malformed tested-example marker"
)]
#[case(
    "<!-- tested-example: sample-1 -->\nprose\n```rust\nfn main() {}\n```",
    "must immediately precede a fence"
)]
#[case("<!-- tested-example: sample-1 -->\n", "marker has no fence")]
fn malformed_examples_fail_with_a_useful_reason(#[case] source: &str, #[case] expected: &str) {
    let document = tested_examples::PublishedDocument::new("fixture.md", source);
    let error = tested_examples::parse_document(&document, &HashSet::new())
        .expect_err("malformed examples should fail loading");

    assert!(error.contains(expected), "unexpected loader error: {error}");
}

#[test]
fn duplicate_identifiers_fail_across_documents() {
    let first = "<!-- tested-example: sample-1 -->\n```rust\nfn main() {}\n```";
    let duplicate = "<!-- tested-example: sample-1 -->\n~~~rust\nfn main() {}\n~~~";

    let error = tested_examples::load_documents(&[("first.md", first), ("second.md", duplicate)])
        .expect_err("duplicate identifiers should fail loading");

    assert!(error.contains("duplicate tested-example identifier `sample-1`"));
}
