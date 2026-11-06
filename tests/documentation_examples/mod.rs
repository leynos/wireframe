//! Loads fenced examples from user-facing Markdown for executable contracts.
//!
//! Every fence in each document listed in [`DOCUMENTS`] must follow a
//! `tested-example` marker, so an example cannot be published without an owner
//! in the test suite. Rust fences then run as doctests through the
//! `#[cfg(doctest)]` items in `src/lib.rs`; other languages are validated by
//! `tests/documentation_examples_tests.rs`.
//!
//! The documents are embedded with `include_str!` so tests read the shipped
//! text rather than a copy, and Cargo rebuilds them when a document changes.

use std::collections::HashSet;

/// A documented file's path and its contents, embedded at compile time.
pub const DOCUMENTS: &[(&str, &str)] = &[
    ("README.md", include_str!("../../README.md")),
    (
        "docs/users-guide.md",
        include_str!("../../docs/users-guide.md"),
    ),
    (
        "docs/v0-4-0-migration-guide.md",
        include_str!("../../docs/v0-4-0-migration-guide.md"),
    ),
];

const MARKER_PREFIX: &str = "<!-- tested-example: ";
const MARKER_SUFFIX: &str = " -->";
const EMPTY_MARKER: &str = "<!-- tested-example: -->";

/// One marked fenced example loaded from a user-facing document.
#[derive(Debug, Eq, PartialEq)]
pub struct DocumentedExample {
    /// Document the example was read from.
    pub source: &'static str,
    /// Stable identifier declared by the `tested-example` marker.
    pub id: String,
    /// Fence info string, such as `rust,no_run` or `toml`.
    pub info: String,
    /// Exact text inside the fence, including a trailing newline.
    pub body: String,
}

impl DocumentedExample {
    /// Language named by the info string, without rustdoc attributes.
    #[must_use]
    pub fn language(&self) -> &str { self.info.split(',').next().unwrap_or_default() }

    /// Report whether rustdoc would treat this fence as a doctest.
    #[must_use]
    pub fn is_rust(&self) -> bool { self.language() == "rust" }
}

/// Load every marked example, rejecting unmarked or duplicate fences.
///
/// # Errors
///
/// Returns a located message when a marker or fence is malformed, a fence is
/// unmarked or unterminated, or an identifier is used twice across documents.
pub fn load_documented_examples() -> Result<Vec<DocumentedExample>, String> {
    let mut examples = Vec::new();
    for &(source, contents) in DOCUMENTS {
        examples.extend(parse_document(source, contents)?);
    }
    let mut ids = HashSet::new();
    for example in &examples {
        if !ids.insert(example.id.as_str()) {
            return Err(format!(
                "duplicate tested-example identifier '{}'",
                example.id
            ));
        }
    }
    Ok(examples)
}

/// Load the documented example identified by `id`.
///
/// # Errors
///
/// Returns a message when the documents are invalid or `id` is absent.
pub fn documented_example(id: &str) -> Result<DocumentedExample, String> {
    load_documented_examples()?
        .into_iter()
        .find(|example| example.id == id)
        .ok_or_else(|| format!("documented example '{id}' should exist"))
}

/// Parse one document's fences, requiring a marker before each.
///
/// # Errors
///
/// Returns a located message for any malformed marker or fence.
pub fn parse_document(
    source: &'static str,
    contents: &str,
) -> Result<Vec<DocumentedExample>, String> {
    let mut lines = contents.lines().enumerate();
    let mut examples = Vec::new();
    while let Some((index, line)) = lines.next() {
        let locate = |message: &str| format!("{source}:{} {message}", index + 1);
        if line.trim_end() == EMPTY_MARKER {
            return Err(locate("tested-example identifier must not be empty"));
        }
        if let Some(id) = parse_marker(line) {
            if id.trim().is_empty() {
                return Err(locate("tested-example identifier must not be empty"));
            }
            examples.push(read_marked_example(source, index, id, &mut lines)?);
        } else if FenceOpening::parse(line).is_some() {
            return Err(locate("fence is missing a tested-example marker"));
        } else if line != line.trim_start() && FenceOpening::parse(line.trim_start()).is_some() {
            // Rustdoc still runs an indented fence, but this scanner would not
            // see it, so it could escape the marker requirement unnoticed.
            return Err(locate("fence must start at column 0"));
        }
    }
    Ok(examples)
}

fn parse_marker(line: &str) -> Option<&str> {
    line.strip_prefix(MARKER_PREFIX)?
        .strip_suffix(MARKER_SUFFIX)
}

fn read_marked_example<'a>(
    source: &'static str,
    marker_index: usize,
    id: &str,
    lines: &mut impl Iterator<Item = (usize, &'a str)>,
) -> Result<DocumentedExample, String> {
    let (fence_index, fence) = lines
        .find(|(_, line)| !line.is_empty())
        .ok_or_else(|| format!("{source}:{} marker has no fence", marker_index + 1))?;
    let opening = FenceOpening::parse(fence).ok_or_else(|| {
        format!(
            "{source}:{} expected an opening fence after marker",
            fence_index + 1
        )
    })?;
    if opening.info.is_empty() {
        return Err(format!(
            "{source}:{} fence should declare a language",
            fence_index + 1
        ));
    }
    let body = read_fence_body(source, fence_index, opening, lines)?;
    Ok(DocumentedExample {
        source,
        id: id.to_owned(),
        info: opening.info.to_owned(),
        body,
    })
}

/// The opening line of a fenced code block.
///
/// A fence is a run of at least three backticks or tildes at column 0, closed
/// only by a run of the same character at least as long, so a longer fence can
/// quote a shorter one.
#[derive(Clone, Copy)]
struct FenceOpening<'a> {
    delimiter: char,
    run_length: usize,
    info: &'a str,
}

impl<'a> FenceOpening<'a> {
    /// Recognize `line` as a fence opening, capturing its delimiter and run.
    fn parse(line: &'a str) -> Option<Self> {
        let delimiter = line.chars().next().filter(|ch| matches!(ch, '`' | '~'))?;
        let info = line.trim_start_matches(delimiter);
        // Both delimiters are one byte, so the byte difference is the run.
        let run_length = line.len() - info.len();
        (run_length >= 3).then_some(Self {
            delimiter,
            run_length,
            info: info.trim(),
        })
    }

    /// Report whether `line` closes this fence.
    fn is_closed_by(self, line: &str) -> bool {
        let candidate = line.trim_end();
        candidate.len() >= self.run_length && candidate.chars().all(|ch| ch == self.delimiter)
    }
}

fn read_fence_body<'a>(
    source: &str,
    fence_index: usize,
    opening: FenceOpening<'_>,
    lines: &mut impl Iterator<Item = (usize, &'a str)>,
) -> Result<String, String> {
    let mut body = String::new();
    for (_, line) in lines {
        if opening.is_closed_by(line) {
            return Ok(body);
        }
        body.push_str(line);
        body.push('\n');
    }
    Err(format!(
        "{source}:{} fence is not terminated",
        fence_index + 1
    ))
}

/// Report the first hidden rustdoc line in a Rust example body, if any.
///
/// Rustdoc strips lines beginning with `# `, but GitHub and other Markdown
/// renderers show them verbatim. A reader copying the example would then get
/// something other than what the test suite compiled.
#[must_use]
pub fn hidden_rustdoc_line(body: &str) -> Option<&str> {
    body.lines().find(|line| {
        let trimmed = line.trim_start();
        trimmed == "#" || (trimmed.starts_with("# ") && !trimmed.starts_with("#!"))
    })
}
