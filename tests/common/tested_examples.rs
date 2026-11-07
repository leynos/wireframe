//! Shared parser for marked examples in published user-facing documents.

use std::collections::HashSet;

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

#[derive(Debug, Clone, Copy)]
struct Fence {
    marker: char,
    length: usize,
}

#[derive(Debug)]
struct OpenExample {
    fence: Fence,
    id: String,
    language: String,
    start_line: usize,
}

/// Load every marked example from the published user-facing document set.
pub fn load_published_examples() -> Result<Vec<TestedExample>, String> {
    let mut identifiers = HashSet::new();
    let mut examples = Vec::new();
    for (path, text) in USER_DOCUMENTS {
        examples.extend(parse_document(
            &PublishedDocument::new(path, text),
            &mut identifiers,
        )?);
    }
    Ok(examples)
}

/// Parse one source document while sharing identifiers with prior documents.
pub fn parse_document(
    document: &PublishedDocument<'_>,
    identifiers: &mut HashSet<String>,
) -> Result<Vec<TestedExample>, String> {
    let mut examples = Vec::new();
    let mut open: Option<OpenExample> = None;

    for (index, line) in document.lines().iter().copied().enumerate() {
        if let Some(active) = open.as_ref() {
            if is_closing_fence(line, active.fence) {
                examples.push(finish_example(document, active, index)?);
                open = None;
            }
            continue;
        }

        validate_marker_line(document, index)?;
        open = parse_open_example(document, index, identifiers)?;
    }

    if let Some(active) = open {
        return Err(format!(
            "{}:{}: tested example `{}` has an unterminated fence",
            document.path(),
            active.start_line,
            active.id
        ));
    }

    Ok(examples)
}

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

fn parse_open_example(
    document: &PublishedDocument<'_>,
    index: usize,
    identifiers: &mut HashSet<String>,
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
    if !identifiers.insert(id.clone()) {
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
    Ok(Some(OpenExample {
        fence,
        id,
        language: language.to_owned(),
        start_line: line_number,
    }))
}

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

fn parse_marker(line: &str) -> Option<String> {
    let comment = line.trim();
    let body = comment
        .strip_prefix("<!-- tested-example: ")?
        .strip_suffix(" -->")?;
    (!body.is_empty()).then(|| body.to_owned())
}

fn fence_source(
    document: &PublishedDocument<'_>,
    start_line: usize,
    closing_line: usize,
) -> Option<String> {
    let mut source = document.lines().get(start_line..closing_line)?.join("\n");
    source.push_str(if start_line < closing_line { "\n" } else { "" });
    Some(source)
}

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

fn opening_fence(line: &str) -> Option<Fence> {
    let indentation = line
        .chars()
        .take_while(|character| *character == ' ')
        .count();
    if indentation > 3 {
        return None;
    }
    let remainder = line.get(indentation..)?;
    let marker = remainder.chars().next()?;
    if !matches!(marker, '`' | '~') {
        return None;
    }
    let length = remainder
        .chars()
        .take_while(|character| *character == marker)
        .count();
    (length >= 3).then_some(Fence { marker, length })
}

fn is_closing_fence(line: &str, fence: Fence) -> bool {
    let indentation = line
        .chars()
        .take_while(|character| *character == ' ')
        .count();
    if indentation > 3 {
        return false;
    }
    let Some(remainder) = line.get(indentation..) else {
        return false;
    };
    let length = remainder
        .chars()
        .take_while(|character| *character == fence.marker)
        .count();
    remainder
        .get(length..)
        .is_some_and(|suffix| length >= fence.length && suffix.trim().is_empty())
}

fn fence_info(line: &str, fence: Fence) -> &str {
    let indentation = line
        .chars()
        .take_while(|character| *character == ' ')
        .count();
    let Some(remainder) = line.get(indentation..) else {
        return "";
    };
    let info_start = remainder
        .chars()
        .take_while(|character| *character == fence.marker)
        .count();
    remainder.get(info_start..).unwrap_or_default().trim()
}
