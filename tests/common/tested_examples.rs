//! Shared parser for marked examples in published user-facing documents.
//!
//! The parser walks a document once, pairing every `tested-example` marker with
//! the fence that follows it. Parsing is a query: it reports the identifiers a
//! document claims instead of writing them into caller-owned state, so a
//! document that fails validation cannot leave a partial claim behind.

use std::collections::HashSet;

#[path = "tested_examples/parsing.rs"]
mod parsing;

#[cfg(test)]
#[path = "tested_examples/tests.rs"]
mod tests;

use parsing::{Fence, fence_info, fence_source, is_closing_fence, opening_fence, parse_marker};

const USER_DOCUMENTS: &[(&str, &str)] = &[
    ("docs/api.md", include_str!("../../docs/api.md")),
    (
        "docs/developers-guide.md",
        include_str!("../../docs/developers-guide.md"),
    ),
    (
        "docs/users-guide.md",
        include_str!("../../docs/users-guide.md"),
    ),
    (
        "docs/v0-1-0-to-v0-2-0-migration-guide.md",
        include_str!("../../docs/v0-1-0-to-v0-2-0-migration-guide.md"),
    ),
    (
        "docs/v0-2-0-to-v0-3-0-migration-guide.md",
        include_str!("../../docs/v0-2-0-to-v0-3-0-migration-guide.md"),
    ),
    (
        "docs/v0-3-0-to-v0-4-0-migration-guide.md",
        include_str!("../../docs/v0-3-0-to-v0-4-0-migration-guide.md"),
    ),
    (
        "docs/rstest-bdd-v0-6-0-migration-guide.md",
        include_str!("../../docs/rstest-bdd-v0-6-0-migration-guide.md"),
    ),
    (
        "docs/hardening-wireframe-a-guide-to-production-resilience.md",
        include_str!("../../docs/hardening-wireframe-a-guide-to-production-resilience.md"),
    ),
    (
        "docs/wireframe-testing-crate.md",
        include_str!("../../docs/wireframe-testing-crate.md"),
    ),
    (
        "docs/rstest-bdd-users-guide.md",
        include_str!("../../docs/rstest-bdd-users-guide.md"),
    ),
    (
        "docs/wireframe-rstest-bdd-users-guide.md",
        include_str!("../../docs/wireframe-rstest-bdd-users-guide.md"),
    ),
    (
        "docs/whitaker-users-guide.md",
        include_str!("../../docs/whitaker-users-guide.md"),
    ),
    (
        "docs/server/configuration.md",
        include_str!("../../docs/server/configuration.md"),
    ),
    (
        "docs/mocking-network-outages-in-rust.md",
        include_str!("../../docs/mocking-network-outages-in-rust.md"),
    ),
    (
        "docs/rust-testing-with-rstest-fixtures.md",
        include_str!("../../docs/rust-testing-with-rstest-fixtures.md"),
    ),
    (
        "docs/behavioural-testing-in-rust-with-cucumber.md",
        include_str!("../../docs/behavioural-testing-in-rust-with-cucumber.md"),
    ),
    (
        "docs/rust-doctest-dry-guide.md",
        include_str!("../../docs/rust-doctest-dry-guide.md"),
    ),
    (
        "examples/async_stream.md",
        include_str!("../../examples/async_stream.md"),
    ),
    (
        "examples/multi_packet.md",
        include_str!("../../examples/multi_packet.md"),
    ),
    (
        "examples/ping_pong.md",
        include_str!("../../examples/ping_pong.md"),
    ),
];

/// A fenced example extracted from the exact published Markdown source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestedExample {
    /// Stable identifier written in the marker comment.
    pub id: String,
    /// Repository-relative path of the Markdown source.
    pub document: String,
    /// Language identifier attached to the Markdown fence.
    pub language: String,
    /// Exact text between the opening and closing fences.
    pub source: String,
    /// One-based line containing the opening fence.
    pub start_line: usize,
}

/// A published Markdown document split into the lines scanned for examples.
///
/// Grouping the path with its lines keeps the parser helpers from threading a
/// bare path and a bare line slice through every call, and gives diagnostics a
/// single source for the document name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishedDocument<'a> {
    path: &'a str,
    lines: Vec<&'a str>,
}

