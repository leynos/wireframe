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

/// Return how many spaces `text` carries before its first other character.
fn leading_spaces(text: &str) -> usize {
    text.chars()
        .take_while(|character| *character == ' ')
        .count()
}

/// Return the fence when `line` opens one.
///
/// Markdown lets a fence be indented by up to three spaces while still opening
/// a block, and requires at least three marker characters, so both rules are
/// enforced here rather than at each call site.
pub(super) fn opening_fence(line: SourceLine<'_>) -> Option<Fence> {
    let text = line.text();
    let indentation = leading_spaces(text);
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
    let indentation = leading_spaces(text);
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
    let indentation = leading_spaces(text);
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
    let indentation = leading_spaces(text);
    if indentation > 3 {
        return None;
    }
    let remainder = text.get(indentation..)?.strip_prefix('>')?;
    let mut content = remainder.strip_prefix(' ').unwrap_or(remainder);
    while let Some(nested) = nested_quote_content(content) {
        content = nested;
    }
    Some(content)
}

/// Return the content after a nested quote marker, when one follows `text`.
///
/// CommonMark allows up to three spaces before a block-quote marker, so those
/// spaces belong to the container rather than to the quoted text. Four or more
/// spaces mean the remainder is an indented code block inside the quote, whose
/// indentation the caller must still see.
fn nested_quote_content(text: &str) -> Option<&str> {
    let spaces = leading_spaces(text);
    if spaces > 3 {
        return None;
    }
    let remainder = text.get(spaces..)?.strip_prefix('>')?;
    Some(remainder.strip_prefix(' ').unwrap_or(remainder))
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
    if !is_marker_like(content) && opening_fence(SourceLine::new(content)).is_none() {
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

/// What remains of a line once its container prefixes are removed.
///
/// The loader walks top-level blocks only, so a fence reached through a list
/// marker or an indentation run would otherwise be skipped without comment.
#[derive(Debug, Clone, Copy)]
struct ContainerPrefix<'a> {
    /// Text after every quote marker, list marker and indentation run.
    content: &'a str,
    /// Whether a list marker or indentation run could hide an example.
    hides_example: bool,
}

/// Reject an example hidden behind a list marker or an indentation run.
///
/// A four-space indent is a list continuation only while an item is open, and
/// the walk deliberately does not track that state, so the rule fails closed:
/// any prefix that could be a container rejects example syntax unless the
/// indentation is small enough for a top-level fence. An indented code block
/// whose first content looks like a fence or marker is therefore rejected too;
/// the diagnostic names the line and tells the author how to recover.
pub(super) fn reject_list_container_example(
    document: &PublishedDocument<'_>,
    index: usize,
) -> Result<(), String> {
    let Some(line) = document.line(index) else {
        return Ok(());
    };
    let prefix = strip_container_prefix(line);
    if !prefix.hides_example {
        return Ok(());
    }
    if !is_marker_like(prefix.content) && opening_fence(SourceLine::new(prefix.content)).is_none() {
        return Ok(());
    }
    Err(container_diagnostic(document.path(), index + 1))
}

/// Build the diagnostic for an example hidden inside a list container.
fn container_diagnostic(path: &str, line_number: usize) -> String {
    format!(
        concat!(
            "{}:{}: tested examples inside a list item or indented block are not ",
            "scanned; move the fence out of the list and indent it by at most ",
            "three spaces"
        ),
        path, line_number
    )
}

/// Return whether `text` begins with a tested-example marker comment.
pub(super) fn is_marker_like(text: &str) -> bool {
    text.trim_start().starts_with("<!-- tested-example")
}

/// Strip interleaved quote markers, list markers and indentation from `line`.
///
/// CommonMark lets those containers nest in any order before a block, so each
/// pass measures the whitespace run, then removes one quote or list marker and
/// repeats. Whitespace after a quote marker is measured after its one optional
/// space, because the remaining indentation belongs to the quoted content.
fn strip_container_prefix(line: &str) -> ContainerPrefix<'_> {
    let mut remainder = line;
    let mut hides_example = false;
    loop {
        let (content, is_indented) = strip_indentation(remainder);
        hides_example |= is_indented;
        remainder = content;
        if let Some(content) = quote_content(remainder) {
            remainder = content;
            continue;
        }
        if let Some(content) = list_item_content(remainder) {
            hides_example = true;
            remainder = content;
            continue;
        }
        return ContainerPrefix {
            content: remainder,
            hides_example,
        };
    }
}

/// Remove a leading whitespace run and report whether it can start code.
///
/// A tab or four spaces put the remaining content in an indented code block,
/// out of reach of the top-level walk, even when no list item is open.
fn strip_indentation(text: &str) -> (&str, bool) {
    let mut consumed = 0;
    let mut spaces = 0;
    let mut has_tab = false;
    for character in text.chars() {
        match character {
            ' ' => spaces += 1,
            '\t' => has_tab = true,
            _ => break,
        }
        consumed += character.len_utf8();
    }
    (&text[consumed..], has_tab || spaces >= 4)
}

/// Remove one block-quote marker and its single optional following space.
///
/// Further spaces stay in the returned content so the caller measures them as
/// quoted indentation rather than as part of the marker.
fn quote_content(text: &str) -> Option<&str> {
    let remainder = text.strip_prefix('>')?;
    Some(remainder.strip_prefix(' ').unwrap_or(remainder))
}

/// Remove one list-item marker, leaving its required whitespace in place.
fn list_item_content(text: &str) -> Option<&str> { text.get(list_marker_length(text)?..) }

/// Return the byte length of a leading list-item marker.
///
/// CommonMark accepts `-`, `+` or `*`, or one to nine digits followed by `.`
/// or `)`, and requires a space, tab or end of line after the marker. That
/// terminator is what keeps `-->`, `---` and `***` from looking like list
/// markers.
fn list_marker_length(text: &str) -> Option<usize> {
    match text.chars().next()? {
        '-' | '+' | '*' => is_item_terminator(text.get(1..)?.chars().next()).then_some(1),
        '0'..='9' => {
            let digits = text.chars().take_while(char::is_ascii_digit).count();
            if digits > 9 {
                return None;
            }
            let delimiter = text.get(digits..)?.chars().next()?;
            if !matches!(delimiter, '.' | ')') {
                return None;
            }
            let follows = text.get(digits + 1..)?.chars().next();
            is_item_terminator(follows).then_some(digits + 1)
        }
        _ => None,
    }
}

/// Return whether `character` can terminate a list-item marker.
fn is_item_terminator(character: Option<char>) -> bool {
    matches!(character, None | Some(' ' | '\t'))
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
