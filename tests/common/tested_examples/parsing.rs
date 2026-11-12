//! Fence and marker mechanics for the shared documentation example loader.
//!
//! The parent module decides which fences are examples and in what order; this
//! module answers the narrower questions about a single line: does it open or
//! close a fence, what info string does it carry, where does its body end, and
//! what identifier does a marker comment name. Keeping those rules here leaves
//! the parent free to walk the document and keeps both files within the
//! repository's module size limit.

use super::PublishedDocument;

/// One line of published Markdown, as the parser sees it.
///
/// The parser's unit is a source line rather than a bare string, so wrapping it
/// gives the line-oriented helpers a type instead of an unstructured `&str`.
#[derive(Debug, Clone, Copy)]
pub(super) struct SourceLine<'a> {
    text: &'a str,
}

impl<'a> SourceLine<'a> {
    /// Wrap `text` as one line of published Markdown.
    pub(super) fn new(text: &'a str) -> Self { Self { text } }

    fn text(&self) -> &'a str { self.text }
}

/// A fence marker and the number of characters it repeats.
#[derive(Debug, Clone, Copy)]
pub(super) struct Fence {
    /// The fence character, either a backtick or a tilde.
    pub(super) marker: char,
    /// How many marker characters the fence repeats.
    pub(super) length: usize,
}

/// Return the fence when `line` opens one.
///
/// Markdown lets a fence be indented by up to three spaces while still opening
/// a block, and requires at least three marker characters, so both rules are
/// enforced here rather than at each call site.
pub(super) fn opening_fence(line: SourceLine<'_>) -> Option<Fence> {
    let text = line.text();
    let indentation = text
        .chars()
        .take_while(|character| *character == ' ')
        .count();
    if indentation > 3 {
        return None;
    }
    let remainder = text.get(indentation..)?;
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

/// Return whether `line` closes `fence`.
///
/// A closing fence repeats at least as many marker characters as the opener and
/// carries no info string, so a shorter run inside the body is not mistaken for
/// the close.
pub(super) fn is_closing_fence(line: SourceLine<'_>, fence: Fence) -> bool {
    let text = line.text();
    let indentation = text
        .chars()
        .take_while(|character| *character == ' ')
        .count();
    if indentation > 3 {
        return false;
    }
    let Some(remainder) = text.get(indentation..) else {
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

/// Return the info string that follows the fence marker on `line`.
pub(super) fn fence_info(line: SourceLine<'_>, fence: Fence) -> &str {
    let text = line.text();
    let indentation = text
        .chars()
        .take_while(|character| *character == ' ')
        .count();
    let Some(remainder) = text.get(indentation..) else {
        return "";
    };
    let info_start = remainder
        .chars()
        .take_while(|character| *character == fence.marker)
        .count();
    remainder.get(info_start..).unwrap_or_default().trim()
}

/// Return the text between the opening fence and `closing_line`.
///
/// The body keeps a trailing newline when it has any lines at all, so the value
/// matches the bytes a reader sees between the fences.
pub(super) fn fence_source(
    document: &PublishedDocument<'_>,
    start_line: usize,
    closing_line: usize,
) -> Option<String> {
    let mut source = document.lines().get(start_line..closing_line)?.join("\n");
    source.push_str(if start_line < closing_line { "\n" } else { "" });
    Some(source)
}

/// Return the content of `line` once its block-quote markers are removed.
///
/// CommonMark allows a block quote to contain fenced code, but the loader only
/// scans top-level blocks. Reporting the container here lets the parent reject
/// such a fence with a remedy instead of skipping it without comment.
pub(super) fn block_quote_content(line: SourceLine<'_>) -> Option<&str> {
    let text = line.text();
    let indentation = text
        .chars()
        .take_while(|character| *character == ' ')
        .count();
    if indentation > 3 {
        return None;
    }
    let mut content = text.get(indentation..)?;
    if !content.starts_with('>') {
        return None;
    }
    while let Some(remainder) = content.strip_prefix('>') {
        content = remainder.strip_prefix(' ').unwrap_or(remainder);
    }
    Some(content)
}

/// Reject an example that a block quote would hide from the document walk.
///
/// The walk only understands top-level blocks, so a fence or marker inside a
/// block quote would be skipped and never validated. Failing loudly keeps the
/// inventory complete and tells the author how to recover.
pub(super) fn reject_block_quote_example(
    document: &PublishedDocument<'_>,
    index: usize,
) -> Result<(), String> {
    let Some(content) = document
        .line(index)
        .map(SourceLine::new)
        .and_then(block_quote_content)
    else {
        return Ok(());
    };
    let is_marker = content.trim_start().starts_with("<!-- tested-example");
    if !is_marker && opening_fence(SourceLine::new(content)).is_none() {
        return Ok(());
    }
    Err(format!(
        concat!(
            "{}:{}: tested examples inside a block quote are not scanned; ",
            "move the fence out of the quote"
        ),
        document.path(),
        index + 1
    ))
}

/// Return the identifier from a `<!-- tested-example: id -->` comment.
///
/// The surrounding spacing is part of the grammar: a marker with extra spaces or
/// missing delimiters is reported rather than silently accepted.
pub(super) fn parse_marker(line: SourceLine<'_>) -> Option<String> {
    let comment = line.text().trim();
    let body = comment
        .strip_prefix("<!-- tested-example: ")?
        .strip_suffix(" -->")?;
    (!body.is_empty()).then(|| body.to_owned())
}

/// Return whether `identifier` follows the marker grammar.
///
/// Identifiers appear in test names and diagnostics, so they are restricted to
/// lower-case ASCII letters, digits, `-` and `_`, and must not start with `-`
/// or `_`.
pub(super) fn valid_identifier(identifier: &str) -> bool {
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