impl<'a> PublishedDocument<'a> {
    /// Split `text` into the lines scanned for tested examples.
    pub fn new(path: &'a str, text: &'a str) -> Self {
        Self {
            path,
            lines: text.lines().collect(),
        }
    }

    fn path(&self) -> &'a str { self.path }

    fn line(&self, index: usize) -> Option<&'a str> { self.lines.get(index).copied() }

    fn lines(&self) -> &[&'a str] { &self.lines }
}

/// A document's examples together with the identifiers it claimed.
///
/// Returning the claims instead of writing them into a caller-owned set keeps
/// parsing a query, so a rejected document leaves no partial identifier behind.
#[derive(Debug, Default)]
pub struct ParsedDocument {
    /// Examples in the order they appear in the document.
    pub examples: Vec<TestedExample>,
    /// Identifier and one-based marker line for each claimed example.
    pub identifiers: Vec<(String, usize)>,
}

/// An example whose opening fence has been read but not yet closed.
#[derive(Debug)]
struct OpenExample {
    /// The fence that opened the example, used to detect its close.
    fence: Fence,
    /// Stable identifier written in the preceding marker comment.
    id: String,
    /// Language named by the first token of the fence info string.
    language: String,
    /// One-based line containing the opening fence.
    start_line: usize,
}

/// Repository-relative paths of every document the shared loader scans.
#[cfg(test)]
fn document_paths() -> impl Iterator<Item = &'static str> {
    USER_DOCUMENTS.iter().map(|(path, _)| *path)
}

/// Repository-relative paths named by the Rustdoc module list.
///
/// The list lives in `src/tested_documentation.rs` as `include_str!`
/// attributes, so reading it at compile time keeps the two inventories
/// comparable without a second hand-maintained copy.
#[cfg(test)]
fn rustdoc_document_paths() -> impl Iterator<Item = &'static str> {
    include_str!("../../src/tested_documentation.rs")
        .lines()
        .filter_map(|line| {
            line.trim()
                .strip_prefix("#[doc = include_str!(\"../")?
                .strip_suffix("\")]")
        })
}

/// Load examples from an explicit document list, sharing identifiers.
///
/// This is the merge boundary: a document's identifiers enter the shared set
/// only after it parses cleanly, so a later duplicate is rejected without
/// corrupting the claims made by earlier documents.
pub fn load_documents(documents: &[(&str, &str)]) -> Result<Vec<TestedExample>, String> {
    let mut identifiers = HashSet::new();
    let mut examples = Vec::new();
    for (path, text) in documents {
        let parsed = parse_document(&PublishedDocument::new(path, text), &identifiers)?;
        identifiers.extend(parsed.identifiers.into_iter().map(|(id, _)| id));
        examples.extend(parsed.examples);
    }
    Ok(examples)
}

/// Load every marked example from the published user-facing document set.
pub fn load_published_examples() -> Result<Vec<TestedExample>, String> {
    load_documents(USER_DOCUMENTS)
}

/// Parse one document without mutating caller-owned state.
///
/// `known` carries the identifiers claimed by earlier documents so a duplicate
/// is rejected here; the returned identifiers are left for the caller to merge
/// after a successful parse.
pub fn parse_document(
    document: &PublishedDocument<'_>,
    known: &HashSet<String>,
) -> Result<ParsedDocument, String> {
    let mut parsed = ParsedDocument::default();
    let mut open: Option<OpenExample> = None;

    for (index, line) in document.lines().iter().copied().enumerate() {
        if let Some(active) = open.as_ref() {
            if is_closing_fence(line, active.fence) {
                parsed
                    .examples
                    .push(finish_example(document, active, index)?);
                open = None;
            }
            continue;
        }

        validate_marker_line(document, index)?;
        open = parse_open_example(document, index, known, &mut parsed)?;
    }

    if let Some(active) = open {
        return Err(format!(
            "{}:{}: tested example `{}` has an unterminated fence",
            document.path(),
            active.start_line,
            active.id
        ));
    }

    Ok(parsed)
}

/// Reject a marker comment that is malformed or not attached to a fence.
///
/// A marker is only meaningful when the very next line opens a fence; reporting
/// the mismatch here points the author at the marker rather than at a later
/// "missing its marker" error on the fence itself.
fn validate_marker_line(document: &PublishedDocument<'_>, index: usize) -> Result<(), String> {
    let Some(line) = document.line(index) else {
        return Ok(());
    };
    if !line.trim_start().starts_with("<!-- tested-example") {
        return Ok(());
    }
    let path = document.path();
    let line_number = index + 1;
    parse_marker(line)
        .ok_or_else(|| format!("{path}:{line_number}: malformed tested-example marker"))?;
    let next_line = document
        .line(index + 1)
        .ok_or_else(|| format!("{path}:{line_number}: tested-example marker has no fence"))?;
    if opening_fence(next_line).is_none() {
        return Err(format!(
            "{path}:{line_number}: tested-example marker must immediately precede a fence"
        ));
    }
    Ok(())
}

/// Record the example that `index` opens, or return `None` for other lines.
///
/// The identifier is validated before it is recorded, so a rejected example
/// never appears in the returned claims.
fn parse_open_example(
    document: &PublishedDocument<'_>,
    index: usize,
    known: &HashSet<String>,
    parsed: &mut ParsedDocument,
) -> Result<Option<OpenExample>, String> {
    let Some(line) = document.line(index) else {
        return Ok(None);
    };
    let Some(fence) = opening_fence(line) else {
        return Ok(None);
    };
    let path = document.path();
    let line_number = index + 1;
    let marker_line = index
        .checked_sub(1)
        .and_then(|previous| document.line(previous));
    let id = marker_line
        .and_then(parse_marker)
        .ok_or_else(|| format!("{path}:{line_number}: fenced example is missing its marker"))?;
    if !valid_identifier(&id) {
        return Err(format!(
            "{path}:{line_number}: tested-example identifier `{id}` is invalid"
        ));
    }
    let is_duplicate =
        known.contains(&id) || parsed.identifiers.iter().any(|(seen, _)| *seen == id);
    if is_duplicate {
        return Err(format!(
            "{path}:{line_number}: duplicate tested-example identifier `{id}`"
        ));
    }

    let language = fence_info(line, fence)
        .split(|character: char| character.is_whitespace() || character == ',')
        .next()
        .unwrap_or_default();
    if language.is_empty()
        || matches!(
            language,
            "no_run" | "ignore" | "compile_fail" | "should_panic"
        )
    {
        return Err(format!(
            "{path}:{line_number}: fenced example has no language identifier"
        ));
    }
    // `index` is zero-based for the fence and therefore one-based for the
    // marker line immediately above it.
    parsed.identifiers.push((id.clone(), index));
    Ok(Some(OpenExample {
        fence,
        id,
        language: language.to_owned(),
        start_line: line_number,
    }))
}

/// Build the example whose closing fence is on `closing_line`.
fn finish_example(
    document: &PublishedDocument<'_>,
    active: &OpenExample,
    closing_line: usize,
) -> Result<TestedExample, String> {
    let Some(source) = fence_source(document, active.start_line, closing_line) else {
        return Err(format!(
            "{}:{}: invalid tested-example fence boundaries",
            document.path(),
            active.start_line
        ));
    };
    Ok(TestedExample {
        id: active.id.clone(),
        document: document.path().to_owned(),
        language: active.language.clone(),
        source,
        start_line: active.start_line,
    })
}

/// Return whether `identifier` follows the marker grammar.
///
/// Identifiers appear in test names and diagnostics, so they are restricted to
/// lower-case ASCII letters, digits, `-` and `_`, and must not start with `-`
/// or `_`.
fn valid_identifier(identifier: &str) -> bool {
    identifier.chars().all(|character| {
        character.is_ascii_lowercase()
            || character.is_ascii_digit()
            || character == '-'
            || character == '_'
    }) && identifier
        .chars()
        .next()
        .is_some_and(|first| first.is_ascii_lowercase() || first.is_ascii_digit())
}
